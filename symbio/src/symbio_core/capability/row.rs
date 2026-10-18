//! 能力表的**行词汇**——收集口 trait 收敛为 `put` / `get` / `list` 三方法后，
//! 一张表装下全部收集通道（AR-025 §3 的目标形状）。
//!
//! ## 族是**一个一个**落位的（Tool ✓ → Model ✓ → Prompt ✓ → Mount → Option）
//!
//! [`CapabilityRowKind`] 有六个值，但 [`CapabilityRowPayload`] 只随各族落地长变体：
//! 4a 是 `Tool`，4b 加 `Model`，其余随 4c–4e；`Policy` 随步骤 6（表搬进 core）
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
//! ## 生效位（`active`）：单槽族把「谁在槽里」也变成行的一部分
//!
//! 模型族的注册口原本是**一次批量**（生效者单槽 + 已启用目录），理由是
//! 「生效者必然是目录里的一员——拆成两次注册就允许了它不在目录里这个非法状态」。
//! 表只有 `put` / `get` / `list` 三个方法，于是那条不变量改由**结构**承载：生效不是
//! 一个与目录并列的槽，而是**目录里某一行的位**——[`CapabilityRow::model`] 的
//! `active` 参数。生效者不在目录里从此不可表达。
//!
//! 唯一生效由写入方维持：`put` 收到带生效位的行时，先把同族旧生效行降为普通行
//! （[`CapabilityRow::demote`]），见 `DefaultToolVisitor::put` 的单槽移交。
//!
//! ## 谁住在这里
//!
//! 与 `CapabilityMeta` 同域同理：全部收集通道（工具 / 模型服务 / 提示词 /
//! 挂载 / 选项）共用同一张表，契约是**跨插件**的，故类型在 core；表的默认实现
//! （`DefaultToolVisitor`）在 `providers/collectors`，策略行在 session 的
//! `loop_registry`——三处各住各的，见 AR-025 §3。

use crate::symbio_core::{Capability, CapabilityMeta, ModelProvider};
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
/// 每个族在此加一个变体，**不在 trait 上加方法**——后者是十六方法形态的成因。
#[derive(Clone)]
pub enum CapabilityRowPayload {
    /// 工具：可调用对象（注册收口点已裹审批闸，见 `DefaultToolVisitor::put`）。
    Tool(Arc<dyn Capability>),
    /// 模型服务：一个已启用的 provider。
    ///
    /// `active` 是**生效位**（单槽族的「谁在槽里」）：会话引擎取的是它，插件按 id
    /// 取的是不带位的行——两者同表，故「生效者不在目录里」不可表达（见模块文档）。
    Model {
        provider: Arc<dyn ModelProvider>,
        active: bool,
    },
    /// 系统提示词：一段文字，行名就是段名。
    ///
    /// 段名是**覆盖键**，不是分类——不要用命名前缀去表达「这是人格」「这是记忆」
    /// （`capability/prompt.rs` 的模块文档展开这条）。
    Prompt { text: String },
}

/// 表里的一行。
///
/// 两个字段，各是一个事实：**名字**（表键）与**载荷**（这一行到底是什么）。
/// 族别不单独存——[`Self::kind`] 从载荷派生，结构上不存在「两种答案」（若另存
/// 一格，就得防它俩哪天不一致，而那正是不一致的定义）。
///
/// 构造只走 [`CapabilityRow::tool`] / [`CapabilityRow::model`] / [`CapabilityRow::prompt`]：
/// 工具与模型行的名字从载荷自述派生（能力的 `meta().name` / provider 的 `provider_id()`），
/// 提示词行的名字是调用方给的**覆盖键**，调用方没有别的口子可填。
/// 读取侧三件套——[`Self::kind`] / [`Self::meta`] / [`Self::cap`] 覆盖工具族的
/// 全部现行消费形态（列表喂模型、按名执行、声明位读取），模型族走 [`Self::provider`]。
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

    /// 模型行：名字 = `provider.provider_id()`。
    ///
    /// `active` 只由**写出整批的那一方**填（`capability::model::register`）：它知道
    /// 这一批里哪个是当次生效者。表把「唯一生效」接过去，见 [`Self::demote`]。
    pub fn model(provider: Arc<dyn ModelProvider>, active: bool) -> Self {
        let name = provider.provider_id().to_string();
        Self {
            name,
            payload: CapabilityRowPayload::Model { provider, active },
        }
    }

    /// 系统提示词行：名字 = 段名（覆盖键）。
    pub fn prompt(name: &str, text: String) -> Self {
        Self {
            name: name.to_string(),
            payload: CapabilityRowPayload::Prompt { text },
        }
    }

    /// 这一行属于哪个族（与载荷同源，不存在两种答案）。
    pub fn kind(&self) -> CapabilityRowKind {
        match &self.payload {
            CapabilityRowPayload::Tool(_) => CapabilityRowKind::Tool,
            CapabilityRowPayload::Model { .. } => CapabilityRowKind::Model,
            CapabilityRowPayload::Prompt { .. } => CapabilityRowKind::Prompt,
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
            CapabilityRowPayload::Model { .. } | CapabilityRowPayload::Prompt { .. } => None,
        }
    }

    /// 取模型服务；非模型族为 `None`。
    pub fn provider(&self) -> Option<&Arc<dyn ModelProvider>> {
        match &self.payload {
            CapabilityRowPayload::Model { provider, .. } => Some(provider),
            CapabilityRowPayload::Tool(_) | CapabilityRowPayload::Prompt { .. } => None,
        }
    }

    /// 取系统提示词正文；非提示词族为 `None`。
    pub fn prompt_text(&self) -> Option<&str> {
        match &self.payload {
            CapabilityRowPayload::Prompt { text } => Some(text),
            CapabilityRowPayload::Tool(_) | CapabilityRowPayload::Model { .. } => None,
        }
    }

    /// 这一行是否占着**生效位**（单槽族的「谁在槽里」）。
    ///
    /// 无槽的族恒 `false`：目前只有模型族有位（挂载根的槽随 4d 落位）。
    pub fn active(&self) -> bool {
        match &self.payload {
            CapabilityRowPayload::Model { active, .. } => *active,
            CapabilityRowPayload::Tool(_) | CapabilityRowPayload::Prompt { .. } => false,
        }
    }

    /// 同一行的**退位副本**（生效位清零；无位的族原样返回）。
    ///
    /// 单槽族的写入方在收下新生效行前用它降下旧行——表的行是 `Arc`，就地改位等于
    /// 让读者看见两次变化；换一个副本入表，读出的一直是一份完整的行。
    pub fn demote(&self) -> Self {
        match &self.payload {
            CapabilityRowPayload::Model { provider, .. } => Self {
                name: self.name.clone(),
                payload: CapabilityRowPayload::Model {
                    provider: Arc::clone(provider),
                    active: false,
                },
            },
            CapabilityRowPayload::Tool(_) | CapabilityRowPayload::Prompt { .. } => self.clone(),
        }
    }
}

#[cfg(test)]
#[path = "row.test.rs"]
mod tests;
