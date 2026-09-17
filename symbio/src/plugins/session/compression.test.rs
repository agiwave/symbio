//! `compression` 模块的单元测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）：
//! `compression.rs` 只保留生产代码，测试全部放本文件。

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

#[test]
fn test_extract_snapshot() {
    let out = "thinking... <state_snapshot>\n  <overall_goal>done</overall_goal>\n</state_snapshot> trailing";
    let s = extract_snapshot(out).unwrap();
    assert!(s.starts_with("<state_snapshot>"));
    assert!(s.contains("done"));
    assert!(extract_snapshot("no xml here").is_none());
    assert!(extract_snapshot("<state_snapshot></state_snapshot>").is_none());
}

#[test]
fn test_extract_snapshot_rejects_unvalidated_prose() {
    // 真实上下文的降级形态：过程叙述 + 重复结论，没有闭合的快照块。
    // 调用方在重试后仍提取失败时必须回滚，不能再包裹原始散文。
    let prose = "Let me analyze the conversation history carefully.\n\nCompleted items: checks passed.\nNow structure the snapshot.\nCompleted items: checks passed.";
    assert!(extract_snapshot(prose).is_none());
    assert!(extract_snapshot("<state_snapshot>truncated").is_none());
    assert!(extract_snapshot("  \n ").is_none());
}

#[test]
fn test_snapshot_excludes_surrounding_prose() {
    let output = "process notes\n<state_snapshot><overall_goal>continue</overall_goal></state_snapshot>\nrepeated notes";
    let snapshot = extract_snapshot(output).unwrap();
    assert!(!snapshot.contains("notes"));
    assert!(snapshot.contains("continue"));
}

/// 模板回归锚点：Signal-to-noise 规则必须包含（1）旧快照对账规则——
/// 防错误知识跨快照遗传（真实事故：一条错误论断存活三个快照周期，与
/// 纠错证据并存于同一快照的两个分节）；（2）key_knowledge 卫生与
/// completed_items 瘦身规则——防过程性知识与易变数字常驻快照。
/// 注：模板文本变化会改变 compression_prompt_fingerprint，属预期
/// （指纹仅写入快照 meta 作溯源，无硬编码依赖）。
#[test]
fn test_compression_prompt_has_reconciliation_and_hygiene_rules() {
    let prompt = get_compression_prompt();
    assert!(
        prompt.contains("unverified INPUT, not ground truth"),
        "模板缺少旧快照对账规则（防错误跨代遗传）"
    );
    assert!(
        prompt.contains("never copy old <key_knowledge> forward unchecked"),
        "模板缺少禁止盲目继承 key_knowledge 的规则"
    );
    assert!(
        prompt.contains("state invariants instead"),
        "模板缺少 key_knowledge 卫生规则（不变量替代易变数字）"
    );
    assert!(
        prompt.contains("one line per item"),
        "模板缺少 completed_items 瘦身规则"
    );
}

/// 模板回归锚点：可再生事实的**指针化**，以及内核提示词的**去项目化**。
///
/// 事故一（为什么要有指针化规则）：某轮压缩把「session store kinds =
/// file/sqlite/memory」写进 key_knowledge，而该三后端选型早已被删除、事实表
/// §4 也已更新。过期事实以权威口吻进入快照后，直接误导了下一轮判断。根因不是
/// 模型抄错，而是让摘要器复述可再生的仓库事实——摘要器只拿到 history JSON、
/// 没有工具，规则里"旧快照是未验证的输入"它根本无力执行。
///
/// 事故二（为什么规则里不许出现项目专名）：第一版规则直接写了
/// `docs/CURRENT.md` 与 "plugins / store backends"。本文件是**通用**压缩器，
/// 随 agent 分发到任意仓库；把某个仓库的目录约定编进内核，等于让内核依赖
/// 它不该知道的东西，对其他项目既无效又是腐化点。项目特定知识归项目侧
/// （persona / 事实表自身的说明），内核只保留可迁移的原则。
///
/// 故三条断言缺一不可：
/// 1. 指针化原则在场（区分可再生 vs 仅对话可知）；
/// 2. 反向护栏在场（仅对话可知者必须完整保留，防"少写点"式误读）；
/// 3. 项目专名缺席——用不含任何路径的通用措辞表达前两条。
#[test]
fn test_compression_prompt_delegates_regenerable_facts_generically() {
    let prompt = get_compression_prompt();
    assert!(
        prompt.contains("REGENERABLE"),
        "模板缺少「可再生事实指针化」原则"
    );
    assert!(
        prompt.contains("MUST be kept in full"),
        "模板缺少反向护栏：仅对话可知的事实必须完整保留"
    );
    // 护栏必须限定指针化的适用边界，否则会被误读成整体减少信息量。
    assert!(
        prompt.contains("never a licence to lose judgement"),
        "模板未声明指针化不等于丢弃判断依据"
    );
    // 内核不得携带任何具体仓库的路径或模块约定
    for forbidden in ["CURRENT.md", "docs/", "symbio/"] {
        assert!(
            !prompt.contains(forbidden),
            "通用压缩提示词混入项目专有知识：{forbidden}"
        );
    }
}

/// 诉求3：压缩模板只走 system role，user 消息只携带待压缩数据
/// （prepare_compression / build_compression_request 均不得内嵌模板）。
#[test]
fn test_compression_request_user_message_carries_data_only() {
    let msgs = vec![
        user_msg(&"x".repeat(1000)),
        assistant_msg(&"y".repeat(1000)),
        user_msg(&"z".repeat(1000)),
    ];
    let (msg, _, _) = prepare_compression(&msgs).unwrap();
    let text = msg
        .content
        .as_ref()
        .map(|c| c.to_text())
        .unwrap_or_default();
    assert!(text.contains("Chat History to Summarize"));
    assert!(
        !text.contains("<state_snapshot>"),
        "压缩模板不得随 user 消息下发"
    );

    let req = build_compression_request(&msgs, Some("keep file paths"));
    let req_text = req
        .content
        .as_ref()
        .map(|c| c.to_text())
        .unwrap_or_default();
    assert!(req_text.contains("keep file paths"));
    assert!(!req_text.contains("<state_snapshot>"));
}

/// 诉求3：落库快照渲染后不残留 XML 标签——
/// 历史中的 assistant 消息不再示范 state_snapshot/key_knowledge 结构。
#[test]
fn test_render_snapshot_for_history_strips_xml_tags() {
    let xml = extract_snapshot(
        "<state_snapshot>\n<key_knowledge>\n- A\n- B\n</key_knowledge>\n</state_snapshot>",
    )
    .unwrap();
    let rendered = render_snapshot_for_history(&xml);
    assert!(!rendered.contains("<state_snapshot>"));
    assert!(!rendered.contains("<key_knowledge>"));
    assert!(rendered.contains("【关键知识】"));
    assert!(rendered.contains("- A"));
}

#[test]
fn test_prepare_compression_rejects_when_no_split() {
    // 单条 User 消息（巨型单轮场景）：无切分点 ⇒ prepare_compression 返回 None
    let msgs = vec![user_msg(&"x".repeat(100_000))];
    assert!(prepare_compression(&msgs).is_none());
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
    entries: &[(&str, crate::symbio_core::ToolContextRetention)],
) -> std::collections::HashMap<String, crate::symbio_core::ToolContextRetention> {
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
    let view = build_request_view(&msgs, 15, &retention, false, 12, 3, 200, false);

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

    let view = build_request_view(&msgs, 15, &retention, false, 12, 3, 200, true);
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
    let plain = build_request_view(&msgs, 15, &retention, false, 12, 3, 200, false);
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
    let view = build_request_view(&msgs, 0, &retention, true, 2, 3, 200, false);

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
fn test_render_snapshot_for_history_prepends_timestamp() {
    // 快照是"截至某时刻"的状态切片：【进行中】/【待确认】会随后续轮次自然
    // 过期。头部必须自带时点声明，否则读者无法区分"快照说未完成"与
    // "实际早已完成"（真实事故：快照称"u6/u7 编译阻塞"，实际早已全绿）。
    let xml = extract_snapshot(
        "<state_snapshot>\n<in_progress_items>\n- 旧任务\n</in_progress_items>\n</state_snapshot>",
    )
    .unwrap();
    let rendered = render_snapshot_for_history(&xml);
    assert!(rendered.starts_with("[快照时点："));
    assert!(rendered.contains("以最新消息为准]"));
    assert!(rendered.contains("【进行中】"));
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
        crate::symbio_core::ToolContextRetention::LastOnly,
    )]);
    let view = build_request_view(&msgs, 15, &retention, false, 12, 3, 200, false);

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
    let view = build_request_view(&msgs, 1, &retention, false, 12, 3, 200, false);

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
        crate::symbio_core::ToolContextRetention::LastOnly,
    )]);
    let view = build_request_view(&msgs, 0, &retention, false, 12, 3, 200, false);
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
        crate::symbio_core::ToolContextRetention::LastOnly,
    )]);
    let view = build_request_view(&msgs, 15, &retention, false, 12, 3, 200, true);

    assert_eq!(view.len(), msgs.len() + 1);
    // 末尾是 nudge；倒数第二条仍是保留原文的最新工具结果
    assert!(view_text(view.last().unwrap()).contains("system note"));
    assert_eq!(view_text(&view[view.len() - 2]), "new content");
}

// ── 水位提醒 / 本地兜底压缩（chat_loop 直接调用，审计 §4.2 补测）──────────

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

#[test]
fn test_emergency_tail_compression_keeps_turn_boundary() {
    // 每条 ~108 token（100 中文字 + 8 开销），target=60 只容得下最后一条；
    // 反向累计后起点回退到最近一条 user（轮边界，不切断 tool_call 配对）
    let msgs = vec![
        cn_msg(&"早".repeat(100)),
        cn_msg(&"期".repeat(100)),
        cn_msg(&"近".repeat(100)),
    ];
    let (out, removed) = emergency_tail_compression(&msgs, 60, None);
    assert_eq!(removed, 2);
    assert_eq!(out.len(), 2); // 截断说明 + 保留的最近一轮
    let head = out[0].content.as_ref().unwrap().to_text();
    assert!(head.contains("CONTEXT TRUNCATED"));
    assert!(!head.contains("完整历史转存"), "无转存路径时不输出该段");
    assert_eq!(
        out[0]
            .meta
            .as_ref()
            .and_then(|m| m.get("compaction"))
            .and_then(|v| v.as_str()),
        Some("emergency_tail")
    );
    assert_eq!(
        out[0]
            .meta
            .as_ref()
            .and_then(|m| m.get("removed_messages"))
            .and_then(|v| v.as_u64()),
        Some(removed as u64)
    );
    assert_eq!(out[1].content.as_ref().unwrap().to_text(), "近".repeat(100));
}

#[test]
fn test_emergency_tail_compression_noop_when_within_target() {
    let msgs = vec![cn_msg("短"), cn_msg("也很短")];
    let (out, removed) = emergency_tail_compression(&msgs, 100_000, Some("/tmp/t.json"));
    assert_eq!(removed, 0);
    assert_eq!(out.len(), msgs.len());
}

#[test]
fn test_emergency_tail_compression_carries_transcript_hint() {
    let msgs = vec![
        cn_msg(&"早".repeat(100)),
        cn_msg(&"期".repeat(100)),
        cn_msg(&"近".repeat(100)),
    ];
    let (out, removed) = emergency_tail_compression(&msgs, 60, Some("/abs/transcript.json"));
    assert_eq!(removed, 2);
    let head = out[0].content.as_ref().unwrap().to_text();
    assert!(head.contains("/abs/transcript.json"));
    assert_eq!(out.len(), msgs.len() - removed + 1);
}

/// 设计约束固化：末条消息无条件保留；若回退轮边界后起点落到数组末尾
/// （唯一可能是末条非 user 且自身超出 target），整次兜底放弃（removed=0）。
/// 这是"宁可发送也不丢当前指令"的保守策略，必须有测试钉住，否则改动时
/// 容易被无声破坏。
#[test]
fn test_emergency_tail_compression_gives_up_when_no_turn_boundary() {
    // 末条是 assistant（无后续 user 轮）：反向累计保留它后，起点回退时
    // 越过它直达唯一 user（index 0）→ 回扫落到数组末尾 → 放弃截断
    let msgs = vec![
        cn_msg("指令"),
        ChatMessage {
            role: Some(MessageRole::Assistant),
            content: Some(MessageContent::Text("答".repeat(500))),
            ..Default::default()
        },
    ];
    let (out, removed) = emergency_tail_compression(&msgs, 10, None);
    assert_eq!(removed, 0);
    assert_eq!(out.len(), 2);
}
