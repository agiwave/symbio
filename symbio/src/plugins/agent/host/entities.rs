//! VDFS 挂载点（`.vdfs/agent`）—— 本插件**直接实现 `VdfsProvider`**。
//!
//! ## 与 [`super::handlers`] 的分工
//!
//! - 列表 / 读写：本模块 override，直接枚举 [`BundleStore`]（bundle 不是
//!   EntityStore 型，不落 `~/.symbio/plugins/<category>/`）；
//! - 写 / 删：bundle 由 [`BundleStore`] 自管目录与 manifest 校验（zip-slip
//!   防护 / 版本硬门槛），故不复用 EntityStore 写盘原语；
//! - 落盘后的变更广播：走 `vdfs::notify_change`（与 EntityStore 型资源同一通道）。
//!
//! 外部访问一律走 `.vdfs/agent/…`。

use super::plugin::AgentPlugin;
use super::store::{classify_entity_path, BundleRecord, BundleStore};
use crate::symbio_core::entities::EntitySummary;
use crate::symbio_core::entities::ENTITY_AGENT;
use crate::symbio_core::vdfs::{host_ctx, notify_change, unwatch_changes, watch_changes};
use crate::symbio_core::vdfs_provider::{
    VdfsAccess, VdfsActionResult, VdfsChangeSink, VdfsContent, VdfsContext, VdfsError, VdfsNewType,
    VdfsNode, VdfsProvider, VdfsResult, VdfsWriteResponse, VFDS_ACTION_EXPORT, VFDS_EXT_FORM,
    VFDS_EXT_ZIP, VFDS_NEW_SOURCE_FILE,
};
use crate::symbio_core::{InvokeRequest, InvokeRequestExt};
use async_trait::async_trait;
use std::sync::Arc;

const LABEL: &str = "智能体";

// ==================== 容器子实体声明（bundle 内部文件） ====================
//
// bundle 条目即容器：内部 prompts / skills / mcps 以
// `<bundle id>/<子类别标签>/<相对路径>` 寻址，实现复用 [`BundleStore`] 的
// 沙箱化方法（路径白名单 `classify_entity_path` + absolutize 双重闸门）。

/// 一类容器子实体（以**标签**而非 kind 作路径段——与会话内部同一口径：
/// 路径是给人看的，kind 是实现标识）
struct ContainerKind {
    kind: &'static str,
    label: &'static str,
    description: &'static str,
    /// 新建时的落位模板（`<name>` 替换为文件名；空 = 不可用户创建）
    path_hint: &'static str,
    /// 新建且内容为空时的默认正文
    default_content: &'static str,
}

const CONTAINER_KINDS: &[ContainerKind] = &[
    ContainerKind {
        kind: "prompt",
        label: "提示词",
        description: "Markdown 片段，无条件追加进系统提示词；可用 YAML frontmatter 设置 priority（缺省 10，小者优先）。",
        path_hint: "prompts/<name>.md",
        default_content: "---\npriority: 10\n---\n\n在此撰写常驻系统提示词（人格 / 全局规则 / 工作流）…",
    },
    ContainerKind {
        kind: "skill",
        label: "技能",
        description: "skills/<name>/SKILL.md，正文作为提示词片段；frontmatter priority 缺省 50。",
        path_hint: "skills/<name>/SKILL.md",
        default_content: "---\npriority: 50\n---\n\n# 技能名称\n\n描述该技能的适用场景、输入输出与执行步骤…",
    },
    ContainerKind {
        kind: "mcp",
        label: "MCP",
        description: "MCP server 配置（YAML），是工具的唯一来源，原样透传给宿主 MCP 客户端。",
        path_hint: "mcps/<name>.yaml",
        default_content: "# MCP server 配置（YAML，原样透传给宿主 MCP 客户端）\ncommand: \"\"\nargs: []\nenv: {}",
    },
];

fn section_of(seg: &str) -> Option<&'static ContainerKind> {
    CONTAINER_KINDS.iter().find(|k| k.label == seg)
}

// ==================== 路径解析 ====================

/// 挂载点内相对路径（至多切三段：条目 / 子类别 / 子实体）
#[derive(Debug)]
enum RelPath<'a> {
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

fn parse_rel_path(path: &str) -> RelPath<'_> {
    let p = path.trim_matches('/');
    if p.is_empty() {
        return RelPath::Root;
    }
    match p.split_once('/') {
        None => RelPath::Item(p),
        Some((id, rest)) => match rest.split_once('/') {
            None => RelPath::Section { id, seg: rest },
            Some((seg, item)) => RelPath::SubItem { id, seg, item },
        },
    }
}

/// 路径末段 → 条目 id（去掉 `.<kind>` 呈现扩展名）
fn id_of(path: &str) -> String {
    let base = path.rsplit('/').next().unwrap_or(path);
    base.strip_suffix(&format!(".{ENTITY_AGENT}"))
        .unwrap_or(base)
        .to_string()
}

// ==================== 节点合成 ====================

/// bundle 记录 → 摘要
fn summary_of(r: &BundleRecord, store: &BundleStore) -> EntitySummary {
    let mut it = EntitySummary::new(
        ENTITY_AGENT,
        r.manifest.id.clone(),
        if r.manifest.name.is_empty() {
            r.manifest.id.clone()
        } else {
            r.manifest.name.clone()
        },
    );
    it.status = "active".to_string();
    if !r.manifest.description.is_empty() {
        it.description = Some(r.manifest.description.clone());
        it.summary = Some(r.manifest.description.clone());
    }
    // 类型特有扩展：版本 / 规格 / 来源层级 / 安装目录 / 内部实体计数
    // （前端 DetailForm info 绑定按需展示）
    // config_type = "bundle"：项级分发键，前端据此分发，必须保留；
    // 明细展示走 DetailForm info 绑定
    let mut counts = (0usize, 0usize, 0usize);
    if let Ok(entries) = store.list_entities(&r.manifest.id) {
        for e in entries {
            match e.kind.as_str() {
                "prompt" => counts.0 += 1,
                "skill" => counts.1 += 1,
                "mcp" => counts.2 += 1,
                _ => {}
            }
        }
    }
    it.extra = serde_json::json!({
        "config_type": "bundle",
        "version": r.manifest.version,
        "spec": r.manifest.spec,
        "requires_spec": r.manifest.requires.spec,
        "scope": r.source.as_str(),
        "dir": r.dir.to_string_lossy(),
        "count_prompt": counts.0,
        "count_skill": counts.1,
        "count_mcp": counts.2,
    });
    it
}

/// 摘要 → VDFS 节点（只读概览表单：`ext = form` + 定义随 `schema` 下发）
///
/// bundle 条目同时是容器（内部可浏览提示词 / 技能 / MCP），但**有详情定义**，
/// 故呈现为表单文件——「浏览内部」走 `enter(<id>/<子类别>)` 的目录语义，
/// 与详情页互不影响。
fn node_of(item: &EntitySummary) -> VdfsNode {
    let mut n = VdfsNode::file(&item.id, item.name.clone(), VdfsAccess::READ);
    n.kind = ENTITY_AGENT.to_string();
    n.ext = Some(VFDS_EXT_FORM.to_string());
    n.schema = serde_json::to_value(super::detail::agent_detail_definition()).ok();
    n.status = item.status.clone();
    n.description = item.description.clone().or_else(|| item.summary.clone());
    n
}

/// 子类别目录节点。可新建的类型由 `path_hint` 决定——非空即可创建，
/// 扩展名取自路径模板（`prompts/<name>.md` → `md`）。
fn section_node(spec: &ContainerKind) -> VdfsNode {
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

/// 子实体条目 → VDFS 节点（文件；`ext` 由文件名推导，渲染器据此分发）
fn container_node(spec: &ContainerKind, it: &EntitySummary) -> VdfsNode {
    let mut n = VdfsNode::file(it.id.clone(), it.name.clone(), VdfsAccess::READ_WRITE);
    n.kind = spec.kind.to_string();
    n.size = it.extra.get("size").and_then(|v| v.as_u64());
    n.description = it.description.clone();
    n
}

impl AgentPlugin {
    /// 依请求上下文构造 BundleStore（每次请求独立，与 route 入口一致）
    fn store_of(ctx: &Arc<dyn InvokeRequest>) -> BundleStore {
        BundleStore::new(ctx.get(crate::symbio_core::WORKDIR).as_deref())
    }
}

/// 子实体条目 → 摘要（含正文，`read` 直接取用）
fn container_item_of(store: &BundleStore, container: &str, rel: &str) -> VdfsResult<EntitySummary> {
    let (kind, name) = classify_entity_path(rel)
        .map_err(|e| VdfsError::invalid(format!("子实体路径不合规：{e}")))?;
    let content = store
        .read_entity(container, rel)
        .map_err(|e| VdfsError::not_found(format!("读取子实体失败：{e}")))?;
    let mut it = EntitySummary::new(kind, rel, name);
    it.status = "active".to_string();
    it.extra = serde_json::json!({
        "container": container,
        "content": content,
    });
    Ok(it)
}

#[async_trait]
impl VdfsProvider for AgentPlugin {
    fn label(&self) -> Option<&str> {
        Some(LABEL)
    }

    fn description(&self) -> Option<&str> {
        Some("Agent bundle（整目录能力包：提示词 / 技能 / MCP 的装配单元）。")
    }

    fn order(&self) -> i32 {
        3
    }

    fn icon(&self) -> Option<&str> {
        Some(ENTITY_AGENT)
    }

    /// 根可列举 + 可递归遍历（bundle 内部有子实体）
    fn root_access(&self) -> VdfsAccess {
        VdfsAccess::LIST_TRAVERSE
    }

    /// bundle 只能整包导入（没有「先建空壳再填字段」的形态）
    fn root_new_types(&self) -> Vec<VdfsNewType> {
        vec![VdfsNewType::new(VFDS_EXT_ZIP, format!("{LABEL}包"))
            .with_description(format!("导入{LABEL}整包（.zip）——整目录覆盖同名条目"))
            .with_source(VFDS_NEW_SOURCE_FILE)]
    }

    async fn list(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<Vec<VdfsNode>> {
        let host = host_ctx(ctx)?;
        let store = Self::store_of(&host);
        match parse_rel_path(path) {
            RelPath::Root => Ok(store
                .list()
                .into_iter()
                .map(|r| node_of(&summary_of(&r, &store)))
                .collect()),
            // 条目即容器：子类别清单（提示词 / 技能 / MCP）
            RelPath::Item(id) => {
                let id = id_of(id);
                // 存在性校验：不存在的条目应报 NotFound 而非给出空类别清单
                store
                    .get(&id)
                    .ok_or_else(|| VdfsError::not_found(format!("未找到{LABEL}「{id}」")))?;
                Ok(CONTAINER_KINDS.iter().map(section_node).collect())
            }
            RelPath::Section { id, seg } => {
                let spec = section_of(seg)
                    .ok_or_else(|| VdfsError::not_found(format!("不存在子类别：{path}")))?;
                let entries = store
                    .list_entities(id)
                    .map_err(|e| VdfsError::not_found(format!("列出子实体失败：{e}")))?;
                Ok(entries
                    .into_iter()
                    .filter(|e| e.kind == spec.kind)
                    .map(|e| {
                        let mut it = EntitySummary::new(&e.kind, e.path.clone(), e.name.clone());
                        it.status = "active".to_string();
                        it.extra = serde_json::json!({
                            "container": id,
                            "priority": e.priority,
                            "size": e.size,
                        });
                        container_node(spec, &it)
                    })
                    .collect())
            }
            RelPath::SubItem { .. } => Err(VdfsError::not_found(format!(
                "子实体是叶子节点，没有子项：{path}"
            ))),
        }
    }

    async fn stat(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<VdfsNode> {
        let host = host_ctx(ctx)?;
        let store = Self::store_of(&host);
        match parse_rel_path(path) {
            // 自身根：名字留空——provider 不知道自己的挂载名，由使用方回填
            RelPath::Root => Ok(VdfsNode::dir("", LABEL, self.root_access())),
            RelPath::Item(id) => {
                let id = id_of(id);
                let r = store
                    .get(&id)
                    .ok_or_else(|| VdfsError::not_found(format!("未找到{LABEL}「{id}」")))?;
                Ok(node_of(&summary_of(&r, &store)))
            }
            RelPath::Section { seg, .. } => section_of(seg)
                .map(section_node)
                .ok_or_else(|| VdfsError::not_found(format!("不存在子类别：{path}"))),
            RelPath::SubItem { id, seg, item } => {
                let spec = section_of(seg)
                    .ok_or_else(|| VdfsError::not_found(format!("不存在子类别：{path}")))?;
                Ok(container_node(spec, &container_item_of(&store, id, item)?))
            }
        }
    }

    async fn read(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<VdfsContent> {
        let host = host_ctx(ctx)?;
        let store = Self::store_of(&host);
        match parse_rel_path(path) {
            // 子实体：bundle 沙箱内读取，正文随摘要下发
            RelPath::SubItem { id, seg, item } => {
                let _ = section_of(seg)
                    .ok_or_else(|| VdfsError::not_found(format!("不存在子类别：{path}")))?;
                let it = container_item_of(&store, id, item)?;
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
            RelPath::Root | RelPath::Section { .. } => Err(VdfsError::invalid(format!(
                "该路径是目录，不可读取内容：{path}"
            ))),
            RelPath::Item(id) => {
                let id = id_of(id);
                let r = store
                    .get(&id)
                    .ok_or_else(|| VdfsError::not_found(format!("未找到{LABEL}「{id}」")))?;
                let item = summary_of(&r, &store);
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

    async fn write(
        &self,
        ctx: &VdfsContext,
        path: &str,
        content: &VdfsContent,
    ) -> VdfsResult<VdfsWriteResponse> {
        let host = host_ctx(ctx)?;
        let store = Self::store_of(&host);
        // 整包导入：bundle **唯一的创建方式**（id 取自包内 manifest，忽略建议名）
        if content.binary {
            if !matches!(parse_rel_path(path), RelPath::Item(_)) {
                return Err(VdfsError::invalid(format!(
                    "{LABEL}整包只能导入到挂载根下：{path}"
                )));
            }
            let bytes = crate::symbio_core::entities::decode_b64(
                content.b64.as_deref().unwrap_or_default(),
            )
            .map_err(|e| VdfsError::invalid(e.0))?;
            let r = store
                .import(&bytes, true)
                .map_err(|e| VdfsError::invalid(format!("导入失败：{e}")))?;
            notify_change(
                ENTITY_AGENT,
                &r.id,
                if r.replaced { "updated" } else { "created" },
            );
            return Ok(VdfsWriteResponse {
                path: r.id,
                created: !r.replaced,
                etag: None,
            });
        }
        // 子实体写回（bundle 沙箱内写入）；新建时按 `path_hint` 落位
        if let RelPath::SubItem { id, seg, item } = parse_rel_path(path) {
            let spec = section_of(seg)
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
            let existed = store
                .list_entities(id)
                .map_err(|e| VdfsError::not_found(format!("列出子实体失败：{e}")))?
                .iter()
                .any(|e| e.path == target);
            store
                .write_entity(id, &target, text)
                .map_err(|e| VdfsError::invalid(format!("写入子实体失败：{e}")))?;
            notify_change(
                ENTITY_AGENT,
                path,
                if existed { "updated" } else { "created" },
            );
            return Ok(VdfsWriteResponse {
                path: path.to_string(),
                created: !existed,
                etag: None,
            });
        }
        // bundle 条目本身不可表单新建 / 覆盖（无「先建空壳」形态）
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
        // 子实体删除（bundle 沙箱内删除）
        if let RelPath::SubItem { id, seg, item } = parse_rel_path(path) {
            let _ = section_of(seg)
                .ok_or_else(|| VdfsError::not_found(format!("不存在子类别：{path}")))?;
            store
                .delete_entity(id, item)
                .map_err(|e| VdfsError::invalid(format!("删除子实体失败：{e}")))?;
            notify_change(ENTITY_AGENT, path, "deleted");
            return Ok(());
        }
        let id = id_of(path);
        store
            .delete(&id)
            .map_err(|e| VdfsError::invalid(format!("删除{LABEL}失败：{e}")))?;
        notify_change(ENTITY_AGENT, &id, "deleted");
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
        match action {
            VFDS_ACTION_EXPORT => {
                if !matches!(parse_rel_path(path), RelPath::Item(_)) {
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
                let filename = format!("{id}.zip");
                let data =
                    serde_json::to_value(crate::symbio_core::schemas::entities::EntityExport {
                        id: id.clone(),
                        filename: filename.clone(),
                        b64: crate::symbio_core::entities::encode_b64(&bytes),
                    })
                    .map_err(|e| VdfsError::internal(format!("导出结果序列化失败: {e}")))?;
                Ok(VdfsActionResult {
                    action: VFDS_ACTION_EXPORT.to_string(),
                    ok: true,
                    message: format!("已打包「{filename}」"),
                    data: Some(data),
                })
            }
            _ => Err(VdfsError::NotImplemented),
        }
    }

    async fn watch(&self, _ctx: &VdfsContext, path: &str, sink: VdfsChangeSink) -> VdfsResult<()> {
        watch_changes(ENTITY_AGENT, path, sink).await
    }

    async fn unwatch(&self, _ctx: &VdfsContext, path: &str) -> VdfsResult<()> {
        unwatch_changes(ENTITY_AGENT, path).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 挂载点内相对路径至多切三段：条目 / 子类别 / 子实体
    #[test]
    fn rel_path_parses_three_segments() {
        assert!(matches!(parse_rel_path(""), RelPath::Root));
        assert!(matches!(parse_rel_path("/"), RelPath::Root));
        assert!(matches!(parse_rel_path("b1"), RelPath::Item("b1")));
        assert!(matches!(
            parse_rel_path("b1.agent"),
            RelPath::Item("b1.agent")
        ));
        match parse_rel_path("b1/提示词") {
            RelPath::Section { id, seg } => {
                assert_eq!(id, "b1");
                assert_eq!(seg, "提示词");
            }
            other => panic!("期望 Section，实际：{other:?}"),
        }
        match parse_rel_path("b1/提示词/prompts/a.md") {
            RelPath::SubItem { id, seg, item } => {
                assert_eq!(id, "b1");
                assert_eq!(seg, "提示词");
                assert_eq!(item, "prompts/a.md");
            }
            other => panic!("期望 SubItem，实际：{other:?}"),
        }
    }

    /// 子类别以**标签**（人读的路径段）寻址，不是 kind
    #[test]
    fn section_is_addressed_by_label() {
        assert_eq!(section_of("提示词").map(|s| s.kind), Some("prompt"));
        assert_eq!(section_of("技能").map(|s| s.kind), Some("skill"));
        assert_eq!(section_of("MCP").map(|s| s.kind), Some("mcp"));
        assert!(section_of("prompt").is_none());
    }

    /// 路径末段 → bundle id（去掉 `.agent` 呈现扩展名）
    #[test]
    fn id_of_strips_presentation_extension() {
        assert_eq!(id_of("demo"), "demo");
        assert_eq!(id_of("demo.agent"), "demo");
    }
}
