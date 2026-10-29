//! 主体规格的**数据面**：`ActorSpec` 五字段、模式闭集、作用域（DE-023 D-188 C2b）
//!
//! 这里只有形状，没有授权读法：`trivial`（平凡值）与 `can_*`（能力位判定）读的是
//! 本机**部署表**（`authz` / `governance`），它们不是语法而是宿主事实（D-099），
//! 因此留在 `symbio_core::actors` 作为自由函数（`trivial_actor_spec` /
//! `actor_can_land_speech` …），不进内核。

/// 主体模式（[DE-001 §4](../../../docs/003-design/001-核心架构.md "DE-001-核心架构")：机制，**3 个封顶**）。
///
/// 模式选择的判据是函数不是清单：可写成确定性规则 → [`Pattern::Decider`]；
/// 只需重述已有事实 → [`Pattern::Translator`]；否则 → [`Pattern::Reasoner`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pattern {
    /// 规则 / 代码：确定，毫秒级（意图判定、就绪判定、权限、熔断、仲裁）。
    Decider,
    /// LLM：非确定，秒–分钟（规划、生成、语义判定、工具调用）。
    Reasoner,
    /// 事件 → 另一种表示：确定，百毫秒。**不创造信息**（输出可追溯到输入事件）。
    Translator,
}

/// 作用域（[DE-001 §8](../../../docs/003-design/001-核心架构.md "DE-001-核心架构")）。
///
/// `root` 是**一等输入**，不是"没有子智能体"的特例（推论：
/// 不出现 `if (children.is_empty())` 分支）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    Root,
    /// 子智能体：挂在其父主体的作用域下。
    Child(String),
}

/// 主体规格（[DE-001 §4](../../../docs/003-design/001-核心架构.md "DE-001-核心架构") 的 `ActorSpec`，五字段）。
#[derive(Debug, Clone)]
pub struct ActorSpec {
    /// 身份（数据，无限增长——如 `"session/main"`）。
    pub principal: String,
    /// 三种模式之一（机制，封顶）。
    pub pattern: Pattern,
    /// 能力名（数据）。这里不做枚举闭集与校验（归 ⑥ `governance`）。
    pub capabilities: Vec<String>,
    /// I3 时延预算（毫秒）。
    pub budget_ms: u64,
    /// 作用域（递归）。
    pub scope: Scope,
}
