//! `plugins/model/tool_accumulator.rs` 的单元测试 —— 与实现同级分文件。
//!
//! 自 `symbio_core/llm/turn.test.rs` 迁入（该文件里测累积器的那 10 条），
//! 断言随 `get_completed(&mut self)` → `finish(self)` 的收口语义调整：
//! 「重复调用返回同一 id」这条不再是运行时兜底，而是**结构事实**——
//! 节点 id 由 `process_delta` 分配并原样带进 `finish` 的产物。

use super::*;

/// apinex qwen-3.8-max 网关的真实行为：
/// 首个增量携带合法 id，后续增量重复发送 `id:""`。
/// 空串不得覆盖合法 wire id——否则最终得到 Some("")，工具调用被误判为
/// id 缺失而跳过，落库的 ToolCall 节点无结果子节点。
/// 节点 id 与 wire id 分离：节点 id 稳定，wire id 保留 provider 原值。
#[test]
fn empty_id_delta_does_not_overwrite_real_id() {
    let mut acc = TurnToolCallAccumulator::default();
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

    let done = acc.finish();
    assert_eq!(done.len(), 1);
    assert_eq!(done[0].id.as_deref(), Some(node1.as_str()));
    assert_eq!(
        done[0].wire_id.as_deref(),
        Some("call_8f3a59f5f8e14258a427e432")
    );
    assert_eq!(done[0].name.as_deref(), Some("get_weather"));
}

/// 供应商始终未返回 id 时，节点 id 在首个增量即确定；`process_delta` 返回的
/// `node_id` 与 `finish()` 产物里的 `id` **必须是同一个值**——流式帧用的是前者、
/// 落库用的是后者，不一致会使工具结果子节点变孤儿。wire id 为 None，请求构建回退节点 id。
#[test]
fn missing_id_gets_stable_generated_guid() {
    let mut acc = TurnToolCallAccumulator::default();
    let (stream_id, wire, _, _, _) =
        acc.process_delta(0, None, Some("vdfs_list"), Some("{\"path\": \".\"}"));
    assert!(!stream_id.is_empty(), "流式期间即应有非空节点 id");
    assert_ne!(stream_id, "tc-0", "不得再使用 index 占位符");
    assert_eq!(wire, stream_id, "无 wire id 时回退为节点 id");

    let done = acc.finish();
    assert_eq!(
        done[0].id.as_deref(),
        Some(stream_id.as_str()),
        "流式帧 id 与落库 id 必须一致"
    );
    assert!(
        done[0].wire_id.is_none(),
        "供应商未提供 id ⇒ wire_id 为 None"
    );
}

/// 纯空白 id 视为"不合法"，与缺失同等对待。
#[test]
fn whitespace_id_treated_as_missing() {
    let mut acc = TurnToolCallAccumulator::default();
    let (node_id, _, _, _, _) = acc.process_delta(0, Some("   "), None, Some("{}"));
    assert!(!node_id.trim().is_empty());
}

/// 空串 name 不得覆盖首个增量的合法 name（与 id 同理）。
#[test]
fn empty_name_delta_does_not_overwrite_real_name() {
    let mut acc = TurnToolCallAccumulator::default();
    acc.process_delta(0, Some("call_x"), Some("cmd.exe"), Some(""));
    acc.process_delta(0, Some(""), Some(""), Some("{}"));

    let done = acc.finish();
    assert_eq!(done[0].wire_id.as_deref(), Some("call_x"));
    assert_eq!(done[0].name.as_deref(), Some("cmd.exe"));
}

/// 多个并行工具调用（不同 index）互不干扰，各自持有独立的节点 id 与 wire id；
/// 且 `finish` **按 wire index 升序**返回——否则 `HashMap` 的随机迭代顺序会让
/// 同一批工具的执行顺序在两次运行间漂移。
#[test]
fn parallel_tool_calls_keep_separate_ids() {
    let mut acc = TurnToolCallAccumulator::default();
    acc.process_delta(0, Some("call_a"), Some("f1"), Some("{}"));
    acc.process_delta(1, Some("call_b"), Some("f2"), Some("{}"));

    let done = acc.finish();
    let wire_ids: Vec<_> = done.iter().filter_map(|t| t.wire_id.clone()).collect();
    assert_eq!(
        wire_ids,
        vec!["call_a".to_string(), "call_b".to_string()],
        "按 index 升序返回"
    );
    assert_ne!(done[0].id, done[1].id, "节点 id 互不相同");
}

/// 乱序到达的分片同样按 index 收口（真实流里 index 可能先 1 后 0）。
#[test]
fn finish_sorts_by_wire_index() {
    let mut acc = TurnToolCallAccumulator::default();
    acc.process_delta(2, Some("call_c"), Some("f3"), Some("{}"));
    acc.process_delta(0, Some("call_a"), Some("f1"), Some("{}"));
    acc.process_delta(1, Some("call_b"), Some("f2"), Some("{}"));

    let wire_ids: Vec<_> = acc
        .finish()
        .iter()
        .filter_map(|t| t.wire_id.clone())
        .collect();
    assert_eq!(
        wire_ids,
        vec![
            "call_a".to_string(),
            "call_b".to_string(),
            "call_c".to_string()
        ]
    );
}

/// 回归（本次迁移的根因之一）：跨轮复用同一 provider id 的两个累积器
/// （模拟同一会话的两轮 LLM 请求）必须产生**不同的节点 id**——否则第二轮
/// 的工具调用会更新到第一轮的老节点。
#[test]
fn reused_wire_id_across_turns_yields_distinct_node_ids() {
    let mut turn1 = TurnToolCallAccumulator::default();
    let (node1, _, _, _, _) = turn1.process_delta(0, Some("call_0"), Some("f"), Some("{}"));
    let mut turn2 = TurnToolCallAccumulator::default();
    let (node2, wire2, _, _, _) = turn2.process_delta(0, Some("call_0"), Some("f"), Some("{}"));

    assert_eq!(wire2, "call_0");
    assert_ne!(node1, node2, "跨轮同 wire id 必须产生不同节点 id");
}

/// 空串/纯空白参数是无参工具的合法形态（`from_str("")` 必失败）：
/// 必须视为 `{}` 且**不得**标记 parse_error，否则无参工具会被误拒。
#[test]
fn empty_arguments_treated_as_empty_object() {
    let mut acc = TurnToolCallAccumulator::default();
    acc.process_delta(0, Some("call_e"), Some("vdfs_list"), Some(""));
    acc.process_delta(1, Some("call_w"), Some("vdfs_list"), Some("   "));

    let done = acc.finish();
    assert_eq!(done.len(), 2);
    for tc in &done {
        assert_eq!(tc.arguments, serde_json::json!({}));
        assert!(tc.parse_error.is_none(), "空参数不得标记为解析失败");
    }
}

/// 合法 JSON 参数照常解析，parse_error 为 None。
#[test]
fn valid_arguments_parse_without_error() {
    let mut acc = TurnToolCallAccumulator::default();
    acc.process_delta(
        0,
        Some("call_v"),
        Some("cmd"),
        Some(r#"{"command": "dir"}"#),
    );

    let done = acc.finish();
    assert_eq!(done[0].arguments, serde_json::json!({"command": "dir"}));
    assert!(done[0].parse_error.is_none());
}

/// 回归（卡思考根因）：非空但非法的参数 JSON（典型为 max_tokens 截断）
/// **不得**静默回退 `{}`——必须保留原文于 parse_error，由执行侧拒绝执行。
/// 静默 `{}` 会让工具报「缺少必填参数」，模型看不懂原因便原样重试。
#[test]
fn truncated_arguments_flagged_not_silently_emptied() {
    let mut acc = TurnToolCallAccumulator::default();
    let raw = r#"{"command": "cargo test"#;
    acc.process_delta(0, Some("call_t"), Some("cmd"), Some(raw));

    let done = acc.finish();
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
    let mut acc = TurnToolCallAccumulator::default();
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
