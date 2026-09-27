//! `context` 门面（触发与度量）的单元测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）：
//! `context.rs` 只保留生产代码，测试全部放本文件。
//! 请求视图层与提示词/快照协议的测试分别在 `view.test.rs` / `prompt.test.rs`。

use super::*;
use crate::symbio_core::schemas::session::chat_message::MessageContent;

fn user_msg(text: &str) -> ChatMessage {
    ChatMessage {
        id: uuid::Uuid::new_v4().to_string(),
        role: Some(MessageRole::User),
        msg_type: Some(MessageType::Text),
        content: Some(MessageContent::Text(text.to_string())),
        ..Default::default()
    }
}

fn assistant_msg(text: &str) -> ChatMessage {
    ChatMessage {
        id: uuid::Uuid::new_v4().to_string(),
        role: Some(MessageRole::Assistant),
        msg_type: Some(MessageType::Text),
        content: Some(MessageContent::Text(text.to_string())),
        ..Default::default()
    }
}

fn tool_call_msg() -> ChatMessage {
    ChatMessage {
        id: uuid::Uuid::new_v4().to_string(),
        role: Some(MessageRole::Assistant),
        msg_type: Some(MessageType::ToolCall),
        name: Some("vdfs_read".to_string()),
        content: Some(MessageContent::Text(r#"{"path":"a.rs"}"#.to_string())),
        ..Default::default()
    }
}

fn tool_result_msg() -> ChatMessage {
    ChatMessage {
        id: uuid::Uuid::new_v4().to_string(),
        role: Some(MessageRole::Tool),
        msg_type: Some(MessageType::Text),
        content: Some(MessageContent::Text("file content".to_string())),
        ..Default::default()
    }
}

#[test]
fn test_estimate_tokens_counts_tool_calls_and_chinese() {
    // 中文 3 字节/字：旧的 len()/4 会严重低估；tokenizer 应给出合理值
    let m = user_msg(&"你好世界。".repeat(100)); // 500 字 ≈ 500~1000 tok
    let est = estimate_message_tokens(&m);
    assert!(est >= 400, "中文估算不应低估: {est}");

    // ToolCall 参数（content 即 JSON）必须被计入
    let tc = tool_call_msg();
    assert!(estimate_message_tokens(&tc) > 8);
}

#[test]
fn test_estimate_context_tokens_includes_overhead() {
    let msgs = vec![user_msg("hi")];
    let est = estimate_context_tokens(&msgs, 5000);
    assert!(est >= 5000, "请求级固定开销必须计入");
}

#[test]
fn test_split_point_never_full_compress_on_assistant_tail() {
    // U A U A：末尾是 Assistant（旧版会全量压缩）。保留 30% ⇒ 应在早期 User 边界切分。
    let msgs = vec![
        user_msg(&"x".repeat(1000)),
        assistant_msg(&"y".repeat(1000)),
        user_msg(&"z".repeat(1000)),
        assistant_msg(&"w".repeat(1000)),
    ];
    let split = find_compress_split_point(&msgs, 0.7);
    assert!(
        split > 0 && split < msgs.len(),
        "不得全量压缩, split={split}"
    );
    assert_eq!(msgs[split].role, Some(MessageRole::User));
}

#[test]
fn test_split_point_zero_on_pending_tool_call() {
    // 尾部 ToolCall 无结果子节点（continuation 场景）⇒ 必须放弃压缩
    let msgs = vec![
        user_msg(&"x".repeat(1000)),
        assistant_msg(&"y".repeat(1000)),
        tool_call_msg(),
    ];
    assert_eq!(find_compress_split_point(&msgs, 0.7), 0);
}

#[test]
fn test_split_point_ok_when_tool_call_paired() {
    // 配对完整的 ToolCall/Tool 不阻塞压缩
    let msgs = vec![
        user_msg(&"x".repeat(1000)),
        assistant_msg(&"y".repeat(1000)),
        tool_call_msg(),
        tool_result_msg(),
        user_msg(&"z".repeat(1000)),
    ];
    let split = find_compress_split_point(&msgs, 0.7);
    assert!(split > 0, "配对完整时不应放弃压缩");
}

fn turn_root_msg(id: &str) -> ChatMessage {
    ChatMessage {
        id: id.to_string(),
        parent_id: None,
        role: Some(MessageRole::Assistant),
        msg_type: Some(MessageType::Turn),
        content: Some(MessageContent::Text("turn".to_string())),
        ..Default::default()
    }
}

/// 切分点回归：必须前移到当前用户指令（保留区 = 用户指令 + 完整 Turn 子树）。
/// 旧实现切在本 Turn 首个 ToolCall，用户指令与 Turn 根被压进快照，保留区
/// 只剩 parent 悬空的 ToolCall 子树 → provider 400。
#[test]
fn test_turn_split_idx_starts_at_user_instruction() {
    let cur_call = ChatMessage {
        parent_id: Some("cur-root".to_string()),
        ..tool_call_msg()
    };
    let old_call = ChatMessage {
        parent_id: Some("old-root".to_string()),
        ..tool_call_msg()
    };
    let msgs = vec![
        user_msg("旧问题"),
        turn_root_msg("old-root"),
        old_call,
        tool_result_msg(),
        user_msg("新问题"),
        turn_root_msg("cur-root"),
        cur_call,
        tool_result_msg(),
    ];
    // 当前用户指令在下标 4：保留区从"新问题"起，含完整 Turn 子树
    assert_eq!(find_turn_user_split_idx(&msgs, "cur-root"), 4);
    // 旧 Turn：其用户指令即会话首条 ⇒ 0（run_context_compact 以 split==0 中止）
    assert_eq!(find_turn_user_split_idx(&msgs, "old-root"), 0);
    // Turn 不存在 ⇒ 0（中止）
    assert_eq!(find_turn_user_split_idx(&msgs, "no-such-turn"), 0);
}

/// 回退与中止语义：Turn 前无根级 User ⇒ 切在 Turn 根（子树仍完整）；
/// 压缩后快照(User)即历史 ⇒ 切点 0 → 中止，避免把快照自身当作待压缩历史。
#[test]
fn test_turn_split_idx_fallback_and_abort() {
    let turn = turn_root_msg("cur-root");
    let msgs = vec![assistant_msg("开场白"), turn.clone(), tool_call_msg()];
    assert_eq!(find_turn_user_split_idx(&msgs, "cur-root"), 1);

    let snapshot = user_msg("[CONTEXT SNAPSHOT]");
    let msgs2 = vec![snapshot, turn];
    assert_eq!(find_turn_user_split_idx(&msgs2, "cur-root"), 0);
}

/// 滞后口径回归：迟滞比较必须用扣除请求级 overhead 后的内容侧读数。
/// 旧实现直接比较含 overhead 的 current：overhead 越大越容易虚高越过
/// post_tokens×1.15 地板，迟滞保护失效 → 压缩高频触发。
#[test]
fn test_hysteresis_compares_content_tokens_excluding_overhead() {
    let make_snapshot = || {
        let mut m = assistant_msg("snapshot");
        m.meta = Some(serde_json::json!({
            "compacted": true,
            "post_tokens": 1000
        }));
        m
    };
    let floor = (1000.0 * COMPACT_HYSTERESIS_FACTOR) as usize;

    // 内容水位低于地板，但 current = 内容 + overhead 虚高越过地板：
    // 旧口径（直接比较 current）会触发，新口径（扣除 overhead）必须拦截。
    let msgs = vec![make_snapshot(), user_msg(&"中文字符填充".repeat(50))];
    let content_tokens = estimate_context_tokens(&msgs, 0);
    assert!(
        content_tokens < floor,
        "前提：内容水位 {content_tokens} 应低于地板 {floor}"
    );
    let overhead = floor - content_tokens + 10;
    let current = content_tokens + overhead;
    assert!(current >= floor, "前提：current 应虚高越过地板");
    let threshold_limit = (current as f64 / 0.7 * 0.99) as usize;
    assert!(!should_start_compression(
        &msgs,
        threshold_limit,
        false,
        overhead
    ));

    // 对照：内容真实增长越过地板后，迟滞放行（overhead 不应造成过度抑制）。
    let grown = vec![make_snapshot(), user_msg(&"中文字符填充".repeat(400))];
    let grown_tokens = estimate_context_tokens(&grown, 0);
    assert!(
        grown_tokens >= floor,
        "前提：{grown_tokens} 应不低于地板 {floor}"
    );
    let grown_overhead = 500;
    let grown_current = grown_tokens + grown_overhead;
    let grown_limit = (grown_current as f64 / 0.7 * 0.99) as usize;
    assert!(should_start_compression(
        &grown,
        grown_limit,
        false,
        grown_overhead
    ));
}

#[test]
fn test_should_start_compression_hysteresis() {
    // 压缩后 post_tokens=1000，当前估算 1100（< 1150）⇒ 即使超 70% 阈值也跳过
    let mut snapshot = assistant_msg("snapshot");
    snapshot.meta = Some(serde_json::json!({
        "compacted": true,
        "post_tokens": 1000
    }));
    // 构造一条足够大的历史让 current 估算落在 (1000, 1150) 区间
    let msgs = vec![snapshot, user_msg(&"中文字符填充".repeat(50))];
    let small_limit = estimate_context_tokens(&msgs, 0); // ≈ current
                                                         // 阈值设为远小于 current ⇒ 无迟滞时会触发
    let threshold_limit = (small_limit as f64 / 0.7 * 0.99) as usize;
    // current/threshold ≈ 0.7/0.99 > 1 ⇒ 超阈值，但 current < 1150 ⇒ 迟滞应拦截
    assert!(!should_start_compression(&msgs, threshold_limit, false, 0));
    // force 不受迟滞限制
    assert!(should_start_compression(&msgs, threshold_limit, true, 0));
}

/// 诉求3：压缩模板只走 system role，请求消息只携带待压缩对话
/// （prepare_compression / build_compression_request 均不得内嵌模板）。
#[test]
fn test_compression_request_user_message_carries_data_only() {
    let text_of =
        |m: &ChatMessage| -> String { m.content.as_ref().map(|c| c.to_text()).unwrap_or_default() };
    let msgs = vec![
        user_msg(&"x".repeat(1000)),
        assistant_msg(&"y".repeat(1000)),
        user_msg(&"z".repeat(1000)),
    ];
    let (req, _, _) = prepare_compression(&msgs).unwrap();
    // 末尾一条是压缩指令：模板必须只在 system 侧出现
    let instruction = text_of(req.last().expect("末尾为压缩指令"));
    assert!(
        !instruction.contains("<state_snapshot>"),
        "压缩模板不得随请求消息下发"
    );

    let req = build_compression_request(&msgs, Some("keep file paths"));
    let instruction = text_of(req.last().expect("末尾为压缩指令"));
    assert!(
        instruction.contains("keep file paths"),
        "hints 必须随指令下发"
    );
    assert!(!instruction.contains("<state_snapshot>"));
}

#[test]
fn test_prepare_compression_rejects_when_no_split() {
    // 单条 User 消息（巨型单轮场景）：无切分点 ⇒ prepare_compression 返回 None
    let msgs = vec![user_msg(&"x".repeat(100_000))];
    assert!(prepare_compression(&msgs).is_none());
}

// ── 水位提醒（chat_loop 直接调用，审计 §4.2 补测）──────────────────────────
//
// 本文件**不再**有"本地兜底压缩"的测试：`emergency_tail_compression` 已随
// "压缩失败不得裁剪历史"一并删除（见本文件末节与 `docs/DECISIONS.md` ADR-018）。

/// 长度可预测的中文消息（CJK ≈ 1 token/字 + 8 框架开销）。
fn cn_msg(text: &str) -> ChatMessage {
    user_msg(text)
}

#[test]
fn test_should_emit_context_nudge_threshold() {
    // 阈值 55%：limit=100 → 55 token 处触发
    let below = vec![cn_msg(&"字".repeat(40))]; // 40 + 8 = 48
    let above = vec![cn_msg(&"字".repeat(50))]; // 50 + 8 = 58
    assert!(!should_emit_context_nudge(&below, 100, 0));
    assert!(should_emit_context_nudge(&above, 100, 0));
    // overhead 计入总水位（此前只被 should_start_compression 使用）
    assert!(should_emit_context_nudge(&below, 100, 20));
    // 空历史永不提醒
    assert!(!should_emit_context_nudge(&[], 100, 1000));
}
