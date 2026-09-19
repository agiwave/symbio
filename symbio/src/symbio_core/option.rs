//! 选项收集机制（跨插件共享设施）
//!
//! ## 与能力收集机制的关系
//!
//! 会话能力（工具 / 模型服务 / 系统提示词）经 `CapabilityVisitor` +
//! `Plugin::traverse(TRAVERSE_AVAILABLE_TOOLS)` 收集（收集管线由 session 插件持有，
//! 见 `plugins/session/chat_pipeline.rs`）。
//! 本模块是同一机制的**平行第二通道**：选项（会话输入区下方的可选项）
//! 经 [`OptionVisitor`] + `Plugin::traverse(TRAVERSE_AVAILABLE_OPTIONS)` 收集。
//!
//! ```text
//! session ──collect_options(parent, ctx)──▶ parent.traverse(available_options)
//!                                             ├─ session : 工作目录 / 运行模式 / 风险等级 / 心跳
//!                                             ├─ agent   : 智能体选择
//!                                             └─ model   : Model 选择
//! ```
//!
//! ## 为什么不复用 CapabilityVisitor
//!
//! 两者收集的**产物语义**完全不同：能力是「可调用对象」（工具实例 /
//! 协议适配器），选项是「可展示的数据节点」（显示信息 + 状态 + 类型）。
//! 混装进同一 visitor 会让 `list_capability` 的消费方（LLM 工具协议）
//! 被迫理解选项数据结构。故新定义 visitor trait，共享的是**收集机制**
//! （traverse 广播 + 上下文键 + 失败降级），而不是数据结构。
//!
//! ## 契约
//!
//! - 收集是**广播**：宿主对父插件调一次 `traverse`，所有插件在同一契约下
//!   按 `ctx[PATH] == TRAVERSE_AVAILABLE_OPTIONS` 判定是否贡献；
//! - 收集是**无状态**的：每次调用返回全新 visitor（节点携带本次会话的
//!   实时选中值，不可跨请求复用）；
//! - 单插件失败只记日志，不中断收集。

use crate::symbio_core::schemas::options::OptionNode;
use crate::symbio_core::{InvokeRequest, InvokeRequestExt, Plugin, PATH};
use async_trait::async_trait;
use indexmap::IndexMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// 遍历可用选项的常量路径（与 `TRAVERSE_AVAILABLE_TOOLS` 平行）
pub const TRAVERSE_AVAILABLE_OPTIONS: &str = "available_options";

/// 选项收集器 —— 各插件在 `traverse` 中把选项节点注册进来。
///
/// 语义与 [`crate::symbio_core::CapabilityVisitor`] 一致：按 id 去重、
/// 后者覆盖（保留先注册的槽位），列表按 `order` 稳定排序。
#[async_trait]
pub trait OptionVisitor: Send + Sync + 'static {
    /// 注册一个选项节点（同 id 覆盖）
    async fn register_option(&self, node: OptionNode);

    /// 批量注册（默认逐个注册）
    async fn register_batch(&self, nodes: Vec<OptionNode>) {
        for node in nodes {
            self.register_option(node).await;
        }
    }

    /// 列出已注册的选项节点（按 `order` 升序稳定排序）
    async fn list_options(&self) -> Vec<OptionNode>;
}

/// 默认选项收集器：内存 IndexMap 实现，一次收集一个实例。
pub struct DefaultOptionVisitor {
    nodes: Arc<RwLock<IndexMap<String, OptionNode>>>,
}

impl DefaultOptionVisitor {
    pub fn new() -> Self {
        Self {
            nodes: Arc::new(RwLock::new(IndexMap::new())),
        }
    }
}

impl Default for DefaultOptionVisitor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl OptionVisitor for DefaultOptionVisitor {
    async fn register_option(&self, node: OptionNode) {
        let id = node.id.clone();
        let mut nodes = self.nodes.write().await;
        nodes.insert(id, node);
    }

    async fn list_options(&self) -> Vec<OptionNode> {
        let nodes = self.nodes.read().await;
        let mut out: Vec<OptionNode> = nodes.values().cloned().collect();
        // 稳定排序：order 相同者保持注册顺序（IndexMap 保序）
        out.sort_by_key(|n| n.order);
        out
    }
}

/// 向所有插件广播「贡献选项」，返回装配好的选项收集器。
///
/// 调用方（选项宿主 = session 插件）需在 `ctx` 中预先设置好各插件判定
/// 所需的上下文键——通常是**会话当前状态**（`SESSION_ID` / `AGENT_ID` /
/// `WORKDIR` 等），贡献插件据此回填节点的 `value`（当前选中值）。
///
/// 失败降级语义与 `collect_capabilities`（`plugins/session/chat_pipeline.rs`）一致：
/// 父插件缺失返回空收集器，单个插件 traverse 失败只记日志。
pub async fn collect_options(
    parent: Option<&Arc<dyn Plugin>>,
    ctx: &Arc<dyn InvokeRequest>,
) -> Arc<dyn OptionVisitor> {
    let visitor: Arc<dyn OptionVisitor> = Arc::new(DefaultOptionVisitor::new());

    let Some(parent) = parent else {
        return visitor;
    };

    let traverse_ctx = ctx.fork();
    traverse_ctx.set(PATH, TRAVERSE_AVAILABLE_OPTIONS.to_string());
    traverse_ctx.set(crate::symbio_core::OPTION_VISITOR, visitor.clone());

    if let Err(e) = parent
        .clone()
        .traverse(String::new(), traverse_ctx.clone())
        .await
    {
        crate::plugin_warn!(
            "core",
            "collect_options: traverse 失败（选项集可能不完整）: {:?}",
            e
        );
    }

    visitor
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbio_core::schemas::options::{OptionAction, OptionType};
    use crate::symbio_core::vdfs_provider::VDFS_STATUS_DISABLED;

    #[tokio::test]
    async fn register_dedup_and_order() {
        let v = DefaultOptionVisitor::new();
        v.register_option(OptionNode::new("b", "B", OptionType::Invoke).with_order(20))
            .await;
        v.register_option(OptionNode::new("a", "A", OptionType::Invoke).with_order(10))
            .await;
        // 同 id 覆盖（保留先注册槽位），order 生效
        let mut over = OptionNode::new("b", "B2", OptionType::Sub).with_order(20);
        over.status = VDFS_STATUS_DISABLED.to_string();
        v.register_option(over).await;

        let list = v.list_options().await;
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, "a");
        assert_eq!(list[1].id, "b");
        assert_eq!(list[1].label, "B2");
        assert_eq!(list[1].option_type, OptionType::Sub);
    }

    #[tokio::test]
    async fn register_batch_appends() {
        let v = DefaultOptionVisitor::new();
        let action = OptionAction {
            endpoint: "worker/session/update".to_string(),
            ..Default::default()
        };
        v.register_batch(vec![
            OptionNode::invoke("x", "X", action.clone()),
            OptionNode::invoke("y", "Y", action),
        ])
        .await;
        let ids: Vec<String> = v.list_options().await.into_iter().map(|n| n.id).collect();
        assert_eq!(ids, vec!["x", "y"]);
    }

    #[tokio::test]
    async fn collect_without_parent_returns_empty() {
        let ctx: Arc<dyn InvokeRequest> =
            Arc::new(crate::symbio_core::SimpleRequest::new(None, None));
        let v = collect_options(None, &ctx).await;
        assert!(v.list_options().await.is_empty());
    }
}
