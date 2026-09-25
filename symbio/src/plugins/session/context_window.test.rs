//! `context_window` 模块的单元测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）：
//! `context_window.rs` 只保留生产代码，测试全部放本文件。

use super::*;
use crate::symbio_core::schemas::session::chat_message::MessageStatus;

fn tc_msg(id: &str, name: &str, args: &str) -> ChatMessage {
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

fn tool_result(parent: &str, text: &str) -> ChatMessage {
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

fn text(content: &str) -> ChatMessage {
    ChatMessage {
        id: format!("t-{}", content),
        role: Some(MessageRole::User),
        msg_type: Some(MessageType::Text),
        content: Some(MessageContent::Text(content.to_string())),
        status: Some(MessageStatus::Completed),
        ..Default::default()
    }
}

/// 构建 短工具名 → 保留策略 映射（模拟会话循环运行时从 CapabilityVisitor 动态解析）
fn retention_map(
    entries: &[(&str, CapabilityToolContextRetention)],
) -> HashMap<String, CapabilityToolContextRetention> {
    entries.iter().map(|(n, r)| (n.to_string(), *r)).collect()
}

fn param_of(msg: &ChatMessage) -> String {
    msg.content
        .as_ref()
        .map(|c| c.to_text())
        .unwrap_or_default()
}

/// LastOnly：同名工具多次调用，仅最新一次参数/结果完整，旧的骨架化
#[test]
fn last_only_keeps_only_latest_call_of_same_tool() {
    let messages = vec![
        text("第一轮"),
        tc_msg("tc1", "local/todo_write", r#"{"todos":"v1 很长很长"}"#),
        tool_result("tc1", "ok"),
        tc_msg("tc2", "local/todo_write", r#"{"todos":"v2 很长很长"}"#),
        tool_result("tc2", "ok"),
        text("第二轮"),
    ];
    let ret = retention_map(&[("todo_write", CapabilityToolContextRetention::LastOnly)]);
    let out = apply_layered_sliding_window(&messages, 15, &ret);

    let tc1 = out.iter().find(|m| m.id == "tc1").unwrap();
    let tc2 = out.iter().find(|m| m.id == "tc2").unwrap();
    assert!(
        param_of(tc1).contains("skeletonized"),
        "旧调用参数应被骨架化: {:?}",
        tc1.content
    );
    assert_eq!(
        param_of(tc2),
        r#"{"todos":"v2 很长很长"}"#,
        "最新调用参数应原样保留"
    );

    let r1 = out
        .iter()
        .find(|m| m.parent_id.as_deref() == Some("tc1"))
        .unwrap();
    let r2 = out
        .iter()
        .find(|m| m.parent_id.as_deref() == Some("tc2"))
        .unwrap();
    assert!(param_of(r1).contains("skeletonized"), "旧结果应被骨架化");
    assert_eq!(param_of(r2), "ok", "最新结果应原样保留");
}

/// LastN(2)：保留最近 2 次，更早的骨架化
#[test]
fn last_n_keeps_n_latest_calls() {
    let messages = vec![
        tc_msg("a1", "local/search", r#"{"q":"1"}"#),
        tool_result("a1", "r1"),
        tc_msg("a2", "local/search", r#"{"q":"2"}"#),
        tool_result("a2", "r2"),
        tc_msg("a3", "local/search", r#"{"q":"3"}"#),
        tool_result("a3", "r3"),
    ];
    let ret = retention_map(&[("search", CapabilityToolContextRetention::LastN(2))]);
    let out = apply_layered_sliding_window(&messages, 15, &ret);

    let a1 = out.iter().find(|m| m.id == "a1").unwrap();
    let a2 = out.iter().find(|m| m.id == "a2").unwrap();
    let a3 = out.iter().find(|m| m.id == "a3").unwrap();
    assert!(param_of(a1).contains("skeletonized"));
    assert_eq!(param_of(a2), r#"{"q":"2"}"#);
    assert_eq!(param_of(a3), r#"{"q":"3"}"#);
}

/// 未声明策略的工具不受影响（回归保护：默认行为与旧版一致）
#[test]
fn tools_without_retention_are_unaffected() {
    let messages = vec![
        tc_msg("x1", "vdfs_read", r#"{"path":"a.rs"}"#),
        tool_result("x1", "content"),
        tc_msg("x2", "vdfs_read", r#"{"path":"b.rs"}"#),
        tool_result("x2", "content2"),
    ];
    let ret = HashMap::new();
    let out = apply_layered_sliding_window(&messages, 15, &ret);
    let x1 = out.iter().find(|m| m.id == "x1").unwrap();
    let x2 = out.iter().find(|m| m.id == "x2").unwrap();
    assert_eq!(param_of(x1), r#"{"path":"a.rs"}"#);
    assert_eq!(param_of(x2), r#"{"path":"b.rs"}"#);
}

/// 全局窗口：超出的 ToolCall 自身参数也骨架化（压缩 tool_call 参数占比）
#[test]
fn global_window_skeletonizes_tool_call_params() {
    let mut messages = Vec::new();
    for i in 0..6 {
        messages.push(tc_msg(
            &format!("g{i}"),
            "vdfs_read",
            &format!(r#"{{"path":"f{i}.rs","blob":"x".repeat(50)}}"#),
        ));
        messages.push(tool_result(&format!("g{i}"), "done"));
    }
    // 全局窗口只保留最近 2 个
    let ret = HashMap::new();
    let out = apply_layered_sliding_window(&messages, 2, &ret);
    let g0 = out.iter().find(|m| m.id == "g0").unwrap();
    let g3 = out.iter().find(|m| m.id == "g3").unwrap();
    let g4 = out.iter().find(|m| m.id == "g4").unwrap();
    let g5 = out.iter().find(|m| m.id == "g5").unwrap();
    assert!(param_of(g0).contains("skeletonized"));
    assert!(param_of(g3).contains("skeletonized"));
    assert!(!param_of(g4).contains("skeletonized"));
    assert!(!param_of(g5).contains("skeletonized"));
    // 结果子节点同样骨架化
    let r0 = out
        .iter()
        .find(|m| m.parent_id.as_deref() == Some("g0"))
        .unwrap();
    assert!(param_of(r0).contains("skeletonized"));
    // 状态保持 Completed（不产生 Failed/孤儿，配对合法）
    assert_eq!(g0.status, Some(MessageStatus::Completed));
}

/// 工具级 LastOnly 优先于全局窗口（即使仍在全局窗口内也骨架化旧调用）
#[test]
fn retention_applies_even_within_global_window() {
    let messages = vec![
        tc_msg("p1", "local/todo_write", r#"{"v":1}"#),
        tool_result("p1", "ok"),
        tc_msg("p2", "local/todo_write", r#"{"v":2}"#),
        tool_result("p2", "ok"),
    ];
    // 全局窗口 15 足够大，但 LastOnly 仍骨架化 p1
    let ret = retention_map(&[("todo_write", CapabilityToolContextRetention::LastOnly)]);
    let out = apply_layered_sliding_window(&messages, 15, &ret);
    let p1 = out.iter().find(|m| m.id == "p1").unwrap();
    let p2 = out.iter().find(|m| m.id == "p2").unwrap();
    assert!(param_of(p1).contains("skeletonized"));
    assert_eq!(param_of(p2), r#"{"v":2}"#);
}

/// 回归：声明 LastOnly 的工具，其**最新一次**调用滚出全局窗口后仍完整保留。
/// 旧实现全局窗口判定优先，导致长会话中任务清单/最新状态被骨架化丢失
/// （真实会话中发生过：模型因看不到清单而整单重写）。
#[test]
fn last_only_latest_call_survives_beyond_global_window() {
    let mut messages = Vec::new();
    // 20 个无策略工具调用把全局窗口（15）挤满，todo_write 最新调用被推出窗口
    for i in 0..20 {
        messages.push(tc_msg(
            &format!("f{i}"),
            "vdfs_read",
            &format!(r#"{{"path":"f{i}.rs"}}"#),
        ));
        messages.push(tool_result(&format!("f{i}"), "content"));
    }
    messages.push(tc_msg(
        "latest",
        "local/todo_write",
        r#"{"todos":"清单 v3"}"#,
    ));
    messages.push(tool_result("latest", "已更新任务清单，共 3 项。"));
    let ret = retention_map(&[("todo_write", CapabilityToolContextRetention::LastOnly)]);
    let out = apply_layered_sliding_window(&messages, 15, &ret);

    // 无策略工具：全局窗口语义不变（前 6 条被骨架化）
    assert!(param_of(out.iter().find(|m| m.id == "f0").unwrap()).contains("skeletonized"));
    // LastOnly 工具：最新调用虽在全局窗口之外（第 21 个 ToolCall），仍完整保留
    let latest = out.iter().find(|m| m.id == "latest").unwrap();
    assert_eq!(
        param_of(latest),
        r#"{"todos":"清单 v3"}"#,
        "LastOnly 最新调用不应被全局窗口骨架化"
    );
    let latest_result = out
        .iter()
        .find(|m| m.parent_id.as_deref() == Some("latest"))
        .unwrap();
    assert_eq!(param_of(latest_result), "已更新任务清单，共 3 项。");
}

/// 骨架化占位符保留摘要：失败结果保留错误类型与首行原因
#[test]
fn skeletonized_failure_keeps_error_digest() {
    let messages = vec![
        tc_msg("e1", "vdfs_read", r#"{"path":"agent/README.md"}"#),
        {
            let mut m = tool_result("e1", "读取失败：os error 2 (系统找不到指定的文件。)");
            m.meta = Some(serde_json::json!({
                "success": false,
                "failure_kind": "not_found",
            }));
            m
        },
    ];
    let ret = HashMap::new();
    let out = apply_layered_sliding_window(&messages, 0, &ret);
    let r = out
        .iter()
        .find(|m| m.parent_id.as_deref() == Some("e1"))
        .unwrap();
    let content = param_of(r);
    assert!(
        content.contains("failed"),
        "失败骨架化应标注 failed: {content}"
    );
    assert!(
        content.contains("not_found"),
        "失败骨架化应保留 failure_kind: {content}"
    );
    assert!(
        content.contains("os error 2"),
        "失败骨架化应保留错误原因: {content}"
    );
}

/// 骨架化占位符保留摘要：成功结果保留首行摘要
#[test]
fn skeletonized_success_keeps_first_line_digest() {
    let messages = vec![
        tc_msg("s1", "vdfs_read", r#"{"path":"session/README.md"}"#),
        tool_result("s1", "# session 插件\n\n会话编排唯一入口……（后续 300 行）"),
    ];
    let ret = HashMap::new();
    let out = apply_layered_sliding_window(&messages, 0, &ret);
    let r = out
        .iter()
        .find(|m| m.parent_id.as_deref() == Some("s1"))
        .unwrap();
    let content = param_of(r);
    assert!(
        content.contains("successfully"),
        "成功骨架化应标注 successfully: {content}"
    );
    assert!(
        content.contains("# session 插件"),
        "成功骨架化应保留首行摘要: {content}"
    );
    assert!(
        !content.contains("后续 300 行"),
        "骨架化不应保留全文: {content}"
    );
}

/// 质量底线：首个非空行是碎片行（`150: }]`）时跳过取下一个有内容的行，
/// 而非产出 `Summary: 150: }]` 这类无信息量摘要（真实会话实证的痛点）。
#[test]
fn skeletonized_success_skips_fragment_first_line() {
    let messages = vec![
        tc_msg("s2", "local/grep", r#"{"pattern":"TODO"}"#),
        tool_result(
            "s2",
            "150: }]\n\nsrc/main.rs:12: TODO refactor\nsrc/lib.rs:3: TODO docs",
        ),
    ];
    let ret = HashMap::new();
    let out = apply_layered_sliding_window(&messages, 0, &ret);
    let r = out
        .iter()
        .find(|m| m.parent_id.as_deref() == Some("s2"))
        .unwrap();
    let content = param_of(r);
    assert!(
        content.contains("src/main.rs:12: TODO refactor"),
        "应跳过碎片行取首个有内容行: {content}"
    );
    assert!(
        !content.contains("150: }]"),
        "碎片行不应出现在摘要: {content}"
    );
}

/// 质量底线：全碎片内容（仅闭合括号/标点）时省略 Summary 子句而非输出空摘要
#[test]
fn skeletonized_success_all_fragment_omits_summary() {
    let messages = vec![
        tc_msg("s3", "local/grep", r#"{"pattern":"TODO"}"#),
        tool_result("s3", "}]"),
    ];
    let ret = HashMap::new();
    let out = apply_layered_sliding_window(&messages, 0, &ret);
    let r = out
        .iter()
        .find(|m| m.parent_id.as_deref() == Some("s3"))
        .unwrap();
    let content = param_of(r);
    assert!(
        content.contains("successfully"),
        "占位符仍应标注 successfully: {content}"
    );
    assert!(
        !content.contains("Summary:"),
        "全碎片时应省略 Summary 子句: {content}"
    );
}

/// 骨架化摘要判定：meta.success 优先于文本启发式
#[test]
fn failure_detection_prefers_structured_meta() {
    let messages = vec![tc_msg("m1", "local/shell", r#"{"command":"dir"}"#), {
        // 文本含 "failed" 但 meta.success=true → 仍视为成功
        let mut m = tool_result("m1", "0 failed tests, all passed");
        m.meta = Some(serde_json::json!({ "success": true }));
        m
    }];
    let ret = HashMap::new();
    let out = apply_layered_sliding_window(&messages, 0, &ret);
    let r = out
        .iter()
        .find(|m| m.parent_id.as_deref() == Some("m1"))
        .unwrap();
    assert!(
        param_of(r).contains("successfully"),
        "meta.success=true 应判定为成功"
    );
}

/// 骨架化保留定位锚点：参数骨架化保留首个定位参数，
/// 结果摘要前缀配对调用的锚点（真实会话实证：无主摘要"[行 585-602]"
/// 让模型不知对应哪个文件，只能整目录重读）
#[test]
fn skeletonized_call_and_result_keep_anchor_param() {
    let messages = vec![
        tc_msg(
            "a1",
            "vdfs_read",
            r#"{"path":"gateway/server.rs","limit":50}"#,
        ),
        tool_result("a1", "[行 585-602，共 621 行]"),
    ];
    let ret = HashMap::new();
    let out = apply_layered_sliding_window(&messages, 0, &ret);

    let call = out.iter().find(|m| m.id == "a1").unwrap();
    let call_text = param_of(call);
    assert!(
        call_text.contains("(path=gateway/server.rs)"),
        "参数骨架化应保留 path 锚点: {call_text}"
    );
    assert!(
        !call_text.contains("limit"),
        "锚点之外的参数不应保留: {call_text}"
    );

    let result = out
        .iter()
        .find(|m| m.parent_id.as_deref() == Some("a1"))
        .unwrap();
    let result_text = param_of(result);
    assert!(
        result_text.contains("(path=gateway/server.rs)"),
        "结果摘要应前缀配对调用的锚点: {result_text}"
    );
    assert!(
        result_text.contains("[行 585-602"),
        "首行摘要仍应保留: {result_text}"
    );
}

/// 无定位参数的调用（如 todo_write 的 todos）退回通用占位符，不强行编造锚点
#[test]
fn skeletonized_without_anchor_falls_back_to_generic() {
    let messages = vec![
        tc_msg("w1", "local/todo_write", r#"{"todos":"清单 v1 很长很长"}"#),
        tool_result("w1", "已更新"),
    ];
    let ret = HashMap::new();
    let out = apply_layered_sliding_window(&messages, 0, &ret);
    let call = out.iter().find(|m| m.id == "w1").unwrap();
    assert_eq!(
        param_of(call),
        "[System Info: Tool call input parameters skeletonized to save context.]",
        "无锚点参数应退回通用占位符"
    );
}

/// P2-1：单行 JSON 结果骨架化时应产出语义摘要（count + 条目名列表），
/// 而非 96 字符裸切片（真实会话实证：`{"count":16,"entries":[{"modified":…`
/// 对模型毫无信息量，迫使重跑工具）。
#[test]
fn json_result_gets_semantic_digest() {
    let json_body = format!(
        r#"{{"count":16,"entries":[{{"name":"main.rs","path":"src/main.rs","modified":1788948661}}],"next":[{}]}}"#,
        vec![r#""x""#; 400].join(",")
    );
    let messages = vec![
        tc_msg("j1", "local/glob", r#"{"pattern":"**/*.rs"}"#),
        tool_result("j1", &json_body),
    ];
    let ret = HashMap::new();
    let out = apply_layered_sliding_window(&messages, 0, &ret);
    let result = out
        .iter()
        .find(|m| m.parent_id.as_deref() == Some("j1"))
        .unwrap();
    let text = param_of(result);
    assert!(text.contains("count=16"), "应提取规模字段: {text}");
    assert!(
        text.contains("first=main.rs"),
        "单条目应给出定位名（first=）: {text}"
    );
    assert!(!text.contains(r#""next""#), "不应残留大体积切片: {text}");
}

/// P2-1 升级：列表类输出的摘要要给出**多个**条目名（只给首条目无法一次建模，
/// 模型只能逐轮重跑工具——这是真实 agent 会话报告的头号摩擦）。
#[test]
fn json_list_digest_lists_multiple_entry_names() {
    let entries: Vec<String> = (0..23)
        .map(|i| format!(r#"{{"name":"entry{i}.md","type":"file"}}"#))
        .collect();
    let body = format!(r#"{{"count":23,"entries":[{}]}}"#, entries.join(","));
    let messages = vec![
        tc_msg("l1", "vdfs_list", r#"{"path":"."}"#),
        tool_result("l1", &body),
    ];
    let ret = HashMap::new();
    let out = apply_layered_sliding_window(&messages, 0, &ret);
    let text = param_of(
        out.iter()
            .find(|m| m.parent_id.as_deref() == Some("l1"))
            .unwrap(),
    );
    assert!(text.contains("count=23"), "规模感不应丢: {text}");
    assert!(
        text.contains("entry0.md") && text.contains("entry7.md"),
        "应列出前若干条目名: {text}"
    );
    assert!(
        text.contains("+15"),
        "应标注未列出的余量（23 - 8 = 15）: {text}"
    );
    assert!(!text.contains(r#""type""#), "条目字段不应整包保留: {text}");

    // 预算不变式：names 列表必须闭合（收尾 `]` 不能被摘要预算截掉，
    // 否则余量标注与括号一起消失，摘要变成噪声——实测踩过）
    let summary = text
        .split("Summary: ")
        .nth(1)
        .expect("摘要应存在")
        .split(". Re-run")
        .next()
        .unwrap();
    assert!(
        summary.ends_with(']'),
        "摘要不应被预算截断在 names 列表中间: {summary}"
    );
}

/// 取回指引：结果占位符必须告诉模型"重跑哪个工具能拿回全文"——
/// 核对成本高到模型选择"用自信语气包装未验证结论"时，这条指引是最小成本的解。
#[test]
fn skeletonized_result_carries_retry_hint() {
    let ret = HashMap::new();
    // 成功结果
    let messages = vec![
        tc_msg("h1", "vdfs_list", r#"{"path":"."}"#),
        tool_result("h1", r#"{"count":2,"entries":[{"name":"a"},{"name":"b"}]}"#),
    ];
    let out = apply_layered_sliding_window(&messages, 0, &ret);
    let text = param_of(
        out.iter()
            .find(|m| m.parent_id.as_deref() == Some("h1"))
            .unwrap(),
    );
    assert!(
        text.contains("Re-run vdfs_list to get the full output."),
        "应给出重跑指引: {text}"
    );

    // 失败结果同样带指引（错误摘要与取回指引并存）
    let mut failed = tool_result("h1", "boom");
    failed.meta = Some(serde_json::json!({ "success": false, "failure_kind": "io" }));
    let messages = vec![tc_msg("h1", "vdfs_list", r#"{"path":"."}"#), failed];
    let out = apply_layered_sliding_window(&messages, 0, &ret);
    let text = param_of(
        out.iter()
            .find(|m| m.parent_id.as_deref() == Some("h1"))
            .unwrap(),
    );
    assert!(text.contains("failed"), "失败仍应标注: {text}");
    assert!(
        text.contains("Re-run vdfs_list"),
        "失败结果也应给出重跑指引: {text}"
    );
}

/// 无配对调用时结果保持原样（取回指引只在能指认工具名时才给出，不编造）
#[test]
fn result_without_parent_call_is_left_untouched() {
    let messages = vec![
        tc_msg("n1", "local/shell", r#"{"command":"dir"}"#),
        ChatMessage {
            id: "orphan".to_string(),
            parent_id: Some("n1".to_string()),
            role: Some(MessageRole::Tool),
            msg_type: Some(MessageType::Text),
            content: Some(MessageContent::Text("body".to_string())),
            status: Some(MessageStatus::Completed),
            ..Default::default()
        },
    ];
    let ret = HashMap::new();
    let out = apply_layered_sliding_window(&messages, 0, &ret);
    let text = param_of(out.iter().find(|m| m.id == "orphan").unwrap());
    assert!(
        text.contains("Re-run local/shell"),
        "有配对调用时应带上工具名: {text}"
    );

    // 孤儿结果（parent 指向不存在的节点）→ 不骨架化，故无指引可言
    let orphan = ChatMessage {
        id: "orphan2".to_string(),
        parent_id: Some("missing".to_string()),
        role: Some(MessageRole::Tool),
        msg_type: Some(MessageType::Text),
        content: Some(MessageContent::Text("body2".to_string())),
        status: Some(MessageStatus::Completed),
        ..Default::default()
    };
    let out = apply_layered_sliding_window(&[orphan], 0, &ret);
    assert_eq!(param_of(&out[0]), "body2", "孤立结果不应被改写");
}

/// pretty-print 的多行 JSON 也要走语义摘要：只解析首行（`{`）必然失败，
/// 摘要会退化成 `"count": 16,` 这类中间行切片。
#[test]
fn pretty_printed_json_gets_semantic_digest() {
    let body = r#"{
  "count": 3,
  "entries": [
    { "name": "a.rs" },
    { "name": "b.rs" },
    { "name": "c.rs" }
  ]
}"#;
    let messages = vec![
        tc_msg("p9", "vdfs_list", r#"{"path":"."}"#),
        tool_result("p9", body),
    ];
    let ret = HashMap::new();
    let out = apply_layered_sliding_window(&messages, 0, &ret);
    let text = param_of(
        out.iter()
            .find(|m| m.parent_id.as_deref() == Some("p9"))
            .unwrap(),
    );
    assert!(text.contains("count=3"), "应解析整体 JSON 取规模: {text}");
    assert!(
        text.contains("names=[a.rs,b.rs,c.rs]"),
        "应列出条目名: {text}"
    );
}

/// P2-1：无规模/条目名可比的对象 → 顶层键名兜底；纯数组取长度 + 元素名；
/// 非 JSON / 解析失败 → 原有首行切片行为不变（回归保护）。
#[test]
fn json_digest_falls_back_to_structure() {
    // 顶层键名兜底
    let keys_only = r#"{"alpha":1,"beta":2,"gamma":3,"delta":4}"#;
    let d1 = success_digest(keys_only);
    assert!(d1.contains("keys=alpha,beta"), "对象应兜底键名列表: {d1}");
    // 字符串数组：长度 + 元素名（旧实现只报 `items=string/string/string`，
    // 既不给"有几条"也不给"是什么"，实测对建模无用）
    let d2 = success_digest(r#"["a","b","c"]"#);
    assert_eq!(d2, "count=3 names=[a,b,c]", "数组应取长度与元素名: {d2}");
    // 数值数组：无条目名可比，只报长度
    let d3 = success_digest("[1,2,3]");
    assert_eq!(d3, "count=3", "数值数组只报长度: {d3}");
    // 单条目：退化为 first=<name>（少一层方括号歧义）
    let d4 = success_digest(r#"{"count":1,"entries":[{"name":"only.rs"}]}"#);
    assert_eq!(d4, "count=1 first=only.rs", "单条目应退化为 first=: {d4}");
    // 非 JSON / 解析失败 → 原有首行切片行为不变（回归保护）
    let plain = "just a plain line of output";
    assert_eq!(success_digest(plain), plain);
}
