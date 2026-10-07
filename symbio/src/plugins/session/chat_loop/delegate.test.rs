//! 三项真源的回归测试——证明它**会红**。
//!
//! 两处负向断言是本文件的重点，各自钉住一条**被否决的方案**：
//! - `length_zero_is_off`：阈值 0 若判成「任何长度都命中」，每条寒暄都会开后台会话
//!   （ADR-047 被否决方案第一条）；
//! - `empty_prefix_never_matches`：空前缀若不跳过，`starts_with("")` 恒真 ⇒ 一切
//!   消息都命中，判定权等于没有。

use super::*;
use crate::symbio_core::CapabilityCategory;

/// 出厂参数：前缀 `/work `、关键词空集、阈值 0。
fn factory(text: &str) -> Option<&'static str> {
    worker_start_reason(text, FORCE_PREFIX, KEYWORDS, MIN_CHARS)
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
    assert_eq!(delegate_section(None, &[]), None);
}

#[test]
fn section_carries_the_hit_reason_verbatim() {
    let section = delegate_section(Some("/work 帮我重构"), &[]).unwrap();
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
    let section = delegate_section(Some("今天天气不错"), &[]).unwrap();
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
    let section = delegate_section(
        None,
        &[meta("file_read", Some(CapabilityCategory::FileOperation))],
    )
    .unwrap();
    assert!(!section.contains("委派判定"), "实际：{section}");
    assert!(section.contains("【能力目录】"), "实际：{section}");
}

#[test]
fn section_puts_verdict_before_digest() {
    // 判定在前、目录在后：模型先看到「本轮动不动手」，再看到「我有哪些能力」。
    let section = delegate_section(
        Some("/work 重构"),
        &[meta("file_read", Some(CapabilityCategory::FileOperation))],
    )
    .unwrap();
    let verdict = section.find("委派判定").unwrap();
    let digest = section.find("【能力目录】").unwrap();
    assert!(verdict < digest, "次序反了：{section}");
}

#[test]
fn section_survives_empty_capability_list_when_verdict_exists() {
    // 能力空 ⇒ 目录不出现，但判定仍在（两条真源各自独立决定自己在不在）。
    let section = delegate_section(Some("/work 重构"), &[]).unwrap();
    assert!(section.contains("reason=explicit_prefix"));
    assert!(!section.contains("【能力目录】"), "实际：{section}");
}
