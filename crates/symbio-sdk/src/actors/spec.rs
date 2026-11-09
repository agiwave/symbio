//! 主体规格的**授权读法**：平凡值、模式派生与能力位判定（宿主侧自由函数）
//!
//! `ActorSpec` / `Pattern` / `Scope` 的数据面已随 D-188 C2b 住 `symbio_kernel::actors`；
//! 本模块只剩三个读本机**部署表**的动作：平凡值构造（`trivial_actor_spec`）、模式派生
//! （`pattern_for_principal`）与能力位判定（`actor_can_*`）。
//!
//! ## 为什么是自由函数而不是方法
//!
//! 两条约束同时成立：宿主 crate 不能为内核类型写固有 `impl`（orphan 规则），而
//! `authz` / `governance` 是**部署表**而不是语法（D-099），不能进内核
//! （[DE-023 D-188](../../../../docs/003-design/023-内核边界与扩展局部性.md "DE-023-内核边界与扩展局部性")）。
//! 于是「数据在核内、读法在宿主」只能以自由函数表达——判的对象（规格）与构造的来源
//! （表行）仍是单一链条，只是链条跨了 crate 边界。

use symbio_kernel::actors::{ActorSpec, Pattern};
// `Scope` 只被 `trivial_actor_spec`（`cfg(test)`）用到 ⇒ 非测试构建里引它必报 unused。
#[cfg(test)]
use symbio_kernel::actors::Scope;

/// 模式判据——`ActorSpec` 生产构造点用的那一支，住在定义域内。
///
/// 与授权表同源，不另立清单：持 `reply.*` ⇒ [`Pattern::Reasoner`]（会话主体按
/// 模型驱动跑轮）；无发言权而有 `produce.artifact` ⇒ [`Pattern::Translator`]
/// （对等主体的权威动作是**产生产物**，复现 §3 的 ActorSpec 对等行）；
/// 其余 ⇒ [`Pattern::Decider`]。判据问的是矩阵谓词本身（`can_reply` /
/// `can_produce_artifact`），调用方不抄能力名清单。
///
/// 为什么住在 core 而不是内核：`authz` 是部署表（D-099），主体派生不该散进插件层
/// （no-direct-call-audit 的 NDC-001 同一条纪律——插件层只见谓词结果，不见主体类型名）。
pub fn pattern_for_principal(principal: &str) -> Pattern {
    let matrix = crate::authz::production_matrix();
    if matrix.can_reply(principal) {
        Pattern::Reasoner
    } else if matrix.can_produce_artifact(principal) {
        Pattern::Translator
    } else {
        Pattern::Decider
    }
}

/// 平凡值主体（[DE-031 §4](../../../../docs/003-design/031-最小闭环.md "DE-031-最小闭环")）：
/// 全规则驱动、预算放宽、单主体——平凡值下系统完整运行。
///
/// **测试面**构造点（ADR-067：生产按真实授权行立规格）；`cfg(test)` 挂在这里，
/// 非测试构建里没有消费者。
///
/// ## `capabilities` 取该主体**自己的授权行**
///
/// 空表与「平凡值下系统**完整运行**」自相矛盾：收束闸判读规格身上的能力集之后，
/// 空表意味着平凡值连一轮对话都收束不了。所以它必须是**真值**，不能是「算了却
/// 看不见」的占位。
///
/// 平凡值 = **权限取实（该主体的登记行）、模式与预算取平凡**（规则驱动、
/// 放宽预算、根作用域）。`session/main` 因此持 `reply.*` 能收束，而
/// `session/autonomous`（只持 `define.work`）照旧无发言权——「自主行为不得冒充
/// 用户对话」这条不因平凡值而失效。
#[cfg(test)]
pub fn trivial_actor_spec(principal: impl Into<String>) -> ActorSpec {
    let principal = principal.into();
    ActorSpec {
        capabilities: crate::authz::production_matrix().grant_names_of(&principal),
        pattern: Pattern::Decider,
        budget_ms: 60_000,
        scope: Scope::Root,
        principal,
    }
}

/// 写侧判定（**调度者**）：本规格能否落发放事实（`task.ready` /
/// `task.assigned`）。
///
/// 读的是**自己身上**的 `capabilities`（构造时从授权表行取数，`grant_names_of`），
/// 不再回头查表——写侧闸判若直接问 `matrix`，同一数据就有两个入口。判定收归规格后，
/// **判的对象（规格）与构造的来源（表行）有了单一链条**。
///
/// `assign.work` 是调度者的**全部**权威（DE-034 §3 的 ActorSpec 行只持它）；缺它 ⇒
/// 调度者一条事实都不写（fail-closed）——没有资格的调度不该留下「我派过」的痕迹。
pub fn actor_can_assign_work(spec: &ActorSpec) -> bool {
    holds(spec, crate::governance::Capability::AssignWork.name())
}

/// 能力集是否含该名。名字对错由**构造期**兜住（`from_names` 认不出即拒绝整体
/// 构造）——读侧照单全收，不把拼写问题变成第二处判定。
fn holds(spec: &ActorSpec, name: &str) -> bool {
    spec.capabilities.iter().any(|c| c == name)
}
