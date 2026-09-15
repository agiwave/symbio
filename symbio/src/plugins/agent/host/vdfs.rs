//! VDFS 挂载点（`.vdfs/agent`）—— 本插件**直接实现 `VdfsProvider`**。
//!
//! ## 与其它资源插件的分工差异
//!
//! bundle 是**目录自管型**资源：它的落盘由 [`BundleStore`] 负责（工作区级 +
//! 全局级双层、zip-slip 防护、版本硬门槛），**不经 `vdfs_service`**——
//! `vdfs_service` 的三种拓扑都是「`<homedir>/plugins/<类别>/<id>/…`」这一固定落位，
//! 而 bundle 要同时看见工作目录与系统目录两层，寻址规则本身是 bundle 语义的一部分。
//!
//! 相同的是**广播**：落盘后一律走 `vdfs::notify_change`，与 `vdfs_service` 三个
//! 实现投的是同一条频道，订阅方无需区分资源住在哪儿。
//!
//! 外部访问一律走 `.vdfs/agent/…`。

use super::plugin::AgentPlugin;
use super::store::{classify_item_path, BundleRecord, BundleStore};
use crate::providers::vdfs_service;
use crate::symbio_core::vdfs::{host_ctx, notify_change, unwatch_changes, watch_changes};
use crate::symbio_core::vdfs_provider::{
    VdfsAccess, VdfsActionResult, VdfsChangeSink, VdfsContent, VdfsContext, VdfsError, VdfsNewType,
    VdfsNode, VdfsProvider, VdfsResult, VdfsWriteResponse, VDFS_ACTION_EXPORT, VDFS_CHANGE_CREATED,
    VDFS_CHANGE_DELETED, VDFS_CHANGE_UPDATED, VDFS_EXT_FORM, VDFS_EXT_ZIP, VDFS_NEW_SOURCE_FILE,
};
use crate::symbio_core::{InvokeRequest, InvokeRequestExt, PLUGIN_AGENT};
use async_trait::async_trait;
use std::sync::Arc;

const LABEL: &str = "智能体";

// ==================== 容器子条目声明（bundle 内部文件） ====================
//
// bundle 条目即容器：内部 prompts / skills / mcps 以
// `<bundle id>/<子类别标签>/<相对路径>` 寻址，实现复用 [`BundleStore`] 的
// 沙箱化方法（路径白名单 `classify_item_path` + absolutize 双重闸门）。

/// 一类容器子条目（以**标签**而非 kind 作路径段——与会话内部同一口径：
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

/// 挂载点内相对路径（至多切三段：条目 / 子类别 / 子条目）
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

/// 路径末段 → 条目 id（去掉 `.agent` 呈现扩展名）
fn id_of(path: &str) -> String {
    vdfs_service::entry::id_of(path, PLUGIN_AGENT)
}

// ==================== 节点合成 ====================

/// bundle 概览（`read` 与详情表单共用的信息载荷）
fn bundle_info(r: &BundleRecord, store: &BundleStore) -> serde_json::Value {
    // 内部条目计数：概览要回答「这个包里有什么」，目录清单是它的唯一真相源
    let mut counts = (0usize, 0usize, 0usize);
    if let Ok(entries) = store.list_items(&r.manifest.id) {
        for e in entries {
            match e.kind.as_str() {
                "prompt" => counts.0 += 1,
                "skill" => counts.1 += 1,
                "mcp" => counts.2 += 1,
                _ => {}
            }
        }
    }
    serde_json::json!({
        // config_type = "bundle"：项级图标 / 详情分发的顶层键（VdfsNode attributes flatten）
        "config_type": "bundle",
        "version": r.manifest.version,
        "spec": r.manifest.spec,
        "requires_spec": r.manifest.requires.spec,
        "scope": r.source.as_str(),
        "dir": r.dir.to_string_lossy(),
        "count_prompt": counts.0,
        "count_skill": counts.1,
        "count_mcp": counts.2,
    })
}

/// bundle 记录 → VDFS 节点（只读概览表单：`ext = form` + 定义随 `schema` 下发）
///
/// bundle 条目同时是容器（内部可浏览提示词 / 技能 / MCP），但**有详情定义**，
/// 故呈现为表单文件——「浏览内部」走 `enter(<id>/<子类别>)` 的目录语义，
/// 与详情页互不影响。
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

/// 子类别目录节点。可新建的类型由 `path_hint` 决定——非空即可创建，
/// 扩展名取自路径模板（`prompts/<name>.md` → `md`）。
fn section_node(spec: &'static ContainerKind) -> VdfsNode {
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

/// 子条目 → VDFS 节点（文件；`ext` 由文件名推导，渲染器据此分发）
fn container_node(
    spec: &ContainerKind,
    path: &str,
    name: &str,
    size: Option<u64>,
    priority: Option<i64>,
) -> VdfsNode {
    let mut n = VdfsNode::file(path, name, VdfsAccess::READ_WRITE);
    n.kind = spec.kind.to_string();
    n.size = size;
    if let Some(p) = priority {
        n.attributes
            .insert("priority".to_string(), serde_json::json!(p));
    }
    n
}

impl AgentPlugin {
    /// 依请求上下文构造 BundleStore（每次请求独立，与 route 入口一致）
    fn store_of(ctx: &Arc<dyn InvokeRequest>) -> BundleStore {
        BundleStore::new(ctx.get(crate::symbio_core::WORKDIR).as_deref())
    }

    /// 子条目 → `(节点, 正文)`（`read` 直接取用正文）
    fn container_item(
        store: &BundleStore,
        container: &str,
        rel: &str,
    ) -> VdfsResult<(VdfsNode, String)> {
        let (kind, name) = classify_item_path(rel)
            .map_err(|e| VdfsError::invalid(format!("子条目路径不合规：{e}")))?;
        let content = store
            .read_item(container, rel)
            .map_err(|e| VdfsError::not_found(format!("读取子条目失败：{e}")))?;
        let spec = CONTAINER_KINDS.iter().find(|k| k.kind == kind);
        // `rel` 是节点地址、`name` 是展示标题：与目录型资源同一口径
        let node = match spec {
            Some(spec) => container_node(spec, rel, &name, None, None),
            None => {
                let mut n = VdfsNode::file(rel, name, VdfsAccess::READ_WRITE);
                n.kind = kind.to_string();
                n
            }
        };
        Ok((node, content))
    }
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
            RelPath::Root => Ok(store
                .list()
                .into_iter()
                .map(|r| bundle_node(&r, &store))
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
                    .list_items(id)
                    .map_err(|e| VdfsError::not_found(format!("列出子条目失败：{e}")))?;
                Ok(entries
                    .into_iter()
                    .filter(|e| e.kind == spec.kind)
                    .map(|e| container_node(spec, &e.path, &e.name, Some(e.size), e.priority))
                    .collect())
            }
            RelPath::SubItem { .. } => Err(VdfsError::not_found(format!(
                "子条目是叶子节点，没有子项：{path}"
            ))),
        }
    }

    async fn stat(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<VdfsNode> {
        let host = host_ctx(ctx)?;
        let store = Self::store_of(&host);
        match parse_rel_path(path) {
            // 自身根：**名字留空**——provider 不知道自己的挂载名，由使用方回填
            RelPath::Root => Ok(VdfsNode::dir("", LABEL, self.root_access())),
            RelPath::Item(id) => {
                let id = id_of(id);
                let r = store
                    .get(&id)
                    .ok_or_else(|| VdfsError::not_found(format!("未找到{LABEL}「{id}」")))?;
                Ok(bundle_node(&r, &store))
            }
            RelPath::Section { seg, .. } => section_of(seg)
                .map(section_node)
                .ok_or_else(|| VdfsError::not_found(format!("不存在子类别：{path}"))),
            RelPath::SubItem { id, item, .. } => Ok(Self::container_item(&store, id, item)?.0),
        }
    }

    async fn read(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<VdfsContent> {
        let host = host_ctx(ctx)?;
        let store = Self::store_of(&host);
        match parse_rel_path(path) {
            // 子条目：bundle 沙箱内读取正文
            RelPath::SubItem { id, item, .. } => {
                let (node, content) = Self::container_item(&store, id, item)?;
                let _ = node;
                Ok(VdfsContent::text(path, content))
            }
            RelPath::Root | RelPath::Section { .. } => Err(VdfsError::invalid(format!(
                "该路径是目录，不可读取内容：{path}"
            ))),
            // bundle 条目本身：读的是**概览**（详情表单 `binding: info` 的输入）
            RelPath::Item(id) => {
                let id = id_of(id);
                let r = store
                    .get(&id)
                    .ok_or_else(|| VdfsError::not_found(format!("未找到{LABEL}「{id}」")))?;
                let text = serde_json::to_string_pretty(&bundle_info(&r, &store))
                    .map_err(|e| VdfsError::internal(format!("概览序列化失败：{e}")))?;
                Ok(VdfsContent::text(path, text).with_mime("application/json"))
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
        // 子条目写回（bundle 沙箱内写入）；新建时按 `path_hint` 落位
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
                .list_items(id)
                .map_err(|e| VdfsError::not_found(format!("列出子条目失败：{e}")))?
                .iter()
                .any(|e| e.path == target);
            store
                .write_item(id, &target, text)
                .map_err(|e| VdfsError::invalid(format!("写入子条目失败：{e}")))?;
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
        // 子条目删除（bundle 沙箱内删除）
        if let RelPath::SubItem { id, seg, item } = parse_rel_path(path) {
            let _ = section_of(seg)
                .ok_or_else(|| VdfsError::not_found(format!("不存在子类别：{path}")))?;
            store
                .delete_item(id, item)
                .map_err(|e| VdfsError::invalid(format!("删除子条目失败：{e}")))?;
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

    /// 挂载点内相对路径至多切三段：条目 / 子类别 / 子条目
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

    /// 子类别可新建的类型由路径模板推导（`prompts/<name>.md` → `md`）
    #[test]
    fn section_node_declares_new_type_from_path_hint() {
        let prompt = section_node(section_of("提示词").unwrap());
        assert_eq!(prompt.new_types.len(), 1);
        assert_eq!(prompt.new_types[0].ext, "md");
        // 条目名 = 展示标题缺省同源（描述为空时不硬造）
        assert_eq!(prompt.name, "提示词");
    }
}
