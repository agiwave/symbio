//! `symbio/src/symbio_core/turn.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

/// apinex qwen-3.8-max 网关的真实行为：
/// 首个增量携带合法 id，后续增量重复发送 `id:""`。
/// 空串不得覆盖合法 wire id——否则最终得到 Some("")，工具调用被误判为
/// id 缺失而跳过，落库的 ToolCall 节点无结果子节点。
/// 节点 id 与 wire id 分离：节点 id 稳定，wire id 保留 provider 原值。
#[test]
fn empty_id_delta_does_not_overwrite_real_id() {
    let mut acc = ToolCallAccumulator::default();
    let (node1, wire1, _, _, snapshot1) = acc.process_delta(
        0,
        Some("call_8f3a59f5f8e14258a427e432"),
        Some("get_weather"),
        Some(""),
    );
    assert!(snapshot1, "首个增量必须要求完整快照（接收端尚无此节点）");
    assert_eq!(wire1, "call_8f3a59f5f8e14258a427e432");
    assert!(!node1.is_empty());
    assert_ne!(node1, wire1, "节点 id 是本地分配的，不等于 wire id");

    // 后续增量：id:""（该网关的真实行为）
    let (node2, wire2, args, _, snapshot2) =
        acc.process_delta(0, Some(""), None, Some("{\"city\": \"Paris\"}"));
    assert!(
        !snapshot2,
        "后续增量未改身份字段 ⇒ 只需窄追加（Append），不得整条重发"
    );
    assert_eq!(node2, node1, "节点 id 在同一调用内稳定");
    assert_eq!(wire2, "call_8f3a59f5f8e14258a427e432");
    assert_eq!(args, "{\"city\": \"Paris\"}");

    let done = acc.get_completed();
    assert_eq!(done.len(), 1);
    assert_eq!(done[0].id.as_deref(), Some(node1.as_str()));
    assert_eq!(
        done[0].wire_id.as_deref(),
        Some("call_8f3a59f5f8e14258a427e432")
    );
    assert_eq!(done[0].name.as_deref(), Some("get_weather"));
}

/// 供应商始终未返回 id 时，节点 id 在首个增量即确定（流式/落库/执行三处一致，
/// 且重复取值幂等——chat_loop 与 into_messages 各取一次，不一致会使结果子节点
/// 变孤儿）；wire id 为 None，请求构建回退节点 id。
#[test]
fn missing_id_gets_stable_generated_guid() {
    let mut acc = ToolCallAccumulator::default();
    let (stream_id, wire, _, _, _) =
        acc.process_delta(0, None, Some("vdfs_list"), Some("{\"path\": \".\"}"));
    assert!(!stream_id.is_empty(), "流式期间即应有非空节点 id");
    assert_ne!(stream_id, "tc-0", "不得再使用 index 占位符");
    assert_eq!(wire, stream_id, "无 wire id 时回退为节点 id");

    let done1 = acc.get_completed();
    let done2 = acc.get_completed();
    assert_eq!(done1[0].id.as_deref(), Some(stream_id.as_str()));
    assert_eq!(
        done2[0].id.as_deref(),
        Some(stream_id.as_str()),
        "重复调用 get_completed 必须返回同一节点 id"
    );
    assert!(
        done1[0].wire_id.is_none(),
        "供应商未提供 id ⇒ wire_id 为 None"
    );
}

/// 纯空白 id 视为"不合法"，与缺失同等对待。
#[test]
fn whitespace_id_treated_as_missing() {
    let mut acc = ToolCallAccumulator::default();
    let (node_id, _, _, _, _) = acc.process_delta(0, Some("   "), None, Some("{}"));
    assert!(!node_id.trim().is_empty());
}

/// 空串 name 不得覆盖首个增量的合法 name（与 id 同理）。
#[test]
fn empty_name_delta_does_not_overwrite_real_name() {
    let mut acc = ToolCallAccumulator::default();
    acc.process_delta(0, Some("call_x"), Some("cmd.exe"), Some(""));
    acc.process_delta(0, Some(""), Some(""), Some("{}"));

    let done = acc.get_completed();
    assert_eq!(done[0].wire_id.as_deref(), Some("call_x"));
    assert_eq!(done[0].name.as_deref(), Some("cmd.exe"));
}

/// 多个并行工具调用（不同 index）互不干扰，各自持有独立的节点 id 与 wire id。
#[test]
fn parallel_tool_calls_keep_separate_ids() {
    let mut acc = ToolCallAccumulator::default();
    acc.process_delta(0, Some("call_a"), Some("f1"), Some("{}"));
    acc.process_delta(1, Some("call_b"), Some("f2"), Some("{}"));

    let mut done = acc.get_completed();
    done.sort_by(|a, b| a.id.cmp(&b.id));
    let wire_ids: Vec<_> = done.iter().filter_map(|t| t.wire_id.clone()).collect();
    assert!(wire_ids.contains(&"call_a".to_string()));
    assert!(wire_ids.contains(&"call_b".to_string()));
    let node_ids: Vec<_> = done.iter().filter_map(|t| t.id.clone()).collect();
    assert_ne!(node_ids[0], node_ids[1], "节点 id 互不相同");
}

/// 回归（本次迁移的根因之一）：跨轮复用同一 provider id 的两个累积器
/// （模拟同一会话的两轮 LLM 请求）必须产生**不同的节点 id**——否则第二轮
/// 的工具调用会更新到第一轮的老节点。
#[test]
fn reused_wire_id_across_turns_yields_distinct_node_ids() {
    let mut turn1 = ToolCallAccumulator::default();
    let (node1, _, _, _, _) = turn1.process_delta(0, Some("call_0"), Some("f"), Some("{}"));
    let mut turn2 = ToolCallAccumulator::default();
    let (node2, wire2, _, _, _) = turn2.process_delta(0, Some("call_0"), Some("f"), Some("{}"));

    assert_eq!(wire2, "call_0");
    assert_ne!(node1, node2, "跨轮同 wire id 必须产生不同节点 id");
}

/// 空串/纯空白参数是无参工具的合法形态（`from_str("")` 必失败）：
/// 必须视为 `{}` 且**不得**标记 parse_error，否则无参工具会被误拒。
#[test]
fn empty_arguments_treated_as_empty_object() {
    let mut acc = ToolCallAccumulator::default();
    acc.process_delta(0, Some("call_e"), Some("vdfs_list"), Some(""));
    acc.process_delta(1, Some("call_w"), Some("vdfs_list"), Some("   "));

    let done = acc.get_completed();
    assert_eq!(done.len(), 2);
    for tc in &done {
        assert_eq!(tc.arguments, serde_json::json!({}));
        assert!(tc.parse_error.is_none(), "空参数不得标记为解析失败");
    }
}

/// 合法 JSON 参数照常解析，parse_error 为 None。
#[test]
fn valid_arguments_parse_without_error() {
    let mut acc = ToolCallAccumulator::default();
    acc.process_delta(
        0,
        Some("call_v"),
        Some("cmd"),
        Some(r#"{"command": "dir"}"#),
    );

    let done = acc.get_completed();
    assert_eq!(done[0].arguments, serde_json::json!({"command": "dir"}));
    assert!(done[0].parse_error.is_none());
}

/// 回归（卡思考根因）：非空但非法的参数 JSON（典型为 max_tokens 截断）
/// **不得**静默回退 `{}`——必须保留原文于 parse_error，由执行侧拒绝执行。
/// 静默 `{}` 会让工具报「缺少必填参数」，模型看不懂原因便原样重试。
#[test]
fn truncated_arguments_flagged_not_silently_emptied() {
    let mut acc = ToolCallAccumulator::default();
    let raw = r#"{"command": "cargo test"#;
    acc.process_delta(0, Some("call_t"), Some("cmd"), Some(raw));

    let done = acc.get_completed();
    assert_eq!(done[0].arguments, serde_json::json!({}), "占位仍为 空对象");
    assert_eq!(
        done[0].parse_error.as_deref(),
        Some(raw),
        "必须保留残破原文"
    );
}

/// 参数分片：**只有首个增量**要求完整快照，其后每一片都只要求窄追加。
///
/// 这是「ToolCall 请求参数与 Text / Reasoning 同构」的协议契约——帧面只有
/// 「整条替换」与「尾部追加」两种语义，接收端不必按节点类型去猜。
#[test]
fn only_first_args_fragment_requires_snapshot() {
    let mut acc = ToolCallAccumulator::default();
    let (_, _, args1, _, snap1) =
        acc.process_delta(0, Some("call_s"), Some("vdfs_read"), Some("{\"pa"));
    let (_, _, args2, _, snap2) = acc.process_delta(0, None, None, Some("th\": \".\"}"));
    let (_, _, args3, _, snap3) = acc.process_delta(0, None, None, Some(""));

    assert!(snap1, "首片建节点 ⇒ 完整快照");
    assert!(!snap2, "第二片仅为参数增长 ⇒ 窄追加");
    assert!(!snap3, "空片不改动任何东西 ⇒ 窄追加（调用方跳过）");
    assert_eq!(args1, "{\"pa");
    assert_eq!(args2, "{\"path\": \".\"}");
    assert_eq!(args3, args2, "空片不改变累积值");
}

fn out(text: &str, reasoning: &str) -> TurnOutput {
    TurnOutput {
        text: text.to_string(),
        reasoning: reasoning.to_string(),
        ..Default::default()
    }
}

#[test]
fn is_reasoning_only_requires_empty_text_and_no_tools() {
    // 只有 reasoning、无正文、无工具 → reasoning-only
    assert!(out("", "思考").is_reasoning_only(0));
    // 空白正文同样视为「无文本回复」
    assert!(out("  \n ", "思考").is_reasoning_only(0));
    // 有正文 → 非 reasoning-only
    assert!(!out("回复", "思考").is_reasoning_only(0));
    // 有工具调用 → 非 reasoning-only（reasoning 需保留为独立子节点）
    assert!(!out("", "思考").is_reasoning_only(1));
    // 无 reasoning → 非 reasoning-only
    assert!(!out("", "").is_reasoning_only(0));
}

#[test]
fn effective_text_falls_back_to_reasoning_only_when_reasoning_only() {
    assert_eq!(out("", "思考").effective_text(0), "思考");
    assert_eq!(out("回复", "思考").effective_text(0), "回复");
    // 有工具时不回退，正文为空即为空
    assert_eq!(out("", "思考").effective_text(1), "");
}

/// 端到端（纯内存）：reasoning-only 的 TurnOutput 落库消息里
/// 同一段 reasoning 只出现一次，且没有 Reasoning 子节点。
#[test]
fn into_messages_reasoning_only_has_no_duplicate_content() {
    let reasoning = "让我想想这个问题的关键点。";
    let msgs = out("", reasoning).into_messages("turn-x", 0);

    assert_eq!(msgs.len(), 2, "应为 Turn + 单个 Text 子节点");
    assert_eq!(msgs[0].msg_type, Some(MessageType::Turn));
    assert!(
        !msgs
            .iter()
            .any(|m| m.msg_type == Some(MessageType::Reasoning)),
        "reasoning-only 不得产生 Reasoning 子节点"
    );

    let occurrences = msgs
        .iter()
        .filter(|m| {
            m.content
                .as_ref()
                .map(|c| c.to_text().contains(reasoning))
                .unwrap_or(false)
        })
        .count();
    assert_eq!(occurrences, 1, "同一段 reasoning 只能落库一份（factor=1）");
}

#[test]
fn into_messages_reasoning_with_reply_keeps_two_children() {
    let msgs = out("这是回复", "这是思考").into_messages("turn-y", 0);
    assert_eq!(msgs.len(), 3);
    assert_eq!(
        msgs.iter()
            .filter(|m| m.msg_type == Some(MessageType::Reasoning))
            .count(),
        1
    );
    assert_eq!(
        msgs.iter()
            .filter(|m| m.msg_type == Some(MessageType::Text))
            .count(),
        1
    );
}
