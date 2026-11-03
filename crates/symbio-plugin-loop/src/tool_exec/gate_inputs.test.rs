//! `GateInputs::of` 的四格中性读数：授权矩阵、`cost.ledger` 台账、两档预算。
//!
//! 判据那一半（时延 / 预算 / 熔断事实）住在 `symbio-plugin-tool` 的执行体；本文件只
//! 钉环侧交出去的**值**（DE-039 §6，DE-048 D-311）。

use super::*;
use symbio_kernel::adapters::LatencyTier;
use symbio_kernel::event::{Event, Seq};
use symbio_kernel::plugin::PluginSimpleRequest;
use symbio_kernel::store::{EventStore, Store};

fn test_ctx() -> Arc<dyn PluginInvokeRequest> {
    Arc::new(PluginSimpleRequest::new(None, None))
}

/// 未绑智能体（无 `AGENT_ID`）的上下文 ⇒ 会话主主体。
fn ctx_with_agent(agent_id: &str) -> Arc<dyn PluginInvokeRequest> {
    let req = PluginSimpleRequest::new(None, None);
    req.set(symbio_sdk::keys::AGENT_ID, agent_id.to_string());
    Arc::new(req)
}

/// 判据那一半：授权是**读矩阵**得来的（fail-closed），不是默认放行。
///
/// `agent:*` 身份会话在会话装配点经绑定登记拿到会话面缺省行（含
/// `produce.artifact`，D-100）——本用例直读合流面（不经装配点），取的是部署表那一行
/// 的判据；`Refuse` 只可能来自**不属于这一族**的主体——判据为 false 就是 false，没有
/// 任何「认不出 ⇒ 放行」的旁路。
#[test]
fn external_execution_capability_is_read_from_the_matrix() {
    assert!(
        symbio_sdk::authz::production_matrix().can_write_name(
            symbio_sdk::authz::PRINCIPAL_MAIN,
            symbio_sdk::authz::CAP_EXTERNAL_EXECUTION
        ),
        "agent:main 持外部执行能力（本机部署事实）"
    );
    assert!(
        !symbio_sdk::authz::production_matrix().can_write_name(
            symbio_sdk::authz::PRINCIPAL_USER,
            symbio_sdk::authz::CAP_EXTERNAL_EXECUTION
        ),
        "非 agent 主体不持外部执行能力 ⇒ 判据 false ⇒ Refuse（fail-closed）"
    );

    assert!(
        GateInputs::of(&[], &test_ctx()).authorized,
        "未绑智能体 ⇒ 会话主主体 ⇒ 持能力"
    );
    assert!(
        !GateInputs::of(&[], &ctx_with_agent("person/local")).authorized,
        "非 agent 主体经 `of` 读出来必须是 false，而不是默认放行"
    );
}

/// 台账读的是**交给执行体的事实值**，两档预算按 D-044 的口径取静态表。
///
/// 历史事故：曾有实现按 `<插件目录>/v2-events.wal` 取账（漏了会话 id 那层）⇒
/// `spent_ms` 恒 0 ⇒ 预算分支永远走不到，且没有任何东西会变红。D-084 §1-A
/// 之后「读哪份事实」收进会话存储层，本层拿的是值：**空值 ⇒ 零账，带成本格的值
/// ⇒ 照实读出**。判据两侧都钉在值上，路径错位在这一层已无立足点。
#[tokio::test]
async fn the_ledger_comes_from_the_facts_value() {
    let empty: Vec<Event> = Vec::new();
    let none = GateInputs::of(&empty, &test_ctx());
    assert_eq!(none.spent_ms, 0, "还没记过账 ⇒ 零账（不是「不许执行」）");
    assert_eq!(
        none.requested_ms,
        LatencyTier::Deep.budget_ms(),
        "本次申请 = 工具循环所在档的额度"
    );
    assert_eq!(
        none.budget_ms,
        LatencyTier::Autonomic.budget_ms(),
        "总预算 = 会话累计成本天花板"
    );

    let facts = EventStore::new();
    facts
        .append(
            symbio_kernel::event::Event::pending(
                "v2f-ledger",
                symbio_plugin_api::events::EVENT_CHAT_ASSISTANT_FINAL,
                symbio_kernel::event::Entity::Turn,
                symbio_kernel::event::Verb::Closed,
                0,
                symbio_sdk::authz::PRINCIPAL_MAIN,
            )
            .with_cost_ms(LatencyTier::Autonomic.budget_ms() + 1),
        )
        .expect("入格");
    assert_eq!(
        GateInputs::of(&facts.range(Seq::new(0)), &test_ctx()).spent_ms,
        LatencyTier::Autonomic.budget_ms() + 1,
        "值里的账必须被读出来（否则预算分支永远走不到）"
    );
}
