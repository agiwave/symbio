//! 实体 → VDFS 适配器 —— 「任何 [`EntityProvider`] 自动成为一个挂载点」
//!
//! ## 为什么需要它
//!
//! 实体机制（`entities/*`）与 VDFS 是**同一批资源**的两套寻址方式：前者按
//! 「类型（kind）」组织，后者按「路径与访问位」组织。若每种资源各写一份 VDFS
//! provider，同一份读写逻辑就会分叉成两套——这正是迁移要消除的东西。
//!
//! 本适配器把 [`EntityProvider`] 的既有能力**原样**接到 [`VdfsProvider`] 上：
//!
//! | VDFS | 复用 |
//! |---|---|
//! | 挂载点的标签 / 顺序 / 可写性 / 可新建类型 | `provider_registry()` 的 `label` / `order` / `supports_upload` |
//! | `list` | [`EntityProvider::list_items`] |
//! | `stat` / 节点呈现 | [`EntityProvider::summarize`] + [`EntityProvider::detail_definition`] |
//! | `read` | 摘要 `extra.config`（完整配置，与实体详情页**同源**） |
//! | `write` | [`entity_write`]（`entities/upload` 的 manifest 分支同一实现） |
//! | `delete` | [`entity_delete`]（`entities/delete` 同一实现） |
//! | `watch` | provider 侧变更广播（写 / 删时触发，**非轮询**） |
//!
//! 因此新增一种实体类型时，VDFS 侧**零改动**——它自动获得一个挂载点。
//!
//! ## 节点 `ext` 的选取
//!
//! `detail_definition` 有定义 → `ext = form`（前端通用表单渲染器解析 `schema`）；
//! 无定义 → `ext = <kind>`（前端回退机制级只读视图）。`ext` 是详情渲染器的
//! **唯一分发键**，适配器不参与渲染决策。
//!
//! [`entity_write`]: crate::symbio_core::entities::entity_write
//! [`entity_delete`]: crate::symbio_core::entities::entity_delete

use super::host::{from_plugin_error, host_ctx};
use crate::symbio_core::entities::{
    self, ContainerKindSpec, EntityProvider, EntityProviderInfo, EntitySummary,
};
use crate::symbio_core::vdfs_provider::{
    VdfsAccess, VdfsChange, VdfsChangeSink, VdfsContent, VdfsContext, VdfsError, VdfsNewType,
    VdfsNode, VdfsProvider, VdfsResult, VdfsWriteResponse, VFDS_CHANGE_CREATED,
    VFDS_CHANGE_DELETED, VFDS_CHANGE_UPDATED, VFDS_EXT_FORM,
};
use crate::symbio_core::InvokeRequest;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};

/// 挂载点内的路径解析结果（容器子实体寻址，与会话的 S6 寻址同构）
///
///   ``            → 挂载根（条目清单）
///   `<id>`         → 条目；有容器子实体时是**目录**（子类别清单）
///   `<id>/<seg>`   → 某子类别的条目清单（`seg` = 子类别标签）
///   `<id>/<seg>/<item>` → 单个子实体（`item` 可为相对路径，如
///                        `skills/x/SKILL.md`——bundle 内文件的 id 含 `/`）
enum AdapterPath<'a> {
    Root,
    Item(&'a str),
    Section {
        id: &'a str,
        seg: &'a str,
    },
    SubItem {
        id: &'a str,
        seg: &'a str,
        item: &'a str,
    },
}

/// 解析挂载点内相对路径（至多切三段：条目 / 子类别 / 子实体）
fn parse_adapter_path(path: &str) -> AdapterPath<'_> {
    let p = path.trim_matches('/');
    if p.is_empty() {
        return AdapterPath::Root;
    }
    match p.split_once('/') {
        None => AdapterPath::Item(p),
        Some((id, rest)) => match rest.split_once('/') {
            None => AdapterPath::Section { id, seg: rest },
            Some((seg, item)) => AdapterPath::SubItem { id, seg, item },
        },
    }
}

/// 把 [`EntityProvider`] 适配为 VDFS 挂载点（详见模块文档）
pub struct EntityVdfsAdapter {
    /// 实体类型（同时是挂载名与节点 `kind`）
    kind: &'static str,
    /// 被适配的实体 provider（与实体机制**同一个实例**，状态共享）
    provider: Arc<dyn EntityProvider>,
    /// 变更广播源（`write` / `delete` 时投递；`watch` 订阅后转发给 sink）
    changes: broadcast::Sender<VdfsChange>,
    /// 已订阅路径 → 转发任务（`unwatch` 时取消）
    watch_tasks: Mutex<HashMap<String, tokio::task::JoinHandle<()>>>,
}

impl EntityVdfsAdapter {
    pub fn new(kind: &'static str, provider: Arc<dyn EntityProvider>) -> Self {
        let (changes, _) = broadcast::channel(64);
        Self {
            kind,
            provider,
            changes,
            watch_tasks: Mutex::new(HashMap::new()),
        }
    }

    /// 注册表条目——挂载点的标签 / 顺序 / 能力**单一真相源**
    fn info(&self) -> Option<&'static EntityProviderInfo> {
        entities::provider_registry()
            .iter()
            .find(|p| p.kind == self.kind)
    }

    fn label_of(&self) -> &'static str {
        self.info().map(|i| i.label).unwrap_or(self.kind)
    }

    /// 该类型在 VDFS 侧是否可创建 / 可删除。
    ///
    /// 两个条件缺一不可：注册表声明 `supports_upload`（对外承诺），且 provider
    /// 真的能走 `entity_write`（有 `category` + `manifest_file`，即 EntityStore 型）。
    /// 后者把 bundle 型（agent：zip 上传、目录自管）自动降级为只读，避免「声明了
    /// 新建但落盘必失败」的不一致。
    fn writable(&self) -> bool {
        self.info().is_some_and(|i| i.supports_upload)
            && self.provider.category().is_some()
            && self.provider.manifest_file().is_some()
    }

    fn node_access(&self) -> VdfsAccess {
        if self.writable() {
            VdfsAccess::READ_WRITE
        } else {
            VdfsAccess::READ
        }
    }

    /// 路径末段 → 实体 id（去掉 `.<kind>` 呈现扩展名；无扩展名时原样）
    fn id_of(&self, path: &str) -> String {
        let base = path.rsplit('/').next().unwrap_or(path);
        base.strip_suffix(&format!(".{}", self.kind))
            .unwrap_or(base)
            .to_string()
    }

    /// 单个实体的摘要：EntityStore 型直读主文件；其余（如 agent bundle）从清单里找
    async fn summary_of(
        &self,
        host: &Arc<dyn InvokeRequest>,
        id: &str,
    ) -> VdfsResult<EntitySummary> {
        match (self.provider.category(), self.provider.manifest_file()) {
            (Some(category), Some(manifest)) => {
                let store = entities::storage_service(host).map_err(from_plugin_error)?;
                let content = store
                    .entity_store()
                    .read_entity(category, id, manifest)
                    .await
                    .map_err(|e| VdfsError::not_found(format!("未找到实体「{id}」：{e}")))?;
                Ok(self.provider.summarize(host, id, Some(&content)).await)
            }
            _ => {
                let items = self
                    .provider
                    .list_items(host)
                    .await
                    .map_err(from_plugin_error)?;
                items
                    .into_iter()
                    .find(|i| i.id == id)
                    .ok_or_else(|| VdfsError::not_found(format!("未找到实体「{id}」")))
            }
        }
    }

    /// 摘要 → VDFS 节点（详情渲染器由 `ext` 唯一决定）
    async fn node_of(&self, host: &Arc<dyn InvokeRequest>, item: &EntitySummary) -> VdfsNode {
        let def = self.provider.detail_definition(host, &item.id).await;
        // 条目**有容器子实体且无详情定义**（如 agent bundle）⇒ 视为目录：
        // 点进去浏览内部（提示词 / 技能 / MCP），而不是展开一个没有内容的详情面板。
        if def.is_none() && !self.container_kinds().is_empty() {
            let mut n = VdfsNode::dir(&item.id, item.name.clone(), VdfsAccess::LIST);
            n.kind = self.kind.to_string();
            n.status = item.status.clone();
            n.description = item.description.clone().or_else(|| item.summary.clone());
            return n;
        }
        let mut n = VdfsNode::file(&item.id, item.name.clone(), self.node_access());
        n.kind = self.kind.to_string();
        match def {
            Some(def) => {
                // 定义驱动表单：schema 随列表下发（前端选中即渲染，无需二次请求）
                n.ext = Some(VFDS_EXT_FORM.to_string());
                n.schema = serde_json::to_value(&def).ok();
            }
            None => {
                // 无定义：前端按 kind 回退机制级只读视图
                n.ext = Some(self.kind.to_string());
            }
        }
        n.status = item.status.clone();
        n.description = item.description.clone().or_else(|| item.summary.clone());
        n.updated_at = item.updated_at;
        n
    }

    /// 本类型的容器子实体声明（空 = 条目不是容器）
    fn container_kinds(&self) -> &'static [ContainerKindSpec] {
        entities::container_kinds_for(self.kind)
    }

    /// 路径段（子类别标签）→ 子实体声明。
    ///
    /// 以**标签**而非 kind 作路径段——与会话内部（S6 的「子会话」/「工作目录」）
    /// 同一口径：路径是给人看的，kind 是实现标识。
    fn section_of(&self, seg: &str) -> Option<&'static ContainerKindSpec> {
        self.container_kinds().iter().find(|k| k.label == seg)
    }

    /// 子类别目录节点。可新建的类型由 `path_hint` 决定——非空即可创建，
    /// 扩展名取自路径模板（`prompts/<name>.md` → `md`）。
    fn section_node(&self, spec: &ContainerKindSpec) -> VdfsNode {
        let mut n = VdfsNode::dir(spec.label, spec.label, VdfsAccess::LIST);
        n.kind = spec.kind.to_string();
        n.description = Some(spec.description.to_string());
        if let Some(ext) = spec.path_hint.rsplit_once('.').map(|(_, e)| e) {
            if !ext.is_empty() {
                n.new_types = vec![VdfsNewType::new(ext, spec.label)
                    .with_description(format!("新建{}（{}）", spec.label, spec.path_hint))];
            }
        }
        n
    }

    /// 子实体摘要 → VDFS 节点（文件；`ext` 由文件名推导，渲染器据此分发）。
    ///
    /// `name` 用**相对路径**（唯一，可含 `/`），`title` 用 basename（可读）。
    fn container_node(&self, spec: &ContainerKindSpec, it: &EntitySummary) -> VdfsNode {
        let mut n = VdfsNode::file(it.id.clone(), it.name.clone(), VdfsAccess::READ_WRITE);
        n.kind = spec.kind.to_string();
        n.size = it.extra.get("size").and_then(|v| v.as_u64());
        n.description = it.description.clone();
        n
    }

    /// 广播一次变更（`watch` 的 sink 由此收到，前端因此无需轮询）
    fn notify(&self, path: &str, change: &str) {
        // 无订阅者时 send 返回 Err，属正常（不是错误路径）
        drop(self.changes.send(VdfsChange::new(path, change)));
    }
}

#[async_trait]
impl VdfsProvider for EntityVdfsAdapter {
    fn label(&self) -> Option<&str> {
        Some(self.label_of())
    }

    fn description(&self) -> Option<&str> {
        Some("统一实体机制中的一类资源；读写删与实体管理页共用同一实现。")
    }

    fn order(&self) -> i32 {
        self.info().map(|i| i.order).unwrap_or(100)
    }

    fn icon(&self) -> Option<&str> {
        Some(self.kind)
    }

    fn root_access(&self) -> VdfsAccess {
        self.node_access()
    }

    fn root_new_types(&self) -> Vec<VdfsNewType> {
        if !self.writable() {
            return Vec::new();
        }
        vec![
            VdfsNewType::new(self.kind, self.label_of()).with_description(format!(
                "新建{}（先落一份默认配置，随后在详情里完善）",
                self.label_of()
            )),
        ]
    }

    async fn list(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<Vec<VdfsNode>> {
        let host = host_ctx(ctx)?;
        match parse_adapter_path(path) {
            AdapterPath::Root => {
                let items = self
                    .provider
                    .list_items(&host)
                    .await
                    .map_err(from_plugin_error)?;
                let mut out = Vec::with_capacity(items.len());
                for item in items.iter() {
                    out.push(self.node_of(&host, item).await);
                }
                Ok(out)
            }
            // 条目即容器：子类别清单（如 agent bundle 的 提示词 / 技能 / MCP）
            AdapterPath::Item(id) => {
                if self.container_kinds().is_empty() {
                    return Err(VdfsError::not_found(format!(
                        "{}是叶子资源，没有子项：{path}",
                        self.label_of()
                    )));
                }
                // 存在性校验：不存在的条目应报 NotFound 而非给出空类别清单
                self.summary_of(&host, id).await?;
                Ok(self
                    .container_kinds()
                    .iter()
                    .map(|k| self.section_node(k))
                    .collect())
            }
            AdapterPath::Section { id, seg } => {
                let spec = self
                    .section_of(seg)
                    .ok_or_else(|| VdfsError::not_found(format!("不存在子类别：{path}")))?;
                let items = self
                    .provider
                    .list_container_items(&host, Some(spec.kind), id, None)
                    .await
                    .map_err(from_plugin_error)?;
                Ok(items
                    .iter()
                    .map(|it| self.container_node(spec, it))
                    .collect())
            }
            AdapterPath::SubItem { .. } => Err(VdfsError::not_found(format!(
                "子实体是叶子节点，没有子项：{path}"
            ))),
        }
    }

    async fn stat(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<VdfsNode> {
        let host = host_ctx(ctx)?;
        match parse_adapter_path(path) {
            AdapterPath::Root => {
                // 自身根：名字留空——provider 不知道自己的挂载名，由使用方回填
                Ok(VdfsNode::dir("", self.label_of(), self.root_access()))
            }
            AdapterPath::Item(id) => {
                // 容器条目按**目录视图**回答：`stat` 结果被分发层用作「当前目录
                // 节点」，其访问位决定是否给出新建入口。
                if !self.container_kinds().is_empty() {
                    let item = self.summary_of(&host, id).await?;
                    let mut n = VdfsNode::dir(id, item.name.clone(), VdfsAccess::LIST);
                    n.kind = self.kind.to_string();
                    return Ok(n);
                }
                let item = self.summary_of(&host, &self.id_of(path)).await?;
                Ok(self.node_of(&host, &item).await)
            }
            AdapterPath::Section { seg, .. } => self
                .section_of(seg)
                .map(|spec| self.section_node(spec))
                .ok_or_else(|| VdfsError::not_found(format!("不存在子类别：{path}"))),
            AdapterPath::SubItem { id, seg, item } => {
                let spec = self
                    .section_of(seg)
                    .ok_or_else(|| VdfsError::not_found(format!("不存在子类别：{path}")))?;
                let it = self
                    .provider
                    .get_container_item(&host, item, id)
                    .await
                    .map_err(from_plugin_error)?;
                Ok(self.container_node(spec, &it))
            }
        }
    }

    /// 读取完整配置（表单字段值对象）——与实体详情页的预填数据**同源**
    async fn read(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<VdfsContent> {
        let host = host_ctx(ctx)?;
        match parse_adapter_path(path) {
            // 子实体：文件内容由 provider 随摘要下发（bundle 沙箱内读取）
            AdapterPath::SubItem { id, seg, item } => {
                let _ = self
                    .section_of(seg)
                    .ok_or_else(|| VdfsError::not_found(format!("不存在子类别：{path}")))?;
                let it = self
                    .provider
                    .get_container_item(&host, item, id)
                    .await
                    .map_err(from_plugin_error)?;
                let text = it
                    .extra
                    .get("content")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        VdfsError::invalid(format!("该子实体没有可读的文本内容：{path}"))
                    })?
                    .to_string();
                Ok(VdfsContent::text("", text))
            }
            AdapterPath::Root | AdapterPath::Section { .. } => Err(VdfsError::invalid(format!(
                "该路径是目录，不可读取内容：{path}"
            ))),
            AdapterPath::Item(_) => {
                if !self.container_kinds().is_empty() {
                    return Err(VdfsError::invalid(format!(
                        "该路径是目录，不可读取内容：{path}"
                    )));
                }
                let item = self.summary_of(&host, &self.id_of(path)).await?;
                let value = item
                    .extra
                    .get("config")
                    .cloned()
                    .unwrap_or_else(|| item.extra.clone());
                let text = serde_json::to_string_pretty(&value)
                    .map_err(|e| VdfsError::internal(format!("配置序列化失败：{e}")))?;
                Ok(VdfsContent::text("", text).with_mime("application/json"))
            }
        }
    }

    /// 写入：`create` → 用插件声明的**最小 manifest** 落一份默认配置；
    /// 否则 → 以内容为完整 manifest 覆盖（走 `entities/upload` 同一实现）
    async fn write(
        &self,
        ctx: &VdfsContext,
        path: &str,
        content: &VdfsContent,
    ) -> VdfsResult<VdfsWriteResponse> {
        let host = host_ctx(ctx)?;
        // 子实体写回（bundle 沙箱内写入）；新建时按 `path_hint` 落位
        if let AdapterPath::SubItem { id, seg, item } = parse_adapter_path(path) {
            let spec = self
                .section_of(seg)
                .ok_or_else(|| VdfsError::not_found(format!("不存在子类别：{path}")))?;
            let target = if content.create {
                if spec.path_hint.is_empty() {
                    return Err(VdfsError::Forbidden(format!(
                        "{}不支持新建（子类别未声明路径模板）",
                        spec.label
                    )));
                }
                // 路径模板是唯一真相源：`prompts/<name>.md` + 文件名 → 实际路径
                let stem = item.rsplit('/').next().unwrap_or(item);
                let stem = stem.rsplit_once('.').map(|(s, _)| s).unwrap_or(stem);
                spec.path_hint.replace("<name>", stem)
            } else {
                item.to_string()
            };
            let text = content.text.as_deref().unwrap_or("");
            let text = if content.create && text.trim().is_empty() {
                spec.default_content
            } else {
                text
            };
            let resp = self
                .provider
                .put_container_item(&host, &target, text, id)
                .await
                .map_err(from_plugin_error)?;
            self.notify(
                path,
                if resp.created {
                    VFDS_CHANGE_CREATED
                } else {
                    VFDS_CHANGE_UPDATED
                },
            );
            return Ok(VdfsWriteResponse {
                path: path.to_string(),
                created: resp.created,
                etag: None,
            });
        }
        let id = self.id_of(path);
        let text = content.text.as_deref().unwrap_or("");
        let value: serde_json::Value = if text.trim().is_empty() {
            serde_json::json!({})
        } else {
            serde_json::from_str(text)
                .map_err(|e| VdfsError::invalid(format!("实体写入需要合法 JSON：{e}")))?
        };

        let manifest = if content.create {
            // 新建：使用方只给了路径名（= 标题），最小配置由插件自持
            self.provider.new_entity_manifest(&id, &id)
        } else {
            if !value.is_object() {
                return Err(VdfsError::invalid("实体写入需要 JSON 对象"));
            }
            value
        };

        let resp = entities::entity_write(&*self.provider, &host, &id, &manifest)
            .await
            .map_err(from_plugin_error)?;
        self.notify(
            &id,
            if resp.created {
                VFDS_CHANGE_CREATED
            } else {
                VFDS_CHANGE_UPDATED
            },
        );
        Ok(VdfsWriteResponse {
            path: id,
            created: resp.created,
            etag: None,
        })
    }

    async fn delete(&self, ctx: &VdfsContext, path: &str, _recursive: bool) -> VdfsResult<()> {
        if path.is_empty() {
            return Err(VdfsError::Forbidden(format!(
                "{}挂载根不可删除",
                self.label_of()
            )));
        }
        let host = host_ctx(ctx)?;
        // 子实体删除（bundle 沙箱内删除）
        if let AdapterPath::SubItem { id, seg, item } = parse_adapter_path(path) {
            let _ = self
                .section_of(seg)
                .ok_or_else(|| VdfsError::not_found(format!("不存在子类别：{path}")))?;
            self.provider
                .delete_container_item(&host, item, id)
                .await
                .map_err(from_plugin_error)?;
            self.notify(path, VFDS_CHANGE_DELETED);
            return Ok(());
        }
        let id = self.id_of(path);
        // 存在性校验：删除不存在的实体应报 NotFound 而非静默成功
        self.summary_of(&host, &id).await?;
        entities::entity_delete(&*self.provider, &host, &id)
            .await
            .map_err(from_plugin_error)?;
        self.notify(&id, VFDS_CHANGE_DELETED);
        Ok(())
    }

    /// 订阅：把 provider 侧变更广播转发到 sink（`unwatch` 时取消任务）。
    /// provider 是**变更源的持有者**，因此不需要轮询。
    async fn watch(&self, _ctx: &VdfsContext, path: &str, sink: VdfsChangeSink) -> VdfsResult<()> {
        let mut rx = self.changes.subscribe();
        let handle = tokio::spawn(async move {
            while let Ok(change) = rx.recv().await {
                sink(change);
            }
        });
        // 同路径重复订阅：覆盖并取消旧任务（机制保证 watch/unwatch 严格配对，
        // 此处仅作防御）
        let old = self
            .watch_tasks
            .lock()
            .await
            .insert(path.to_string(), handle);
        if let Some(old) = old {
            old.abort();
        }
        Ok(())
    }

    async fn unwatch(&self, _ctx: &VdfsContext, path: &str) -> VdfsResult<()> {
        let handle = self.watch_tasks.lock().await.remove(path);
        if let Some(handle) = handle {
            handle.abort();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbio_core::entities::{ENTITY_AGENT, ENTITY_MODEL, ENTITY_SESSION};

    /// 标签 / 顺序来自注册表（单一真相源），不硬编码
    #[test]
    fn label_and_order_come_from_registry() {
        struct Stub;
        #[async_trait]
        impl EntityProvider for Stub {
            fn kind(&self) -> &'static str {
                ENTITY_MODEL
            }
        }
        let a = EntityVdfsAdapter::new(ENTITY_MODEL, Arc::new(Stub));
        let info = entities::provider_registry()
            .iter()
            .find(|p| p.kind == ENTITY_MODEL)
            .expect("注册表应包含 model");
        assert_eq!(a.label(), Some(info.label));
        assert_eq!(a.order(), info.order);
        assert_eq!(a.icon(), Some(ENTITY_MODEL));
    }

    /// 路径末段 → 实体 id：去 `.<kind>` 扩展名；无扩展名原样
    #[test]
    fn id_of_strips_kind_suffix() {
        struct Stub;
        #[async_trait]
        impl EntityProvider for Stub {
            fn kind(&self) -> &'static str {
                ENTITY_MODEL
            }
        }
        let a = EntityVdfsAdapter::new(ENTITY_MODEL, Arc::new(Stub));
        assert_eq!(a.id_of("gpt-4"), "gpt-4");
        assert_eq!(a.id_of("gpt-4.model"), "gpt-4");
        assert_eq!(a.id_of("a/b/gpt-4.model"), "gpt-4");
    }

    /// 可写类型（注册表 `supports_upload`）声明恰好一项新建类型、访问位可写
    #[test]
    fn writable_kind_declares_one_new_type() {
        struct Stub;
        #[async_trait]
        impl EntityProvider for Stub {
            fn kind(&self) -> &'static str {
                ENTITY_MODEL
            }
            // 可写性双重判定的另一半：EntityStore 归属（分类 + 主清单文件）。
            // 真实 provider（model / skill / mcp）均返回 Some，故桩件也如实给出。
            fn category(&self) -> Option<&'static str> {
                Some(crate::symbio_core::providers::categories::MODEL)
            }
            fn manifest_file(&self) -> Option<&'static str> {
                Some(crate::symbio_core::providers::manifests::PROVIDER)
            }
        }
        let a = EntityVdfsAdapter::new(ENTITY_MODEL, Arc::new(Stub));
        assert!(a.writable(), "model 在注册表中 supports_upload = true");
        let types = a.root_new_types();
        assert_eq!(types.len(), 1);
        assert_eq!(types[0].ext, ENTITY_MODEL);
        assert!(a.root_access().write, "可写类型挂载根应含写位");
    }

    /// 只读类型（注册表 `supports_upload = false`）不声明新建类型、访问位只读
    #[test]
    fn read_only_kind_declares_no_new_type() {
        struct Stub;
        #[async_trait]
        impl EntityProvider for Stub {
            fn kind(&self) -> &'static str {
                ENTITY_SESSION
            }
        }
        let a = EntityVdfsAdapter::new(ENTITY_SESSION, Arc::new(Stub));
        assert!(!a.writable(), "session 在注册表中 supports_upload = false");
        assert!(a.root_new_types().is_empty());
        assert!(!a.root_access().write, "只读类型挂载根不应含写位");
        assert!(a.root_access().read);
    }

    /// bundle 型（注册表声明 `supports_upload = true`，但目录自管、不走 EntityStore）
    /// 降级为只读——避免「声明了新建但落盘必失败」的不一致
    #[test]
    fn bundle_like_kind_degrades_to_read_only() {
        struct Stub;
        #[async_trait]
        impl EntityProvider for Stub {
            fn kind(&self) -> &'static str {
                ENTITY_AGENT
            }
            // category / manifest_file 保持默认 None（bundle 由 BundleStore 自管）
        }
        let a = EntityVdfsAdapter::new(ENTITY_AGENT, Arc::new(Stub));
        let info = entities::provider_registry()
            .iter()
            .find(|p| p.kind == ENTITY_AGENT)
            .expect("注册表应包含 agent");
        assert!(info.supports_upload, "前提：注册表声明 agent 可上传");
        assert!(!a.writable(), "但目录自管 → VDFS 侧降级为只读");
        assert!(a.root_new_types().is_empty());
    }

    /// 容器寻址解析：`<id>` / `<id>/<seg>` / `<id>/<seg>/<item>`
    /// （bundle 内文件的 id 含 `/`，故 `item` 取剩余全部）
    #[test]
    fn container_path_parsing() {
        assert!(matches!(parse_adapter_path(""), AdapterPath::Root));
        assert!(matches!(parse_adapter_path("b1"), AdapterPath::Item("b1")));
        assert!(matches!(
            parse_adapter_path("b1/提示词"),
            AdapterPath::Section {
                id: "b1",
                seg: "提示词"
            }
        ));
        assert!(matches!(
            parse_adapter_path("b1/技能/x/SKILL.md"),
            AdapterPath::SubItem {
                id: "b1",
                seg: "技能",
                item: "x/SKILL.md"
            }
        ));
    }

    /// 子类别以**标签**寻址；可新建性来自 `path_hint`（非空 ⇒ 声明新建类型，
    /// 扩展名取自路径模板 `prompts/<name>.md` → `md`）
    #[test]
    fn sections_are_addressed_by_label() {
        struct Stub;
        #[async_trait]
        impl EntityProvider for Stub {
            fn kind(&self) -> &'static str {
                ENTITY_AGENT
            }
        }
        let a = EntityVdfsAdapter::new(ENTITY_AGENT, Arc::new(Stub));
        assert_eq!(a.container_kinds().len(), 3, "agent 有三类子实体");
        let seg = a.section_of("提示词").expect("标签命中");
        assert_eq!(seg.kind, "prompt", "标签 → 实现 kind");
        let node = a.section_node(seg);
        assert!(node.is_dir());
        assert_eq!(node.new_types.len(), 1, "path_hint 非空 ⇒ 可新建");
        assert_eq!(node.new_types[0].ext, "md");
        assert!(a.section_of("不存在").is_none());
    }

    /// 无容器子实体的类型（model）：子类别为空，非根路径仍是叶子语义
    #[test]
    fn non_container_kind_has_no_sections() {
        struct Stub;
        #[async_trait]
        impl EntityProvider for Stub {
            fn kind(&self) -> &'static str {
                ENTITY_MODEL
            }
        }
        let a = EntityVdfsAdapter::new(ENTITY_MODEL, Arc::new(Stub));
        assert!(a.container_kinds().is_empty());
        assert!(a.section_of("提示词").is_none());
    }

    /// 子实体节点：可读写文件，`name` = 相对路径（唯一，可含 `/`）、
    /// `title` = basename（可读）
    #[test]
    fn container_node_is_editable_file() {
        struct Stub;
        #[async_trait]
        impl EntityProvider for Stub {
            fn kind(&self) -> &'static str {
                ENTITY_AGENT
            }
        }
        let spec = entities::container_kinds_for(ENTITY_AGENT)
            .first()
            .expect("agent 有子类别");
        let mut it = EntitySummary::new(spec.kind, "prompts/foo.md", "foo.md");
        it.extra = serde_json::json!({ "size": 12 });
        let a = EntityVdfsAdapter::new(ENTITY_AGENT, Arc::new(Stub));
        let n = a.container_node(spec, &it);
        assert_eq!(n.name, "prompts/foo.md");
        assert_eq!(n.title, "foo.md");
        assert_eq!(n.access.flags(), "rw", "bundle 内文件可编辑");
        assert_eq!(n.size, Some(12));
        assert!(!n.is_dir());
    }
}
