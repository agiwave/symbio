//! 选项收集机制（跨插件共享设施）
//!
//! ## 与能力收集机制的关系
//!
//! 会话能力（工具 / 模型服务 / 系统提示词）经 `CapabilityVisitor` +
//! `Plugin::traverse(TRAVERSE_AVAILABLE_TOOLS)` 收集。
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
//! ## 本模块只有**契约**：收集器实现与收集管线都在 session
//!
//! 本域只提供一个符号，且它是**多消费方**的：
//!
//! | 符号 | 消费方 | 为什么在这里 |
//! |---|---|---|
//! | [`OptionVisitor`] | session（实现默认收集器 + 读产物）· agent · model（各自在 `traverse` 里注册字段） | 它是上下文键 `OPTION_VISITOR` 的**值类型**；键在 core，值类型只能同在 core |
//!
//! 端点字面量
//! [`TRAVERSE_AVAILABLE_OPTIONS`](crate::symbio_core::TRAVERSE_AVAILABLE_OPTIONS)
//! **不在这里**：它属于 `Plugin::traverse` 的契约，与 `TRAVERSE_AVAILABLE_TOOLS` 同处
//! `plugin::traverse`——同一份协议的两个端点该住在一起。本域只是那条通道的**产出方之一**。
//!
//! 而**收集器实现**（`DefaultOptionVisitor`）住 `providers/collectors/`——
//! 它的**写入者是全体插件**（在 `traverse` 里注册字段），安装它的宿主只是其中之一，
//! 故不隶属于任何插件；契约（trait）在 core，无策略的内存实现在实现层。
//! **收集管线**（`collect_options`）只有 session 一个消费方，按「依赖方数量」判据
//! （[ADR-023](../../../../docs/DECISIONS.md)）已下沉到 `plugins/session/options.rs`
//! —— 与它的平行物 `collect_capabilities` 同处一地（后者一直在 session 的
//! `chat_pipeline.rs` 里）。
//!
//! ```text
//! 在 core：契约（trait + 端点字面量）        ← 两侧都认
//! 在 providers：默认实现（内存收集器）        ← 写入者是全体插件，无宿主归属
//! 在 session：收集管线（collect_options）    ← 只有宿主认
//! ```
//!
//! ## 产物是 `DetailField`，不是自成一体的节点类型
//!
//! 选项（会话输入区下方的可修改项）**就是会话配置表单的字段**，因此产物直接
//! 复用 VDFS 详情方言的 [`DetailField`](crate::symbio_core::schemas::detail::DetailField)
//! ——「候选」「条件显隐/禁用」「子表单」各只有一份实现与一个校验器。这里曾经并列一套
//! `OptionNode`（自带 `option_type` / `action` / `children` / `display` 的独立节点类型）
//! 与 `options/list` 端点，已于 2026-09-23 随「会话选项 schema 化」整体下线
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

/// 选项收集器 —— 各插件在 `traverse` 中把选项注册进来。
///
/// 语义与 [`crate::symbio_core::CapabilityVisitor`] 一致：按 id 去重、
/// 后者覆盖（保留先注册的槽位），列表按 `order` 稳定排序。
///
/// 实现（默认收集器）在 `plugins/session/options.rs`：本 trait 只有**一个**
/// 实现，而它只被会话宿主使用——实现跟着宿主走，契约留在两侧都能看见的地方。
#[async_trait::async_trait]
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
