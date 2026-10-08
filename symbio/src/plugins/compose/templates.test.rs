//! 模板表的判据：表的完整性 + 兜底方向。

use super::*;
// 走生成产线的那个码**不在模板表里**，因此它不在 `templates` 的 use 面上
// （那正是下面 `from_context_has_no_template_row` 要钉的事）。
use crate::symbio_core::REASON_FROM_CONTEXT;

/// 表必须覆盖**每一个**走模板产线的理由码：漏一个 ⇒ 用户收到通用兜底
/// （降级而不失效，但那是兜底，不是正常路径）。
#[test]
fn every_template_bound_reason_has_a_row() {
    for code in [
        REASON_GREETING,
        REASON_THANKS,
        REASON_ACK,
        REASON_EMPTY,
        REASON_CLARIFY,
        REASON_REFUSE,
        REASON_NEEDS_WORK,
        REASON_UNCLASSIFIED,
    ] {
        assert!(lookup(code).is_some(), "理由码「{code}」没有模板行");
    }
}

/// `from_context` **不得**有模板行：它走生成产线，而 `plugin.rs` 先判生成
/// ⇒ 表里那一行永远读不到。留着它比没有更糟——读表的人会以为它生效。
#[test]
fn from_context_has_no_template_row() {
    assert!(
        lookup(REASON_FROM_CONTEXT).is_none(),
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

/// `Report` **不在**模板表里：它的正文随运行现状变，由 [`progress_text`] 填出来。
///
/// 表里每一行都是"与上下文无关的固定措辞"，两者不是同一种数据——把它塞进表里，
/// 就得让 `template_for` 去读 `RunSnapshot`，于是"查表"和"填表"混成一个函数。
#[test]
fn report_has_no_template_row() {
    assert_eq!(template_for(&Verdict::Report), None);
}

/// 汇报把**运行现状说进句子**：轮次与静默时长都出现，且是给人读的量级。
#[test]
fn progress_text_states_the_snapshot() {
    let text = progress_text(&RunSnapshot {
        tool_rounds: 3,
        quiet_ms: 125_000,
    });

    assert!(text.contains('3'), "应说出已完成轮次，实得：{text}");
    assert!(
        text.contains("2 分钟"),
        "125s 应读成「2 分钟」而不是「125 秒」，实得：{text}"
    );
}

/// `tool_rounds = 0` 时**不说"已完成 0 轮"**。
///
/// 编排层不可达（汇报判定要求至少走完一轮），但契约的第二个调用方是网关——
/// 那句话在这里必须说得通，否则外部调用方会拿到一句自相矛盾的汇报。
#[test]
fn progress_text_handles_a_zero_round_snapshot() {
    let text = progress_text(&RunSnapshot {
        tool_rounds: 0,
        quiet_ms: 5_000,
    });

    assert!(!text.contains('0'), "不该说「已完成 0 轮」，实得：{text}");
    assert!(text.contains("5 秒"), "静默时长仍要说出来，实得：{text}");
}

/// 时长分档的边界：不足一分钟说秒，到一分钟改说分钟；负数与 0 抬到 1 秒
/// （"用时约 0 秒"读起来像故障）。
#[test]
fn humanize_ms_switches_scale_at_one_minute() {
    assert_eq!(humanize_ms(-1), "1 秒");
    assert_eq!(humanize_ms(0), "1 秒");
    assert_eq!(humanize_ms(59_999), "59 秒");
    assert_eq!(humanize_ms(60_000), "1 分钟");
    assert_eq!(humanize_ms(3_600_000), "60 分钟");
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
