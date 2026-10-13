//! 主体规格、模式与作用域（[plan/01 §4](../../../../docs/plan/PL-001-核心架构.md "PL-001-核心架构")）。

/// 主体模式（[plan/01 §4](../../../../docs/plan/PL-001-核心架构.md "PL-001-核心架构")：机制，**3 个封顶**）。
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

impl Pattern {
    /// 会话主体的**模式判据**——`ActorSpec` 生产构造点用的那一支，住在定义域内。
    ///
    /// 与授权表同源，不另立清单：持 `reply.*` ⇒ [`Pattern::Reasoner`]（会话主体按
    /// 模型驱动跑轮）；无发言权而有 `produce.artifact` ⇒ [`Pattern::Translator`]
    /// （对等主体的权威动作是**产生产物**，复现 ST-008 §3 的 ActorSpec 对等行）；
    /// 其余 ⇒ [`Pattern::Decider`]。判据问的是矩阵谓词本身（`can_reply` /
    /// `can_produce_artifact`），调用方不抄能力名清单。
    ///
    /// 为什么是**方法**而不是根导出函数：主体类型的派生不该散进插件层
    /// （no-direct-call-audit 的 NDC-001 同一条纪律——插件层只见谓词结果，不见
    /// 主体类型名），而方法挂在 [`Pattern`] 这个既有根名下，不再多占一个根出口。
    pub(crate) fn for_principal(principal: &str) -> Pattern {
        let matrix = crate::symbio_core::authz::matrix_for(principal);
        if matrix.can_reply(principal, true) {
            Pattern::Reasoner
        } else if matrix.can_produce_artifact(principal) {
            Pattern::Translator
        } else {
            Pattern::Decider
        }
    }
}

/// 作用域（[plan/01 §8](../../../../docs/plan/PL-001-核心架构.md "PL-001-核心架构")）。
///
/// `root` 是**一等输入**，不是"没有子智能体"的特例（03 §2 推论：
/// 不出现 `if (children.is_empty())` 分支）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    Root,
    /// 子智能体：挂在其父主体的作用域下。
    Child(String),
}

/// 主体规格（[plan/01 §4](../../../../docs/plan/PL-001-核心架构.md "PL-001-核心架构") 的 `ActorSpec`，五字段）。
#[derive(Debug, Clone)]
pub struct ActorSpec {
    /// 身份（数据，无限增长——如 `"agent:main"`）。
    pub principal: String,
    /// 三种模式之一（机制，封顶）。
    pub pattern: Pattern,
    /// 能力名（数据）。S3 之前不做枚举闭集与校验（归 ⑥ `governance`）。
    pub capabilities: Vec<String>,
    /// I3 时延预算（毫秒）。
    pub budget_ms: u64,
    /// 作用域（递归）。
    pub scope: Scope,
}

impl ActorSpec {
    /// S01 的平凡值主体（[roadmap/S01 §4](../../../../docs/stage/ST-001-最小闭环.md "ST-001-最小闭环")）：
    /// 全规则驱动、预算放宽、单主体——平凡值下系统完整运行。
    pub fn trivial(principal: impl Into<String>) -> Self {
        ActorSpec {
            principal: principal.into(),
            pattern: Pattern::Decider,
            capabilities: Vec::new(),
            budget_ms: 60_000,
            scope: Scope::Root,
        }
    }
}
