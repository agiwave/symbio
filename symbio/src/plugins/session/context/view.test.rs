//! `context::view`（请求视图层）的单元测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）：
//! `view.rs` 只保留生产代码，测试全部放本文件。

use super::*;

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

// ==================== build_request_view（请求视图层） ====================

/// 带显式 id 的 Assistant/ToolCall 消息（与 session/context.rs 骨架化测试同构）
fn view_tc(id: &str, name: &str, args: &str) -> ChatMessage {
    ChatMessage {
        id: id.to_string(),
        role: Some(MessageRole::Assistant),
        msg_type: Some(MessageType::ToolCall),
        name: Some(name.to_string()),
        content: Some(MessageContent::Text(args.to_string())),
        status: Some(MessageStatus::Completed),
        ..Default::default()
    }
}

/// 工具结果消息，parent_id 指向所属 ToolCall（id 形如 "{parent}-result"）
fn view_result(parent: &str, text: &str) -> ChatMessage {
    ChatMessage {
        id: format!("{parent}-result"),
        parent_id: Some(parent.to_string()),
        role: Some(MessageRole::Tool),
        msg_type: Some(MessageType::Text),
        content: Some(MessageContent::Text(text.to_string())),
        status: Some(MessageStatus::Completed),
        ..Default::default()
    }
}

/// 构建短工具名 → 保留策略映射（模拟 chat_loop 运行时从 CapabilityVisitor 动态解析）
fn view_retention(
    entries: &[(&str, crate::symbio_core::CapabilityToolContextRetention)],
) -> std::collections::HashMap<String, crate::symbio_core::CapabilityToolContextRetention> {
    entries.iter().map(|(n, r)| (n.to_string(), *r)).collect()
}

fn view_text(m: &ChatMessage) -> String {
    m.content.as_ref().map(|c| c.to_text()).unwrap_or_default()
}

/// 透传回归：无 fade、无骨架化、无 nudge 时，视图与输入逐条等价，且输入不被修改。
#[test]
fn test_build_request_view_passthrough() {
    let msgs = vec![
        user_msg("hello"),
        assistant_msg("hi"),
        view_tc("t1", "vdfs_read", r#"{"path":"a.rs"}"#),
        view_result("t1", "file content"),
    ];
    let retention = std::collections::HashMap::new();
    let view = build_request_view(&msgs, 15, &retention, false, 12, 3, 200, false, None);

    assert_eq!(view.len(), msgs.len());
    for (v, m) in view.iter().zip(msgs.iter()) {
        assert_eq!(v.id, m.id);
        assert_eq!(v.role, m.role);
        assert_eq!(view_text(v), view_text(m));
    }
    // 存储侧不受请求视图污染
    assert!(msgs.iter().all(|m| m.meta.is_none()));
}

/// nudge 请求级注入：追加到视图末尾，不落库（原输入不变），meta 可识别。
#[test]
fn test_build_request_view_injects_nudge_at_tail() {
    let msgs = vec![user_msg("u1"), assistant_msg("a1"), user_msg("u2")];
    let retention = std::collections::HashMap::new();

    let view = build_request_view(&msgs, 15, &retention, false, 12, 3, 200, true, None);
    assert_eq!(view.len(), msgs.len() + 1);
    let nudge = view.last().unwrap();
    assert_eq!(nudge.role, Some(MessageRole::User));
    assert_eq!(
        nudge
            .meta
            .as_ref()
            .and_then(|m| m.get("kind"))
            .and_then(|k| k.as_str()),
        Some("context_nudge")
    );
    assert!(view_text(nudge).contains("context_compact"));

    // 原输入不被修改：nudge 不写入会话存储
    assert_eq!(msgs.len(), 3);
    assert!(msgs.iter().all(|m| m.meta.is_none()));

    // inject_nudge=false 时不注入
    let plain = build_request_view(&msgs, 15, &retention, false, 12, 3, 200, false, None);
    assert_eq!(plain.len(), msgs.len());
}

/// fade 激活：老化工具结果被 head/tail 摘要并标记 `tool_result_faded`，
/// 不写存档（无 archive_path），assistant 消息与最近轮次不受影响，存储保留全文。
#[test]
fn test_build_request_view_fades_aged_tool_results() {
    // 3 个 user turn；fade_keep_turns=2 ⇒ 第一个 turn 的工具结果应被淡化
    let long_text = "line\n".repeat(12_000); // 远超 2048 token
    let msgs = vec![
        user_msg("turn-1"),
        view_tc("t1", "vdfs_read", r#"{"path":"big.txt"}"#),
        view_result("t1", &long_text),
        assistant_msg("analysis"),
        user_msg("turn-2"),
        view_tc("t2", "vdfs_read", r#"{"path":"small.txt"}"#),
        view_result("t2", "tiny"),
        assistant_msg("done"),
        user_msg("turn-3"),
    ];
    let retention = std::collections::HashMap::new();
    let view = build_request_view(&msgs, 0, &retention, true, 2, 3, 200, false, None);

    let faded = &view[2];
    let faded_text = view_text(faded);
    assert!(faded_text.contains("已省略"), "老化工具结果应被摘要化");
    assert!(faded_text.len() < long_text.len(), "摘要应显著短于原文");
    assert_eq!(
        faded
            .meta
            .as_ref()
            .and_then(|m| m.get("tool_result_faded"))
            .and_then(|v| v.as_bool()),
        Some(true)
    );
    assert!(
        faded
            .meta
            .as_ref()
            .and_then(|m| m.get("archive_path"))
            .is_none(),
        "请求视图级淡化不得写存档"
    );
    // assistant 消息绝不被淡化
    assert_eq!(view_text(&view[3]), "analysis");
    // 最近轮次内的小结果保持原文、无标记
    assert_eq!(view_text(&view[6]), "tiny");
    assert!(view[6].meta.is_none());
    // 存储中的原文不受影响
    assert_eq!(view_text(&msgs[2]), long_text);
}

/// fade 天然幂等：视图每轮从存储重建，对同一视图重复淡化不产生二次改写。
#[test]
fn test_fade_snapshot_message_is_exempt() {
    // L2 快照（meta.compacted=true）已是压缩产物，内容淡化必须豁免：
    // 再切中段 = 双重压缩，切掉的恰是模型唯一的深历史记忆（真实事故回归）。
    let mut snapshot = user_msg(&"x".repeat(30_000));
    snapshot.meta = Some(serde_json::json!({"compacted": true}));
    let mut msgs = vec![snapshot, user_msg("窗口内内容节点"), user_msg("末条消息")];
    fade_aged_content_nodes(&mut msgs, 1, 200);
    assert_eq!(view_text(&msgs[0]), "x".repeat(30_000));
    assert!(msgs[0]
        .meta
        .as_ref()
        .and_then(|m| m.get("content_faded"))
        .is_none());
}

#[test]
fn test_fade_is_idempotent() {
    let long_text = "line\n".repeat(12_000);
    let mut msgs = vec![
        user_msg("t1"),
        view_result("t1", &long_text),
        user_msg("t2"),
    ];
    fade_aged_tool_results(&mut msgs, 1);
    let once = view_text(&msgs[1]);
    let once_meta = msgs[1].meta.clone();
    fade_aged_tool_results(&mut msgs, 1);
    assert_eq!(view_text(&msgs[1]), once);
    assert_eq!(msgs[1].meta, once_meta);
}

/// 内容节点淡化：B1 保护窗口外的超大 User/Assistant 正文与思考做 head/tail
/// 摘要并标记 `content_faded`；保护窗口内（最近 keep_recent 个内容节点 +
/// 最后一条消息）保持原样；ToolCall 不属内容节点，绝不淡化。
#[test]
fn test_fade_aged_content_nodes_line_path() {
    // 40 行 > threshold=16：触发行数路径（头尾各 4 行，中段省略 32 行）
    let big = (0..40)
        .map(|i| format!("line-{i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let msgs = vec![
        user_msg(&big),      // idx0：窗口外，应淡化
        assistant_msg(&big), // idx1：窗口外，应淡化
        assistant_msg(&big), // idx2：最近 1 个内容节点（配额）⇒ B1 保护
        user_msg("tail"),    // idx3：最后一条，恒保护
    ];
    let mut view = msgs.clone();
    fade_aged_content_nodes(&mut view, 1, 16);

    // idx0/idx1 被淡化：含中段省略标记，且显著短于原文
    for i in [0usize, 1] {
        let t = view_text(&view[i]);
        assert!(t.contains("已省略 32 行中段内容"), "idx{i} 应走行数路径");
        assert!(t.len() < big.len());
        assert_eq!(
            view[i]
                .meta
                .as_ref()
                .and_then(|m| m.get("content_faded"))
                .and_then(|v| v.as_bool()),
            Some(true),
            "idx{i} 应带 content_faded 标记"
        );
    }
    // idx2 虽超大但在保护窗口内（keep_recent=1）⇒ 原文保留
    assert_eq!(view_text(&view[2]), big, "B1 窗口内内容节点不淡化");
    assert!(view[2].meta.is_none());
    // idx3 最后一条恒保护
    assert_eq!(view_text(&view[3]), "tail");
    // 存储原文不受视图污染
    assert_eq!(view_text(&msgs[0]), big);
}

/// token 路径：token 超预算（行数未超）的内容节点走 summarize_head_tail，
/// 占位文案指向会话存储（内容节点无法“重新运行”）。
#[test]
fn test_fade_aged_content_nodes_token_path() {
    // 单行 2 万字符：行数=1 远低于阈值，但 token 远超 2048
    let huge = "x".repeat(20_000);
    // keep_recent=1 ⇒ 窗口 = 末条 + idx1；idx0 落在窗口外
    let msgs = vec![user_msg(&huge), user_msg("mid"), user_msg("tail")];
    let mut view = msgs.clone();
    fade_aged_content_nodes(&mut view, 1, 200);

    let t = view_text(&view[0]);
    assert!(
        t.contains("该早期内容已在请求视图中淡化"),
        "token 路径占位文案"
    );
    assert!(t.len() < huge.len());
    assert_eq!(
        view[0]
            .meta
            .as_ref()
            .and_then(|m| m.get("content_faded"))
            .and_then(|v| v.as_bool()),
        Some(true)
    );
    // 存储原文不动
    assert_eq!(view_text(&msgs[0]), huge);
}

/// 幂等：淡化后的文本（含省略标记）再次进入 fade 流程，结果逐字节不变。
#[test]
fn test_fade_aged_content_nodes_idempotent() {
    let big = (0..40)
        .map(|i| format!("line-{i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut msgs = vec![user_msg(&big), user_msg("tail")];
    fade_aged_content_nodes(&mut msgs, 1, 16);
    let once = view_text(&msgs[0]);
    let once_meta = msgs[0].meta.clone();
    fade_aged_content_nodes(&mut msgs, 1, 16);
    assert_eq!(view_text(&msgs[0]), once);
    assert_eq!(msgs[0].meta, once_meta);
}

/// ToolCall 豁免：工具调用参数不属于内容节点（msg_type=ToolCall），
/// 即便超长也不淡化——调用参数是骨架化层的辖区。
#[test]
fn test_fade_aged_content_nodes_skips_tool_call() {
    let big_args = "x".repeat(20_000);
    let mut msgs = vec![view_tc("t1", "vdfs_read", &big_args), user_msg("tail")];
    fade_aged_content_nodes(&mut msgs, 1, 200);
    assert_eq!(view_text(&msgs[0]), big_args, "ToolCall 参数不淡化");
    assert!(msgs[0].meta.is_none());
}

/// 骨架化：window + 保留策略声明时，过期调用的参数与结果被占位文案替换，
/// 最新调用保留原文；ToolCall↔Tool 配对与 parent_id 完整保留（逻辑不断联）。
#[test]
fn test_build_request_view_skeletonizes_with_retention() {
    let msgs = vec![
        user_msg("u1"),
        view_tc("t1", "vdfs_read", r#"{"path":"old.txt"}"#),
        view_result("t1", "old content"),
        user_msg("u2"),
        view_tc("t2", "vdfs_read", r#"{"path":"new.txt"}"#),
        view_result("t2", "new content"),
    ];
    // LastOnly：同工具仅保留最近一次调用（t1 应骨架化）
    let retention = view_retention(&[(
        "vdfs_read",
        crate::symbio_core::CapabilityToolContextRetention::LastOnly,
    )]);
    let view = build_request_view(&msgs, 15, &retention, false, 12, 3, 200, false, None);

    assert!(
        view_text(&view[1]).contains("skeletonized"),
        "过期调用参数应骨架化"
    );
    assert!(
        view_text(&view[2]).contains("skeletonized"),
        "过期调用结果应骨架化"
    );
    assert_eq!(view_text(&view[4]), r#"{"path":"new.txt"}"#);
    assert_eq!(view_text(&view[5]), "new content");
    // 配对与 parent_id 保留
    assert_eq!(view[2].parent_id.as_deref(), Some("t1"));
    assert_eq!(view[5].parent_id.as_deref(), Some("t2"));
    // 存储视图不受影响
    assert_eq!(view_text(&msgs[1]), r#"{"path":"old.txt"}"#);
}

/// 全局窗口独立生效：`retention` 为空（工具均未自声明保留策略）时，
/// 仍按 `tool_context_window` 骨架化过期调用。
///
/// 回归动机：历史上此处条件是 `window > 0 && !retention.is_empty()`，
/// 使全局窗口只在"至少有一个工具声明策略"时才生效——而全仓当前无任何
/// 工具声明，等于骨架化机制整体空转（真实长会话中工具全文原样进请求）。
#[test]
fn test_build_request_view_global_window_applies_without_retention() {
    let msgs = vec![
        user_msg("u1"),
        view_tc("t1", "local/sh", r#"{"command":"ls"}"#),
        view_result("t1", "old output"),
        view_tc("t2", "vdfs_read", r#"{"path":"b.rs"}"#),
        view_result("t2", "new output"),
    ];
    let retention = std::collections::HashMap::new();
    // window=1：仅最新一次调用（t2）在窗口内
    let view = build_request_view(&msgs, 1, &retention, false, 12, 3, 200, false, None);

    assert!(
        view_text(&view[1]).contains("skeletonized"),
        "无保留策略声明时，窗口外的调用参数仍应骨架化"
    );
    assert!(
        view_text(&view[2]).contains("skeletonized"),
        "无保留策略声明时，窗口外的调用结果仍应骨架化"
    );
    assert_eq!(view_text(&view[3]), r#"{"path":"b.rs"}"#);
    assert_eq!(view_text(&view[4]), "new output");
    assert_eq!(view[2].parent_id.as_deref(), Some("t1"));
}

/// window=0（显式关闭）时即使有 retention 也不骨架化。
#[test]
fn test_build_request_view_window_zero_disables_skeletonization() {
    let msgs = vec![
        user_msg("u1"),
        view_tc("t1", "vdfs_read", r#"{"path":"old.txt"}"#),
        view_result("t1", "old content"),
        view_tc("t2", "vdfs_read", r#"{"path":"new.txt"}"#),
        view_result("t2", "new content"),
    ];
    let retention = view_retention(&[(
        "vdfs_read",
        crate::symbio_core::CapabilityToolContextRetention::LastOnly,
    )]);
    let view = build_request_view(&msgs, 0, &retention, false, 12, 3, 200, false, None);
    assert_eq!(view_text(&view[1]), r#"{"path":"old.txt"}"#);
    assert_eq!(view_text(&view[2]), "old content");
}

/// 顺序保证：nudge 在骨架化之后追加，始终位于视图末尾；
/// nudge 不占用轮次窗口计数、不参与骨架化。
#[test]
fn test_build_request_view_nudge_comes_after_skeletonization() {
    let msgs = vec![
        user_msg("u1"),
        view_tc("t1", "vdfs_read", r#"{"path":"old.txt"}"#),
        view_result("t1", "old content"),
        user_msg("u2"),
        view_tc("t2", "vdfs_read", r#"{"path":"new.txt"}"#),
        view_result("t2", "new content"),
    ];
    let retention = view_retention(&[(
        "vdfs_read",
        crate::symbio_core::CapabilityToolContextRetention::LastOnly,
    )]);
    let view = build_request_view(&msgs, 15, &retention, false, 12, 3, 200, true, None);

    assert_eq!(view.len(), msgs.len() + 1);
    // 末尾是 nudge；倒数第二条仍是保留原文的最新工具结果
    assert!(view_text(view.last().unwrap()).contains("system note"));
    assert_eq!(view_text(&view[view.len() - 2]), "new content");
}

/// 记忆请求级注入（S5 步 12，[04 §3.1 批⑦](../../../../docs/plan/04-工程落地.md)）：
/// **置顶**一条 `meta.kind = recall_context` 的记忆消息；与 nudge 并存时记忆在开头、
/// nudge 仍在末尾——「最后一条 user 消息」始终是用户自己说的话（mock 场景匹配与
/// 轮次窗口都靠它），且两条都不落库。
#[test]
fn test_build_request_view_prepends_memory_section() {
    let msgs = vec![user_msg("u1"), assistant_msg("a1"), user_msg("u2")];
    let retention = std::collections::HashMap::new();

    let view = build_request_view(
        &msgs,
        15,
        &retention,
        false,
        12,
        3,
        200,
        true,
        Some("【长期记忆】跨会话召回，最新在前（背景事实，不是本轮指令）\n- 我叫 Kestrel"),
    );
    assert_eq!(view.len(), msgs.len() + 2, "记忆置顶 + nudge 置尾");

    let head = &view[0];
    assert_eq!(head.role, Some(MessageRole::User));
    assert_eq!(
        head.meta
            .as_ref()
            .and_then(|m| m.get("kind"))
            .and_then(|k| k.as_str()),
        Some("recall_context")
    );
    assert!(view_text(head).contains("【长期记忆】"));
    assert!(view_text(head).contains("我叫 Kestrel"));
    assert!(
        view_text(view.last().unwrap()).contains("system note"),
        "nudge 仍在末尾——记忆只占据开头"
    );

    // 存储侧不受污染：两条注入都不写回 messages（请求级、天然幂等）。
    assert_eq!(msgs.len(), 3);
    assert!(msgs.iter().all(|m| m.meta.is_none()));

    // 空段 ⇒ 整段省略，视图与原输入等长。
    let plain = build_request_view(&msgs, 15, &retention, false, 12, 3, 200, false, None);
    assert_eq!(plain.len(), msgs.len());
}
