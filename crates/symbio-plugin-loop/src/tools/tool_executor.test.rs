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
        test_ctx(),
        &[],
        &test_env(),
        &EventStore::new(),
        false,
        &mut Vec::new(),
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
        test_ctx(),
        &[],
        &test_env(),
        &EventStore::new(),
        false,
        &mut Vec::new(),
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
        test_ctx(),
        &[],
        &test_env(),
        &EventStore::new(),
        false,
        &mut Vec::new(),
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
        test_ctx(),
        &[],
        &test_env(),
        &EventStore::new(),
        false,
        &mut Vec::new(),
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

/// 工具终态不变量：**本批每个 ToolCall 都必须以终态收场，且必须有结果子节点**。
///
/// 中止（`is_aborted`）时循环在第一个工具之前就 `break`，本批一个都没执行——
/// 若不收口，这些节点会停在参数流式阶段广播出的 `Streaming` 上：
/// 前端永远转「运行中」（把"卡住"伪装成"在跑"）。`Streaming` 是瞬态状态
/// （只广播、不落盘），停在 Streaming 是**广播层**的收口缺口，不是持久层问题。
///
/// **只补父节点是不够的**（实测事故）：前端 `ToolCallNode` 的「结果」段按**子节点**
/// 渲染，缺子节点就是「有请求、无响应」，而请求视图那边有 `flatten_chat_messages`
/// 的占位兜底 ⇒ 模型看得见、用户看不见，于是它长期潜伏。两条断言必须同时在。
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
        &mut Vec::new(),
    )
    .await;

    assert_eq!(
        msgs.len(),
        2,
        "未执行的调用同样必须有结果子节点，否则前端卡片有请求、无响应"
    );
    for m in &msgs {
        assert_eq!(m.node_role(), Some(NodeRole::Tool));
        assert_eq!(m.status, Some(NodeStatus::Completed));
        assert!(
            m.content
                .as_ref()
                .map(|c| c.to_text().contains("未执行"))
                .unwrap_or(false),
            "结果必须说明「没跑」，而不是伪造一份看起来像工具返回的文本"
        );
    }
    assert_eq!(updates.len(), 2, "两个未执行的调用都必须收到终态快照");
    for (u, id) in updates.iter().zip(["tc1", "tc2"]) {
        // 完整快照语义：id / role / type 保留，终态字段如实应用
        assert_eq!(u.id, id, "快照必须对应它自己的调用");
        assert_eq!(
            u.node_role(),
            Some(NodeRole::Assistant),
            "完整快照保留节点身份"
        );
        assert!(u.is_tool_call(), "快照必须保持工具骨架形态");
        assert_eq!(u.status, Some(NodeStatus::Completed));
        assert_eq!(
            u.meta
                .as_ref()
                .and_then(|m| m.get("failure_kind"))
                .and_then(|v| v.as_str()),
            Some("aborted"),
            "中止批的未执行必须如实标 aborted（`6cf9520d` 教训：全写成 not_executed 是成因谎报）"
        );
        assert!(u.error.is_none(), "未执行不是错误：挂 error 会让前端渲染 ⚠");
    }
    // 结果子节点必须挂在**它自己那一次调用**上（挂错父节点 = 另一个工具的结果）
    let parents: Vec<&str> = msgs.iter().filter_map(|m| m.parent_id.as_deref()).collect();
    assert_eq!(parents, vec!["tc1", "tc2"]);
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

/// 实测事故（本地会话 `3ea6e4d0`）的机制回归：**交互模式下**前一个工具失败 ⇒
/// 本批剩余被跳过 ⇒ 跳过的那些同样必须留下结果子节点。
///
/// 这条路径是用户报告的现场：4 个 `vdfs_read` 只留下 `failure_kind=not_executed`
/// 的父节点、没有任何子节点，UI 上表现为「工具没有响应，会话却继续往后」。
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
        &mut Vec::new(),
    )
    .await;

    assert_eq!(
        msgs.len(),
        2,
        "被跳过的那一次调用必须有其结果子节点（否则前端只有请求、没有响应）"
    );
    let skipped = msgs
        .iter()
        .find(|m| m.parent_id.as_deref() == Some("tc2"))
        .expect("tc2 的结果子节点缺失");
    assert_eq!(skipped.status, Some(NodeStatus::Completed));
    assert_eq!(
        skipped
            .meta
            .as_ref()
            .and_then(|m| m.get("failure_kind"))
            .and_then(|v| v.as_str()),
        Some("not_executed")
    );
    assert_eq!(updates.len(), 2, "两次调用都必须有终态");
    // 父节点的终态补丁：**没跑到**的成因写进 `meta.failure_kind`，且状态是
    // `Completed` 而不是 `Failed`——`Failed` 会让 `get_context_messages` 过滤掉
    // 父 ToolCall 节点，留下「孤儿 tool 结果」→ 下一轮请求携带非法 `tool_call_id`。
    // 工具失败属**信息性**，差异由 `failure_kind` 承载。
    let skipped_parent = updates
        .iter()
        .find(|u| u.id == "tc2")
        .expect("tc2 的父节点终态缺失");
    assert_eq!(skipped_parent.status, Some(NodeStatus::Completed));
    assert!(skipped_parent.error.is_none());
    assert_eq!(
        skipped_parent
            .meta
            .as_ref()
            .and_then(|m| m.get("success"))
            .and_then(|v| v.as_bool()),
        Some(false)
    );
    assert_eq!(
        skipped_parent
            .meta
            .as_ref()
            .and_then(|m| m.get("failure_kind"))
            .and_then(|v| v.as_str()),
        Some("not_executed")
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

/// user_prompt 节点的身份归编排层：`id = result_msg_id`、`parent_id = tool_call_id`。
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

// ==================== 外部执行闸门 ====================

/// 带会话 id 的请求上下文。成本台账不再按它定位——事实轨以**值**交给执行层
/// （D-084 §1-A）；会话 id 仍随上下文走，因为其余工具面按它归属。
fn gate_ctx(session_id: &str) -> Arc<dyn PluginInvokeRequest> {
    let req = PluginSimpleRequest::new(None, None);
    req.set(symbio_sdk::keys::SESSION_ID, session_id.to_string());
    Arc::new(req)
}

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

/// 事实源那一半：台账累计把预算吃满 ⇒ 工具**不执行**，且 `Break` 走出参。
///
/// 这条用例钉的是 [DE-039 §6](../../../../docs/003-design/039-外部执行与熔断.md "DE-039-外部执行与熔断")
/// 验收 2 的前半句（预算耗尽必须判得出来，不允许静默继续）；后半句——
/// 理由真的落成一条 `control × opened` 事件——钉在事实源入格那一侧的用例里
/// （写方是 `derived_facts::record_derived`，由 v2 运行器在轮末收束调用）；工具执行层
/// 没有事实源，入格要到轮末收束那一步才发生。
#[tokio::test]
async fn budget_exhausted_refuses_the_tool_and_reports_a_break() {
    let dir = std::env::temp_dir().join("symbio-test/session-tool-executor-gate");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建 L0 存档目录");

    // 台账：一条已耗超过会话累计上限的收束事件（`cost.ledger` 按 actor 归账，
    // D-044：闸门读台账，不读运行时计数器）。事实轨以**值**交给执行层
    // （D-084 §1-A）——夹具直接造这份切片，不摆存储目录。
    let sid = "gate-sess";
    let facts = EventStore::new();
    facts
        .append(
            symbio_kernel::event::Event::pending(
                "v2f-gate",
                symbio_plugin_api::events::EVENT_CHAT_ASSISTANT_FINAL,
                symbio_kernel::event::Entity::Turn,
                symbio_kernel::event::Verb::Closed,
                0,
                symbio_sdk::authz::PRINCIPAL_MAIN,
            )
            .with_cost_ms(LatencyTier::Autonomic.budget_ms() + 1),
        )
        .expect("入格");

    let sink = ExecEventSink::silent();
    let abort = ExecAbortSignal::new();
    // 父 ToolCall 节点：参数流式阶段已广播过（生产里它就在 `context.messages`），
    // 终态定稿要从这份权威转写取副本——不给它，`emit_parent_finalized` 会判协议违例。
    let parent = ChatNode {
        id: "tc-gate".into(),
        principal: Some(principal_for(NodeRole::Assistant)),
        msg_type: Some(NodeKind::Skeleton),
        meta: Some(serde_json::json!({"style_tag": "tool", "tool_name": "vdfs_list"})),
        status: Some(NodeStatus::Streaming),
        ..Default::default()
    };
    let tcs = vec![TurnToolCallInfo {
        id: Some("tc-gate".into()),
        wire_id: None,
        name: Some("vdfs_list".into()),
        arguments: json!({ "path": "." }),
        parse_error: None,
        overflow_notice: None,
    }];
    let mut breaks: Vec<&'static str> = Vec::new();
    let (msgs, updates) = process_tool_calls_in_env(
        tcs,
        &None,
        &sink,
        &abort,
        gate_ctx(sid),
        std::slice::from_ref(&parent),
        &env_for(&dir),
        &facts,
        false,
        &mut Vec::new(),
        &mut breaks,
    )
    .await;

    assert_eq!(msgs.len(), 1, "被闸门拦下的调用也必须有结果子节点");
    assert!(
        msgs[0]
            .content
            .as_ref()
            .map(|c| c.to_text().contains("Refused:"))
            .unwrap_or(false),
        "结果要写明是被拒的，不是照常执行后的输出"
    );
    assert_eq!(updates.len(), 1, "父节点必须收到终态（否则永远转下去）");
    assert_eq!(
        updates[0]
            .meta
            .as_ref()
            .and_then(|m| m.get("failure_kind"))
            .and_then(|v| v.as_str()),
        Some(failure_kind::TOOL_UNAVAILABLE),
        "熔断不是「等待用户」，是让模型改走别的路"
    );
    assert_eq!(
        breaks,
        vec!["budget-exhausted"],
        "Break 必须经出参交回调用方——它没有这一格就是静默继续（DE-039 §6 验收 2）"
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// 预算在生效而非常量（DE-039 §6.4 的反向用例在生产判据上的形状）：
/// 同一份事实源，预算放得比已耗宽 ⇒ 同一次判定翻成放行、出参为空。
#[tokio::test]
async fn relaxing_the_budget_turns_the_same_facts_into_a_pass() {
    let inputs = BreakerInputs::of(&[], &gate_ctx("gate-open"));
    assert_eq!(inputs.spent_ms, 0, "空事实切片 ⇒ 零账，不是故障");
    assert!(inputs.authorized, "agent:main 持外部执行能力");

    // 预算收紧到「已耗 + 申请」之下 ⇒ 熔断；放宽到之上 ⇒ 放行。
    assert_eq!(
        super::super::CircuitBreaker.gate(
            inputs.authorized,
            inputs.spent_ms,
            inputs.requested_ms,
            inputs.spent_ms + inputs.requested_ms - 1,
            0,
            symbio_kernel::adapters::LatencyTier::Deep.budget_ms(),
        ),
        super::super::GateDecision::Break {
            reason: "budget-exhausted"
        },
        "预算真的在参与判定（常量判定拿不出这一对相反的结论）"
    );
    assert_eq!(
        super::super::CircuitBreaker.gate(
            inputs.authorized,
            inputs.spent_ms,
            inputs.requested_ms,
            inputs.spent_ms + inputs.requested_ms + 1,
            0,
            symbio_kernel::adapters::LatencyTier::Deep.budget_ms(),
        ),
        super::super::GateDecision::Allow
    );
}

/// 台账读的是**交给闸门的事实值**。
///
/// 历史事故：曾有实现按 `<插件目录>/v2-events.wal` 取账（漏了会话 id 那层）⇒
/// `spent_ms` 恒 0 ⇒ 预算分支永远走不到，且没有任何东西会变红。D-084 §1-A
/// 之后「读哪份事实」收进会话存储层，本层拿的是值：**空值 ⇒ 零账，带成本格的值
/// ⇒ 照实读出**。判据两侧都钉在值上，路径错位在这一层已无立足点。
#[tokio::test]
async fn the_ledger_comes_from_the_facts_value() {
    let empty: Vec<Event> = Vec::new();
    assert_eq!(
        BreakerInputs::of(&empty, &test_ctx()).spent_ms,
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
        BreakerInputs::of(&facts.range(Seq::new(0)), &test_ctx()).spent_ms,
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
        &mut Vec::new(),
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

// ==================== 批内并行：只读并行、其余串行、收口按序 ====================
//
// 这组用例钉四件事，各自都有一条能红的判据：
// 1. 两个只读调用**真的同时在途**（用 2 方屏障；串行实现会死等 → 超时红）；
// 2. `Medium` 调用与任何调用**不重叠**（峰值并发 = 1；写侧顺序不确定是并行最坏的代价）；
// 3. 交互模式整批串行（审批 / 提问一次只处理一张卡）；
// 4. 结果与事件**逐项按调用顺序**收口：完成次序被故意倒过来，结果次序仍不变。
//
// 另有中止传播：批次里一个信号中止 ⇒ 所有在途调用都收到同一信号并各自收敛
// （同一 `ExecAbortSignal`，不是每条各造一个）。

use std::sync::atomic::{AtomicUsize, Ordering};

use symbio_kernel::capability::CapabilityRiskLevel;

/// 记录并发窗口的假工具：不进则退，峰值可读。
struct ProbeTool {
    meta: CapabilityMeta,
    barrier: Option<Arc<tokio::sync::Barrier>>,
    sleep_ms: u64,
    /// 执行体返回错误：用于「部分失败」——失败那一笔只关自己的账。
    fail: bool,
    active: Arc<AtomicUsize>,
    peak: Arc<AtomicUsize>,
}

impl ProbeTool {
    fn new(
        name: &str,
        risk: CapabilityRiskLevel,
        active: &Arc<AtomicUsize>,
        peak: &Arc<AtomicUsize>,
    ) -> Self {
        Self {
            meta: CapabilityMeta {
                name: name.to_string(),
                risk: Some(risk),
                ..Default::default()
            },
            barrier: None,
            sleep_ms: 0,
            fail: false,
            active: active.clone(),
            peak: peak.clone(),
        }
    }

    fn with_barrier(mut self, barrier: Arc<tokio::sync::Barrier>) -> Self {
        self.barrier = Some(barrier);
        self
    }

    fn with_sleep(mut self, ms: u64) -> Self {
        self.sleep_ms = ms;
        self
    }

    fn with_failure(mut self) -> Self {
        self.fail = true;
        self
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
        let now = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(now, Ordering::SeqCst);
        if let Some(barrier) = &self.barrier {
            // 只有两条**同时在途**才过得去：串行实现会一直等在这里。
            barrier.wait().await;
        }
        if self.sleep_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(self.sleep_ms)).await;
        }
        self.active.fetch_sub(1, Ordering::SeqCst);
        if self.fail {
            return Err(PluginError::InternalError(format!(
                "boom:{}",
                self.meta.name
            )));
        }
        Ok(serde_json::json!({ "content": format!("done:{}", self.meta.name) }))
    }
}

/// 无钩子的父插件：`fire_hook` 拿到 `Err` ⇒ 按缺省放行（生产上无钩子插件同形）。
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

async fn ctx_with_probes(tools: Vec<Arc<ProbeTool>>) -> Arc<dyn PluginInvokeRequest> {
    let req = PluginSimpleRequest::new(None, None);
    let visitor = Arc::new(DefaultToolVisitor::new());
    for tool in tools {
        visitor
            .put(CapabilityRow::tool(tool as Arc<dyn Capability>))
            .await;
    }
    // `port.tool` 行：真执行经端口合同取——替身执行器见 `test_support.rs`。
    super::super::super::test_support::put_tool_port_row(&visitor).await;
    req.set(
        symbio_sdk::capability::CAPABILITY_VISITOR,
        visitor as Arc<dyn Registry>,
    );
    Arc::new(req) as Arc<dyn PluginInvokeRequest>
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
            .put(CapabilityRow::tool(Arc::new(ProbeTool::new(
                "probe_x",
                CapabilityRiskLevel::Low,
                &Arc::new(AtomicUsize::new(0)),
                &Arc::new(AtomicUsize::new(0)),
            )) as Arc<dyn Capability>))
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

fn probe_calls(pairs: &[(&str, &str)]) -> Vec<TurnToolCallInfo> {
    pairs
        .iter()
        .map(|(id, name)| TurnToolCallInfo {
            id: Some((*id).to_string()),
            wire_id: None,
            name: Some((*name).to_string()),
            arguments: serde_json::json!({}),
            parse_error: None,
            overflow_notice: None,
        })
        .collect()
}

fn msg_text(node: &ChatNode) -> String {
    node.content
        .as_ref()
        .map(|c| c.to_text())
        .unwrap_or_default()
}

/// 记录广播帧的出口（`ExecEventSink::Direct`）：用来钉「每条调用的运行态先于其结果」。
struct FrameLog(Arc<std::sync::Mutex<Vec<ChatNode>>>);

#[async_trait::async_trait]
impl symbio_kernel::exec::ExecTranscriptWriter for FrameLog {
    async fn apply(&self, message: ChatNode) {
        self.0.lock().unwrap().push(message);
    }
}

/// 只读调用并行执行；完成次序被倒过来，**结果仍按调用顺序**收口；
/// 每条调用的 `Streaming` 帧先于它自己的结果帧。
#[tokio::test]
async fn read_only_calls_run_concurrently_and_report_in_call_order() {
    let dir = test_dir().join("t190-parallel");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建 L0 存档目录");

    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    // a 先开跑但最后完成（60ms），b 后开跑先完成（20ms）：完成次序 = b, a。
    let a = Arc::new(
        ProbeTool::new("probe_a", CapabilityRiskLevel::Low, &active, &peak)
            .with_barrier(barrier.clone())
            .with_sleep(60),
    );
    let b = Arc::new(
        ProbeTool::new("probe_b", CapabilityRiskLevel::Low, &active, &peak)
            .with_barrier(barrier.clone())
            .with_sleep(20),
    );
    let ctx = ctx_with_probes(vec![a, b]).await;
    let parent = Some(Arc::new(NopParent) as Arc<dyn Plugin>);
    let frames = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = ExecEventSink::direct(Arc::new(FrameLog(frames.clone())));
    let abort = ExecAbortSignal::new();
    let nodes = vec![
        tool_call_node("tc-a", "probe_a"),
        tool_call_node("tc-b", "probe_b"),
    ];
    let mut effects = Vec::new();
    let mut breaks = Vec::new();

    let (msgs, updates) = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        process_tool_calls_in_env(
            probe_calls(&[("tc-a", "probe_a"), ("tc-b", "probe_b")]),
            &parent,
            &sink,
            &abort,
            ctx,
            &nodes,
            &env_for(&dir),
            &EventStore::new(),
            false,
            &mut effects,
            &mut breaks,
        ),
    )
    .await
    .expect("两个只读调用必须能同时在途：串行实现会一直卡在 2 方屏障上");

    assert_eq!(
        peak.load(Ordering::SeqCst),
        2,
        "只读调用的执行窗口必须重叠（峰值并发 = 2）"
    );
    assert_eq!(msgs.len(), 2);
    assert!(
        msg_text(&msgs[0]).contains("done:probe_a"),
        "结果批序 = 调用顺序（第一条必须是 a，实际: {}）",
        msg_text(&msgs[0])
    );
    assert!(
        msg_text(&msgs[1]).contains("done:probe_b"),
        "结果批序 = 调用顺序（第二条必须是 b，实际: {}）",
        msg_text(&msgs[1])
    );

    // ── 事件批序：每条调用的 Streaming 帧先于它自己的结果帧，结果帧按调用顺序 ──
    let frames = frames.lock().unwrap().clone();
    let result_idx = |tc: &str| {
        frames.iter().position(|f| {
            f.node_role() == Some(NodeRole::Tool) && f.parent_id.as_deref() == Some(tc)
        })
    };
    let running_idx = |tc: &str| {
        frames
            .iter()
            .position(|f| f.id == tc && f.status == Some(NodeStatus::Streaming))
    };
    for tc in ["tc-a", "tc-b"] {
        let running = running_idx(tc).unwrap_or_else(|| panic!("{tc} 缺少运行态帧"));
        let result = result_idx(tc).unwrap_or_else(|| panic!("{tc} 缺少结果帧"));
        assert!(
            running < result,
            "{tc} 的运行态必须先于其结果（running={running}, result={result}）"
        );
    }
    assert!(
        result_idx("tc-a").unwrap() < result_idx("tc-b").unwrap(),
        "结果帧顺序必须与调用顺序一致（完成次序相反也不许倒）"
    );

    // 父节点终态同样按调用顺序。
    assert_eq!(updates.len(), 2);
    assert_eq!(updates[0].id, "tc-a");
    assert_eq!(updates[1].id, "tc-b");
    assert_eq!(updates[0].status, Some(NodeStatus::Completed));
    assert_eq!(updates[1].status, Some(NodeStatus::Completed));

    std::fs::remove_dir_all(&dir).ok();
}

// 写侧档独占执行窗口的判据已随并发策略移入执行体（`symbio-plugin-tool::port`，
// DE-047 D-311）；环侧只钉「不再替执行体做策略」：一笔债准备完就交出去。
/// 环不按风险档串行：两条调用一起在途，由执行体自己决定并发（D-311）。
#[tokio::test]
async fn the_loop_does_not_serialize_calls_by_risk() {
    let dir = test_dir().join("t190-no-policy");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建 L0 存档目录");

    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let side = Arc::new(
        ProbeTool::new("probe_write", CapabilityRiskLevel::Medium, &active, &peak).with_sleep(20),
    );
    let read = Arc::new(
        ProbeTool::new("probe_read", CapabilityRiskLevel::Low, &active, &peak).with_sleep(20),
    );
    let calls = probe_calls(&[("tc-w", "probe_write"), ("tc-r", "probe_read")]);
    let nodes = vec![
        tool_call_node("tc-w", "probe_write"),
        tool_call_node("tc-r", "probe_read"),
    ];
    let ctx = ctx_with_probes(vec![side, read]).await;
    let parent = Some(Arc::new(NopParent) as Arc<dyn Plugin>);
    let sink = ExecEventSink::silent();
    let abort = ExecAbortSignal::new();
    let mut effects = Vec::new();
    let mut breaks = Vec::new();
    let (msgs, _updates) = process_tool_calls_in_env(
        calls,
        &parent,
        &sink,
        &abort,
        ctx,
        &nodes,
        &env_for(&dir),
        &EventStore::new(),
        false,
        &mut effects,
        &mut breaks,
    )
    .await;

    assert_eq!(msgs.len(), 2, "两条都必须有结果");
    assert_eq!(
        peak.load(Ordering::SeqCst),
        2,
        "环不再按风险档串行：两条调用应一起在途（策略归执行体）"
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// 交互模式整批串行——即使每条都是只读档（审批 / 提问一次只处理一张卡）。
#[tokio::test]
async fn interactive_mode_keeps_the_batch_serial() {
    let dir = test_dir().join("t190-interactive");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建 L0 存档目录");

    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let a = Arc::new(
        ProbeTool::new("probe_i1", CapabilityRiskLevel::Low, &active, &peak).with_sleep(20),
    );
    let b = Arc::new(
        ProbeTool::new("probe_i2", CapabilityRiskLevel::Low, &active, &peak).with_sleep(20),
    );
    let ctx = ctx_with_probes(vec![a, b]).await;
    ctx.set(symbio_sdk::keys::MODE, "interactive".to_string());
    let parent = Some(Arc::new(NopParent) as Arc<dyn Plugin>);
    let sink = ExecEventSink::silent();
    let abort = ExecAbortSignal::new();
    let nodes = vec![
        tool_call_node("tc-i1", "probe_i1"),
        tool_call_node("tc-i2", "probe_i2"),
    ];
    let mut effects = Vec::new();
    let mut breaks = Vec::new();

    let (msgs, _updates) = process_tool_calls_in_env(
        probe_calls(&[("tc-i1", "probe_i1"), ("tc-i2", "probe_i2")]),
        &parent,
        &sink,
        &abort,
        ctx,
        &nodes,
        &env_for(&dir),
        &EventStore::new(),
        false,
        &mut effects,
        &mut breaks,
    )
    .await;

    assert_eq!(msgs.len(), 2);
    assert_eq!(
        peak.load(Ordering::SeqCst),
        1,
        "交互模式必须整批串行（审批卡不能两张同时推到用户面前）"
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// 部分失败：同批两条只读调用，一条失败一条成功——失败那一笔**只关自己的账**，
/// 兄弟照常跑完并正常收口。「整批失败」会让已经成功的世界副作用被事实面否认。
#[tokio::test]
async fn a_failed_sibling_does_not_cancel_the_rest_of_the_batch() {
    let dir = test_dir().join("t190-partial-failure");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建 L0 存档目录");

    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    // 失败的那条先结束（20ms），成功的那条后结束（60ms）：若失败会取消兄弟，
    // 成功那条就不会出现在结果里。
    let bad = Arc::new(
        ProbeTool::new("probe_bad", CapabilityRiskLevel::Low, &active, &peak)
            .with_sleep(20)
            .with_failure(),
    );
    let good = Arc::new(
        ProbeTool::new("probe_good", CapabilityRiskLevel::Low, &active, &peak).with_sleep(60),
    );
    let ctx = ctx_with_probes(vec![bad, good]).await;
    let parent = Some(Arc::new(NopParent) as Arc<dyn Plugin>);
    let sink = ExecEventSink::silent();
    let abort = ExecAbortSignal::new();
    let nodes = vec![
        tool_call_node("tc-bad", "probe_bad"),
        tool_call_node("tc-good", "probe_good"),
    ];
    let mut effects = Vec::new();
    let mut breaks = Vec::new();

    let (msgs, updates) = process_tool_calls_in_env(
        probe_calls(&[("tc-bad", "probe_bad"), ("tc-good", "probe_good")]),
        &parent,
        &sink,
        &abort,
        ctx,
        &nodes,
        &env_for(&dir),
        &EventStore::new(),
        false,
        &mut effects,
        &mut breaks,
    )
    .await;

    assert_eq!(msgs.len(), 2, "失败不得吞掉兄弟的结果");
    assert_eq!(peak.load(Ordering::SeqCst), 2, "两条必须同时在途");
    assert!(
        msg_text(&msgs[0]).contains("boom:probe_bad"),
        "第一条是自己的失败结果（实际: {}）",
        msg_text(&msgs[0])
    );
    assert!(
        msg_text(&msgs[1]).contains("done:probe_good"),
        "兄弟照常收口（实际: {}）",
        msg_text(&msgs[1])
    );

    assert_eq!(updates.len(), 2);
    assert_eq!(
        updates[0]
            .meta
            .as_ref()
            .and_then(|m| m.get("failure_kind"))
            .and_then(|v| v.as_str()),
        Some("error"),
        "失败那一笔的父终态带 failure_kind=error"
    );
    assert_eq!(
        updates[1]
            .meta
            .as_ref()
            .and_then(|m| m.get("success"))
            .and_then(|v| v.as_bool()),
        Some(true),
        "成功那一笔不受兄弟失败影响"
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// 中止传播：一次中止 ⇒ 批内**所有在途**调用都收到同一信号并各自收敛。
#[tokio::test]
async fn abort_reaches_every_in_flight_call() {
    let dir = test_dir().join("t190-abort");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建 L0 存档目录");

    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    // 两条都长睡：若中止没有传到某一条，它会一直占到 10s 超时。
    let a = Arc::new(
        ProbeTool::new("probe_abort_a", CapabilityRiskLevel::Low, &active, &peak)
            .with_sleep(10_000),
    );
    let b = Arc::new(
        ProbeTool::new("probe_abort_b", CapabilityRiskLevel::Low, &active, &peak)
            .with_sleep(10_000),
    );
    let ctx = ctx_with_probes(vec![a, b]).await;
    let parent = Some(Arc::new(NopParent) as Arc<dyn Plugin>);
    let sink = ExecEventSink::silent();
    let abort = ExecAbortSignal::new();
    let nodes = vec![
        tool_call_node("tc-aa", "probe_abort_a"),
        tool_call_node("tc-ab", "probe_abort_b"),
    ];
    let mut effects = Vec::new();
    let mut breaks = Vec::new();

    // 等两条都真正开跑再中止——中止发生在启动之前测的是另一条路径。
    let started = active.clone();
    let stopper = abort.clone();
    let trigger = tokio::spawn(async move {
        for _ in 0..500 {
            if started.load(Ordering::SeqCst) == 2 {
                stopper.abort();
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    });

    let (msgs, updates) = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        process_tool_calls_in_env(
            probe_calls(&[("tc-aa", "probe_abort_a"), ("tc-ab", "probe_abort_b")]),
            &parent,
            &sink,
            &abort,
            ctx,
            &nodes,
            &env_for(&dir),
            &EventStore::new(),
            false,
            &mut effects,
            &mut breaks,
        ),
    )
    .await
    .expect("中止后必须立即收口（在途调用收到同一信号）");
    trigger.await.ok();

    assert_eq!(msgs.len(), 2, "两条在途调用都必须留下结果");
    for msg in &msgs {
        assert!(
            msg_text(msg).contains("被用户中止"),
            "每条在途调用都要收敛为中止结果（实际: {}）",
            msg_text(msg)
        );
        assert_eq!(msg.status, Some(NodeStatus::Completed));
    }
    assert_eq!(updates.len(), 2, "父节点终态一条都不能少");
    for update in &updates {
        assert_eq!(update.status, Some(NodeStatus::Completed));
    }

    std::fs::remove_dir_all(&dir).ok();
}
