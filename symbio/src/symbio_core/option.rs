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
//! ## 产物是 `DetailField`，不是自成一体的节点类型
//!
//! 选项（会话输入区下方的可修改项）**就是会话配置表单的字段**，因此产物直接
//! 复用 VDFS 详情方言的 [`DetailField`]——「候选」「条件显隐/禁用」「子表单」
//! 各只有一份实现与一个校验器。这里曾经并列一套 `OptionNode`（自带 `option_type`
//! / `action` / `children` / `display` 的独立节点类型）与 `options/list` 端点，
//! 已于 2026-09-23 随「会话选项 schema 化」整体下线
//! （`docs/archive/session-options-unification.md` §9 S4）。
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
//! - 收集是**无状态**的：每次调用返回全新 visitor（产物是「有哪些字段 + 候选
//!   有哪些」的**声明**，不含任何会话状态，故本可跨请求复用；不缓存只是因为
//!   候选集来自运行期数据）；
//! - 单插件失败只记日志，不中断收集。

use crate::symbio_core::schemas::detail::DetailField;
use crate::symbio_core::{InvokeRequest, InvokeRequestExt, Plugin, PATH};
use async_trait::async_trait;
use indexmap::IndexMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// 遍历可用选项的常量路径（与 `TRAVERSE_AVAILABLE_TOOLS` 平行）
pub const TRAVERSE_AVAILABLE_OPTIONS: &str = "available_options";

/// 选项收集器 —— 各插件在 `traverse` 中把选项注册进来。
///
/// 语义与 [`crate::symbio_core::CapabilityVisitor`] 一致：按 id 去重、
/// 后者覆盖（保留先注册的槽位），列表按 `order` 稳定排序。
#[async_trait]
pub trait OptionVisitor: Send + Sync + 'static {
    /// 注册一个选项字段（同 `key` 覆盖）。
    ///
    /// `order` **不下发**：它只是跨插件排序用的号段约定（插件之间不可见，只能
    /// 约定数字，见 `plugins/session/options.rs` 模块文档），收集层用完即弃。
    /// 前端收到的是一个**已排好序**的数组——数组序 = 展示序，比「各自按 order
    /// 再排一次」是更强的保证。
    async fn register_option_field(&self, order: i32, field: DetailField);

    /// 列出已注册的选项字段（按 `order` 升序稳定排序，`order` 已剥离）
    async fn list_option_fields(&self) -> Vec<DetailField>;
}

/// 默认选项收集器：内存 IndexMap 实现，一次收集一个实例。
pub struct DefaultOptionVisitor {
    fields: Arc<RwLock<IndexMap<String, (i32, DetailField)>>>,
}

impl DefaultOptionVisitor {
    pub fn new() -> Self {
        Self {
            fields: Arc::new(RwLock::new(IndexMap::new())),
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
    async fn register_option_field(&self, order: i32, field: DetailField) {
        let key = field.key.clone();
        let mut fields = self.fields.write().await;
        fields.insert(key, (order, field));
    }

    async fn list_option_fields(&self) -> Vec<DetailField> {
        let fields = self.fields.read().await;
        let mut out: Vec<(i32, DetailField)> = fields.values().cloned().collect();
        // 稳定排序：order 相同者保持注册顺序（IndexMap 保序）
        out.sort_by_key(|(order, _)| *order);
        out.into_iter().map(|(_, field)| field).collect()
    }
}

/// 向所有插件广播「贡献选项」，返回装配好的选项收集器。
///
/// 调用方（选项宿主 = session 插件）需在 `ctx` 中预先设置好各插件判定
/// 所需的上下文键——通常是**运行期可枚举的数据源**（如 agent 目录、Provider
/// 表）的定位依据，贡献插件据此算出**候选集**。
///
/// ⚠️ 「当前选中值」**不在**这里回填：值随会话节点 `attributes.metadata` 下发，
/// 定义只声明「有哪些字段与候选」（`docs/archive/session-options-unification.md`
/// §3.2 / §6）。所以宿主不需要为回填值而注入会话状态。
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
#[path = "option.test.rs"]
mod tests;
