//! 召回段的排版口径（抬头 / 跨会话同文只印一行 / 降级说得出来 / 空不印空壳）。
//!
//! 这四条原来住 `loop` 的 `memory_events.test.rs`（`prompt_section_*` 两条）。排版随
//! 能力迁走后判据跟着走——判的是**读数的出口**，不是谁调用它。

use super::*;
use symbio_plugin_api::context::{RecallFacts, RecallLine};

fn line<'a>(seq: u64, content: &'a str, tag: &'a str) -> RecallLine<'a> {
    RecallLine { seq, content, tag }
}

/// 抬头 + 逐条一行、跨会话同文只印一行、空读数不印空壳标题。
#[test]
fn prints_one_line_per_memory_and_nothing_when_empty() {
    let lines = [
        line(3, "另一句", "经验"),
        line(1, "同文的一句", "经验"),
        line(2, "同文的一句", "经验"),
    ];
    let facts = RecallFacts {
        degraded: false,
        lines: &lines,
    };
    let section = prompt_section(&facts).expect("有条目 ⇒ 有段");
    assert!(
        section.starts_with(RECALL_SECTION_HEAD),
        "抬头是唯一锚点：{section}"
    );
    assert_eq!(
        section.matches("同文的一句").count(),
        1,
        "两个会话各记了一条同样的内容 ⇒ 排版只印一行（事实仍各留一格）"
    );
    assert!(section.contains("- 「另一句」"));
    assert!(
        section.contains("不要在本轮执行"),
        "条目要说清身份：旧会话里的祈使句被新一轮照做，就是当轮任务被顶掉（实测）"
    );
    assert!(
        section.find("另一句").unwrap() < section.find("同文的一句").unwrap(),
        "次序照读数来（最新在前）"
    );

    // 空读数 ⇒ None（不印空壳标题）。
    assert!(prompt_section(&RecallFacts {
        degraded: false,
        lines: &[],
    })
    .is_none());
}

/// 降级标注：「只显示了一部分」必须**说得出来**；不降级时不加（不谎称降级）；
/// 降级但零条目仍整段省略（不印「降级的空壳」）。
///
/// 这是 `degraded` 从**检索口径**走到**用户眼前**的出口判据：读方若只取条目就把降级
/// 丢了，静默的形态从「空且不标」变成「有部分结果且不标」——两者都是同一个静默。
#[test]
fn says_degraded_instead_of_pretending_completeness() {
    let lines = [line(2, "片段 2", "经验"), line(1, "片段 1", "经验")];
    let degraded = RecallFacts {
        degraded: true,
        lines: &lines,
    };
    let section = prompt_section(&degraded).expect("有条目 ⇒ 有段");
    assert!(
        section.contains(RECALL_DEGRADED_NOTE),
        "降级时如实标注（不谎称完整）：{section}"
    );
    assert!(
        section.find(RECALL_DEGRADED_NOTE).unwrap() < section.find("片段").unwrap(),
        "标注在正文之前——读的人先看到「只是一部分」"
    );

    // 反向：不降级 ⇒ 不加标注（不谎称降级）。
    let full = RecallFacts {
        degraded: false,
        lines: &lines,
    };
    let full_section = prompt_section(&full).expect("有条目 ⇒ 有段");
    assert!(
        !full_section.contains(RECALL_DEGRADED_NOTE),
        "不降级不标注：{full_section}"
    );

    // 反向：降级但零条目 ⇒ 仍整段省略。生产不可达（配额 ≥ 1 时先扫到一条才降级），
    // 但出口必须钉住：空却贴着「只是一部分」比空壳更糟。
    assert!(
        prompt_section(&RecallFacts {
            degraded: true,
            lines: &[],
        })
        .is_none(),
        "空读数不印「降级的空壳」"
    );
}

/// 字节上界：按**整条**追加，装不下就停（不切半行）。
#[test]
fn stops_at_the_byte_budget_without_cutting_a_line_in_half() {
    let long = "很长的一句".repeat(RECALL_MAX_BYTES / 8);
    let lines = [line(1, &long, "经验")];
    let section = prompt_section(&RecallFacts {
        degraded: false,
        lines: &lines,
    })
    .expect("抬头本身就在预算内");
    assert!(
        !section.contains("很长的一句"),
        "装不下就整条不印（半行会印出一条不存在的记忆）"
    );
    assert!(section.starts_with(RECALL_SECTION_HEAD));
}
