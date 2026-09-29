//! 模板表的判据：抄本一致性 + 表的完整性 + 兜底方向。

use super::*;
// 走生成产线的那个码**不在模板表里**，因此它不在 `templates` 的 use 面上
// （那正是下面 `from_context_has_no_template_row` 要钉的事）。
use super::super::reasons::REASON_FROM_CONTEXT;

/// **抄本一致性**：本插件的词表与 `triage` 的逐字一致。
///
/// 这是"两份抄本"这件事的判据。漂移的表现是**用户收到一句通用兜底话**——功能还在，
/// 但没有任何错误信号，属于必须外置成断言的那类失效。
#[test]
fn the_vocabulary_matches_triage() {
    use crate::plugins::triage::reasons as t;

    assert_eq!(REASON_GREETING, t::REASON_GREETING);
    assert_eq!(REASON_THANKS, t::REASON_THANKS);
    assert_eq!(REASON_ACK, t::REASON_ACK);
    assert_eq!(REASON_EMPTY, t::REASON_EMPTY);
    assert_eq!(REASON_FROM_CONTEXT, t::REASON_FROM_CONTEXT);
    assert_eq!(REASON_CLARIFY, t::REASON_CLARIFY);
    assert_eq!(REASON_REFUSE, t::REASON_REFUSE);
    assert_eq!(REASON_NEEDS_WORK, t::REASON_NEEDS_WORK);
    assert_eq!(REASON_UNCLASSIFIED, t::REASON_UNCLASSIFIED);
}

/// 表必须覆盖**每一个**走模板产线的理由码：漏一个 ⇒ 用户收到通用兜底
/// （降级而不失效，但那是兜底，不是正常路径）。
#[test]
fn every_template_bound_reason_has_a_row() {
    use crate::plugins::triage::reasons as t;

    for code in [
        t::REASON_GREETING,
        t::REASON_THANKS,
        t::REASON_ACK,
        t::REASON_EMPTY,
        t::REASON_CLARIFY,
        t::REASON_REFUSE,
        t::REASON_NEEDS_WORK,
        t::REASON_UNCLASSIFIED,
    ] {
        assert!(lookup(code).is_some(), "理由码「{code}」没有模板行");
    }
}

/// `from_context` **不得**有模板行：它走生成产线，而 `plugin.rs` 先判生成
/// ⇒ 表里那一行永远读不到。留着它比没有更糟——读表的人会以为它生效。
#[test]
fn from_context_has_no_template_row() {
    use crate::plugins::triage::reasons as t;

    assert!(
        lookup(t::REASON_FROM_CONTEXT).is_none(),
        "`from_context` 走生成产线，模板表里不该有它的行"
    );
}

/// 表本身干净：无空行、无空文本、无重复键。
#[test]
fn the_table_has_no_blank_or_duplicate_rows() {
    for (i, (code, text)) in TEMPLATES.iter().enumerate() {
        assert!(!code.trim().is_empty(), "第 {i} 行的键是空白");
        assert!(!text.trim().is_empty(), "「{code}」的模板文本是空白");
        assert!(
            TEMPLATES.iter().skip(i + 1).all(|(other, _)| other != code),
            "键「{code}」重复"
        );
    }
}

/// 每个理由码都查得到文本：`Answered` 的三个模板码 + `Escalate` 的两个。
#[test]
fn answered_and_escalate_reasons_resolve_to_their_own_templates() {
    let answered = template_for(&Verdict::Answered {
        reason: REASON_GREETING.to_string(),
    });
    assert_eq!(
        answered.as_deref(),
        Some("你好，我在。有什么事直接说就行。")
    );

    let escalate = template_for(&Verdict::Escalate {
        reason: REASON_NEEDS_WORK.to_string(),
    });
    assert_eq!(escalate.as_deref(), Some("好，我来处理。"));
}

/// 兜底**随变体而不同**：同为未知码，`Answered` 说"好的"（本轮已收尾），
/// `Escalate` 说"我来处理"（本轮会干活）。用同一个兜底会让其中一个变成谎话。
#[test]
fn the_fallback_depends_on_the_variant() {
    let answered = template_for(&Verdict::Answered {
        reason: "未来才有的码".to_string(),
    });
    let escalate = template_for(&Verdict::Escalate {
        reason: "未来才有的码".to_string(),
    });

    assert_eq!(answered.as_deref(), Some(FALLBACK_ANSWERED));
    assert_eq!(escalate.as_deref(), Some(FALLBACK_ESCALATE));
    assert_ne!(answered, escalate, "两个变体的兜底不能是同一句");
}

/// `Report` 没有模板：它的措辞要从运行现状组织，而那个快照到 S4 才有生产者。
/// 本批**不编**一句话——编出来的必然与界面上的真实进展不符。
#[test]
fn report_has_no_template_yet() {
    assert_eq!(template_for(&Verdict::Report), None);
}

/// 空输入不是"没模板"，而是一条**正常**的模板行：它是规则表判出来的
/// `Answered { empty }`，用户该收到一句"你还没说要做什么"。
#[test]
fn empty_input_is_a_normal_template_row_not_a_missing_one() {
    let text = template_for(&Verdict::Answered {
        reason: REASON_EMPTY.to_string(),
    })
    .expect("空输入应有模板");
    assert!(!text.trim().is_empty());
    assert_ne!(text, FALLBACK_ANSWERED, "空输入不该落到通用兜底");
}
