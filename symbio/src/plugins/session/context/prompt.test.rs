//! `context::prompt`（压缩提示词与 `<state_snapshot>` 快照协议）的单元测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）：
//! `prompt.rs` 只保留生产代码，测试全部放本文件。

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

/// 模板回归锚点（OpenCode 对标增量）：
/// 1. <next_step> 节在场——恢复后第一步动作必须有专属槽位（仅列条目不给
///    动作会降低恢复质量）；
/// 2. key_knowledge 的 Decision: 前缀约定——决策+被否方案+理由必须结构化
///    呈现，不能淹没在事实清单里；
/// 3. 无工具声明——摘要模型是裸 LLM 调用，"列问题前先验证"必须降级为
///    "保留区原文可答则直接作答"，否则规则不可执行；
/// 4. open_questions 区分「等待输入」与「待调查」两种恢复行为。
#[test]
fn test_compression_prompt_has_next_step_decision_and_no_tool_rules() {
    let prompt = get_compression_prompt();
    assert!(
        prompt.contains("<next_step>"),
        "模板缺少 next_step 节（恢复后第一步动作）"
    );
    assert!(
        prompt.contains("Prefix each decision with \"Decision:\""),
        "模板缺少 Decision: 前缀约定（决策需含被否方案与理由）"
    );
    assert!(
        prompt.contains("You have no tool access during compaction"),
        "模板缺少无工具声明（压缩是裸 LLM 调用，验证类规则必须可执行）"
    );
    assert!(
        prompt.contains("waiting-on-input"),
        "模板缺少 open_questions 的等待输入/待调查区分"
    );
}

/// 落库渲染必须与模板分节结构同步：新增 <next_step> 节后，渲染映射若缺失，
/// 原始 XML 标签会残留在历史消息里，诱导后续对话模仿同类标签输出。
#[test]
fn test_render_snapshot_for_history_maps_next_step() {
    let xml = extract_snapshot(
        "<state_snapshot>\n<next_step>\n- 先跑测试\n</next_step>\n</state_snapshot>",
    )
    .unwrap();
    let rendered = render_snapshot_for_history(&xml);
    assert!(rendered.contains("【下一步】"));
    assert!(!rendered.contains("<next_step>"));
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

/// 压缩请求**不再**把历史序列化成 JSON——模型看到的是对话，不是数据转储。
///
/// 旧写法 `serde_json::to_string(history)` 会把每条消息的 `id` / `parent_id` /
/// `seq` / `status` / `meta` 一并倒给模型：全是模型不关心却要原样付费的字符
/// （字段名、引号、转义、嵌套括号），且随历史长度线性放大。
///
/// 这条测试锁住"省掉的是什么"：一旦有人图省事改回 JSON，这里会立刻红。
#[test]
fn compression_request_carries_history_as_conversation_not_json() {
    let text_of =
        |m: &ChatMessage| -> String { m.content.as_ref().map(|c| c.to_text()).unwrap_or_default() };
    // 数据必须带 `parent_id`：否则序列化时被 `skip_serializing_if` 跳过，
    // "改回 JSON 转储"这条回归就**抓不到**（字段名根本不出现在输出里）。
    let m1 = user_msg("第一条");
    let mut m2 = assistant_msg("第二条");
    m2.parent_id = Some(m1.id.clone());
    let msgs = vec![m1, m2];

    let req = build_compression_request(&msgs, None);

    assert_eq!(
        req.len(),
        msgs.len() + 1,
        "历史逐条保留，末尾追加一条压缩指令"
    );
    assert_eq!(req[0].id, msgs[0].id, "历史消息原样透传（不重造节点）");
    assert_eq!(req[1].id, msgs[1].id);
    assert_eq!(text_of(&req[0]), "第一条", "正文原样透传，不做序列化");
    assert_eq!(text_of(&req[1]), "第二条");

    // 最直接的一条：历史若被序列化进指令，正文必然出现在指令的 content 里
    let instruction = text_of(req.last().expect("末尾为压缩指令"));
    assert!(
        !instruction.contains("第一条") && !instruction.contains("第二条"),
        "历史不得被序列化进压缩指令（否则又回到 JSON 转储）"
    );
    for m in &req {
        assert!(
            !text_of(m).contains("\"parent_id\""),
            "历史不得以 JSON 形式下发（发现存储字段名）"
        );
    }
    assert!(
        !instruction.contains("Chat History to Summarize"),
        "旧 JSON 转储的标题不应再出现"
    );
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
