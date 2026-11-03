//! 拒止骨架与工具批处理的单元测试（原 `gates.test.rs` + `tool_executor.test.rs` 的批处理段）。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。开窗判据（时延 / 预算）、
//! 协议判定与 Pre/PostToolUse 钩子都已搬进 `symbio-plugin-tool` 的执行体，本文件
//! 只钉**映射不猜**：声明闸（能力包的 `before`）的出口经 [`Refusal`] 如实交回，
//! 主体派生与落笔同源；批处理只验「登记了、说了、没多写」三条（D-311 / D-312）。

use super::*;
use symbio_kernel::capability::failure_kind;
use symbio_kernel::capability::Registry;
use symbio_kernel::exec::ExecAbortSignal;
use symbio_kernel::exec::ExecEventSink;
use symbio_kernel::llm::TurnToolCallInfo;
use symbio_kernel::plugin::PluginInvokeRequestExt;
use symbio_kernel::plugin::PluginSimpleRequest;
use symbio_kernel::store::EventStore;
use symbio_plugin_api::effects::{BeforeRefusal, EffectInput};
use symbio_plugin_api::port::PortFactEmit;
use symbio_providers::DefaultToolVisitor;
use symbio_sdk::capability::CAPABILITY_VISITOR;
use symbio_sdk::chat::chat_node::{MessageContent, NodeStatus};
use symbio_sdk::chat::principal::Principal;

use super::super::envelope::RoundEnv;

/// 声明闸的默认出口是**可审计拒绝**：拒因进结果，违约承诺照记（淘汰即此类）。
#[test]
fn from_before_keeps_the_audited_default() {
    let judged = BeforeRefusal::new(
        "淘汰",
        "主体已收回".to_string(),
        failure_kind::PERMISSION_DENIED,
    );
    let refusal = Refusal::from_before(judged);
    assert_eq!(refusal.gate, "淘汰");
    assert_eq!(refusal.reason, "主体已收回");
    assert_eq!(refusal.prefix, "Refused");
    assert_eq!(refusal.kind, failure_kind::PERMISSION_DENIED);
    assert!(refusal.notes_delegation);
}

/// 实现显式去掉承诺记账 ⇒ 骨架不记（任务表：工具没跑，声明无从说起）。
#[test]
fn from_before_honours_the_implementations_trace() {
    let judged = BeforeRefusal::new(
        "任务表",
        "不持资格".to_string(),
        failure_kind::PERMISSION_DENIED,
    )
    .without_delegation_note();
    let refusal = Refusal::from_before(judged);
    assert!(!refusal.notes_delegation);
}

/// 主体派生（原 `EliminationInputs::of` 的那一条判据）：未绑智能体 ⇒ `session/main`；
/// 裸名归会话一族；带 namespace 的地址原样。
///
/// 声明闸与落笔读的是**同一个主体**——派生点因此留在环侧，而不是各闸自算。
#[test]
fn the_principal_comes_from_the_agent_id() {
    fn with_agent(agent_id: Option<&str>) -> Arc<dyn PluginInvokeRequest> {
        let req = PluginSimpleRequest::new(None, None);
        if let Some(id) = agent_id {
            req.set(symbio_sdk::keys::AGENT_ID, id.to_string());
        }
        Arc::new(req)
    }

    assert_eq!(
        batch_principal(&with_agent(None)),
        symbio_sdk::authz::PRINCIPAL_MAIN
    );
    assert_eq!(batch_principal(&with_agent(Some("ghost"))), "session/ghost");
    assert_eq!(
        batch_principal(&with_agent(Some("person/local")),),
        "person/local"
    );
}

// ── 批处理夹具（原 `tool_executor.test.rs` 的最小形态）────────────────────────

fn tc(id: &str, name: &str, args: Value) -> TurnToolCallInfo {
    TurnToolCallInfo {
        id: Some(id.into()),
        wire_id: None,
        name: Some(name.into()),
        arguments: args,
        parse_error: None,
        overflow_notice: None,
    }
}

fn result_text(msg: &ChatNode) -> String {
    msg.content
        .as_ref()
        .map(|c| c.to_text())
        .unwrap_or_default()
}

/// 造已广播的 ToolCall 父节点（`ToolCallDelta` 必然先广播过完整节点）。
fn tc_context(ids: &[&str]) -> Vec<ChatNode> {
    ids.iter()
        .map(|id| ChatNode {
            id: (*id).into(),
            principal: Some(Principal::from_legacy("session/main")),
            msg_type: Some(symbio_sdk::chat::chat_node::NodeKind::Skeleton),
            meta: Some(serde_json::json!({"style_tag": "tool", "tool_name": "vdfs_list"})),
            status: Some(NodeStatus::Streaming),
            ..Default::default()
        })
        .collect()
}

fn test_ctx() -> Arc<dyn PluginInvokeRequest> {
    Arc::new(PluginSimpleRequest::new(None, None))
}

/// 装了 `port.tool` 行的上下文：替身执行器与生产 `ToolPort` 同形的最小子集。
async fn ctx_with_tool_port() -> Arc<dyn PluginInvokeRequest> {
    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
    let visitor: Arc<dyn Registry> = Arc::new(DefaultToolVisitor::new());
    crate::test_support::put_tool_port_row(&visitor).await;
    ctx.set(CAPABILITY_VISITOR, visitor);
    ctx
}

/// 测试用执行信封：写口静默、产物闸结论默认放行（D-316 的中性路径值）。
fn test_env() -> RoundEnv {
    RoundEnv {
        scratch_dir: std::env::temp_dir().join("symbio-test/session-tool-execution"),
        emit: PortFactEmit::silent(),
        round: 1,
        round_id: "round-1".into(),
        produced_by: 1,
        allowed: true,
        denied_reason: String::new(),
    }
}

// ── 重复调用：登记、说得出、不拦截 ──────────────────────────────────────────

/// 判据键取 `Value::to_string()`（对象按键序规范化），否则「模型重发同一条、
/// 序列化差一个空格」就数不出来。
#[test]
fn call_signature_normalizes_key_order() {
    let a = serde_json::json!({ "pattern": "x", "path": "." }).to_string();
    let b = serde_json::json!({ "path": ".", "pattern": "x" }).to_string();
    assert_eq!(a, b, "serde_json 按键序规范化——签名判据依赖它");
    assert_eq!(
        call_signature("content_search", &a),
        call_signature("content_search", &b)
    );
    assert_ne!(
        call_signature("content_search", &a),
        call_signature("vdfs_list", &a),
        "同参不同名不是重复"
    );
}

/// 第二次同参调用不再静默：结果正文末尾带 `[重复调用]`，第一次不带。
#[tokio::test]
async fn a_repeat_in_the_same_batch_says_so_on_the_result() {
    let sink = ExecEventSink::silent();
    let abort = ExecAbortSignal::new();
    let store = EventStore::new();
    let tcs = vec![
        tc("tc-a", "vdfs_list", serde_json::json!({ "path": "." })),
        tc("tc-b", "vdfs_list", serde_json::json!({ "path": "." })),
    ];
    let mut effects: Vec<EffectInput> = Vec::new();

    let (msgs, _updates) = run_calls(
        tcs,
        &None,
        &sink,
        &abort,
        ctx_with_tool_port().await,
        &tc_context(&["tc-a", "tc-b"]),
        &test_env(),
        &store,
        false,
        &mut effects,
    )
    .await;

    assert_eq!(msgs.len(), 2);
    let first = result_text(&msgs[0]);
    let second = result_text(&msgs[1]);
    assert!(!first.contains("重复调用"), "第一次不该被说成重复：{first}");
    assert!(
        second.contains("重复调用") && second.contains("vdfs_list"),
        "同批第二次同参调用必须带重复提示：{second}"
    );
}

/// 跨请求的重发才是事故主形态：上一条调用早已落进转写，本轮原样再发。
#[tokio::test]
async fn a_repeat_from_the_transcript_says_so_on_the_result() {
    let sink = ExecEventSink::silent();
    let abort = ExecAbortSignal::new();
    let store = EventStore::new();
    let mut context = tc_context(&["tc-old"]);
    context[0].content = Some(MessageContent::Text(r#"{"path":"."}"#.into()));
    let mut effects: Vec<EffectInput> = Vec::new();

    let (msgs, _updates) = run_calls(
        vec![tc(
            "tc-new",
            "vdfs_list",
            serde_json::json!({ "path": "." }),
        )],
        &None,
        &sink,
        &abort,
        ctx_with_tool_port().await,
        &context,
        &test_env(),
        &store,
        false,
        &mut effects,
    )
    .await;

    let text = result_text(&msgs[0]);
    assert!(
        text.contains("重复调用"),
        "与转写既有调用同参 ⇒ 提示重复：{text}"
    );
}

/// 反向保险：名字相同、参数不同 ⇒ 不是重复（否则每次分页都挨一句冤枉）。
#[tokio::test]
async fn a_different_argument_is_not_a_repeat() {
    let sink = ExecEventSink::silent();
    let abort = ExecAbortSignal::new();
    let store = EventStore::new();
    let mut context = tc_context(&["tc-old"]);
    context[0].content = Some(MessageContent::Text(r#"{"path":"a"}"#.into()));
    let mut effects: Vec<EffectInput> = Vec::new();

    let (msgs, _updates) = run_calls(
        vec![tc(
            "tc-new",
            "vdfs_list",
            serde_json::json!({ "path": "b" }),
        )],
        &None,
        &sink,
        &abort,
        ctx_with_tool_port().await,
        &context,
        &test_env(),
        &store,
        false,
        &mut effects,
    )
    .await;

    let text = result_text(&msgs[0]);
    assert!(!text.contains("重复调用"), "换了参数就不是重复：{text}");
}

// ── 未启动的两类：中止与超限丢弃（D-311 / D-312）───────────────────────────

/// 节点是投影：中止批的未启动调用**不由执行体补节点**，只等环侧关账落
/// `failed{why}`，展示面按事实渲染（否则同一笔出现两张结果卡）。
#[tokio::test]
async fn aborted_batch_terminates_every_tool_call() {
    let sink = ExecEventSink::silent();
    let abort = ExecAbortSignal::new();
    abort.abort();
    let store = EventStore::new();
    let tcs = vec![
        tc("tc1", "vdfs_list", serde_json::json!({ "path": "." })),
        tc("tc2", "vdfs_list", serde_json::json!({ "path": "." })),
    ];
    let mut effects: Vec<EffectInput> = Vec::new();

    let (msgs, updates) = run_calls(
        tcs,
        &None,
        &sink,
        &abort,
        ctx_with_tool_port().await,
        &tc_context(&["tc1", "tc2"]),
        &test_env(),
        &store,
        false,
        &mut effects,
    )
    .await;

    assert!(
        msgs.is_empty(),
        "执行体不写节点：未执行的两笔都不产生结果卡"
    );
    assert!(
        updates.is_empty(),
        "父调用卡的终态由投影器按 `failed{{why}}` 渲染"
    );
    assert!(effects.is_empty(), "没跑 = 没履约，不产生效果输入");
}

/// megabatch 守卫的**反馈面**：带 `overflow_notice` 的调用不执行、直接落反馈结果。
#[tokio::test]
async fn overflow_dropped_call_returns_feedback_without_execution() {
    let sink = ExecEventSink::silent();
    let abort = ExecAbortSignal::new();
    let store = EventStore::new();
    let notice =
        "本次调用未执行：本条响应内的工具调用数超过上限（32），超出上限的调用已被系统丢弃。"
            .to_string();
    let tcs = vec![TurnToolCallInfo {
        id: Some("tc-overflow".into()),
        wire_id: None,
        name: Some("vdfs_list".into()),
        arguments: serde_json::json!({ "path": "." }),
        parse_error: None,
        overflow_notice: Some(notice.clone()),
    }];
    let mut effects: Vec<EffectInput> = Vec::new();

    let (msgs, updates) = run_calls(
        tcs,
        &None,
        &sink,
        &abort,
        test_ctx(),
        &tc_context(&["tc-overflow"]),
        &test_env(),
        &store,
        false,
        &mut effects,
    )
    .await;

    assert_eq!(msgs.len(), 1, "丢弃调用必须有结果子节点（工具终态不变量）");
    assert_eq!(msgs[0].status, Some(NodeStatus::Completed));
    assert_eq!(
        result_text(&msgs[0]),
        notice,
        "结果正文 = 流层给的反馈（原样）"
    );
    assert_eq!(
        msgs[0]
            .meta
            .as_ref()
            .and_then(|m| m.get("failure_kind"))
            .and_then(|v| v.as_str()),
        Some("overflow_dropped"),
        "成因必须可指认，不得混进 not_executed"
    );
    assert_eq!(updates.len(), 1, "父节点必须同步收敛");
    assert_eq!(updates[0].status, Some(NodeStatus::Completed));
}

/// 交互模式下「前一笔待用户 / 失败 ⇒ 后续不再跑」的判据只读最后一笔的终态。
#[test]
fn interaction_break_only_follows_a_pending_or_failed_parent() {
    fn parent(status: NodeStatus, kind: Option<&str>) -> ChatNode {
        ChatNode {
            id: "tc".into(),
            principal: Some(Principal::from_legacy("session/main")),
            msg_type: Some(symbio_sdk::chat::chat_node::NodeKind::Skeleton),
            status: Some(status),
            meta: Some(match kind {
                Some(k) => serde_json::json!({ "failure_kind": k }),
                None => serde_json::json!({}),
            }),
            ..Default::default()
        }
    }

    assert!(!interaction_blocked(&[]));
    assert!(interaction_blocked(&[parent(
        NodeStatus::WaitingUserAction,
        Some(failure_kind::NEEDS_APPROVAL)
    )]));
    assert!(interaction_blocked(&[parent(
        NodeStatus::Completed,
        Some(failure_kind::NEEDS_APPROVAL)
    )]));
    assert!(interaction_blocked(&[parent(
        NodeStatus::Completed,
        Some(failure_kind::ERROR)
    )]));
    assert!(!interaction_blocked(&[parent(NodeStatus::Completed, None)]));
}

/// 缺 `port.tool` ⇒ 结果说得出为什么没跑（不是静默跳过）。
#[tokio::test]
async fn a_missing_tool_port_fails_loudly_on_the_result() {
    let sink = ExecEventSink::silent();
    let abort = ExecAbortSignal::new();
    let store = EventStore::new();
    let mut effects: Vec<EffectInput> = Vec::new();

    let (msgs, updates) = run_calls(
        vec![tc("tc-x", "vdfs_list", serde_json::json!({ "path": "." }))],
        &None,
        &sink,
        &abort,
        test_ctx(),
        &tc_context(&["tc-x"]),
        &test_env(),
        &store,
        false,
        &mut effects,
    )
    .await;

    assert_eq!(msgs.len(), 1, "有调用必有结果");
    assert_eq!(msgs[0].parent_id.as_deref(), Some("tc-x"));
    let text = result_text(&msgs[0]);
    assert!(
        text.contains("port.tool") && text.contains("缺席"),
        "结果应指出端口缺席：{text}"
    );
    assert_eq!(updates.len(), 1, "父节点必须同步收敛");
    assert_eq!(updates[0].status, Some(NodeStatus::Completed));
}

/// 等待用户的交互卡也是**工具的结果**：与普通结果共享同一套 id 约定
/// （`id = result_node_id`、`parent_id = tool_call_id`），身份只写 `principal`（D-147）。
#[test]
fn build_user_prompt_message_uses_the_orchestrator_owned_ids() {
    let pending = symbio_plugin_api::port::PendingPrompt {
        text: "请确认".into(),
        prompt: serde_json::json!({ "kind": "approval" }),
        failure_kind: failure_kind::NEEDS_APPROVAL.to_string(),
    };
    let msg = build_user_prompt_message("result-of-tc", "tc-1", &pending, "shell");
    assert_eq!(msg.id, "result-of-tc");
    assert_eq!(msg.parent_id.as_deref(), Some("tc-1"));
    assert_eq!(
        msg.msg_type,
        Some(symbio_sdk::chat::chat_node::NodeKind::Interaction)
    );
    assert_eq!(msg.status, Some(NodeStatus::WaitingUserAction));
    let meta = msg.meta.as_ref().expect("交互卡必须带 meta");
    assert_eq!(meta["tool_name"], serde_json::json!("shell"));
    assert_eq!(
        meta["failure_kind"],
        serde_json::json!(failure_kind::NEEDS_APPROVAL)
    );
}
