//! 测试替身：core 的测试面不再出根。

use symbio_platform::kernel::actors::{ActorSpec, Pattern, Scope};
use symbio_platform::sdk::authz::production_matrix;

/// 平凡值主体：权限取该主体的真实授权行，模式与预算取平凡值。
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
