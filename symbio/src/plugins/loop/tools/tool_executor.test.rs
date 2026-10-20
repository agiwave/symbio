//! `tool_executor` 模块的单元测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）：
//! `tools/tool_executor.rs` 只保留生产代码，测试全部放本文件。

use super::*;
use crate::symbio_core::PluginSimpleRequest;
// `TaskStatus` 是**测试自己**要的：它只出现在断言里，父模块的签名用不到它，
// 因此 `use super::*` 带不过来（记录类型自 6b-① 起住 core 词汇表）。
use crate::symbio_core::facts::TaskStatus;
// `Store` 是**测试自己**要的（`store.append` 是 trait 方法；父模块顶层不导入它——
// 生产侧的读发生在 `tool_executor/breaker.rs` 子模块里）；`use super::*` 因此带不过来。
use crate::symbio_core::Store;
// `failure_kind` 与 `Seq` 同样是**测试自己**要的：批首闸门读数与逐条执行的闸门段
// 已按 T-179 阶段拆去 `tools/dispatch/`，父模块顶层不再导入它们——测试自己拿。
use crate::symbio_core::{failure_kind, Seq};
// `LatencyTier` 是**测试自己**要的：它只被父模块里的熔断段用到，那段已按域
// 拆去 `tool_executor/breaker.rs`（[plan/13 C3](../../../../../docs/archive/13-质量改进总纲.md)），
// 父模块顶层不再导入，`use super::*` 便带不过来——测试自己拿。
use crate::providers::DefaultToolVisitor;
use crate::symbio_core::{
    Capability, CapabilityMeta, CapabilityRow, ExecEnv, PluginError, PluginInvokeRequestExt,
    Registry,
};
use crate::symbio_core::{EventStore, LatencyTier};

fn test_ctx() -> Arc<dyn PluginInvokeRequest> {
    Arc::new(PluginSimpleRequest::new(None, None))
}

/// 会话的 L0 存档目录（测试用临时目录）——工具结果存档落在这里。
/// 生产上由 session 在派发前算好，作为**值**传进来；执行层不拼会话存储路径。
fn test_dir() -> std::path::PathBuf {
    std::env::temp_dir().join("symbio-test/session-tool-executor")
}

// ── T-158 注册面：闸门问声明，测试喂一份与生产同形的声明 ────────────────
//
// 机制化之前这些用例直接写工具名调 `note_*`；判据搬到 `CapabilityMeta` 的
// 自声明位之后，同一批用例经 `ToolDecls` 喂注册面——生产三件套各 declare 各的
// 位（`agent_run` 派活 + 立约、`todo_write` 任务表），外加一个什么都不声明的
// 普通工具（生产里绝大多数工具的形态）。

/// 只带元数据的假工具：注册表怎么读，测试就怎么喂。
struct DeclTool(CapabilityMeta);

#[async_trait::async_trait]
impl crate::symbio_core::Capability for DeclTool {
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
                provides_assign_work: assign_work,
                records_delegation_promise: promise,
                owns_task_table: task_table,
                ..Default::default()
            })) as Arc<dyn Capability>))
            .await;
    }
    ctx.set(
        crate::symbio_core::CAPABILITY_VISITOR,
        visitor as Arc<dyn Registry>,
    );
    ToolDecls::of_metas(&crate::symbio_core::tool_metas(&ctx).await)
}

// ==================== extract_result：判定顺序即约定 ====================
//
// 这组用例的作用是**钉住判定顺序**——它现在是 `extract_result` 文档里的表，
// 不再是「读源码才知道」的实现细节。顺序变了这里会红。

#[test]
fn content_wins_over_output() {
    // 1 优先于 2：两个都在时取 content
    assert_eq!(
        extract_result(&json!({"content": "正文", "output": "命令输出"})),
        "正文"
    );
}

#[test]
fn output_is_read_when_content_absent() {
    // `shell` 的返回形状（字段名约定见 `local/shell.rs::execute_streaming`）
    assert_eq!(extract_result(&json!({"output": "ls 结果"})), "ls 结果");
}

#[test]
fn success_true_falls_back_to_the_whole_payload() {
    // 3：既无 content 也无 output、但 success=true → 整包 JSON
    // （形状不透明时宁可把结构给模型看，也不要凭空造一段文本）
    assert_eq!(
        extract_result(&json!({"success": true, "n": 3})),
        r#"{"n":3,"success":true}"#
    );
}

#[test]
fn success_false_renders_the_error_field() {
    assert_eq!(
        extract_result(&json!({"success": false, "error": "权限不足"})),
        "Error: 权限不足"
    );
}

#[test]
fn success_false_without_error_says_unknown() {
    assert_eq!(
        extract_result(&json!({"success": false})),
        "Error: unknown error"
    );
}

#[test]
fn bare_string_is_taken_as_is() {
    assert_eq!(extract_result(&json!("直接给一段文本")), "直接给一段文本");
}

#[test]
fn opaque_payload_is_stringified() {
    // 5：兜底——形状完全不认识时整包 JSON（含 `VdfsContent` 这类结构体序列化）
    assert_eq!(
        extract_result(&json!({"path": "a.txt", "lines": 12})),
        r#"{"lines":12,"path":"a.txt"}"#
    );
    assert_eq!(extract_result(&json!(null)), "null");
    assert_eq!(extract_result(&json!(42)), "42");
}

#[test]
fn non_string_content_does_not_hijack_the_read() {
    // `content` 不是字符串 ⇒ 第 1 条不成立，继续往下走（不是「命中即返回」）。
    // 这是顺序判定与「取第一个存在的键」的分水岭：后者会把数组
    // 原样丢给模型。
    assert_eq!(
        extract_result(&json!({"content": [1, 2], "output": "真正的输出"})),
        "真正的输出"
    );
}

#[test]
fn empty_content_string_is_a_hit() {
    // 空串是**合法**的正文（工具确实没有内容可说），不跳到 output
    assert_eq!(extract_result(&json!({"content": "", "output": "x"})), "");
}

#[test]
fn result_field_constants_match_the_documented_order() {
    // 常量与文档表里的字段名一致（改常量就得改文档，反之亦然）
    assert_eq!((RESULT_CONTENT, RESULT_OUTPUT), ("content", "output"));
}

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

    let (msgs, updates) = process_tool_calls_async(
        tcs,
        &None,
        &sink,
        &abort,
        test_ctx(),
        &[],
        &test_dir(),
        &EventStore::new(),
        false,
        &mut Vec::new(),
        &mut Vec::new(),
        &mut Vec::new(),
    )
    .await;

    assert_eq!(msgs.len(), 1, "必须生成失败结果子节点（而非跳过）");
    assert_eq!(msgs[0].role, Some(NodeRole::Tool));
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

    let (msgs, updates) = process_tool_calls_async(
        tcs,
        &None,
        &sink,
        &abort,
        test_ctx(),
        &[],
        &test_dir(),
        &EventStore::new(),
        false,
        &mut Vec::new(),
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

    let (msgs, updates) = process_tool_calls_async(
        tcs,
        &None,
        &sink,
        &abort,
        test_ctx(),
        &[],
        &test_dir(),
        &EventStore::new(),
        false,
        &mut Vec::new(),
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

    let (msgs, updates) = process_tool_calls_async(
        tcs,
        &None,
        &sink,
        &abort,
        test_ctx(),
        &[],
        &test_dir(),
        &EventStore::new(),
        false,
        &mut Vec::new(),
        &mut Vec::new(),
        &mut Vec::new(),
    )
    .await;

    assert_eq!(msgs.len(), 1, "必须生成失败结果子节点（而非跳过）");
    assert_eq!(msgs[0].role, Some(NodeRole::Tool));
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

/// [T-140] 步 2 判据：**第二次同参调用不再静默**。
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

    let (msgs, _updates) = process_tool_calls_async(
        tcs,
        &None,
        &sink,
        &abort,
        test_ctx(),
        &tc_context(&["tc-a", "tc-b"]),
        &test_dir(),
        &EventStore::new(),
        false,
        &mut Vec::new(),
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

    let (msgs, _updates) = process_tool_calls_async(
        vec![tc("tc-new", "vdfs_list", json!({ "path": "." }))],
        &None,
        &sink,
        &abort,
        test_ctx(),
        &context,
        &test_dir(),
        &EventStore::new(),
        false,
        &mut Vec::new(),
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

    let (msgs, _updates) = process_tool_calls_async(
        vec![tc("tc-new", "vdfs_list", json!({ "path": "b" }))],
        &None,
        &sink,
        &abort,
        test_ctx(),
        &context,
        &test_dir(),
        &EventStore::new(),
        false,
        &mut Vec::new(),
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

/// 回归（真实事故）：中文参数摘要不得在字节边界上 panic。
///
/// 事故现场：`write_file` 传入含中文的 `content`，`args_summary(.., 200)`
/// 的 `&s[..200]` 落在 `'的'`（bytes 199..202）内部 →
/// `tokio-rt-worker` panic → ChatLoop join 失败 → 会话异常终止。
#[test]
fn args_summary_handles_multibyte_args() {
    use serde_json::json;

    // 构造必然跨越 200 字节边界的中文参数（'的' 3 字节）
    let payload = "参数摘要：截断到 max_chars，用于日志打印（避免超长参数刷屏）。".repeat(8);
    let args = json!({ "path": "a.rs", "content": payload });

    // 任意 max 都必须安全返回（旧实现会在部分 max 上 panic）
    for max in [1usize, 2, 3, 199, 200, 201, 512] {
        let s = args_summary(&args, max);
        assert!(
            s.chars().count() <= max + 32,
            "max={max} 摘要过长：{} 字节",
            s.len()
        );
    }

    // 超长时带长度标注，便于日志排查
    let long = args_summary(&args, 200);
    assert!(long.ends_with(&format!("(len={})", args.to_string().len())));
    assert!(long.contains('…'));

    // 短参数原样返回
    assert_eq!(args_summary(&json!({"a": 1}), 200), "{\"a\":1}");
}

/// 造两个已广播的 ToolCall 父节点（模拟 settle_reasoning 之后的权威转写形态——
/// ToolCallDelta 必然先广播过完整节点，`context.messages` 里一定有它们）。
fn tc_context(ids: &[&str]) -> Vec<ChatNode> {
    ids.iter()
        .map(|id| ChatNode {
            id: (*id).into(),
            role: Some(NodeRole::Assistant),
            msg_type: Some(NodeKind::Skeleton),
            meta: Some(serde_json::json!({"style_tag": "tool", "tool_name": "vdfs_list"})),
            status: Some(NodeStatus::Streaming),
            ..Default::default()
        })
        .collect()
}

/// S20.1 不变量：**本批每个 ToolCall 都必须以终态收场，且必须有结果子节点**。
///
/// 中止（`is_aborted`）时循环在第一个工具之前就 `break`，本批一个都没执行——
/// 若不收口，这些节点会停在参数流式阶段广播出的 `Streaming` 上：
/// 前端永远转「运行中」（把"卡住"伪装成"在跑"）。`Streaming` 是瞬态状态
/// （只广播、不落盘），停在 Streaming 是**广播层**的收口缺口，不是持久层问题。
///
/// **只补父节点是不够的**（S23 实测事故）：前端 `ToolCallNode` 的「结果」段按**子节点**
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

    let (msgs, updates) = process_tool_calls_async(
        tcs,
        &None,
        &sink,
        &abort,
        test_ctx(),
        &tc_context(&["tc1", "tc2"]),
        &test_dir(),
        &EventStore::new(),
        false,
        &mut Vec::new(),
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
        assert_eq!(m.role, Some(NodeRole::Tool));
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
        assert_eq!(u.role, Some(NodeRole::Assistant), "完整快照保留节点身份");
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

    let (msgs, updates) = process_tool_calls_async(
        tcs,
        &None,
        &sink,
        &abort,
        test_ctx(),
        &tc_context(&["tc-overflow"]),
        &test_dir(),
        &EventStore::new(),
        false,
        &mut Vec::new(),
        &mut Vec::new(),
        &mut Vec::new(),
    )
    .await;

    assert_eq!(msgs.len(), 1, "丢弃调用必须有结果子节点（S20.1 不变量）");
    assert_eq!(msgs[0].role, Some(NodeRole::Tool));
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
    ctx.set(crate::symbio_core::MODE, "interactive".to_string());
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

    let (msgs, updates) = process_tool_calls_async(
        tcs,
        &None,
        &sink,
        &abort,
        ctx,
        &tc_context(&["tc1", "tc2"]),
        &test_dir(),
        &EventStore::new(),
        false,
        &mut Vec::new(),
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
}

/// 未执行的终态必须是 `Completed` 而非 `Failed`。
///
/// `Failed` 会让 `get_context_messages` 过滤掉父 ToolCall 节点，留下"孤儿 tool 结果"
/// → 下一轮 LLM 请求携带非法 `tool_call_id`。与既有口径一致：工具失败属**信息性**，
/// 差异由 `meta.failure_kind` 承载。
#[test]
fn not_executed_patch_is_completed_not_failed() {
    // 完整副本 + 就地应用终态（发射端从权威转写取副本后调用）
    let mut p = ChatNode {
        id: "tc1".into(),
        role: Some(NodeRole::Assistant),
        msg_type: Some(NodeKind::Skeleton),
        meta: Some(serde_json::json!({"style_tag": "tool"})),
        ..Default::default()
    };
    apply_not_executed(&mut p, "not_executed");
    assert_eq!(p.id, "tc1", "整条替换语义下 id 必须保持");
    assert_eq!(p.status, Some(NodeStatus::Completed));
    assert!(p.error.is_none());
    assert_eq!(
        p.meta.as_ref().and_then(|m| m.get("success")),
        Some(&json!(false))
    );
    assert_eq!(
        p.meta.as_ref().and_then(|m| m.get("failure_kind")),
        Some(&json!("not_executed"))
    );
    // 成因可区分：「被钩子拦下」与「批次没轮到」不是同一件事，
    // 共用一个标记会让事后排查只能靠猜。
    let mut blocked = ChatNode {
        id: "tc1".into(),
        ..Default::default()
    };
    apply_not_executed(&mut blocked, "blocked");
    assert_eq!(
        blocked.meta.as_ref().and_then(|m| m.get("failure_kind")),
        Some(&json!("blocked"))
    );
}

// ==================== 工具执行期间的中止感知 ====================

/// 中止置位 → 等待立即结束。
///
/// 非流式工具（`read_file` / `web_search` / MCP / `codebase_search` …，即绝大多数
/// 工具）在执行期间是**单次 await**，没有任何帧可观测。若不与中止信号 select，
/// 用户按下停止后工具照跑（最长硬超时 600s），整个 Turn 继续推进——用户以为停了，
/// 其实没停。
///
/// 收口前这条语义由「`handle_abort` 往通道投一帧 `{"type":"abort"}`」承载，
/// 因此测试要造帧、造通道、还要覆盖 cancel_token 与「通道关闭不算中止」两条边界。
/// 现在中止只有一个原语（[`ExecAbortSignal`]），一个用例即可锁死契约。
#[tokio::test]
async fn wait_tool_abort_returns_when_signalled() {
    let abort = ExecAbortSignal::new();
    let waiter = abort.clone();

    let handle = tokio::spawn(async move { wait_tool_abort(&waiter).await });

    abort.abort();
    tokio::time::timeout(std::time::Duration::from_secs(2), handle)
        .await
        .expect("中止置位后应立即返回，而不是继续等工具")
        .expect("等待任务不应 panic");
}

/// 已置位的信号（上游已判定）→ 立即返回，不挂起。
#[tokio::test]
async fn wait_tool_abort_returns_immediately_when_already_aborted() {
    let abort = ExecAbortSignal::new();
    abort.abort();
    tokio::time::timeout(
        std::time::Duration::from_millis(200),
        wait_tool_abort(&abort),
    )
    .await
    .expect("信号已置位时应立即返回");
}

/// **未被中止就不得返回**——回归测试。
///
/// 这是「工具照跑但用户以为停了」的反面：若等待函数在没有任何中止来源时提前返回，
/// `select!` 会立刻 drop 掉仍在运行的工具 future，把它误报成「被用户中止」。
/// 收口前这条由「通道关闭不算中止」的用例守着；现在通道不在执行期了，
/// 契约变成更直接的一句：**只有 `abort()` 能让它返回**。
#[tokio::test]
async fn wait_tool_abort_stays_pending_without_signal() {
    let abort = ExecAbortSignal::new();
    let r = tokio::time::timeout(
        std::time::Duration::from_millis(300),
        wait_tool_abort(&abort),
    )
    .await;
    assert!(
        r.is_err(),
        "无中止来源时提前返回：正在跑的工具会被无故丢弃并报成「用户中止」"
    );
    assert!(!abort.is_aborted(), "未中止时信号不得被置位");
}

// ── 工具结果的「等待用户」意图 ────────────────────────────────────────────────
//
// 契约**工具只声明意图与载荷，节点归编排层构造**。工具不自建 Session 通道、
// 发一个随机 id 的节点，由消费方解回来改 id 再重播一次——同一个逻辑节点因此有两个
// id，前端出现重复审批卡，且 resume 只删得掉一个。

/// `failure_kind` 是**控制流**判据，因此只认闭集里的两个 pending 取值。
///
/// 反面同样重要：`error` / `permission_denied` / `tool_unavailable` 都是**信息性**
/// 标记。把它们当成「等待用户」会让自动模式下**无人可授权**的工具把会话永久挂起
/// ——本该让 LLM 换条路继续，却变成等一个永远不会来的回答。
#[test]
fn pending_prompt_from_only_accepts_the_pending_kinds() {
    for kind in ["needs_approval", "needs_interaction"] {
        let data = json!({
            "content": "需要确认：执行 cmd",
            "prompt": { "kind": "confirm" },
            "failure_kind": kind,
        });
        let p = pending_prompt_from(&data).unwrap_or_else(|| panic!("{kind} 应判为等待用户"));
        assert_eq!(p.failure_kind, kind);
        assert_eq!(p.text, "需要确认：执行 cmd");
        assert_eq!(p.prompt["kind"], json!("confirm"));
    }

    for kind in ["error", "permission_denied", "tool_unavailable"] {
        let data = json!({ "content": "x", "failure_kind": kind });
        assert!(
            pending_prompt_from(&data).is_none(),
            "{kind} 是信息性标记，不得收口为等待用户（会让会话永久挂起）"
        );
    }

    // 没有 failure_kind ⇒ 普通结果
    assert!(pending_prompt_from(&json!({ "content": "ok" })).is_none());
}

/// user_prompt 节点的身份归编排层：`id = result_msg_id`、`parent_id = tool_call_id`。
///
/// 这两个 id 正是「同一逻辑节点两个 id」事故的正反面。工具侧现在**拿不到**它们
/// （[`PendingPrompt`] 里没有 id 字段），所以那类 bug 在类型层面不成立。
#[test]
fn build_user_prompt_message_uses_the_orchestrator_owned_ids() {
    let pending = PendingPrompt {
        text: "请回答问题以继续".into(),
        prompt: json!({ "kind": "question", "questions": [] }),
        failure_kind: crate::symbio_core::failure_kind::NEEDS_INTERACTION.into(),
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
    assert_eq!(msg.role, Some(NodeRole::Tool));
    // [T-147d-iii] 交互卡也是工具的结果：主体归工具一族，工具名随行入格
    assert_eq!(
        msg.principal.as_ref().map(|p| p.id.as_str()),
        Some("vdfs/write")
    );

    let meta = msg.meta.expect("user_prompt 必须带 meta");
    assert_eq!(meta["failure_kind"], json!("needs_interaction"));
    assert_eq!(meta["prompt"]["kind"], json!("question"));
    assert_eq!(meta["tool_name"], json!("vdfs_write"));
}

/// **跨工具契约**：`agent_run` 把子会话审批转成载荷回传时，本层必须认出它。
///
/// 两个文件各自演化时这条契约最容易静默断掉：工具改了字段名，编排层就再也识别
/// 不出「待用户动作」，审批卡直接消失（且不报错）。形状与
/// `agent/host/subagent.rs::RelayOutcome::Pending` 的序列化逐字对应。
#[test]
fn subagent_pending_payload_is_recognized_as_pending_intent() {
    let payload = json!({
        "content": "需要确认：删除文件",
        "success": false,
        "failure_kind": crate::symbio_core::failure_kind::NEEDS_APPROVAL,
        "prompt": { "tool_name": "agent_run", "args": { "session_id": "child-1" } },
    });
    let p = pending_prompt_from(&payload).expect("子会话待审批载荷必须被识别");

    assert_eq!(p.text, "需要确认：删除文件");
    assert_eq!(p.prompt["tool_name"], json!("agent_run"));
    assert_eq!(
        p.failure_kind,
        crate::symbio_core::failure_kind::NEEDS_APPROVAL
    );
    // 载荷里没有身份字段 ⇒ [`PendingPrompt`] 无法承载悬空引用（类型层面成立）
}

// ==================== note_delegation：代际立约的唯一观察点 ====================

/// 只有 `agent_run` 立约——别的工具是本会话自办，办没办成是执行结果不是承诺。
#[tokio::test]
async fn note_delegation_ignores_non_delegating_tools() {
    let decls = factory_decls().await;
    let mut out = Vec::new();
    let args = json!({ "prompt": "让 reviewer 复查这段", "agent_id": "reviewer" });

    note_delegation(&mut out, &decls, "c1", "vdfs_read", &args, true, "");
    note_delegation(&mut out, &decls, "c2", "read", &args, true, "");
    assert!(out.is_empty(), "非 `agent_run` 不产生承诺记录：{out:?}");

    note_delegation(&mut out, &decls, "c3", "agent_run", &args, true, "");
    assert_eq!(out.len(), 1);
    assert_eq!(
        out[0].id, "c3",
        "承诺号 = 转写里的 ToolCall 节点 id（`c-offer-<id>` 由此成格）"
    );
    assert_eq!(
        out[0].promise, "让 reviewer 复查这段",
        "承诺内容取 `prompt` 参数"
    );
    assert!(out[0].ok);
    assert!(out[0].why.is_empty(), "守约不必带 why，违约必须带");
}

/// **先到先得**：同一次调用可能在两个分支各记一笔（终态广播找不到节点 ⇒ 末尾
/// 收口会再记一次「未执行」），后到的收口不许把跑成功的改写成没跑。
#[tokio::test]
async fn note_delegation_never_overwrites_an_earlier_judgement() {
    let decls = factory_decls().await;
    let mut out = Vec::new();
    let args = json!({ "prompt": "交给子智能体" });

    note_delegation(&mut out, &decls, "c1", "agent_run", &args, true, "");
    note_delegation(
        &mut out,
        &decls,
        "c1",
        "agent_run",
        &args,
        false,
        "未执行（中止或同批前序未结束）",
    );
    assert_eq!(out.len(), 1, "同一承诺只入格一次");
    assert!(out[0].ok, "先记下的那次更接近事实，收口分支不得覆盖");

    note_delegation(
        &mut out,
        &decls,
        "c1",
        "agent_run",
        &args,
        false,
        "被 PreToolUse 钩子拦下",
    );
    assert_eq!(out.len(), 1);
    assert!(out[0].ok);

    // 没跑到的那一次（一次都没记过）照常入账，且带得出违约原因。
    note_delegation(
        &mut out,
        &decls,
        "c2",
        "agent_run",
        &args,
        false,
        "被 PreToolUse 钩子拦下",
    );
    assert_eq!(out.len(), 2);
    assert!(!out[1].ok);
    assert_eq!(out[1].why, "被 PreToolUse 钩子拦下");
}

// ==================== note_tasks：任务表出参的唯一写入点（批⑨） ====================
//
// 判据三条，各钉一条反向：**短名**匹配（不反演全名、不误吃 MCP 工具）、**只认成功**
// （失败的调用没改清单）、**id 缺席按位次兜底**（与 local/todo_write 同一条格式——
// 两处各写一次是跨插件契约，改名必须一起改）。

#[tokio::test]
async fn note_tasks_records_a_successful_todo_write() {
    let decls = factory_decls().await;
    let mut out = Vec::new();
    let args = json!({
        "todos": [
            { "content": "分析架构", "status": "in_progress" },
            { "id": "t2", "content": "写实现", "status": "completed", "depends_on": ["task_1"] },
            { "id": "t3", "content": "写文档", "status": "pending" }
        ],
        "merge": false
    });
    note_tasks(&mut out, &decls, "local/todo_write", &args, true);

    assert_eq!(out.len(), 1, "一次成功调用 = 一条声明");
    let items = &out[0].items;
    assert_eq!(items.len(), 3);
    assert_eq!(items[0].id, "task_1", "id 缺席按位次兜底（与工具同格式）");
    assert_eq!(items[0].status, TaskStatus::InProgress);
    assert!(items[0].depends_on.is_empty(), "省略 depends_on ⇒ []");
    assert_eq!(items[1].status, TaskStatus::Completed);
    assert_eq!(items[1].depends_on, vec!["task_1".to_string()]);
    assert_eq!(items[2].status, TaskStatus::Pending);
    assert_eq!(out[0].items[0].goal, "分析架构");
}

#[tokio::test]
async fn note_tasks_ignores_failures_and_other_tools() {
    let decls = factory_decls().await;
    let mut out = Vec::new();
    let args = json!({ "todos": [{ "content": "甲" }] });

    // 失败的调用没改清单 ⇒ 不记（记它等于替工具编一条事实）。
    note_tasks(&mut out, &decls, "local/todo_write", &args, false);
    // 别的工具：短名判据只认 todo_write。
    note_tasks(&mut out, &decls, "local/shell", &args, true);
    // MCP 的同名工具走 `mcp__srv__todo_write` 线上名，短名判据**不反演** ⇒ 不误吃。
    note_tasks(&mut out, &decls, "mcp__srv__todo_write", &args, true);
    // 裸 `todo_write`（无挂载点前缀）同样要认。
    note_tasks(&mut out, &decls, "todo_write", &args, true);

    assert_eq!(out.len(), 1, "只有成功的 todo_write 被记账");
}

#[tokio::test]
async fn note_tasks_skips_payloads_that_cannot_be_a_task_list() {
    let decls = factory_decls().await;
    let mut out = Vec::new();
    note_tasks(&mut out, &decls, "local/todo_write", &json!({}), true);
    note_tasks(
        &mut out,
        &decls,
        "local/todo_write",
        &json!({ "todos": [{ "content": "" }] }),
        true,
    );
    assert!(out.is_empty(), "缺 todos / 空 content ⇒ 不是一份清单");
}

// ==================== 外部执行闸门（S8 第 20 步，04 §3.1 批⑩） ====================

/// 带会话 id 的请求上下文。成本台账不再按它定位——事实轨以**值**交给执行层
/// （ADR-084 §1-A）；会话 id 仍随上下文走，因为其余工具面按它归属。
fn gate_ctx(session_id: &str) -> Arc<dyn PluginInvokeRequest> {
    let req = PluginSimpleRequest::new(None, None);
    req.set(crate::symbio_core::SESSION_ID, session_id.to_string());
    Arc::new(req)
}

/// 判据那一半：授权是**读矩阵**得来的（fail-closed），不是默认放行。
///
/// `agent:*` 身份会话在**会话装配点经绑定登记**拿到会话面缺省行（含
/// `produce.artifact`，[ADR-100]）——本用例直读合流面（不经装配点），取的是
/// 部署表那一行的判据；`Refuse` 只可能来自**不属于这一族**的主体——判据为 false
/// 就是 false，没有任何「认不出 ⇒ 放行」的旁路。另一侧的判据（预算）见下方端到端用例。
#[test]
fn external_execution_capability_is_read_from_the_matrix() {
    assert!(
        crate::symbio_core::authz::production_matrix().can_write_name(
            crate::symbio_core::authz::PRINCIPAL_MAIN,
            crate::symbio_core::authz::CAP_EXTERNAL_EXECUTION
        ),
        "agent:main 持外部执行能力（本机部署事实）"
    );
    assert!(
        !crate::symbio_core::authz::production_matrix().can_write_name(
            crate::symbio_core::authz::PRINCIPAL_USER,
            crate::symbio_core::authz::CAP_EXTERNAL_EXECUTION
        ),
        "非 agent 主体不持外部执行能力 ⇒ 判据 false ⇒ Refuse（fail-closed）"
    );
}

/// 事实源那一半：台账累计把预算吃满 ⇒ 工具**不执行**，且 `Break` 走出参。
///
/// 这条用例钉的是 [roadmap/S09 §6](../../../../../docs/009-task/129-外部执行与熔断.md "T-129-外部执行与熔断")
/// 验收 2 的前半句（预算耗尽必须判得出来，不允许静默继续）；后半句——
/// 理由真的落成一条 `control × opened` 事件——钉在事实源入格那一侧的用例里
/// （写方是 `derived_facts::record_derived`，由 v2 运行器在轮末收束调用）；工具执行层
/// 没有事实源，入格要到轮末收束那一步才发生。
#[tokio::test]
async fn budget_exhausted_refuses_the_tool_and_reports_a_break() {
    let dir = std::env::temp_dir().join("symbio-test/session-tool-executor-gate");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建 L0 存档目录");

    // 台账：一条已耗超过会话累计上限的收束事件（`cost_ledger` 按 actor 归账，
    // ADR-044：闸门读台账，不读运行时计数器）。事实轨以**值**交给执行层
    // （ADR-084 §1-A）——夹具直接造这份切片，不摆存储目录。
    let sid = "gate-sess";
    let facts = EventStore::new();
    facts
        .append(
            crate::symbio_core::Event::pending(
                "v2f-gate",
                crate::symbio_core::EVENT_ASSISTANT_FINAL,
                crate::symbio_core::Entity::Turn,
                crate::symbio_core::Verb::Closed,
                0,
                crate::symbio_core::authz::PRINCIPAL_MAIN,
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
        role: Some(NodeRole::Assistant),
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
    let (msgs, updates) = process_tool_calls_async(
        tcs,
        &None,
        &sink,
        &abort,
        gate_ctx(sid),
        std::slice::from_ref(&parent),
        &dir,
        &facts,
        false,
        &mut Vec::new(),
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
        "Break 必须经出参交回调用方——它没有这一格就是静默继续（S09 §6 验收 2）"
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// 预算在生效而非常量（S09 §6.4 的反向用例在生产判据上的形状）：
/// 同一份事实源，预算放得比已耗宽 ⇒ 同一次判定翻成放行、出参为空。
#[tokio::test]
async fn relaxing_the_budget_turns_the_same_facts_into_a_pass() {
    let inputs = BreakerInputs::of(&[], &gate_ctx("gate-open"));
    assert_eq!(inputs.spent_ms, 0, "空事实切片 ⇒ 零账，不是故障");
    assert!(inputs.authorized, "agent:main 持外部执行能力");

    // 预算收紧到「已耗 + 申请」之下 ⇒ 熔断；放宽到之上 ⇒ 放行。
    assert_eq!(
        crate::symbio_core::CircuitBreaker.gate(
            inputs.authorized,
            inputs.spent_ms,
            inputs.requested_ms,
            inputs.spent_ms + inputs.requested_ms - 1,
            0,
            crate::symbio_core::LatencyTier::Deep.budget_ms(),
        ),
        crate::symbio_core::GateDecision::Break {
            reason: "budget-exhausted"
        },
        "预算真的在参与判定（常量判定拿不出这一对相反的结论）"
    );
    assert_eq!(
        crate::symbio_core::CircuitBreaker.gate(
            inputs.authorized,
            inputs.spent_ms,
            inputs.requested_ms,
            inputs.spent_ms + inputs.requested_ms + 1,
            0,
            crate::symbio_core::LatencyTier::Deep.budget_ms(),
        ),
        crate::symbio_core::GateDecision::Allow
    );
}

/// 台账读的是**交给闸门的事实值**。
///
/// 历史事故：曾有实现按 `<插件目录>/v2-events.wal` 取账（漏了会话 id 那层）⇒
/// `spent_ms` 恒 0 ⇒ 预算分支永远走不到，且没有任何东西会变红。ADR-084 §1-A
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
            crate::symbio_core::Event::pending(
                "v2f-ledger",
                crate::symbio_core::EVENT_ASSISTANT_FINAL,
                crate::symbio_core::Entity::Turn,
                crate::symbio_core::Verb::Closed,
                0,
                crate::symbio_core::authz::PRINCIPAL_MAIN,
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

/// 淘汰闸是**可审计拒绝**：结果回灌 + 父节点收敛 + 违约承诺照记（S13 §3）。
///
/// 判据批首读一次（部署级撤销结论由 session 折成一个值），这里钉的是接线：
/// 声明了 `provides_assign_work` 的 `agent_run` 在 `elimination_revoked=true` 时
/// 不执行，且「约了却没派成」如实落进承诺出参（`note_delegation` ok=false）。
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
                provides_assign_work: true,
                records_delegation_promise: true,
                ..Default::default()
            })) as Arc<dyn Capability>))
            .await;
        req.set(
            crate::symbio_core::CAPABILITY_VISITOR,
            visitor as Arc<dyn Registry>,
        );
        Arc::new(req) as Arc<dyn PluginInvokeRequest>
    };

    let sink = ExecEventSink::silent();
    let abort = ExecAbortSignal::new();
    let parent = ChatNode {
        id: "tc-elim".into(),
        // 身份写 `principal`，不写存量 `role` 格（S-014 棘轮）。
        principal: Some(crate::symbio_core::principal::Principal::tool_principal(
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
    let mut delegations = Vec::new();
    let (msgs, updates) = process_tool_calls_async(
        tcs,
        &None,
        &sink,
        &abort,
        ctx,
        std::slice::from_ref(&parent),
        &dir,
        &EventStore::new(),
        true,
        &mut delegations,
        &mut Vec::new(),
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
    assert_eq!(delegations.len(), 1, "拒止同样要记一笔承诺");
    assert!(!delegations[0].ok, "没派成 = 没履约");
    assert!(
        delegations[0].why.contains("assign.work 已被淘汰回路收回"),
        "违约理由要写明成因（实际: {}）",
        delegations[0].why
    );

    std::fs::remove_dir_all(&dir).ok();
}

// ==================== 任务表写侧闸：纯产出主体不立任务（S03 §3，T-107） ====================
//
// 钉的是**闸门的接线**（判据本身在 `task_table.test.rs`）：`todo_write` 到达执行窗口
// 之前被拒——`Refused:` 回灌模型、父节点收敛、工具没有执行（没有路由往返）、
// **不记任务声明**（`note_tasks` 只在跑完后记成功调用）。
//
// T-158 之后这道闸的判据是**注册面**：ctx 的 `CAPABILITY_VISITOR` 里要有一位
// 自声明 `owns_task_table` 的 `todo_write`（生产即如此）——本用例的 ctx 因此带
// 一份与生产同形的注册面，否则闸「看不见」这次调用（无声明 = 不命中，由 e2e
// 兜真实行为）。

/// 工人主体（只持 `produce.artifact`）调 `todo_write` ⇒ 拒止。
///
/// 今天没有任何生产路径让工人调任务表——此用例防的是**未来**：角色行落地后，
/// 一个绑 `worker` 的子会话调 `todo_write` 被诚实拒止，而不是静默以主智能体
/// 的资格写下去。C15 的精神：产出者不自我认证。
#[tokio::test]
async fn a_producer_only_principal_is_refused_the_task_table() {
    let dir = std::env::temp_dir().join("symbio-test/session-tool-executor-task-table");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建插件目录");

    let worker_ctx = {
        let req = PluginSimpleRequest::new(None, None);
        req.set(crate::symbio_core::AGENT_ID, "worker".to_string());
        // 注册面：`todo_write` 自声明任务表位（生产同形；T-158 后闸问声明）。
        let visitor = Arc::new(DefaultToolVisitor::new());
        visitor
            .put(CapabilityRow::tool(Arc::new(DeclTool(CapabilityMeta {
                name: "todo_write".to_string(),
                owns_task_table: true,
                ..Default::default()
            })) as Arc<dyn Capability>))
            .await;
        req.set(
            crate::symbio_core::CAPABILITY_VISITOR,
            visitor as Arc<dyn Registry>,
        );
        Arc::new(req) as Arc<dyn PluginInvokeRequest>
    };

    let sink = ExecEventSink::silent();
    let abort = ExecAbortSignal::new();
    let parent = ChatNode {
        id: "tc-task".into(),
        role: Some(NodeRole::Assistant),
        msg_type: Some(NodeKind::Skeleton),
        meta: Some(serde_json::json!({"style_tag": "tool", "tool_name": "local/todo_write"})),
        status: Some(NodeStatus::Streaming),
        ..Default::default()
    };
    let tcs = vec![TurnToolCallInfo {
        id: Some("tc-task".into()),
        wire_id: None,
        name: Some("local/todo_write".into()),
        arguments: json!({ "todos": [{ "id": "t1", "content": "干活", "status": "completed" }] }),
        parse_error: None,
        overflow_notice: None,
    }];
    let mut tasks: Vec<TaskDeclaration> = Vec::new();
    let (msgs, updates) = process_tool_calls_async(
        tcs,
        &None,
        &sink,
        &abort,
        worker_ctx,
        std::slice::from_ref(&parent),
        &dir,
        &EventStore::new(),
        false,
        &mut Vec::new(),
        &mut tasks,
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
        text.contains("Refused:") && text.contains("define.work / assert.verification"),
        "结果要写明是被任务表闸拒的（实际: {text}）"
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
    assert!(
        tasks.is_empty(),
        "工具没跑 ⇒ 不记任务声明（跑了才判，同批⑨口径）"
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// 平凡值会话（无 `AGENT_ID` ⇒ `agent:main`，持 `define.work`）**不被本闸拦**。
///
/// 这是「闸不改变现有行为」的反向对照：主会话的 `todo_write` 照常走到路由
/// （无 parent ⇒ 路由失败是另一回事，结果里不会有任务表闸的 `Refused:`）。
#[tokio::test]
async fn the_trivial_session_is_not_refused_the_task_table() {
    let dir = std::env::temp_dir().join("symbio-test/session-tool-executor-task-table-open");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建插件目录");

    let sink = ExecEventSink::silent();
    let abort = ExecAbortSignal::new();
    let parent = ChatNode {
        id: "tc-task2".into(),
        role: Some(NodeRole::Assistant),
        msg_type: Some(NodeKind::Skeleton),
        meta: Some(serde_json::json!({"style_tag": "tool", "tool_name": "local/todo_write"})),
        status: Some(NodeStatus::Streaming),
        ..Default::default()
    };
    let tcs = vec![TurnToolCallInfo {
        id: Some("tc-task2".into()),
        wire_id: None,
        name: Some("local/todo_write".into()),
        arguments: json!({ "todos": [] }),
        parse_error: None,
        overflow_notice: None,
    }];
    let mut tasks: Vec<TaskDeclaration> = Vec::new();
    let (msgs, _updates) = process_tool_calls_async(
        tcs,
        &None,
        &sink,
        &abort,
        test_ctx(),
        std::slice::from_ref(&parent),
        &dir,
        &EventStore::new(),
        false,
        &mut Vec::new(),
        &mut tasks,
        &mut Vec::new(),
    )
    .await;

    assert_eq!(msgs.len(), 1);
    let text = msgs[0]
        .content
        .as_ref()
        .map(|c| c.to_text())
        .unwrap_or_default();
    assert!(
        !text.contains("define.work / assert.verification"),
        "平凡值会话（agent:main 持 define.work）不得被任务表闸拒（实际: {text}）"
    );

    std::fs::remove_dir_all(&dir).ok();
}

// ==================== T-190 批内并行：只读并行、其余串行、收口按序 ====================
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

use crate::symbio_core::CapabilityRiskLevel;

/// 记录并发窗口的假工具：不进则退，峰值可读。
struct ProbeTool {
    meta: CapabilityMeta,
    barrier: Option<Arc<tokio::sync::Barrier>>,
    sleep_ms: u64,
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
}

#[async_trait::async_trait]
impl crate::symbio_core::Capability for ProbeTool {
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
        Ok(serde_json::json!({ "content": format!("done:{}", self.meta.name) }))
    }
}

/// 无钩子的父插件：`fire_hook` 拿到 `Err` ⇒ 按缺省放行（生产上无钩子插件同形）。
struct NopParent;

#[async_trait::async_trait]
impl Plugin for NopParent {
    fn meta(&self) -> crate::symbio_core::PluginMeta {
        crate::symbio_core::PluginMeta::new("nop", "Nop")
    }

    async fn route(
        self: Arc<Self>,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> crate::symbio_core::PluginInvokeResponse<PluginPayload> {
        Err(PluginError::NotFound("nop".into()))
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> crate::symbio_core::PluginInvokeResponse<PluginPayload> {
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
    req.set(
        crate::symbio_core::CAPABILITY_VISITOR,
        visitor as Arc<dyn Registry>,
    );
    Arc::new(req) as Arc<dyn PluginInvokeRequest>
}

/// 父 ToolCall 的权威副本（执行层从 `context_messages` 取它做终态快照）。
fn tool_call_node(id: &str, name: &str) -> ChatNode {
    ChatNode {
        id: id.to_string(),
        // 身份写 `principal`，不写存量 `role` 格（S-014 棘轮）。
        principal: Some(crate::symbio_core::principal::Principal::tool_principal(
            name,
        )),
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
impl crate::symbio_core::ExecTranscriptWriter for FrameLog {
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
    let mut delegations = Vec::new();
    let mut tasks = Vec::new();
    let mut breaks = Vec::new();

    let (msgs, updates) = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        process_tool_calls_async(
            probe_calls(&[("tc-a", "probe_a"), ("tc-b", "probe_b")]),
            &parent,
            &sink,
            &abort,
            ctx,
            &nodes,
            &dir,
            &EventStore::new(),
            false,
            &mut delegations,
            &mut tasks,
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
        frames
            .iter()
            .position(|f| f.role == Some(NodeRole::Tool) && f.parent_id.as_deref() == Some(tc))
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

/// 写侧调用（`Medium`）独占执行窗口：与只读调用一前一后都不得重叠。
#[tokio::test]
async fn side_effect_calls_never_overlap_with_another_call() {
    let dir = test_dir().join("t190-serial");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建 L0 存档目录");

    for (first_is_side_effect, label) in [(true, "写侧在前"), (false, "写侧在后")] {
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let side = Arc::new(ProbeTool::new(
            "probe_write",
            CapabilityRiskLevel::Medium,
            &active,
            &peak,
        ));
        let read = Arc::new(
            ProbeTool::new("probe_read", CapabilityRiskLevel::Low, &active, &peak).with_sleep(20),
        );
        let (calls, nodes) = if first_is_side_effect {
            (
                probe_calls(&[("tc-w", "probe_write"), ("tc-r", "probe_read")]),
                vec![
                    tool_call_node("tc-w", "probe_write"),
                    tool_call_node("tc-r", "probe_read"),
                ],
            )
        } else {
            (
                probe_calls(&[("tc-r", "probe_read"), ("tc-w", "probe_write")]),
                vec![
                    tool_call_node("tc-r", "probe_read"),
                    tool_call_node("tc-w", "probe_write"),
                ],
            )
        };
        let ctx = ctx_with_probes(vec![side, read]).await;
        let parent = Some(Arc::new(NopParent) as Arc<dyn Plugin>);
        let sink = ExecEventSink::silent();
        let abort = ExecAbortSignal::new();
        let mut delegations = Vec::new();
        let mut tasks = Vec::new();
        let mut breaks = Vec::new();
        let (msgs, _updates) = process_tool_calls_async(
            calls,
            &parent,
            &sink,
            &abort,
            ctx,
            &nodes,
            &dir,
            &EventStore::new(),
            false,
            &mut delegations,
            &mut tasks,
            &mut breaks,
        )
        .await;

        assert_eq!(msgs.len(), 2, "{label}：两条都必须有结果");
        assert_eq!(
            peak.load(Ordering::SeqCst),
            1,
            "{label}：`Medium` 调用不得与任何调用重叠（峰值并发必须 = 1）"
        );
    }

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
    ctx.set(crate::symbio_core::MODE, "interactive".to_string());
    let parent = Some(Arc::new(NopParent) as Arc<dyn Plugin>);
    let sink = ExecEventSink::silent();
    let abort = ExecAbortSignal::new();
    let nodes = vec![
        tool_call_node("tc-i1", "probe_i1"),
        tool_call_node("tc-i2", "probe_i2"),
    ];
    let mut delegations = Vec::new();
    let mut tasks = Vec::new();
    let mut breaks = Vec::new();

    let (msgs, _updates) = process_tool_calls_async(
        probe_calls(&[("tc-i1", "probe_i1"), ("tc-i2", "probe_i2")]),
        &parent,
        &sink,
        &abort,
        ctx,
        &nodes,
        &dir,
        &EventStore::new(),
        false,
        &mut delegations,
        &mut tasks,
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
    let mut delegations = Vec::new();
    let mut tasks = Vec::new();
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
        process_tool_calls_async(
            probe_calls(&[("tc-aa", "probe_abort_a"), ("tc-ab", "probe_abort_b")]),
            &parent,
            &sink,
            &abort,
            ctx,
            &nodes,
            &dir,
            &EventStore::new(),
            false,
            &mut delegations,
            &mut tasks,
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
