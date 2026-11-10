//! 测试替身：core 的测试面**不再出根**（[DE-023](https://example.invalid) A1 之后的债）。
//!
//! ## 为什么它们住这里而不是 core
//!
//! 拆 crate（D-191）之后，`#[cfg(test)]` 在 core 里**只在编译 core 自己的测试时生效**，
//! 别的 crate 依赖它时那个分支根本不存在——所以测试替身放 core 就只能常开为 `pub`，
//! 把 C-004 账本撑高一截（那是虚高，不是共享面）。
//!
//! 落点是这里：`symbio-testkit` 是**普通 crate**，插件以 `[dev-dependencies]` 引入，
//! 生产构建根本看不见它。测试替身归测试设施，core 的公开面因此只剩「真被两处以上
//! 生产代码看见」的东西。
//!
//! ## 消费方
//!
//! | 替身 | 消费方 |
//! |---|---|
//! | [`trivial_actor_spec`] | `symbio-plugin-task`（`task`）· `symbio-plugin-session`（`store/wal`） |
//!
//! ## 模型桩为什么退场（`D-438`）
//!
//! 这里曾住着一份实现 `LlmAdapter` 的零 LLM 桩（`StubLlmAdapter`），服务于「彩排真实
//! 执行形状」那组用例。`LlmAdapter` 本身在 `D-437` 之后失去了生产实现——模型往返走
//! `port.model` 的执行体，`sdk::adapters` 也随之整块退场——桩就再没有对象可替：模型
//! 调用的闭环由 e2e 的 `mock-llm` 承接，桩再造一份「假模型」只会让测试以为自己在验接线。

use symbio_kernel::actors::{ActorSpec, Pattern, Scope};
use symbio_sdk::authz::production_matrix;

/// 平凡值主体（[DE-031 §4](../../../docs/003-design/031-最小闭环.md "DE-031-最小闭环")）：
/// 全规则驱动、预算放宽、单主体——平凡值下系统完整运行。
///
/// **测试面**构造点（ADR-067：生产按真实授权行立规格）。它住测试设施而不是
/// 共享层：`cfg(test)` 对下游 crate 不生效（见本模块文档），放 core 就只能
/// 以 `pub` 常开，那是把测试面的开销记到公开面账上。
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
pub fn trivial_actor_spec(principal: impl Into<String>) -> ActorSpec {
    let principal = principal.into();
    ActorSpec {
        capabilities: production_matrix().grant_names_of(&principal),
        pattern: Pattern::Decider,
        budget_ms: 60_000,
        scope: Scope::Root,
        principal,
    }
}
