//! `tool_executor` 模块的单元测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）：
//! `tool_executor.rs` 只保留生产代码，测试全部放本文件。

use super::*;
use crate::symbio_core::SimpleRequest;

fn test_ctx() -> Arc<dyn InvokeRequest> {
    Arc::new(SimpleRequest::new(None, None))
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
    let sink = EventSink::silent();
    let abort = AbortSignal::new();
    let tcs = vec![ToolCallInfo {
        id: None,
        wire_id: None,
        name: Some("vdfs_list".into()),
        arguments: json!({ "path": "." }),
        parse_error: None,
    }];

    let (msgs, updates) =
        process_tool_calls_async(tcs, &None, &sink, &abort, test_ctx(), &[]).await;

    assert_eq!(msgs.len(), 1, "必须生成失败结果子节点（而非跳过）");
    assert_eq!(msgs[0].role, Some(MessageRole::Tool));
    assert_eq!(msgs[0].status, Some(MessageStatus::Completed));
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
    assert_eq!(updates[0].status, Some(MessageStatus::Completed));
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
    let sink = EventSink::silent();
    let abort = AbortSignal::new();
    let tcs = vec![ToolCallInfo {
        id: Some(String::new()),
        wire_id: None,
        name: Some("vdfs_list".into()),
        arguments: json!({}),
        parse_error: None,
    }];

    let (msgs, updates) =
        process_tool_calls_async(tcs, &None, &sink, &abort, test_ctx(), &[]).await;

    assert_eq!(msgs.len(), 1);
    assert_eq!(updates.len(), 1);
}

/// 工具名缺失/非法 → 同样作为失败处理（结果挂到已知 id 上，结构完整）。
#[tokio::test]
async fn missing_tool_name_is_recorded_as_failure() {
    let sink = EventSink::silent();
    let abort = AbortSignal::new();
    let tcs = vec![ToolCallInfo {
        id: Some("tc-known".into()),
        wire_id: None,
        name: Some(String::new()),
        arguments: json!({}),
        parse_error: None,
    }];

    let (msgs, updates) =
        process_tool_calls_async(tcs, &None, &sink, &abort, test_ctx(), &[]).await;

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
    let sink = EventSink::silent();
    let abort = AbortSignal::new();
    let tcs = vec![ToolCallInfo {
        id: Some("tc-broken".into()),
        wire_id: None,
        name: Some("cmd".into()),
        arguments: json!({}),
        parse_error: Some(r#"{"command": "cargo test"#.into()),
    }];

    let (msgs, updates) =
        process_tool_calls_async(tcs, &None, &sink, &abort, test_ctx(), &[]).await;

    assert_eq!(msgs.len(), 1, "必须生成失败结果子节点（而非跳过）");
    assert_eq!(msgs[0].role, Some(MessageRole::Tool));
    assert_eq!(msgs[0].status, Some(MessageStatus::Completed));
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
    assert_eq!(updates[0].status, Some(MessageStatus::Completed));
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
fn tc_context(ids: &[&str]) -> Vec<ChatMessage> {
    ids.iter()
        .map(|id| ChatMessage {
            id: (*id).into(),
            role: Some(MessageRole::Assistant),
            msg_type: Some(MessageType::ToolCall),
            name: Some("vdfs_list".into()),
            status: Some(MessageStatus::Streaming),
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
    let sink = EventSink::silent();
    let abort = AbortSignal::new();
    abort.abort(); // 已中止：本批一个都不执行
    let tcs = vec![
        ToolCallInfo {
            id: Some("tc1".into()),
            wire_id: None,
            name: Some("vdfs_list".into()),
            arguments: json!({ "path": "." }),
            parse_error: None,
        },
        ToolCallInfo {
            id: Some("tc2".into()),
            wire_id: None,
            name: Some("vdfs_list".into()),
            arguments: json!({ "path": "." }),
            parse_error: None,
        },
    ];

    let (msgs, updates) = process_tool_calls_async(
        tcs,
        &None,
        &sink,
        &abort,
        test_ctx(),
        &tc_context(&["tc1", "tc2"]),
    )
    .await;

    assert_eq!(
        msgs.len(),
        2,
        "未执行的调用同样必须有结果子节点，否则前端卡片有请求、无响应"
    );
    for m in &msgs {
        assert_eq!(m.role, Some(MessageRole::Tool));
        assert_eq!(m.status, Some(MessageStatus::Completed));
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
        assert_eq!(u.role, Some(MessageRole::Assistant), "完整快照保留节点身份");
        assert_eq!(u.msg_type, Some(MessageType::ToolCall));
        assert_eq!(u.status, Some(MessageStatus::Completed));
        assert_eq!(
            u.meta
                .as_ref()
                .and_then(|m| m.get("failure_kind"))
                .and_then(|v| v.as_str()),
            Some("not_executed"),
            "未执行必须留下可区分的标记，否则与「跑完了」无从分辨"
        );
        assert!(u.error.is_none(), "未执行不是错误：挂 error 会让前端渲染 ⚠");
    }
    // 结果子节点必须挂在**它自己那一次调用**上（挂错父节点 = 另一个工具的结果）
    let parents: Vec<&str> = msgs.iter().filter_map(|m| m.parent_id.as_deref()).collect();
    assert_eq!(parents, vec!["tc1", "tc2"]);
}

/// 实测事故（本地会话 `3ea6e4d0`）的机制回归：**交互模式下**前一个工具失败 ⇒
/// 本批剩余被跳过 ⇒ 跳过的那些同样必须留下结果子节点。
///
/// 这条路径是用户报告的现场：4 个 `vdfs_read` 只留下 `failure_kind=not_executed`
/// 的父节点、没有任何子节点，UI 上表现为「工具没有响应，会话却继续往后」。
#[tokio::test]
async fn interactive_break_leaves_result_for_skipped_calls() {
    let sink = EventSink::silent();
    let abort = AbortSignal::new();
    // 交互模式 + 无 parent：第一个工具必然以失败告终，触发本批 break
    let ctx = test_ctx();
    ctx.set(crate::symbio_core::MODE, "interactive".to_string());
    let tcs = vec![
        ToolCallInfo {
            id: Some("tc1".into()),
            wire_id: None,
            name: Some("vdfs_read".into()),
            arguments: json!({ "path": "a" }),
            parse_error: None,
        },
        ToolCallInfo {
            id: Some("tc2".into()),
            wire_id: None,
            name: Some("vdfs_read".into()),
            arguments: json!({ "path": "b" }),
            parse_error: None,
        },
    ];

    let (msgs, updates) =
        process_tool_calls_async(tcs, &None, &sink, &abort, ctx, &tc_context(&["tc1", "tc2"]))
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
    assert_eq!(skipped.status, Some(MessageStatus::Completed));
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
    let mut p = ChatMessage {
        id: "tc1".into(),
        role: Some(MessageRole::Assistant),
        msg_type: Some(MessageType::ToolCall),
        ..Default::default()
    };
    apply_not_executed(&mut p, "not_executed");
    assert_eq!(p.id, "tc1", "整条替换语义下 id 必须保持");
    assert_eq!(p.status, Some(MessageStatus::Completed));
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
    let mut blocked = ChatMessage {
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
/// 现在中止只有一个原语（[`AbortSignal`]），一个用例即可锁死契约。
#[tokio::test]
async fn wait_tool_abort_returns_when_signalled() {
    let abort = AbortSignal::new();
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
    let abort = AbortSignal::new();
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
    let abort = AbortSignal::new();
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
// 契约：**工具只声明意图与载荷，节点归编排层构造**。工具曾经自建 Session 通道、
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
    let msg = build_user_prompt_message("res-1", "tc-1", &pending);

    assert_eq!(msg.id, "res-1", "节点 id 必须是结果占位节点 id，不是新造的");
    assert_eq!(
        msg.parent_id.as_deref(),
        Some("tc-1"),
        "必须锚在发起它的 ToolCall 下"
    );
    assert_eq!(msg.msg_type, Some(MessageType::UserPrompt));
    assert_eq!(msg.status, Some(MessageStatus::WaitingUserAction));
    assert_eq!(msg.role, Some(MessageRole::Tool));

    let meta = msg.meta.expect("user_prompt 必须带 meta");
    assert_eq!(meta["failure_kind"], json!("needs_interaction"));
    assert_eq!(meta["prompt"]["kind"], json!("question"));
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
