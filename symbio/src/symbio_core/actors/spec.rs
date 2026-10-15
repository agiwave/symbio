//! 主体规格、模式与作用域（[plan/01 §4](../../../../docs/004-plan/001-核心架构.md "PL-001-核心架构")）。

/// 主体模式（[plan/01 §4](../../../../docs/004-plan/001-核心架构.md "PL-001-核心架构")：机制，**3 个封顶**）。
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

/// 作用域（[plan/01 §8](../../../../docs/004-plan/001-核心架构.md "PL-001-核心架构")）。
///
/// `root` 是**一等输入**，不是"没有子智能体"的特例（03 §2 推论：
/// 不出现 `if (children.is_empty())` 分支）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    Root,
    /// 子智能体：挂在其父主体的作用域下。
    Child(String),
}

/// 主体规格（[plan/01 §4](../../../../docs/004-plan/001-核心架构.md "PL-001-核心架构") 的 `ActorSpec`，五字段）。
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
    /// S01 的平凡值主体（[roadmap/S01 §4](../../../../docs/003-stage/001-最小闭环.md "ST-001-最小闭环")）：
    /// 全规则驱动、预算放宽、单主体——平凡值下系统完整运行。
    ///
    /// ## `capabilities` 取该主体**自己的授权行**（[T-082](../../../../docs/009-task/082-角色行生产构造批.md "T-082-角色行生产构造批") 批①订正）
    ///
    /// 此前这里恒为空表，与「平凡值下系统**完整运行**」自相矛盾：收束闸判读规格
    /// 身上的能力集之后，空表意味着平凡值连一轮对话都收束不了。没早暴露是因为闸
    /// 当时绕道直接查授权表——字段空着也没人问，正是本批要消灭的「算了却看不见」。
    ///
    /// 订正后的平凡值 = **权限取实（该主体的登记行）、模式与预算取平凡**（规则驱动、
    /// 放宽预算、根作用域）。`agent:main` 因此持 `reply.*` 能收束，而
    /// `agent:autonomous`（只持 `define.work`）照旧无发言权——「自主行为不得冒充
    /// 用户对话」这条不因平凡值而失效。
    pub fn trivial(principal: impl Into<String>) -> Self {
        let principal = principal.into();
        ActorSpec {
            capabilities: crate::symbio_core::authz::matrix_for(&principal)
                .grant_names_of(&principal),
            pattern: Pattern::Decider,
            budget_ms: 60_000,
            scope: Scope::Root,
            principal,
        }
    }

    /// 写侧判定（收束 · **发言臂**）：本规格能否落收束格。
    ///
    /// 读的是**自己身上**的 `capabilities`（构造时从授权表行取数，`grant_names_of`），
    /// 不再回头查表——[T-082](../../../../docs/009-task/082-角色行生产构造批.md "T-082-角色行生产构造批") 批①：
    /// 规格字段此前"算了却看不见"，写侧闸判一律直接问 `matrix`，两者是同一数据的
    /// 两个入口。判定收归规格后，**判的对象（规格）与构造的来源（表行）有了单一链条**。
    ///
    /// 「轮次位置 → 能力」的映射不在这里复述：借用
    /// [`crate::symbio_core::governance`] 里那一个（`reply_capability`）——口径只有一份。
    pub fn can_land_speech(&self, first: bool) -> bool {
        let cap = crate::symbio_core::governance::Capability::reply_capability(first);
        self.holds(cap.name())
    }

    /// 写侧判定（收束 · **产物臂**）：本规格能否落 `artifact.added`。
    ///
    /// 与 [`Self::can_land_speech`] 并列而**不是**它的分支（ADR-067）：对等主体只持
    /// `produce.artifact` ⇒ 发言臂拒、产物臂放，它因此能收束为一件产物。
    pub fn can_land_artifact(&self) -> bool {
        self.holds(crate::symbio_core::governance::Capability::ProduceArtifact.name())
    }

    /// 写侧判定（S07 **抢占判定者**）：本规格能否落控制事实（`task.held` /
    /// `control`）。
    ///
    /// 与收束两臂并列的第三个能力读方：`judge.intent` 是判定者的**全部**权威
    /// （[S07 §5](../../../../docs/003-stage/007-插话与实时打断.md "ST-007-插话与实时打断")
    /// 「判定者不得直接发言」——它没有 `reply.*`，只有 `judge.intent`，由
    /// `authz::ROWS` 那一行保证）。此前该字段在生产里"算了却看不见"：`inbox`
    /// 直驱判定者时从不读规格，写控制事实也不查 `judge.intent`
    /// （[T-082](../../../../docs/009-task/082-角色行生产构造批.md "T-082-角色行生产构造批") 批②）。
    pub fn can_judge_intent(&self) -> bool {
        self.holds(crate::symbio_core::governance::Capability::JudgeIntent.name())
    }

    /// 写侧判定（S13 **仲裁者**）：本规格能否落裁决事实（`verdict.asserted`）。
    ///
    /// 与 [`Self::can_judge_intent`] 同一条纪律的第四个能力读方：`assert.verification`
    /// 是仲裁者的**全部**权威（S13 §3 三角色行），C15 构造期保证持它的那一行不持
    /// `define.work` / `produce.artifact`（不成对自验）。轮末淘汰步
    /// （`plugins/session/v2_facts::record_elimination`）写裁决前读它——缺能力 ⇒
    /// 一条裁决都不写（fail-closed：没有资格的判定不该留下事实）。
    pub fn can_assert_verification(&self) -> bool {
        self.holds(crate::symbio_core::governance::Capability::AssertVerification.name())
    }

    /// 能力集是否含该名。名字对错由**构造期**兜住（`from_names` 认不出即拒绝整体
    /// 构造）——读侧照单全收，不把拼写问题变成第二处判定。
    fn holds(&self, name: &str) -> bool {
        self.capabilities.iter().any(|c| c == name)
    }
}
