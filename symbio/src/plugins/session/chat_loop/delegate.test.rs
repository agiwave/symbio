//! 三项真源的回归测试——证明它**会红**。
//!
//! 两处负向断言是本文件的重点，各自钉住一条**被否决的方案**：
//! - `length_zero_is_off`：阈值 0 若判成「任何长度都命中」，每条寒暄都会开后台会话
//!   （ADR-047 被否决方案第一条）；
//! - `empty_prefix_never_matches`：空前缀若不跳过，`starts_with("")` 恒真 ⇒ 一切
//!   消息都命中，判定权等于没有。

use super::*;
// 出厂值的真源在配置面（`delegate.rs` 不持第二份字面量）——判定侧测试显式 import。
use crate::plugins::session::config::{FORCE_PREFIX, KEYWORDS, MIN_CHARS};
use crate::symbio_core::CapabilityCategory;

/// 出厂参数：前缀 `/work `、关键词空集、阈值 0。
fn factory(text: &str) -> Option<&'static str> {
    worker_start_reason(text, FORCE_PREFIX, KEYWORDS, MIN_CHARS)
}

/// 出厂**配置**下的委派段：与生产同一个 `delegate_section_with`，只把默认配置递进去
/// （判定侧不持第二份默认，故「出厂值」与「测试所测」同路）。
fn factory_section(
    utterance: Option<&str>,
    tools: &[CapabilityMeta],
    workers: &[WorkerProgress],
) -> Option<String> {
    delegate_section_with(utterance, tools, workers, &SessionConfig::default())
}

// ── Q1 · 三条旋钮（判据可配：值取自配置面，出厂值只是默认）────────────────
mod knobs {
    use super::*;
    use crate::plugins::session::config::SessionConfig;

    /// 改前缀 ⇒ 判定跟着改，且**出厂前缀此时不再命中**（配置是替换，不是叠加）。
    #[test]
    fn prefix_is_configurable_and_replaces_the_default() {
        let cfg = SessionConfig {
            worker_force_prefix: "#run ".to_string(),
            ..SessionConfig::default()
        };
        let hit = delegate_section_with(Some("#run 重构"), &[], &[], &cfg).unwrap();
        assert!(hit.contains("reason=explicit_prefix"), "{hit}");

        let replaced = delegate_section_with(Some("/work 重构"), &[], &[], &cfg).unwrap();
        assert!(replaced.contains("未命中任何委派判据"), "{replaced}");
    }

    /// 关键词与长度阈值同理：出厂是「空集 / 0」两层关闭，配了才开。
    #[test]
    fn keywords_and_min_chars_are_configurable() {
        let kw = SessionConfig {
            worker_keywords: "重构".to_string(),
            ..SessionConfig::default()
        };
        let hit = delegate_section_with(Some("帮我把这个重构掉"), &[], &[], &kw).unwrap();
        assert!(hit.contains("reason=keyword_hit"), "{hit}");

        let len = SessionConfig {
            worker_min_chars: 4,
            ..SessionConfig::default()
        };
        let long = delegate_section_with(Some("这句话足够长了吧"), &[], &[], &len).unwrap();
        assert!(long.contains("reason=topic_length"), "{long}");

        // 出厂值这两层是**关闭**的：同一条消息在默认配置下不命中（否则每条寒暄
        // 都开一个后台会话，ADR-047 被否决方案第一条）。
        let off = factory_section(Some("帮我把这个重构掉"), &[], &[]).unwrap();
        assert!(off.contains("未命中任何委派判据"), "{off}");
    }
}

// ── Q1 · 三层判据 ──────────────────────────────────────────────────────

#[test]
fn explicit_prefix_hits_and_returns_reason() {
    assert_eq!(factory("/work 帮我重构整个模块"), Some(REASON_PREFIX));
}

#[test]
fn prefix_keeps_its_trailing_space() {
    // 带尾空格是判据的一部分：`/worker`、`/workflow` 是词，不是指令。
    assert_eq!(factory("/worker 是什么"), None);
    assert_eq!(factory("/workflow 怎么写"), None);
    assert_eq!(factory("/work"), None, "没有尾空格不算前缀命中");
}

#[test]
fn empty_prefix_never_matches() {
    // `starts_with("")` 恒真——空前缀必须被跳过，否则一切消息都「该开 worker」。
    assert_eq!(worker_start_reason("随便聊聊", "", &["改造"], 0), None);
    assert_eq!(
        worker_start_reason("随便聊聊", "", &[], 0),
        None,
        "空白配置不该退化成「永远命中」"
    );
}

#[test]
fn keyword_hits_case_insensitively_as_substring() {
    let kw = ["重构", "REFactor"];
    assert_eq!(
        worker_start_reason("帮我重构一下这段", FORCE_PREFIX, &kw, 0),
        Some(REASON_KEYWORD)
    );
    // 子串而非整句相等：大小写不同也要命中。
    assert_eq!(
        worker_start_reason("please refactor this", FORCE_PREFIX, &kw, 0),
        Some(REASON_KEYWORD)
    );
}

#[test]
fn empty_keyword_set_is_off() {
    // 出厂空集 ⇒ 这一层关闭：普通消息不因为「有关键词层」而命中。
    assert_eq!(factory("帮我看看天气"), None);
    // 空串 / 纯空白关键词不许命中任何消息。
    assert_eq!(
        worker_start_reason("任意一句话", FORCE_PREFIX, &["", "  "], 0),
        None
    );
}

#[test]
fn length_zero_is_off() {
    // 阈值 0 必须是「关闭」：若判成 >= 0，任何非空消息都命中。
    assert_eq!(worker_start_reason("嗯", FORCE_PREFIX, &[], 0), None);
    assert_eq!(
        worker_start_reason("这是一段相当长的发言", FORCE_PREFIX, &[], 0),
        None,
        "出厂阈值 0 不该让长消息命中"
    );
}

#[test]
fn length_hits_only_at_or_above_threshold() {
    let text = "一二三四五"; // 5 字
    assert_eq!(worker_start_reason(text, FORCE_PREFIX, &[], 6), None);
    assert_eq!(
        worker_start_reason(text, FORCE_PREFIX, &[], 5),
        Some(REASON_LENGTH)
    );
    // 按**字符**计，不是字节：中文一个字 3 字节，按字节算会提前命中。
    assert_eq!(
        worker_start_reason(text, FORCE_PREFIX, &[], 4),
        Some(REASON_LENGTH)
    );
}

#[test]
fn empty_or_blank_input_is_none() {
    assert_eq!(factory(""), None);
    assert_eq!(factory("   \n\t "), None);
    assert_eq!(factory("///"), None, "纯标点不是「要干活」");
}

#[test]
fn prefix_short_circuits_before_keyword() {
    // 两层同时可命中 ⇒ 回**先判的那层**的理由码（短路顺序是判据的一部分）。
    let hit = worker_start_reason("/work 重构", FORCE_PREFIX, &["重构"], 0);
    assert_eq!(hit, Some(REASON_PREFIX));
}

#[test]
fn no_criteria_hit_returns_none() {
    assert_eq!(
        worker_start_reason("今天天气不错", FORCE_PREFIX, &["改造"], 0),
        None
    );
}

// ── Q2 · 能力目录 ──────────────────────────────────────────────────────

fn meta(name: &str, category: Option<CapabilityCategory>) -> CapabilityMeta {
    CapabilityMeta {
        name: name.to_string(),
        description: String::new(),
        input_schema: serde_json::json!({ "type": "object" }),
        keywords: vec![],
        category,
        examples: None,
        context_retention: None,
        ..Default::default()
    }
}

#[test]
fn empty_tools_is_none() {
    // 空目录不产出段落（与就绪段「候选集空 ⇒ 整段不出现」同一条判据）。
    assert_eq!(capability_digest(&[]), None);
}

#[test]
fn groups_by_category_and_sorts_both_levels() {
    let tools = [
        meta("web_search", Some(CapabilityCategory::Network)),
        meta("file_write", Some(CapabilityCategory::FileOperation)),
        meta("file_read", Some(CapabilityCategory::FileOperation)),
    ];
    let digest = capability_digest(&tools).unwrap();
    let lines: Vec<&str> = digest.lines().collect();

    assert_eq!(lines[0], "【能力目录】按分类分组（只列名字，不列参数）");
    // BTreeMap ⇒ 分类段按字典序（FileOperation < Network）。
    assert_eq!(lines[1], "- FileOperation: file_read, file_write");
    assert_eq!(lines[2], "- Network: web_search");
}

#[test]
fn missing_category_falls_back_to_other() {
    // `category: None` 的兜底是 `Other`，不是「不进目录」——漏掉的能力等于不可见。
    let digest = capability_digest(&[meta("mystery", None)]).unwrap();
    assert!(digest.contains("- Other: mystery"), "实际：{digest}");
}

#[test]
fn short_name_strips_the_prefix() {
    // 与 `prepare_turn_inputs` 解析 `context_retention` 用同一条 `rsplit('/')` 口径。
    let digest = capability_digest(&[meta(
        "mcp/github/create_issue",
        Some(CapabilityCategory::Mcp),
    )])
    .unwrap();
    assert!(digest.contains("- Mcp: create_issue"), "实际：{digest}");
    assert!(
        !digest.contains("mcp/github"),
        "目录里不该留全路径：{digest}"
    );
}

#[test]
fn duplicate_short_names_collapse() {
    let digest = capability_digest(&[
        meta("a/read", Some(CapabilityCategory::FileOperation)),
        meta("b/read", Some(CapabilityCategory::FileOperation)),
    ])
    .unwrap();
    assert!(digest.contains("- FileOperation: read"), "实际：{digest}");
}

#[test]
fn digest_is_order_independent() {
    // `composite::traverse` 遍历的是 `HashMap`，**顺序每次不同**——所以「同一份能力
    // 集合」必须与它被遇上的先后无关。用两份**顺序相反**的清单钉这条：不排序的实现
    // 在这里必然产出两段不同的文本（分组内与分组间都会跟着入参走）。
    let a = [
        meta("zeta", Some(CapabilityCategory::Skill)),
        meta("alpha", Some(CapabilityCategory::Skill)),
        meta("beta", Some(CapabilityCategory::Core)),
    ];
    let b = [
        meta("beta", Some(CapabilityCategory::Core)),
        meta("alpha", Some(CapabilityCategory::Skill)),
        meta("zeta", Some(CapabilityCategory::Skill)),
    ];
    let first = capability_digest(&a).unwrap();
    for _ in 0..8 {
        assert_eq!(
            capability_digest(&b).unwrap(),
            first,
            "入参顺序换了，段落就变了"
        );
        assert_eq!(capability_digest(&a).unwrap(), first);
    }
}

// ── 合成段落（`prepare_turn_inputs` 的装配入口） ────────────────────────

#[test]
fn section_is_none_when_there_is_nothing_to_say() {
    // 无发言（resume / 心跳）且无能力 ⇒ 一个字节都不多。
    assert_eq!(factory_section(None, &[], &[]), None);
}

#[test]
fn section_carries_the_hit_reason_verbatim() {
    let section = factory_section(Some("/work 帮我重构"), &[], &[]).unwrap();
    assert!(
        section.starts_with("- 委派判定：本轮应启动 worker（reason=explicit_prefix）"),
        "实际：{section}"
    );
    // 理由码必须逐字可见：它是审计面，改一个字符就与观测口径脱钩。
    assert!(section.contains(REASON_PREFIX));
}

#[test]
fn section_records_a_miss_too() {
    // 「为什么这轮没动手」是 Q1 存在的理由——未命中也要落事实，否则分不清
    // 「判了不动」与「压根没判」。
    let section = factory_section(Some("今天天气不错"), &[], &[]).unwrap();
    assert!(
        section.contains("- 委派判定：未启动 worker（未命中任何委派判据）"),
        "实际：{section}"
    );
    assert!(
        !section.contains("reason="),
        "未命中不该凭空造理由码：{section}"
    );
}

#[test]
fn section_without_utterance_omits_the_verdict_line() {
    // resume / 心跳：没有用户新发言，判决无从谈起 ⇒ 只剩能力目录。
    let section = factory_section(
        None,
        &[meta("file_read", Some(CapabilityCategory::FileOperation))],
        &[],
    )
    .unwrap();
    assert!(!section.contains("委派判定"), "实际：{section}");
    assert!(section.contains("【能力目录】"), "实际：{section}");
}

#[test]
fn section_puts_verdict_before_digest() {
    // 判定在前、目录在后：模型先看到「本轮动不动手」，再看到「我有哪些能力」。
    let section = factory_section(
        Some("/work 重构"),
        &[meta("file_read", Some(CapabilityCategory::FileOperation))],
        &[],
    )
    .unwrap();
    let verdict = section.find("委派判定").unwrap();
    let digest = section.find("【能力目录】").unwrap();
    assert!(verdict < digest, "次序反了：{section}");
}

#[test]
fn section_survives_empty_capability_list_when_verdict_exists() {
    // 能力空 ⇒ 目录不出现，但判定仍在（两条真源各自独立决定自己在不在）。
    let section = factory_section(Some("/work 重构"), &[], &[]).unwrap();
    assert!(section.contains("reason=explicit_prefix"));
    assert!(!section.contains("【能力目录】"), "实际：{section}");
}

// ── Q3 · worker 进展（三个字段与 B2 逐字同名） ─────────────────────────

/// 夹具：一条 worker 消息。
fn wmsg(
    id: &str,
    role: cm::MessageRole,
    status: Option<cm::MessageStatus>,
    text: &str,
) -> cm::ChatMessage {
    cm::ChatMessage {
        id: id.to_string(),
        role: Some(role),
        msg_type: Some(cm::MessageType::Text),
        status,
        content: Some(cm::MessageContent::Text(text.to_string())),
        ..Default::default()
    }
}

/// 夹具：子会话清单条目。`message_count` **故意与真实 Turn 数对不上**——它是总消息数，
/// 投影若拿它顶 `rounds`，本文件的断言立刻红。
fn sub(id: &str, title: &str, message_count: usize) -> SessionSummary {
    SessionSummary {
        id: id.to_string(),
        title: title.to_string(),
        created_at: 0,
        updated_at: 0,
        metadata: serde_json::json!({ "parent_session_id": "parent-1" }),
        message_count,
        summary: None,
        meta_tags: Vec::new(),
    }
}

/// 夹具：worker 的完整转写。
fn wsession(messages: Vec<cm::ChatMessage>) -> Session {
    Session {
        id: "worker-1".to_string(),
        messages,
        created_at: 0,
        updated_at: 0,
        metadata: serde_json::json!({ "parent_session_id": "parent-1" }),
    }
}

#[test]
fn rounds_counts_user_messages_not_the_summary_count() {
    // 判据 2「数值同源」的落点：前端数过程段 Turn 条数，后端数 user 消息——同一个量。
    let s = wsession(vec![
        wmsg(
            "1",
            cm::MessageRole::User,
            Some(cm::MessageStatus::Completed),
            "第一轮",
        ),
        wmsg(
            "2",
            cm::MessageRole::Assistant,
            Some(cm::MessageStatus::Completed),
            "答复一",
        ),
        wmsg(
            "3",
            cm::MessageRole::User,
            Some(cm::MessageStatus::Completed),
            "第二轮",
        ),
        wmsg(
            "4",
            cm::MessageRole::Assistant,
            Some(cm::MessageStatus::Completed),
            "答复二",
        ),
    ]);
    let p = progress_of(&sub("worker-1", "重构", 99), &s);
    assert_eq!(p.rounds, 2, "rounds 必须是 user 消息数");
    assert_ne!(
        p.rounds, 99,
        "拿 summary.message_count 当 rounds ⇒ 与 B2 不同源（不同量）"
    );
}

#[test]
fn steps_skip_empty_nodes_and_user_prompts() {
    // 判据 3：无内容节点不产出步骤；user 提示是**输入**不是进展。
    let grouping = cm::ChatMessage {
        id: "2".to_string(),
        msg_type: Some(cm::MessageType::Turn),
        ..Default::default()
    };
    let empty = cm::ChatMessage {
        id: "3".to_string(),
        role: Some(cm::MessageRole::Assistant),
        msg_type: Some(cm::MessageType::Text),
        content: Some(cm::MessageContent::Text(String::new())),
        ..Default::default()
    };
    let s = wsession(vec![
        wmsg(
            "1",
            cm::MessageRole::User,
            Some(cm::MessageStatus::Completed),
            "干活",
        ),
        grouping,
        empty,
        wmsg(
            "4",
            cm::MessageRole::Assistant,
            Some(cm::MessageStatus::Completed),
            "产出",
        ),
    ]);
    let p = progress_of(&sub("worker-1", "重构", 0), &s);
    assert_eq!(
        p.steps,
        vec!["产出".to_string()],
        "空节点或用户提示都不得产出步骤"
    );
}

#[test]
fn in_flight_is_false_only_once_a_terminal_state_is_seen() {
    let done = wsession(vec![wmsg(
        "1",
        cm::MessageRole::Assistant,
        Some(cm::MessageStatus::Completed),
        "完",
    )]);
    assert!(!progress_of(&sub("worker-1", "t", 0), &done).in_flight);

    let running = wsession(vec![wmsg(
        "1",
        cm::MessageRole::Assistant,
        Some(cm::MessageStatus::Streaming),
        "写",
    )]);
    assert!(progress_of(&sub("worker-1", "t", 0), &running).in_flight);

    // `status` 缺失 = 没观测到结束 ⇒ 不得宣称已结束。谎报"已完成"会让模型抢跑，
    // 比多报在途糟得多（同「串台比没进展更糟」的取向）。
    let no_status = wsession(vec![cm::ChatMessage {
        id: "1".to_string(),
        ..Default::default()
    }]);
    assert!(
        progress_of(&sub("worker-1", "t", 0), &no_status).in_flight,
        "status 缺失被判成终态 ⇒ 所有存量会话都显示「已结束」"
    );
}

#[test]
fn steps_keep_the_recent_capped_and_truncated() {
    // 进展关心"到哪了"，不是"都干过什么"：超上限丢最早的、单步超长截断——
    // 否则每轮请求都要背着 worker 的全部历史。
    let long = "段".repeat(STEP_MAX_CHARS + 40);
    let messages: Vec<cm::ChatMessage> = (0..STEPS_MAX + 3)
        .map(|i| {
            wmsg(
                &format!("m{i}"),
                cm::MessageRole::Assistant,
                Some(cm::MessageStatus::Completed),
                &format!("{i}:{long}"),
            )
        })
        .collect();
    let p = progress_of(&sub("worker-1", "t", 0), &wsession(messages));
    assert_eq!(p.steps.len(), STEPS_MAX, "超上限必须丢最早的");
    assert!(
        p.steps[0].starts_with("3:"),
        "丢错了端——应保留最近的（首条应为第 3 条），实际：{}",
        p.steps[0]
    );
    assert!(
        p.steps[0].chars().count() <= STEP_MAX_CHARS + 1,
        "单步未截断（含省略号至多 {} 字）",
        STEP_MAX_CHARS + 1
    );
}

#[test]
fn section_carries_worker_fields_verbatim() {
    let p = WorkerProgress {
        id: "worker-1".to_string(),
        title: "重构".to_string(),
        in_flight: true,
        rounds: 3,
        steps: vec!["读代码".to_string()],
    };
    let section = factory_section(Some("/work 继续"), &[], &[p]).unwrap();
    // 三个字段名逐字可见：与 B2 状态行同名，两处漂移时一眼对得出来。
    assert!(section.contains("rounds=3"), "{section}");
    assert!(section.contains("in_flight=true"), "{section}");
    assert!(section.contains("steps=[读代码]"), "{section}");
    assert!(section.contains("`重构`"), "{section}");
}

#[test]
fn section_puts_workers_after_verdict_and_digest() {
    // 次序：模型先看「本轮动不动手」、再看「我有哪些能力」、最后看「worker 到哪了」。
    let p = WorkerProgress {
        id: "w".to_string(),
        title: "t".to_string(),
        ..Default::default()
    };
    let section = factory_section(
        Some("/work 重构"),
        &[meta("file_read", Some(CapabilityCategory::FileOperation))],
        &[p],
    )
    .unwrap();
    let verdict = section.find("委派判定").unwrap();
    let digest = section.find("【能力目录】").unwrap();
    let worker = section.find("worker `").unwrap();
    assert!(verdict < digest && digest < worker, "次序反了：{section}");
}

#[test]
fn section_omits_workers_when_there_are_none() {
    // 没开 worker ⇒ Q3 一个字节都不占（与 Q2 空目录同一条纪律）。
    let section = factory_section(Some("/work 继续"), &[], &[]).unwrap();
    assert!(!section.contains("worker `"), "{section}");
}
