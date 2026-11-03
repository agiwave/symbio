//! `tool_executor` 模块的单元测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）：
//! `tools/tool_executor.rs` 只保留生产代码，测试全部放本文件。

use super::*;
// `NodeRole` 是**测试自己**要的：生产侧写身份只写 `principal`（D-147），
// `role` 那一格只在「构造期还没标身份的节点」与存量夹具里出现，
// `use super::*` 因此带不过来。
use symbio_kernel::plugin::PluginPayload;
use symbio_kernel::plugin::PluginSimpleRequest;
use symbio_sdk::chat::chat_node::NodeRole;
// `Store` 是**测试自己**要的（`store.append` 是 trait 方法；父模块顶层不导入它——
// 生产侧的读发生在 `tool_executor/breaker.rs` 子模块里）；`use super::*` 因此带不过来。
use symbio_kernel::store::Store;
// `failure_kind` 与 `Seq` 同样是**测试自己**要的：批首闸门读数与逐条执行的闸门段
// 已拆去 `tools/dispatch/`，父模块顶层不再导入它们——测试自己拿。
use symbio_kernel::capability::failure_kind;
use symbio_kernel::event::Seq;
// `LatencyTier` 是**测试自己**要的：它只被父模块里的熔断段用到，那段已按域
// 拆去 `tool_executor/breaker.rs`，
// 父模块顶层不再导入，`use super::*` 便带不过来——测试自己拿。
use symbio_kernel::adapters::LatencyTier;
use symbio_kernel::capability::{Capability, CapabilityMeta, CapabilityRow, Registry};
use symbio_kernel::exec::ExecEnv;
use symbio_kernel::plugin::{PluginError, PluginInvokeRequestExt};
use symbio_kernel::store::EventStore;
use symbio_providers::DefaultToolVisitor;

/// 夹具模仿的是**落库/转写面**：身份写 `principal`（D-147），`role` 只是读侧残留格。
fn principal_for(role: NodeRole) -> symbio_sdk::chat::principal::Principal {
    use symbio_sdk::chat::principal::Principal;
    match role {
        NodeRole::User => Principal::from_legacy(symbio_sdk::authz::PRINCIPAL_USER),
        NodeRole::Assistant => Principal::from_legacy("session/main"),
        NodeRole::Tool => Principal::from_legacy("legacy/unknown"),
        NodeRole::System => Principal::from_legacy("system/local"),
    }
}

fn test_ctx() -> Arc<dyn PluginInvokeRequest> {
    Arc::new(PluginSimpleRequest::new(None, None))
}

/// 装了 `port.tool` 行的上下文：协议读数与失败措辞在执行体，环测试只验证映射，
/// 所以替身执行器要能接住 `protocol` 载荷（与生产 `ToolPort` 同形的最小子集）。
async fn ctx_with_tool_port() -> Arc<dyn PluginInvokeRequest> {
    let req = PluginSimpleRequest::new(None, None);
    let visitor = Arc::new(DefaultToolVisitor::new());
    super::super::super::test_support::put_tool_port_row(&visitor).await;
    req.set(
        symbio_sdk::capability::CAPABILITY_VISITOR,
        visitor as Arc<dyn Registry>,
    );
    Arc::new(req) as Arc<dyn PluginInvokeRequest>
}

/// 会话的 L0 存档目录（测试用临时目录）——工具结果存档落在这里。
/// 生产上由 session 在派发前算好，作为**值**传进来；执行层不拼会话存储路径。
fn test_dir() -> std::path::PathBuf {
    std::env::temp_dir().join("symbio-test/session-tool-executor")
}

/// 测试用执行信封：写口静默、产物闸结论默认放行；存档目录随 [`test_dir`]。
/// 产物格由执行体在 `land_only` 出口落（P1b），本夹具不单独断言它。
fn test_env() -> RoundEnv {
    RoundEnv {
        archive_dir: test_dir(),
        ..Default::default()
    }
}

/// 同 [`test_env`]，存档目录用调用方给的（个别用例要对存档文件做断言）。
fn env_for(dir: &std::path::Path) -> RoundEnv {
    RoundEnv {
        archive_dir: dir.to_path_buf(),
        ..Default::default()
    }
}

// ── 注册面：闸门问声明，测试喂一份与生产同形的声明 ────────────────
//
// 机制化之前这些用例直接写工具名调 `note_*`；判据搬到 `CapabilityMeta` 的
// 自声明位之后，同一批用例经 `ToolDecls` 喂注册面——生产三件套各 declare 各的
// 位（`agent_run` 派活 + 立约、`todo_write` 任务表），外加一个什么都不声明的
// 普通工具（生产里绝大多数工具的形态）。

/// 只带元数据的假工具：注册表怎么读，测试就怎么喂。
struct DeclTool(CapabilityMeta);

#[async_trait::async_trait]
impl symbio_kernel::capability::Capability for DeclTool {
    fn meta(&self) -> CapabilityMeta {
        self.0.clone()
    }

    async fn execute(
        &self,
        _args: serde_json::Value,
        _env: &ExecEnv,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> Result<serde_json::Value, PluginError> {
        Ok(serde_json::Value::Null)
    }
}

/// 生产三件套的注册面（`ToolDecls` 批首读的同一来源）。
async fn factory_decls() -> ToolDecls {
    let ctx = test_ctx();
    let visitor = Arc::new(DefaultToolVisitor::new());
    for (name, assign_work, promise, task_table) in [
        ("agent_run", true, true, false),
        ("todo_write", false, false, true),
        ("shell", false, false, false),
    ] {
        visitor
            .put(CapabilityRow::tool(Arc::new(DeclTool(CapabilityMeta {
                name: name.to_string(),
                behaviors: match (assign_work, promise, task_table) {
                    (true, true, false) => &[
                        symbio_plugin_api::behaviors::ASSIGN_WORK,
                        symbio_plugin_api::behaviors::DELEGATION_PROMISE,
                    ],
                    (false, false, true) => &[symbio_plugin_api::behaviors::TASK_TABLE],
                    (false, false, false) => &[],
                    other => unreachable!("夹具只喂生产组合：{other:?}"),
                },
                ..Default::default()
            })) as Arc<dyn Capability>))
            .await;
    }
    ctx.set(
        symbio_sdk::capability::CAPABILITY_VISITOR,
        visitor as Arc<dyn Registry>,
    );
    ToolDecls::of_metas(&symbio_sdk::capability::tool_metas(&ctx).await)
}

// ── 测试用的声明闸 ────────────────────────────────────────────────
//
// 淘汰的真实实现住 `symbio-plugin-agent`（`DelegationGate`），而本 crate 的测试
// 二进制里没有兄弟插件（E-009）。宿主接线的判据因此用一份**同形**的实现：它只按
// 批首读数拒，语义（可审计拒绝 + 违约承诺）由端口统一，不由本实现决定。
struct RevokedGate;

impl symbio_plugin_api::effects::ToolBehavior for RevokedGate {
    fn owner(&self) -> &'static str {
        "tests/revoked"
    }

    fn behavior(&self) -> &'static str {
        symbio_plugin_api::behaviors::ASSIGN_WORK
    }

    fn before(
        &self,
        req: &symbio_plugin_api::effects::BeforeRequest<'_>,
    ) -> symbio_plugin_api::effects::BeforeDecision {
        use symbio_plugin_api::effects::{BeforeDecision, BeforeRefusal};
        if !req.readings.assign_work_revoked {
            return BeforeDecision::Allow;
        }
        BeforeDecision::Refuse(BeforeRefusal::new(
            "淘汰",
            format!(
                "主体 {} 的 assign.work 已被淘汰回路收回（verdict.asserted，声誉低于阈值），本次委派未执行",
                req.principal
            ),
            failure_kind::PERMISSION_DENIED,
        ))
    }
}

symbio_plugin_api::submit_behavior!(RevokedGate);

/// 需求 2 回归：工具调用 id 缺失/非法时不得跳过，必须作为工具调用失败处理——
/// 生成错误结果子节点（喂回 LLM）+ 父节点失败补丁，保证已落库的 ToolCall
/// 节点不会成为"无结果 tool_call"（下轮请求 400）。
#[tokio::test]
async fn missing_tool_call_id_is_recorded_as_failure() {
    let sink = ExecEventSink::silent();
    let abort = ExecAbortSignal::new();
    let tcs = vec![TurnToolCallInfo {
        id: None,
        wire_id: None,
        name: Some("vdfs_list".into()),
        arguments: json!({ "path": "." }),
        parse_error: None,
        overflow_notice: None,
    }];

    let (msgs, updates) = process_tool_calls_in_env(
        tcs,
        &None,
        &sink,
        &abort,
        ctx_with_tool_port().await,
        &[],
        &test_env(),
        &EventStore::new(),
        false,
        &mut Vec::new(),
    )
    .await;

    assert_eq!(msgs.len(), 1, "必须生成失败结果子节点（而非跳过）");
    assert_eq!(msgs[0].node_role(), Some(NodeRole::Tool));
    assert_eq!(msgs[0].status, Some(NodeStatus::Completed));
    assert!(
        msgs[0]
            .content
            .as_ref()
            .map(|c| c.to_text().contains("Error:"))
            .unwrap_or(false),
        "结果内容应携带错误信息"
    );
    assert!(!msgs[0].parent_id.as_deref().unwrap_or("").is_empty());

    assert_eq!(updates.len(), 1, "必须生成父节点失败补丁");
    assert_eq!(updates[0].status, Some(NodeStatus::Completed));
    assert_eq!(
        updates[0]
            .meta
            .as_ref()
            .and_then(|m| m.get("failure_kind"))
            .and_then(|v| v.as_str()),
        Some("error")
    );
}

/// 空串 id 与缺失同等对待（"不合法"）。
#[tokio::test]
async fn empty_tool_call_id_is_recorded_as_failure() {
    let sink = ExecEventSink::silent();
    let abort = ExecAbortSignal::new();
    let tcs = vec![TurnToolCallInfo {
        id: Some(String::new()),
        wire_id: None,
        name: Some("vdfs_list".into()),
        arguments: json!({}),
        parse_error: None,
        overflow_notice: None,
    }];

    let (msgs, updates) = process_tool_calls_in_env(
        tcs,
        &None,
        &sink,
        &abort,
        ctx_with_tool_port().await,
        &[],
        &test_env(),
        &EventStore::new(),
        false,
        &mut Vec::new(),
    )
    .await;

    assert_eq!(msgs.len(), 1);
    assert_eq!(updates.len(), 1);
}

/// 工具名缺失/非法 → 同样作为失败处理（结果挂到已知 id 上，结构完整）。
#[tokio::test]
async fn missing_tool_name_is_recorded_as_failure() {
    let sink = ExecEventSink::silent();
    let abort = ExecAbortSignal::new();
    let tcs = vec![TurnToolCallInfo {
        id: Some("tc-known".into()),
        wire_id: None,
        name: Some(String::new()),
        arguments: json!({}),
        parse_error: None,
        overflow_notice: None,
    }];

    let (msgs, updates) = process_tool_calls_in_env(
        tcs,
        &None,
        &sink,
        &abort,
        ctx_with_tool_port().await,
        &[],
        &test_env(),
        &EventStore::new(),
        false,
        &mut Vec::new(),
    )
    .await;

    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].parent_id.as_deref(), Some("tc-known"));
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].id, "tc-known");
}

/// 回归（卡思考根因）：参数 JSON 解析失败（截断残破）的调用必须被拒绝执行，
/// 走协议失败路径——生成 role=Tool 错误子节点 + 父节点失败补丁，
/// 而非静默以 `{}` 执行（那会让模型看到「缺少必填参数」后原样重试）。
#[tokio::test]
async fn unparseable_arguments_are_refused_not_executed() {
    let sink = ExecEventSink::silent();
    let abort = ExecAbortSignal::new();
    let tcs = vec![TurnToolCallInfo {
        id: Some("tc-broken".into()),
        wire_id: None,
        name: Some("cmd".into()),
        arguments: json!({}),
        parse_error: Some(r#"{"command": "cargo test"#.into()),
        overflow_notice: None,
    }];

    let (msgs, updates) = process_tool_calls_in_env(
        tcs,
        &None,
        &sink,
        &abort,
        ctx_with_tool_port().await,
        &[],
        &test_env(),
        &EventStore::new(),
        false,
        &mut Vec::new(),
    )
    .await;

    assert_eq!(msgs.len(), 1, "必须生成失败结果子节点（而非跳过）");
    assert_eq!(msgs[0].node_role(), Some(NodeRole::Tool));
    assert_eq!(msgs[0].status, Some(NodeStatus::Completed));
    assert_eq!(msgs[0].parent_id.as_deref(), Some("tc-broken"));
    let text = msgs[0]
        .content
        .as_ref()
        .map(|c| c.to_text())
        .unwrap_or_default();
    assert!(
        text.contains("Error:") && text.contains("参数 JSON 解析失败"),
        "结果应说明参数 JSON 解析失败，实际：{text}"
    );

    assert_eq!(updates.len(), 1, "必须生成父节点失败补丁");
    assert_eq!(updates[0].id, "tc-broken");
    assert_eq!(updates[0].status, Some(NodeStatus::Completed));
}

/// 判据：**第二次同参调用不再静默**。
///
/// 事故现场（会话 `cli1a14222066f0`）：模型要的是「一共多少处命中」，而截断结果里
/// 没有一个字的总数，它唯一的动作就是原样重发 ⇒ 同一份结果执行两次、白跑一轮。
/// 这里不要求拦截，只要求**结果里看得见**「这是重复调用」。
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

#[tokio::test]
async fn a_repeat_in_the_same_batch_says_so_on_the_result() {
    let sink = ExecEventSink::silent();
    let abort = ExecAbortSignal::new();
    let tcs = vec![
        tc("tc-a", "vdfs_list", json!({ "path": "." })),
        tc("tc-b", "vdfs_list", json!({ "path": "." })),
    ];

    let (msgs, _updates) = process_tool_calls_in_env(
        tcs,
        &None,
        &sink,
        &abort,
        test_ctx(),
        &tc_context(&["tc-a", "tc-b"]),
        &test_env(),
        &EventStore::new(),
        false,
        &mut Vec::new(),
    )
    .await;

    assert_eq!(msgs.len(), 2);
    let first = result_text(&msgs[0]);
    let second = result_text(&msgs[1]);
    assert!(
        !first.contains("重复调用"),
        "第一次调用不该被说成重复，实际：{first}"
    );
    assert!(
        second.contains("重复调用") && second.contains("vdfs_list"),
        "同批第二次同参调用必须带重复提示，实际：{second}"
    );
}

/// 跨请求的重发才是事故的主形态：上一条调用早已落进转写，本轮请求原样再发一次。
#[tokio::test]
async fn a_repeat_from_the_transcript_says_so_on_the_result() {
    let sink = ExecEventSink::silent();
    let abort = ExecAbortSignal::new();
    // 转写里已有一条 vdfs_list{"path":"."} 的请求节点（id 不在本批 ⇒ 是「之前那次」）
    let mut context = tc_context(&["tc-old"]);
    context[0].content = Some(MessageContent::Text(r#"{"path":"."}"#.into()));

    let (msgs, _updates) = process_tool_calls_in_env(
        vec![tc("tc-new", "vdfs_list", json!({ "path": "." }))],
        &None,
        &sink,
        &abort,
        test_ctx(),
        &context,
        &test_env(),
        &EventStore::new(),
        false,
        &mut Vec::new(),
    )
    .await;

    let text = result_text(&msgs[0]);
    assert!(
        text.contains("重复调用"),
        "与转写里既有调用同参 ⇒ 提示重复，实际：{text}"
    );
}

/// 反向保险：名字相同、参数不同 ⇒ 不是重复（否则每次分页都挨一句冤枉）。
#[tokio::test]
async fn a_different_argument_is_not_a_repeat() {
    let sink = ExecEventSink::silent();
    let abort = ExecAbortSignal::new();
    let mut context = tc_context(&["tc-old"]);
    context[0].content = Some(MessageContent::Text(r#"{"path":"a"}"#.into()));

    let (msgs, _updates) = process_tool_calls_in_env(
        vec![tc("tc-new", "vdfs_list", json!({ "path": "b" }))],
        &None,
        &sink,
        &abort,
        test_ctx(),
        &context,
        &test_env(),
        &EventStore::new(),
        false,
        &mut Vec::new(),
    )
    .await;

    let text = result_text(&msgs[0]);
    assert!(
        !text.contains("重复调用"),
        "换了参数就不是重复，实际：{text}"
    );
}

/// 键序与空白是同一个调用：签名取 `Value::to_string()`（对象按键序规范化），
/// 否则「模型重发同一条、序列化差一个空格」就数不出来。
#[test]
fn call_signature_normalizes_key_order() {
    let a = json!({ "pattern": "x", "path": "." }).to_string();
    let b = json!({ "path": ".", "pattern": "x" }).to_string();
    assert_eq!(a, b, "serde_json 按键序规范化——签名判据依赖它");
    assert_eq!(
        call_signature("content_search", &a),
        call_signature("content_search", &b)
    );
    assert_ne!(
        call_signature("content_search", &a),
        call_signature("vdfs_list", &a),
        "同名不同参不是重复，同参不同名也不是重复"
    );
}

/// 造两个已广播的 ToolCall 父节点（模拟 settle_reasoning 之后的权威转写形态——
/// ToolCallDelta 必然先广播过完整节点，`context.messages` 里一定有它们）。
fn tc_context(ids: &[&str]) -> Vec<ChatNode> {
    ids.iter()
        .map(|id| ChatNode {
            id: (*id).into(),
            principal: Some(principal_for(NodeRole::Assistant)),
            msg_type: Some(NodeKind::Skeleton),
            meta: Some(serde_json::json!({"style_tag": "tool", "tool_name": "vdfs_list"})),
            status: Some(NodeStatus::Streaming),
            ..Default::default()
        })
        .collect()
}

/// 节点是投影（DE-047 D-312）：中止批的未启动调用**不由执行体补节点**。
///
/// 执行体只落事实（`failed{why}`，由环侧关账时写），「未执行」的调用卡 / 结果子项
/// 由投影器按事实渲染（`rebuild.test.rs` 的
/// `a_not_executed_call_is_projected_from_its_failure_fact` 覆盖）。执行体若在此处
/// 写节点，写侧与投影器会各造一份，同一笔调用出现两张结果卡。
#[tokio::test]
async fn aborted_batch_terminates_every_tool_call() {
    let sink = ExecEventSink::silent();
    let abort = ExecAbortSignal::new();
    abort.abort(); // 已中止：本批一个都不执行
    let tcs = vec![
        TurnToolCallInfo {
            id: Some("tc1".into()),
            wire_id: None,
            name: Some("vdfs_list".into()),
            arguments: json!({ "path": "." }),
            parse_error: None,
            overflow_notice: None,
        },
        TurnToolCallInfo {
            id: Some("tc2".into()),
            wire_id: None,
            name: Some("vdfs_list".into()),
            arguments: json!({ "path": "." }),
            parse_error: None,
            overflow_notice: None,
        },
    ];

    let (msgs, updates) = process_tool_calls_in_env(
        tcs,
        &None,
        &sink,
        &abort,
        test_ctx(),
        &tc_context(&["tc1", "tc2"]),
        &test_env(),
        &EventStore::new(),
        false,
        &mut Vec::new(),
    )
    .await;

    assert!(
        msgs.is_empty(),
        "执行体不写节点：未执行的两笔都不产生结果卡"
    );
    assert!(
        updates.is_empty(),
        "执行体不写节点：父调用卡的终态由投影器按 `failed{{why}}` 渲染"
    );
}

/// megabatch 守卫的**反馈面**：带 `overflow_notice` 的调用不执行、直接落反馈结果。
///
/// `6cf9520d` 守卫语义：上限判在流层（model 插件），处置是**反馈**——被丢弃的
/// 调用各落一条结果子节点，正文即流层给的提示，模型下一轮从工具结果里读到
/// 「超限了、后面的调用被丢了、该分批发起」。整轮 Err 的旧方案让模型一无所知
/// （流程无故中断），已否决。
#[tokio::test]
async fn overflow_dropped_call_returns_feedback_without_execution() {
    let sink = ExecEventSink::silent();
    let abort = ExecAbortSignal::new(); // 未中止：能否执行只取决于丢弃标记
    let notice =
        "本次调用未执行：本条响应内的工具调用数超过上限（32），超出上限的调用已被系统丢弃。"
            .to_string();
    let tcs = vec![TurnToolCallInfo {
        id: Some("tc-overflow".into()),
        wire_id: None,
        name: Some("vdfs_list".into()),
        arguments: json!({ "path": "." }),
        parse_error: None,
        overflow_notice: Some(notice.clone()),
    }];

    let (msgs, updates) = process_tool_calls_in_env(
        tcs,
        &None,
        &sink,
        &abort,
        test_ctx(),
        &tc_context(&["tc-overflow"]),
        &test_env(),
        &EventStore::new(),
        false,
        &mut Vec::new(),
    )
    .await;

    assert_eq!(msgs.len(), 1, "丢弃调用必须有结果子节点（工具终态不变量）");
    assert_eq!(msgs[0].node_role(), Some(NodeRole::Tool));
    assert_eq!(msgs[0].status, Some(NodeStatus::Completed));
    let text = msgs[0]
        .content
        .as_ref()
        .map(|c| c.to_text())
        .unwrap_or_default();
    assert_eq!(text, notice, "结果正文 = 流层给的反馈（原样落盘）");
    assert_eq!(
        msgs[0]
            .meta
            .as_ref()
            .and_then(|m| m.get("failure_kind"))
            .and_then(|v| v.as_str()),
        Some("overflow_dropped"),
        "成因必须可指认（overflow_dropped），不得混进 not_executed"
    );

    assert_eq!(updates.len(), 1, "父节点必须同步收敛，否则前端永远转圈");
    assert_eq!(updates[0].id, "tc-overflow");
    assert_eq!(updates[0].status, Some(NodeStatus::Completed));
    assert_eq!(
        updates[0]
            .meta
            .as_ref()
            .and_then(|m| m.get("failure_kind"))
            .and_then(|v| v.as_str()),
        Some("overflow_dropped")
    );
}

/// 交互模式下前一个工具失败 ⇒ 本批剩余被跳过。**跳过的那些不由执行体补节点**：
/// 节点是事实的投影（D-312），「未执行」由环侧关账落 `failed{not_executed}`，
/// 调用卡与结果子项由投影器渲染。执行体再写一份就是同一笔的两张结果卡。
#[tokio::test]
async fn interactive_break_leaves_result_for_skipped_calls() {
    let sink = ExecEventSink::silent();
    let abort = ExecAbortSignal::new();
    // 交互模式 + 无 parent：第一个工具必然以失败告终，触发本批 break
    let ctx = test_ctx();
    ctx.set(symbio_sdk::keys::MODE, "interactive".to_string());
    let tcs = vec![
        TurnToolCallInfo {
            id: Some("tc1".into()),
            wire_id: None,
            name: Some("vdfs_read".into()),
            arguments: json!({ "path": "a" }),
            parse_error: None,
            overflow_notice: None,
        },
        TurnToolCallInfo {
            id: Some("tc2".into()),
            wire_id: None,
            name: Some("vdfs_read".into()),
            arguments: json!({ "path": "b" }),
            parse_error: None,
            overflow_notice: None,
        },
    ];

    let (msgs, updates) = process_tool_calls_in_env(
        tcs,
        &None,
        &sink,
        &abort,
        ctx,
        &tc_context(&["tc1", "tc2"]),
        &test_env(),
        &EventStore::new(),
        false,
        &mut Vec::new(),
    )
    .await;

    assert!(
        msgs.iter().all(|m| m.parent_id.as_deref() != Some("tc2")),
        "被跳过的调用不产生结果卡：投影器按 `failed{{not_executed}}` 渲染"
    );
    assert!(
        updates.iter().all(|u| u.id != "tc2"),
        "被跳过的调用不产生父卡终态：终态同样由投影器渲染"
    );
}

// ── 工具结果的「等待用户」意图 ────────────────────────────────────────────────
//
// 契约**工具只声明意图与载荷，节点归编排层构造**。工具不自建 Session 通道、
// 发一个随机 id 的节点，由消费方解回来改 id 再重播一次——同一个逻辑节点因此有两个
// id，前端出现重复审批卡，且 resume 只删得掉一个。
//
// `failure_kind` 判据与解码（B3 的 pending 半边）已迁 session 的
// `port.test.rs`；这里留的是**节点构造**那一半——它属于环。

/// user_prompt 节点的身份归编排层：`id = result_node_id`、`parent_id = tool_call_id`。
///
/// 这两个 id 正是「同一逻辑节点两个 id」事故的正反面。工具侧现在**拿不到**它们
/// （[`PendingPrompt`] 里没有 id 字段），所以那类 bug 在类型层面不成立。
#[test]
fn build_user_prompt_message_uses_the_orchestrator_owned_ids() {
    let pending = PendingPrompt {
        text: "请回答问题以继续".into(),
        prompt: json!({ "kind": "question", "questions": [] }),
        failure_kind: symbio_kernel::capability::failure_kind::NEEDS_INTERACTION.into(),
    };
    let msg = build_user_prompt_message("res-1", "tc-1", &pending, "vdfs_write");

    assert_eq!(msg.id, "res-1", "节点 id 必须是结果占位节点 id，不是新造的");
    assert_eq!(
        msg.parent_id.as_deref(),
        Some("tc-1"),
        "必须锚在发起它的 ToolCall 下"
    );
    assert_eq!(msg.msg_type, Some(NodeKind::Interaction));
    assert_eq!(msg.status, Some(NodeStatus::WaitingUserAction));
    assert_eq!(
        msg.node_role(),
        Some(NodeRole::Tool),
        "族从主体折出（身份只有主体这一份写，D-147）"
    );
    // 交互卡也是工具的结果：主体归工具一族，工具名随行入格
    assert_eq!(
        msg.principal.as_ref().map(|p| p.id.as_str()),
        Some("vdfs/write")
    );

    let meta = msg.meta.expect("user_prompt 必须带 meta");
    assert_eq!(meta["failure_kind"], json!("needs_interaction"));
    assert_eq!(meta["prompt"]["kind"], json!("question"));
    assert_eq!(meta["tool_name"], json!("vdfs_write"));
}

// ==================== record_effect：效果出参的唯一观察点 ====================
//
// 判据两条：**只有声明过的工具**进效果出参（`agent_run` 声明了立约，`vdfs_read`
// 没有），**先到先得**（同一次调用在两个分支各记一笔时，先记下的那次更接近事实）。
// 「这次调用算不算立约」不在这里判——它在 `symbio-plugin-agent` 的行为实现里
// （`host/behavior.test.rs`），本层只负责把终态交出去。

/// 只有声明了行为的工具进效果出参——别的工具是本会话自办，办没办成是执行结果
/// 不是承诺。
#[tokio::test]
async fn record_effect_ignores_undeclared_tools() {
    let decls = factory_decls().await;
    let mut out = Vec::new();
    let args = json!({ "prompt": "让 reviewer 复查这段", "agent_id": "reviewer" });

    record_effect(&mut out, &decls, "c1", "vdfs_read", &args, true, "", "");
    record_effect(&mut out, &decls, "c2", "read", &args, true, "", "");
    assert!(
        out.is_empty(),
        "没声明任何行为的工具不产生效果输入：{out:?}"
    );

    record_effect(&mut out, &decls, "c3", "agent_run", &args, true, "完成", "");
    assert_eq!(out.len(), 1);
    assert_eq!(
        out[0].call_id, "c3",
        "承诺号 = 转写里的 ToolCall 节点 id（`c-offer-<id>` 由此成格）"
    );
    assert_eq!(
        out[0].args["prompt"],
        json!("让 reviewer 复查这段"),
        "参数原样带走：承诺内容由能力包自己从参数里取"
    );
    assert!(out[0].success);
    assert!(out[0].why.is_empty(), "守约不必带 why，违约必须带");
    assert!(
        out[0]
            .declared
            .contains(&symbio_plugin_api::behaviors::DELEGATION_PROMISE),
        "声明的行为名原样带过去，写口据此选实现"
    );
}

/// **先到先得**：同一次调用可能在两个分支各记一笔（终态广播找不到节点 ⇒ 末尾
/// 收口会再记一次「未执行」），后到的收口不许把跑成功的改写成没跑。
#[tokio::test]
async fn record_effect_never_overwrites_an_earlier_judgement() {
    let decls = factory_decls().await;
    let mut out = Vec::new();
    let args = json!({ "prompt": "交给子智能体" });

    record_effect(&mut out, &decls, "c1", "agent_run", &args, true, "完成", "");
    record_effect(
        &mut out,
        &decls,
        "c1",
        "agent_run",
        &args,
        false,
        "",
        "未执行（中止或同批前序未结束）",
    );
    assert_eq!(out.len(), 1, "同一调用只入出参一次");
    assert!(out[0].success, "先记下的那次更接近事实，收口分支不得覆盖");

    record_effect(
        &mut out,
        &decls,
        "c1",
        "agent_run",
        &args,
        false,
        "",
        "被 PreToolUse 钩子拦下",
    );
    assert_eq!(out.len(), 1);
    assert!(out[0].success);

    // 没跑到的那一次（一次都没记过）照常入账，且带得出违约原因。
    record_effect(
        &mut out,
        &decls,
        "c2",
        "agent_run",
        &args,
        false,
        "",
        "被 PreToolUse 钩子拦下",
    );
    assert_eq!(out.len(), 2);
    assert!(!out[1].success);
    assert_eq!(out[1].why, "被 PreToolUse 钩子拦下");
}

// ==================== 外部执行闸门的中性输入 ====================
//
// 开窗判据（时延 / 预算 / 熔断事实）已搬进 `symbio-plugin-tool` 的执行体，本层只
// 读账并把四个中性值交出去（DE-039 §6，DE-047 D-311）；熔断端到端用例住在
// `symbio-plugin-tool/src/port.test.rs`。

/// 判据那一半：授权是**读矩阵**得来的（fail-closed），不是默认放行。
///
/// `agent:*` 身份会话在**会话装配点经绑定登记**拿到会话面缺省行（含
/// `produce.artifact`，[D-100]）——本用例直读合流面（不经装配点），取的是
/// 部署表那一行的判据；`Refuse` 只可能来自**不属于这一族**的主体——判据为 false
/// 就是 false，没有任何「认不出 ⇒ 放行」的旁路。另一侧的判据（预算）见下方端到端用例。
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
}

/// 台账读的是**交给执行体的事实值**。
///
/// 历史事故：曾有实现按 `<插件目录>/v2-events.wal` 取账（漏了会话 id 那层）⇒
/// `spent_ms` 恒 0 ⇒ 预算分支永远走不到，且没有任何东西会变红。D-084 §1-A
/// 之后「读哪份事实」收进会话存储层，本层拿的是值：**空值 ⇒ 零账，带成本格的值
/// ⇒ 照实读出**。判据两侧都钉在值上，路径错位在这一层已无立足点。
#[tokio::test]
async fn the_ledger_comes_from_the_facts_value() {
    let empty: Vec<Event> = Vec::new();
    assert_eq!(
        GateInputs::of(&empty, &test_ctx()).spent_ms,
        0,
        "还没记过账 ⇒ 零账（不是「不许执行」）"
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

/// 声明闸是**可审计拒绝**：结果回灌 + 父节点收敛 + 违约承诺照记（DE-043 §3）。
///
/// 判据住声明闸（真实实现是 agent 的 `DelegationGate`，DE-025 D-219）；这里钉的是
/// **宿主接线**：工具声明了 `assign_work`、批首读数是「已收回」⇒ 工具不执行，且
/// 「约了却没派成」如实落进承诺出参（`record_effect` ok=false）。
#[tokio::test]
async fn revoked_assign_work_is_an_audited_refusal_with_a_breached_promise() {
    let dir = std::env::temp_dir().join("symbio-test/session-tool-executor-elimination");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建 L0 存档目录");

    let ctx = {
        let req = PluginSimpleRequest::new(None, None);
        let visitor = Arc::new(DefaultToolVisitor::new());
        visitor
            .put(CapabilityRow::tool(Arc::new(DeclTool(CapabilityMeta {
                name: "agent_run".to_string(),
                behaviors: &[
                    symbio_plugin_api::behaviors::ASSIGN_WORK,
                    symbio_plugin_api::behaviors::DELEGATION_PROMISE,
                ],
                ..Default::default()
            })) as Arc<dyn Capability>))
            .await;
        req.set(
            symbio_sdk::capability::CAPABILITY_VISITOR,
            visitor as Arc<dyn Registry>,
        );
        Arc::new(req) as Arc<dyn PluginInvokeRequest>
    };

    let sink = ExecEventSink::silent();
    let abort = ExecAbortSignal::new();
    let parent = ChatNode {
        id: "tc-elim".into(),
        // 身份写 `principal`，不写存量 `role` 格（S-014 棘轮）。
        principal: Some(symbio_sdk::chat::principal::Principal::tool_principal(
            "agent_run",
        )),
        msg_type: Some(NodeKind::Skeleton),
        meta: Some(serde_json::json!({"style_tag": "tool", "tool_name": "agent_run"})),
        status: Some(NodeStatus::Streaming),
        ..Default::default()
    };
    let tcs = vec![TurnToolCallInfo {
        id: Some("tc-elim".into()),
        wire_id: None,
        name: Some("agent_run".into()),
        arguments: json!({ "prompt": "做件事" }),
        parse_error: None,
        overflow_notice: None,
    }];
    let mut effects = Vec::new();
    let (msgs, updates) = process_tool_calls_in_env(
        tcs,
        &None,
        &sink,
        &abort,
        ctx,
        std::slice::from_ref(&parent),
        &env_for(&dir),
        &EventStore::new(),
        true,
        &mut effects,
    )
    .await;

    assert_eq!(msgs.len(), 1, "被闸拦下的调用也必须有结果子节点");
    let text = msgs[0]
        .content
        .as_ref()
        .map(|c| c.to_text())
        .unwrap_or_default();
    assert!(
        text.contains("Refused:") && text.contains("assign.work"),
        "结果要写明是淘汰闸拒的（实际: {text}）"
    );
    assert_eq!(updates.len(), 1, "父节点必须收到终态（否则永远转下去）");
    assert_eq!(
        updates[0]
            .meta
            .as_ref()
            .and_then(|m| m.get("failure_kind"))
            .and_then(|v| v.as_str()),
        Some(failure_kind::PERMISSION_DENIED),
    );
    assert_eq!(effects.len(), 1, "拒止同样要交一条效果输入");
    assert!(!effects[0].success, "没派成 = 没履约");
    assert!(
        effects[0].why.contains("assign.work 已被淘汰回路收回"),
        "违约理由要写明成因（实际: {}）",
        effects[0].why
    );

    std::fs::remove_dir_all(&dir).ok();
}

// ==================== 端口缺席的响亮失败（逐笔驱动仍要有结果） ====================
//
// 批内并行已移除：并发、顺序、熔断都在端口 provider 内部（DE-047 D-311）。
// 这里只保留「表里有工具但没有 `port.tool` 行」这一条读数形态的用例。

use symbio_kernel::capability::CapabilityRiskLevel;

/// 表里能被找到的假工具：只用来制造「命中工具但缺端口行」的读数形态。
struct ProbeTool {
    meta: CapabilityMeta,
}

impl ProbeTool {
    fn new(name: &str, risk: CapabilityRiskLevel) -> Self {
        Self {
            meta: CapabilityMeta {
                name: name.to_string(),
                risk: Some(risk),
                ..Default::default()
            },
        }
    }
}

#[async_trait::async_trait]
impl symbio_kernel::capability::Capability for ProbeTool {
    fn meta(&self) -> CapabilityMeta {
        self.meta.clone()
    }

    async fn execute(
        &self,
        _args: serde_json::Value,
        _env: &ExecEnv,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> Result<serde_json::Value, PluginError> {
        Ok(serde_json::json!({ "content": format!("done:{}", self.meta.name) }))
    }
}

/// 无钩子的父插件：执行体触发钩子时装了钩子插件的行才会生效；没有就按缺省放行。
struct NopParent;

#[async_trait::async_trait]
impl Plugin for NopParent {
    fn meta(&self) -> symbio_kernel::plugin::PluginMeta {
        symbio_kernel::plugin::PluginMeta::new("nop", "Nop")
    }

    async fn route(
        self: Arc<Self>,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> symbio_kernel::plugin::PluginInvokeResponse<PluginPayload> {
        Err(PluginError::NotFound("nop".into()))
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> symbio_kernel::plugin::PluginInvokeResponse<PluginPayload> {
        Err(PluginError::NotFound("nop".into()))
    }
}

/// 端口缺席的响亮失败：表里有工具但没有 `port.tool` 行（零工具装配的
/// 读数形态）⇒ 调用以**错误文本回传模型**收场（失败属信息性，模型据此改道），
/// 不许静默跳过、不许伪造结果。
#[tokio::test]
async fn a_missing_tool_port_fails_loudly_on_the_result() {
    let sink = ExecEventSink::silent();
    let abort = ExecAbortSignal::new();
    // 表里有工具（能命中）但**不装端口行**——薄壳在读端口那一步就该失败。
    let ctx = {
        let req = PluginSimpleRequest::new(None, None);
        let visitor = Arc::new(DefaultToolVisitor::new());
        visitor
            .put(CapabilityRow::tool(
                Arc::new(ProbeTool::new("probe_x", CapabilityRiskLevel::Low))
                    as Arc<dyn Capability>,
            ))
            .await;
        req.set(
            symbio_sdk::capability::CAPABILITY_VISITOR,
            visitor as Arc<dyn Registry>,
        );
        Arc::new(req) as Arc<dyn PluginInvokeRequest>
    };
    let parent = Some(Arc::new(NopParent) as Arc<dyn Plugin>);
    let tcs = vec![TurnToolCallInfo {
        id: Some("tc-missing-port".into()),
        wire_id: None,
        name: Some("probe_x".into()),
        arguments: json!({}),
        parse_error: None,
        overflow_notice: None,
    }];

    let (msgs, updates) = process_tool_calls_in_env(
        tcs,
        &parent,
        &sink,
        &abort,
        ctx,
        &[tool_call_node("tc-missing-port", "probe_x")],
        &test_env(),
        &EventStore::new(),
        false,
        &mut Vec::new(),
    )
    .await;

    assert_eq!(msgs.len(), 1, "缺席也必须有结果子节点（工具终态不变量）");
    let text = msg_text(&msgs[0]);
    assert!(
        text.contains("`port.tool` 缺席") && text.contains("未执行"),
        "结果要写明端口缺席且工具未执行（实际: {text}）"
    );
    assert_eq!(updates.len(), 1, "父节点必须同步收敛");
    assert_eq!(
        updates[0]
            .meta
            .as_ref()
            .and_then(|m| m.get("success"))
            .and_then(|v| v.as_bool()),
        Some(false),
        "缺席是失败（success=false），不是伪造的成功"
    );
}

/// 父 ToolCall 的权威副本（执行层从 `context_messages` 取它做终态快照）。
fn tool_call_node(id: &str, name: &str) -> ChatNode {
    ChatNode {
        id: id.to_string(),
        // 身份写 `principal`，不写存量 `role` 格（S-014 棘轮）。
        principal: Some(symbio_sdk::chat::principal::Principal::tool_principal(name)),
        msg_type: Some(NodeKind::Skeleton),
        meta: Some(serde_json::json!({"style_tag": "tool", "tool_name": name})),
        status: Some(NodeStatus::Streaming),
        ..Default::default()
    }
}

fn msg_text(node: &ChatNode) -> String {
    node.content
        .as_ref()
        .map(|c| c.to_text())
        .unwrap_or_default()
}
