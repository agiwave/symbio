//! 能力表的**行词汇**——收集口 trait 收敛为 `put` / `get` / `list` 三方法后，
//! 一张表装下全部收集通道（AR-025 §3 的目标形状）。
//!
//! ## 本批只落地 Tool 一行，其余五值是**词汇**
//!
//! [`CapabilityRowKind`] 有六个值，但 [`CapabilityRowPayload`] 本批只有
//! `Tool` 一个变体——其余随 4b–4e 各批次落地，`Policy` 随步骤 6（表搬进 core）
//! 与 `State` / `Step` 一并定义。提前把六值写齐的理由：`list(kind)` 的签名
//! 由此在 4b–4e **不再变形**（加一个族 = 加一个 payload 变体，不是加三个方法）；
//! 而不写齐则每一步都要改签名，正是十六方法糜烂的来路。
//!
//! ## 为什么 schema 不另存一份
//!
//! 工具行的元数据（`CapabilityMeta`）**从能力自述派生**：`Row::tool(cap)` 在入表
//! 时取一次 `cap.meta()` 的 name 作键，`row.meta()` 每次现取。两份表示（表里的
//! 键与能力自述的 name）只可能在**改名装饰器**上分叉，而那种装饰器
//! （`PrefixedCapability`）改的正是 `meta().name` 本身——现取永远同步。
//!
//! ## 谁住在这里
//!
//! 与 `CapabilityMeta` 同域同理：全部收集通道（工具 / 模型服务 / 提示词 /
//! 挂载 / 选项）共用同一张表，契约是**跨插件**的，故类型在 core；表的默认实现
//! （`DefaultToolVisitor`）在 `providers/collectors`，策略行在 session 的
//! `loop_registry`——三处各住各的，见 AR-025 §3。

use crate::symbio_core::{Capability, CapabilityMeta};
use std::sync::Arc;

/// 行的族——`list(kind)` 的过滤键，也是「这张表装了什么」的清单。
///
/// 六个值的落位时序见本模块文档；本批只有 [`CapabilityRowKind::Tool`] 可达。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CapabilityRowKind {
    /// 策略行：纯决策（`fn(&State) -> Option<Step>`），无副作用。
    /// **本批不存在**——它是 session 的私有知识，随步骤 6 入表。
    Policy,
    /// 能力行：LLM 可见工具（可调用对象）。
    Tool,
    /// 模型服务（生效者单槽 + 已启用目录）。
    Model,
    /// 系统提示词。
    Prompt,
    /// VDFS 挂载点。
    Mount,
    /// 选项字段（会话输入区的可选项）。
    Option,
}

/// 行的载荷——「这一行到底是什么」。
///
/// 本批只有 [`CapabilityRowPayload::Tool`]；每个后续族在此加一个变体，
/// **不在 trait 上加方法**——后者是十六方法形态的成因。
#[derive(Clone)]
pub enum CapabilityRowPayload {
    /// 工具：可调用对象（注册收口点已裹审批闸，见 `DefaultToolVisitor::put`）。
    Tool(Arc<dyn Capability>),
}

/// 表里的一行。
///
/// 两个字段，各是一个事实：**名字**（表键）与**载荷**（这一行到底是什么）。
/// 族别不单独存——[`Self::kind`] 从载荷派生，结构上不存在「两种答案」（若另存
/// 一格，就得防它俩哪天不一致，而那正是不一致的定义）。
///
/// 构造只走 [`CapabilityRow::tool`]：行的名字从能力自述派生，调用方没有机会
/// 填错。读取侧三件套——[`Self::kind`] / [`Self::meta`] / [`Self::cap`] 覆盖
/// 全部现行消费形态（列表喂模型、按名执行、声明位读取）。
#[derive(Clone)]
pub struct CapabilityRow {
    name: String,
    payload: CapabilityRowPayload,
}

impl CapabilityRow {
    /// 工具行：名字 = `cap.meta().name`（全名，含挂载点前缀）。
    pub fn tool(cap: Arc<dyn Capability>) -> Self {
        let name = cap.meta().name;
        Self {
            name,
            payload: CapabilityRowPayload::Tool(cap),
        }
    }

    /// 这一行属于哪个族（与载荷同源，不存在两种答案）。
    pub fn kind(&self) -> CapabilityRowKind {
        match &self.payload {
            CapabilityRowPayload::Tool(_) => CapabilityRowKind::Tool,
        }
    }

    /// 行的名字（表键；工具族即线上全名）。
    pub fn name(&self) -> &str {
        &self.name
    }

    /// 工具元数据（每次现取，见模块文档「为什么 schema 不另存一份」）。
    pub fn meta(&self) -> CapabilityMeta {
        self.cap().map(|c| c.meta()).unwrap_or_default()
    }

    /// 取可调用对象；非工具族为 `None`（调用方据此给诚实的 NotFound）。
    pub fn cap(&self) -> Option<&Arc<dyn Capability>> {
        match &self.payload {
            CapabilityRowPayload::Tool(cap) => Some(cap),
        }
    }
}

#[cfg(test)]
#[path = "row.test.rs"]
mod tests;
