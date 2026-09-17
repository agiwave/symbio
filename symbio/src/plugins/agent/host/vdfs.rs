//! VDFS 挂载点（`.vdfs/agent`）—— 本插件**直接实现 `VdfsProvider`**。
//!
//! ## 与其它资源插件的分工差异
//!
//! Agent 是**目录自管型**资源：它的落盘由 [`BundleStore`] 负责（工作区级 +
//! 全局级双层、zip-slip 防护、版本硬门槛），**不经 `vdfs_service`**——
//! `vdfs_service` 的三种拓扑都是「`<本插件目录>/<id>/…`」这一固定落位，
//! 而 Agent 要同时看见工作目录与系统目录两层，寻址规则本身是 Agent 语义的一部分。
//!
//! 相同的是**广播**：落盘后一律走 `vdfs::notify_change`，与 `vdfs_service` 三个
//! 实现投的是同一条频道，订阅方无需区分资源住在哪儿。
//!
//! ## 呈现：整棵目录树，不再分类
//!
//! v1 在这里把 Agent 内部拆成「提示词 / 技能 / MCP」三类容器，每类有自己的路径
//! 白名单、新建模板与默认正文——那是宿主在替能力目录解释语义，改一处要改三处，
//! 且与宿主的技能系统 / MCP 客户端天然不同步。
//!
//! v2 直接把 Agent 目录**整棵呈现**：`<id>/<任意相对路径>`。唯一的例外是根下的
//! `AGENTS.md`（§6 人格与记忆），它走内核的 `MemoryFile::node`（带容量闸门），
//! 与工作区记忆同一口径。
//!
//! ## 挂载根下还有一个文件：系统智能体自身的指令
//!
//! `.vdfs/agent/AGENTS.md` 不是某个 bundle 的条目，而是**本应用（系统智能体）自身**的
//! 指令文件（`{homedir}/AGENTS.md`，见 [`super::instruction`]）。它排在列表**最前**：
//! 剩下的都是「装进来的智能体」，而它不是——先摆出来才不会被当成某个包看走眼。
//! 与 bundle 无关的那三个字母 `AGENTS.md` 因此是挂载根下的**保留名**；bundle id 的
//! 字符集要求首字符是小写字母或数字，不可能与之相撞（§5.1）。
//!
//! 外部访问一律走 `.vdfs/agent/…`。

use super::instruction;
use super::memory;
use super::plugin::AgentPlugin;
use super::store::{BundleRecord, BundleStore, FileEntry};
use crate::providers::vdfs_service;
use crate::symbio_core::vdfs::{host_ctx, notify_change, unwatch_changes, watch_changes};
use crate::symbio_core::vdfs_provider::{
    VdfsAccess, VdfsActionResult, VdfsChangeSink, VdfsContent, VdfsContext, VdfsError, VdfsNewType,
    VdfsNode, VdfsProvider, VdfsResult, VdfsWriteResponse, VDFS_ACTION_EXPORT, VDFS_CHANGE_CREATED,
    VDFS_CHANGE_DELETED, VDFS_CHANGE_UPDATED, VDFS_EXT_FORM, VDFS_EXT_ZIP, VDFS_NEW_SOURCE_FILE,
};
use crate::symbio_core::{
    dir_from_ctx, InvokeRequest, InvokeRequestExt, AGENTS_FILE, PLUGIN_AGENT, PLUGIN_FILE,
};
use async_trait::async_trait;
use std::sync::Arc;

const LABEL: &str = "智能体";

// ==================== 路径解析 ====================

/// 挂载点内相对路径：`<id>` 之后是 Agent 目录内的**任意**相对路径
#[derive(Debug)]
enum RelPath<'a> {
    Root,
    /// 系统智能体自身的指令：挂载根下的 `AGENTS.md`（与 bundle 无关，见模块文档）
    Instruction,
    /// Agent 本身（`<id>`）
    Agent {
        id: &'a str,
    },
    /// 人格与记忆：`<id>/AGENTS.md`（§6，走内核记忆门面，不是普通文件）
    Memory {
        id: &'a str,
    },
    /// Agent 目录内的文件 / 子目录；`rel` 可含 `/`
    File {
        id: &'a str,
        rel: &'a str,
    },
}

fn parse_rel_path(path: &str) -> RelPath<'_> {
    let p = path.trim_matches('/');
    if p.is_empty() {
        return RelPath::Root;
    }
    // 保留名：挂载根下的 `AGENTS.md` 是**本应用自身**的指令，不是名为它的 bundle
    // （bundle id 首字符必须是小写字母或数字，两者不可能相撞）
    if p == AGENTS_FILE {
        return RelPath::Instruction;
    }
    match p.split_once('/') {
        None => RelPath::Agent { id: p },
        // 第二段是记忆文件名 → 记忆，而不是「名为 AGENTS.md 的普通文件」
        Some((id, rest)) if rest == AGENTS_FILE => RelPath::Memory { id },
        Some((id, rest)) => RelPath::File { id, rel: rest },
    }
}

/// 路径末段 → 条目 id（去掉 `.agent` 呈现扩展名）
fn id_of(path: &str) -> String {
    vdfs_service::entry::id_of(path, PLUGIN_AGENT)
}

// ==================== 节点合成 ====================

/// Agent 概览（`read` 与详情表单共用的信息载荷）
fn bundle_info(r: &BundleRecord, store: &BundleStore) -> serde_json::Value {
    // 「装了哪些能力」由**目录**回答（§4.1：目录里有就表示已安装），而不是
    // 宿主按类别点数——那是 v1 的做法，与真实的能力来源两份真相。
    let capabilities: Vec<String> = store
        .list_files(&r.manifest.id, "")
        .unwrap_or_default()
        .into_iter()
        .filter(|e| e.is_dir)
        .map(|e| e.path)
        .collect();
    serde_json::json!({
        // config_type = "bundle"：项级图标 / 详情分发的顶层键（VdfsNode attributes flatten）
        "config_type": "bundle",
        "version": r.manifest.version,
        "spec": r.manifest.spec,
        "requires_spec": r.manifest.requires.spec,
        "scope": r.source.as_str(),
        "dir": r.dir.to_string_lossy(),
        "capabilities": capabilities.join("、"),
    })
}

/// Agent 记录 → VDFS 节点（只读概览表单：`ext = form` + 定义随 `schema` 下发）
///
/// Agent 同时是容器（内部可浏览整棵目录），但**有详情定义**，故呈现为表单文件
/// ——「浏览内部」走 `enter(<id>/…)` 的目录语义，与详情页互不影响。
fn bundle_node(r: &BundleRecord, store: &BundleStore) -> VdfsNode {
    let id = r.manifest.id.clone();
    let title = if r.manifest.name.is_empty() {
        id.clone()
    } else {
        r.manifest.name.clone()
    };
    let mut n = VdfsNode::file(id, title, VdfsAccess::READ);
    n.kind = PLUGIN_AGENT.to_string();
    n.ext = Some(VDFS_EXT_FORM.to_string());
    n.schema = serde_json::to_value(super::detail::agent_detail_definition()).ok();
    if !r.manifest.description.is_empty() {
        n.description = Some(r.manifest.description.clone());
    }
    n.attributes = bundle_info(r, store)
        .as_object()
        .cloned()
        .unwrap_or_default();
    n
}

/// Agent 目录内的一条条目 → VDFS 节点
///
/// `id` 是 Agent id；`e.path` 是目录内相对路径，二者拼成节点地址。
fn entry_node(id: &str, e: &FileEntry) -> VdfsNode {
    let name = e.path.rsplit('/').next().unwrap_or(&e.path).to_string();
    let addr = format!("{id}/{}", e.path);
    let mut n = if e.is_dir {
        VdfsNode::dir(addr, name, VdfsAccess::LIST_TRAVERSE)
    } else {
        VdfsNode::file(addr, name, VdfsAccess::READ_WRITE)
    };
    if !e.is_dir {
        n.ext = e.path.rsplit_once('.').map(|(_, ext)| ext.to_string());
        n.size = Some(e.size);
    }
    n
}

impl AgentPlugin {
    /// 依请求上下文构造 BundleStore（每次请求独立，与 route 入口一致）
    ///
    /// bundle 根 = **本插件自己的目录**，取自父插件经 `PLUGIN_DIR` 传下的目录
    /// （`dir_from_ctx`；缺省退回常规落位）——与 `AgentPlugin::build` 同源，
    /// 这里不另拼一份 `<homedir>/…/agent`。
    fn store_of(ctx: &Arc<dyn InvokeRequest>) -> BundleStore {
        let dir = dir_from_ctx(&**ctx, PLUGIN_AGENT);
        BundleStore::new(dir.dir(), ctx.get(crate::symbio_core::WORKDIR).as_deref())
    }

    /// 系统智能体自身指令 → VDFS 节点（`list` 与 `stat` 共用同一份形状）
    async fn instruction_node(&self) -> VdfsNode {
        self.instruction_store()
            .await
            .node(&instruction::node_spec())
    }
}

#[async_trait]
impl VdfsProvider for AgentPlugin {
    fn label(&self) -> Option<&str> {
        Some(LABEL)
    }

    fn description(&self) -> Option<&str> {
        Some("智能体实例（整目录能力包）与本应用自身的指令。")
    }

    fn order(&self) -> i32 {
        3
    }

    fn icon(&self) -> Option<&str> {
        Some(PLUGIN_AGENT)
    }

    /// 根可列举 + 可递归遍历（bundle 内部有子条目）
    fn root_access(&self) -> VdfsAccess {
        VdfsAccess::LIST_TRAVERSE
    }

    /// bundle 只能整包导入（没有「先建空壳再填字段」的形态）
    fn root_new_types(&self) -> Vec<VdfsNewType> {
        vec![VdfsNewType::new(VDFS_EXT_ZIP, format!("{LABEL}包"))
            .with_description(format!("导入{LABEL}整包（.zip）——整目录覆盖同名条目"))
            .with_source(VDFS_NEW_SOURCE_FILE)]
    }

    async fn list(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<Vec<VdfsNode>> {
        let host = host_ctx(ctx)?;
        let store = Self::store_of(&host);
        match parse_rel_path(path) {
            RelPath::Root => {
                // 本应用自身的指令排最前：剩下每一项都是「装进来的智能体」，它不是
                let mut nodes = vec![self.instruction_node().await];
                nodes.extend(store.list().into_iter().map(|r| bundle_node(&r, &store)));
                Ok(nodes)
            }
            // 指令是叶子节点
            RelPath::Instruction => Err(VdfsError::invalid(format!(
                "该路径是文件，不可列举：{path}"
            ))),
            RelPath::Agent { id } => {
                let id = id_of(id);
                // 存在性校验：不存在的条目应报 NotFound 而非给出空清单
                store
                    .get(&id)
                    .ok_or_else(|| VdfsError::not_found(format!("未找到{LABEL}「{id}」")))?;
                // 记忆节点：形状来自内核（`MemoryFile::node`），与 `stat` 同源
                let memory_node = self
                    .memory_store(&store, &id)
                    .await
                    .node(&memory::node_spec());
                let mut nodes = vec![memory_node];
                // 其余按目录原样呈现（根 `AGENTS.md` 已由记忆节点代表，不重复）
                nodes.extend(
                    store
                        .list_files(&id, "")
                        .map_err(|e| VdfsError::not_found(format!("列出目录失败：{e}")))?
                        .into_iter()
                        .filter(|e| e.path != AGENTS_FILE)
                        .map(|e| entry_node(&id, &e)),
                );
                Ok(nodes)
            }
            // 记忆是叶子节点
            RelPath::Memory { .. } => Err(VdfsError::not_found(format!(
                "智能体记忆是叶子节点，没有子项：{path}"
            ))),
            RelPath::File { id, rel } => {
                let id = id_of(id);
                let e = store
                    .stat_item(&id, rel)
                    .map_err(|e| VdfsError::not_found(format!("未找到路径「{path}」：{e}")))?;
                if !e.is_dir {
                    return Err(VdfsError::invalid(format!(
                        "该路径是文件，不可列举：{path}"
                    )));
                }
                Ok(store
                    .list_files(&id, rel)
                    .map_err(|e| VdfsError::not_found(format!("列出目录失败：{e}")))?
                    .into_iter()
                    .map(|e| entry_node(&id, &e))
                    .collect())
            }
        }
    }

    async fn stat(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<VdfsNode> {
        let host = host_ctx(ctx)?;
        let store = Self::store_of(&host);
        // 配置文档按**真实文件名**可达（列表里不并列，进设置走 ConfigurableVisitor）
        if path.trim_matches('/') == PLUGIN_FILE {
            return Ok(self.config_file().node());
        }
        match parse_rel_path(path) {
            // 自身根：**名字留空**——provider 不知道自己的挂载名，由使用方回填
            RelPath::Root => Ok(VdfsNode::dir("", LABEL, self.root_access())),
            // 本应用自身的指令（`{homedir}/AGENTS.md`）
            RelPath::Instruction => Ok(self.instruction_node().await),
            RelPath::Agent { id } => {
                let id = id_of(id);
                let r = store
                    .get(&id)
                    .ok_or_else(|| VdfsError::not_found(format!("未找到{LABEL}「{id}」")))?;
                Ok(bundle_node(&r, &store))
            }
            RelPath::Memory { id } => {
                let id = id_of(id);
                let memory = self.memory_store(&store, &id).await;
                if !memory.has_scope() {
                    return Err(VdfsError::not_found(format!("未找到{LABEL}「{id}」")));
                }
                Ok(memory.node(&memory::node_spec()))
            }
            RelPath::File { id, rel } => {
                let id = id_of(id);
                let e = store
                    .stat_item(&id, rel)
                    .map_err(|e| VdfsError::not_found(format!("未找到路径「{path}」：{e}")))?;
                Ok(entry_node(&id, &e))
            }
        }
    }

    async fn read(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<VdfsContent> {
        let host = host_ctx(ctx)?;
        let store = Self::store_of(&host);
        if path.trim_matches('/') == PLUGIN_FILE {
            return self.config_file().read(self.config_slot()).await;
        }
        match parse_rel_path(path) {
            // 本应用自身的指令（`{homedir}/AGENTS.md`）
            RelPath::Instruction => {
                let text = self
                    .instruction_store()
                    .await
                    .read()
                    .map_err(|e| VdfsError::not_found(format!("读取系统指令失败：{e}")))?;
                Ok(VdfsContent::text(path, text))
            }
            // Agent 目录内的文件：直读（沙箱在 store 里）
            RelPath::File { id, rel } => {
                let id = id_of(id);
                let text = store
                    .read_item(&id, rel)
                    .map_err(|e| VdfsError::not_found(format!("读取失败：{e}")))?;
                Ok(VdfsContent::text(path, text))
            }
            // 智能体记忆：Agent 目录下的 `AGENTS.md`
            RelPath::Memory { id } => {
                let id = id_of(id);
                let text = self
                    .memory_store(&store, &id)
                    .await
                    .read()
                    .map_err(|e| VdfsError::not_found(format!("读取智能体记忆失败：{e}")))?;
                Ok(VdfsContent::text(path, text))
            }
            // Agent 条目本身：读的是**概览**（详情表单 `binding: info` 的输入）
            RelPath::Agent { id } => {
                let id = id_of(id);
                let r = store
                    .get(&id)
                    .ok_or_else(|| VdfsError::not_found(format!("未找到{LABEL}「{id}」")))?;
                let text = serde_json::to_string_pretty(&bundle_info(&r, &store))
                    .map_err(|e| VdfsError::internal(format!("概览序列化失败：{e}")))?;
                Ok(VdfsContent::text(path, text).with_mime("application/json"))
            }
            RelPath::Root => Err(VdfsError::invalid(format!(
                "该路径是目录，不可读取内容：{path}"
            ))),
        }
    }

    async fn write(
        &self,
        ctx: &VdfsContext,
        path: &str,
        content: &VdfsContent,
    ) -> VdfsResult<VdfsWriteResponse> {
        let host = host_ctx(ctx)?;
        let store = Self::store_of(&host);
        // 配置文档：与其它插件同一口径（写自己的 `PLUGIN.yml`）
        if path.trim_matches('/') == PLUGIN_FILE {
            return self.config_file().apply(self.config_slot(), content).await;
        }
        // 系统智能体自身的指令写回（容量闸门在内核里，本插件不重复实现）
        if matches!(parse_rel_path(path), RelPath::Instruction) {
            if content.binary {
                return Err(VdfsError::invalid("AGENTS.md 是文本文件，不接受二进制内容"));
            }
            let instr = self.instruction_store().await;
            let existed = instr.exists();
            let text = content.text.as_deref().unwrap_or_default();
            instr.write(text).map_err(VdfsError::invalid)?;
            notify_change(
                PLUGIN_AGENT,
                path,
                if existed {
                    VDFS_CHANGE_UPDATED
                } else {
                    VDFS_CHANGE_CREATED
                },
            );
            return Ok(VdfsWriteResponse {
                path: path.to_string(),
                created: !existed,
                etag: None,
            });
        }
        // 智能体记忆写回（容量闸门在内核里，本插件不重复实现）
        if let RelPath::Memory { id } = parse_rel_path(path) {
            if content.binary {
                return Err(VdfsError::invalid("智能体记忆是文本文件，不接受二进制内容"));
            }
            let id = id_of(id);
            let memory = self.memory_store(&store, &id).await;
            if !memory.has_scope() {
                return Err(VdfsError::not_found(format!("未找到{LABEL}「{id}」")));
            }
            let existed = memory.exists();
            let text = content.text.as_deref().unwrap_or_default();
            memory.write(text).map_err(VdfsError::invalid)?;
            notify_change(
                PLUGIN_AGENT,
                path,
                if existed {
                    VDFS_CHANGE_UPDATED
                } else {
                    VDFS_CHANGE_CREATED
                },
            );
            return Ok(VdfsWriteResponse {
                path: path.to_string(),
                created: !existed,
                etag: None,
            });
        }
        // 整包导入：Agent **唯一的创建方式**（id 取自包内 manifest，忽略建议名）
        if content.binary {
            if !matches!(parse_rel_path(path), RelPath::Agent { .. }) {
                return Err(VdfsError::invalid(format!(
                    "{LABEL}整包只能导入到挂载根下：{path}"
                )));
            }
            let bytes = vdfs_service::decode_b64(content.b64.as_deref().unwrap_or_default())
                .map_err(|e| VdfsError::invalid(e.0))?;
            let r = store
                .import(&bytes, true)
                .map_err(|e| VdfsError::invalid(format!("导入失败：{e}")))?;
            notify_change(
                PLUGIN_AGENT,
                &r.id,
                if r.replaced {
                    VDFS_CHANGE_UPDATED
                } else {
                    VDFS_CHANGE_CREATED
                },
            );
            return Ok(VdfsWriteResponse {
                path: r.id,
                created: !r.replaced,
                etag: None,
            });
        }
        // Agent 目录内的文件写回（路径沙箱 + 容量闸门都在 store 里）
        if let RelPath::File { id, rel } = parse_rel_path(path) {
            let id = id_of(id);
            let text = content.text.as_deref().unwrap_or("");
            let existed = store.stat_item(&id, rel).is_ok();
            let max_bytes = self.item_max_bytes().await;
            store
                .write_item(&id, rel, text, max_bytes)
                .map_err(|e| VdfsError::invalid(format!("写入失败：{e}")))?;
            notify_change(
                PLUGIN_AGENT,
                path,
                if existed {
                    VDFS_CHANGE_UPDATED
                } else {
                    VDFS_CHANGE_CREATED
                },
            );
            return Ok(VdfsWriteResponse {
                path: path.to_string(),
                created: !existed,
                etag: None,
            });
        }
        // Agent 条目本身不可表单新建 / 覆盖（无「先建空壳」形态）
        Err(VdfsError::Forbidden(format!(
            "{LABEL}只支持整包导入，不支持表单写入：{path}"
        )))
    }

    async fn delete(&self, ctx: &VdfsContext, path: &str, _recursive: bool) -> VdfsResult<()> {
        if path.is_empty() {
            return Err(VdfsError::Forbidden(format!("不可删除挂载点：{path}")));
        }
        let host = host_ctx(ctx)?;
        let store = Self::store_of(&host);
        // 系统指令不可删除（与各层记忆同一口径）：要清空就写入空内容
        if matches!(parse_rel_path(path), RelPath::Instruction) {
            return Err(VdfsError::Forbidden(format!(
                "系统指令不可删除（删除即丢失全部指令）。\
                 如需清空，请向 `{AGENTS_FILE}` 写入空内容。"
            )));
        }
        // 智能体记忆不可删除（与工作区记忆同一口径）：要清空就写入空内容
        if matches!(parse_rel_path(path), RelPath::Memory { .. }) {
            return Err(VdfsError::Forbidden(format!(
                "智能体记忆不可删除（删除即丢失全部长期记忆）。\
                 如需清空，请向 `{AGENTS_FILE}` 写入空内容。"
            )));
        }
        // Agent 目录内的文件 / 子目录
        if let RelPath::File { id, rel } = parse_rel_path(path) {
            let id = id_of(id);
            let e = store
                .stat_item(&id, rel)
                .map_err(|e| VdfsError::not_found(format!("未找到路径「{path}」：{e}")))?;
            if e.is_dir {
                std::fs::remove_dir_all(store.item_path(&id, rel).map_err(VdfsError::invalid)?)
                    .map_err(|e| VdfsError::invalid(format!("删除目录失败：{e}")))?;
            } else {
                store
                    .delete_item(&id, rel)
                    .map_err(|e| VdfsError::invalid(format!("删除失败：{e}")))?;
            }
            notify_change(PLUGIN_AGENT, path, VDFS_CHANGE_DELETED);
            return Ok(());
        }
        let id = id_of(path);
        store
            .delete(&id)
            .map_err(|e| VdfsError::invalid(format!("删除{LABEL}失败：{e}")))?;
        notify_change(PLUGIN_AGENT, &id, VDFS_CHANGE_DELETED);
        Ok(())
    }

    /// 节点动作：「导出」把 bundle 打成 zip 随 `data` 回传
    /// （与二进制写入的整包导入互为逆向）
    async fn action(
        &self,
        ctx: &VdfsContext,
        path: &str,
        action: &str,
        _payload: Option<&serde_json::Value>,
    ) -> VdfsResult<VdfsActionResult> {
        if action != VDFS_ACTION_EXPORT {
            return Err(VdfsError::NotImplemented);
        }
        if !matches!(parse_rel_path(path), RelPath::Agent { .. }) {
            return Err(VdfsError::invalid(format!(
                "「导出」只对{LABEL}条目可用：{path}"
            )));
        }
        let host = host_ctx(ctx)?;
        let store = Self::store_of(&host);
        let id = id_of(path);
        let bytes = store
            .export(&id)
            .map_err(|e| VdfsError::not_found(format!("导出失败：{e}")))?;
        let pack = vdfs_service::VdfsPack::new(&id, &bytes);
        let data = serde_json::to_value(&pack)
            .map_err(|e| VdfsError::internal(format!("导出结果序列化失败: {e}")))?;
        Ok(VdfsActionResult {
            action: VDFS_ACTION_EXPORT.to_string(),
            ok: true,
            message: format!("已打包「{}」", pack.filename),
            data: Some(data),
        })
    }

    async fn watch(&self, _ctx: &VdfsContext, path: &str, sink: VdfsChangeSink) -> VdfsResult<()> {
        watch_changes(PLUGIN_AGENT, path, sink).await
    }

    async fn unwatch(&self, _ctx: &VdfsContext, path: &str) -> VdfsResult<()> {
        unwatch_changes(PLUGIN_AGENT, path).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `<id>` 之后是 Agent 目录内的任意相对路径（可多段）
    #[test]
    fn rel_path_splits_agent_and_inner_path() {
        assert!(matches!(parse_rel_path(""), RelPath::Root));
        assert!(matches!(parse_rel_path("/"), RelPath::Root));
        assert!(matches!(parse_rel_path("b1"), RelPath::Agent { id: "b1" }));
        assert!(matches!(
            parse_rel_path("b1.agent"),
            RelPath::Agent { id: "b1.agent" }
        ));
        match parse_rel_path("b1/skill/foo/SKILL.md") {
            RelPath::File { id, rel } => {
                assert_eq!(id, "b1");
                assert_eq!(rel, "skill/foo/SKILL.md");
            }
            other => panic!("期望 File，实际：{other:?}"),
        }
        // 第二段是记忆文件名 → 记忆，而不是普通文件
        match parse_rel_path("b1/AGENTS.md") {
            RelPath::Memory { id } => assert_eq!(id, "b1"),
            other => panic!("期望 Memory，实际：{other:?}"),
        }
        // 更深处的同名文件仍是普通文件（记忆只在 Agent 根这一层）
        assert!(matches!(
            parse_rel_path("b1/skill/AGENTS.md"),
            RelPath::File { .. }
        ));
    }

    /// 挂载根下的 `AGENTS.md` 是**本应用自身的指令**，不是名为它的 bundle
    ///
    /// 两者不可能相撞：bundle id 的首字符必须是小写字母或数字（§5.1），
    /// 而保留名以大写 `A` 开头。
    #[test]
    fn root_agents_md_is_the_host_instruction_not_a_bundle() {
        assert!(matches!(parse_rel_path(AGENTS_FILE), RelPath::Instruction));
        assert!(matches!(parse_rel_path("/AGENTS.md"), RelPath::Instruction));
        // 带子路径时不再命中保留名（那是一条指向不存在条目的普通 bundle 路径）
        assert!(matches!(
            parse_rel_path("AGENTS.md/x"),
            RelPath::File { .. }
        ));
    }

    /// 路径末段 → Agent id（去掉 `.agent` 呈现扩展名）
    #[test]
    fn id_of_strips_presentation_extension() {
        assert_eq!(id_of("demo"), "demo");
        assert_eq!(id_of("demo.agent"), "demo");
    }
}
