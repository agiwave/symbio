//! 消息节点读数的判据（[ADR-111](../../../../docs/010-adr/111.md)）：
//! 那几行「压到哪儿了 / 什么触发的 / 压掉几条」由后端排好，前端只逐行排版。
//!
//! 这批判据钉的是**口径搬回来之后不再漂移**的三条：
//! 1. **逐字对上旧前端的呈现**——搬口径不改行为，行序、单位、词表都与前端删掉的那四条一致；
//! 2. **缺数字就少一行，绝不补 0**——「水位已降到 0」是最坏的一种谎报；
//! 3. **呈现只在出口出现**——没有数字的节点不占 `stats` 格（`skip_serializing` 因此有效）。

use super::{format_tokens, notice_stats_of, present_message, trigger_label};
use crate::symbio_core::{chat_node as cm, ReadingSection};
use serde_json::json;

/// 一条压缩节点（形状 = 折叠叶；压缩的数字住在 `meta`，owner 是
/// `plugins/loop/context/pipeline.rs::compression_stats`）。
fn compression(meta: serde_json::Value) -> cm::ChatNode {
    cm::ChatNode {
        id: "m1".into(),
        msg_type: Some(cm::NodeKind::Notice),
        meta: Some(meta),
        ..Default::default()
    }
}

fn labels(section: Option<ReadingSection>) -> Vec<(String, String)> {
    section
        .map(|s| {
            s.rows
                .iter()
                .map(|r| (r.label.clone(), r.value.clone()))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn a_successful_compression_reads_water_limit_trigger_and_count() {
    let rows = labels(notice_stats_of(&compression(json!({
        "compact_trigger": "threshold",
        "context_limit": 128_000,
        "before_tokens": 96_000,
        "after_tokens": 44_500,
        "dropped": 12,
    }))))
    .into_iter()
    .map(|(l, v)| format!("{l} {v}"))
    .collect::<Vec<_>>()
    .join(" · ");
    // 行序与文案与前端删掉的那四条口径逐字一致（搬 owner，不改行为）。
    assert_eq!(
        rows,
        "压缩后水位 44.5k · 上限 128k · 触发 自动整理 · 压掉 12 条"
    );
}

#[test]
fn a_failed_compression_reports_current_water_instead_of_an_invented_zero() {
    // 失败路径没有 `after_tokens`（后端只在成功时写）。补 0 会显示成「水位已降到 0」。
    let got = labels(notice_stats_of(&compression(json!({
        "compact_trigger": "retry",
        "context_limit": 128_000,
        "before_tokens": 150_000,
    }))));
    assert_eq!(got[0].0, "当前水位");
    assert_eq!(got[0].1, "150k");
    assert_eq!(got[1].0, "上限");
    assert_eq!(got[2].0, "触发");
    assert_eq!(got[2].1, "手动重试");
    assert_eq!(got.len(), 3, "没有 `dropped` 就不该有「压掉」那一行");
}

#[test]
fn a_notice_without_any_number_carries_no_reading_at_all() {
    // 正文一行说明，没有数字：`stats` 保持 None（前端因此只渲染正文，与旧后端同形）。
    let m = compression(json!({ "style_tag": "compression" }));
    assert!(notice_stats_of(&m).is_none());
    let presented = present_message(m);
    assert!(presented.stats.is_none());
    // 形状派生照旧生效（两条装配互不覆盖）。
    assert_eq!(presented.title.as_deref(), Some("上下文压缩"));
}

#[test]
fn the_compacted_snapshot_reads_its_own_two_facts() {
    // 压缩的**产物**：后端给的字段名不同（`post_tokens` / `transcript_path`），
    // 且它是 `Body` 形状（provider 要求对话以 user 开头），靠 `meta.compacted` 认。
    let got = labels(notice_stats_of(&cm::ChatNode {
        id: "m0".into(),
        msg_type: Some(cm::NodeKind::Body),
        meta: Some(
            json!({ "compacted": true, "post_tokens": 9_000, "transcript_path": "/a/b.json" }),
        ),
        ..Default::default()
    }));
    assert_eq!(
        got,
        vec![
            ("压缩后水位".to_string(), "9k".to_string()),
            ("完整历史".to_string(), "已转存".to_string()),
        ]
    );

    // 转存路径缺失 / 空串 ⇒ 没有「已转存」那一行（存在即为已归档，这条判定也在后端）。
    let got = labels(notice_stats_of(&cm::ChatNode {
        id: "m0".into(),
        msg_type: Some(cm::NodeKind::Body),
        meta: Some(json!({ "compacted": true, "transcript_path": "" })),
        ..Default::default()
    }));
    assert!(got.is_empty());
}

#[test]
fn reasoning_and_other_notices_are_not_read_as_compressions() {
    // `is_compression` 的例外：推理折叠叶不带压缩读数，即使它的 meta 里有同名数字。
    let m = compression(json!({ "style_tag": "reasoning", "after_tokens": 1_000 }));
    assert!(notice_stats_of(&m).is_none());
}

#[test]
fn the_token_unit_is_formatted_in_exactly_one_place() {
    // 单位与精度的表就是前端 `formatTokens` 那张表（<1000 原样、≥100k 取整、其余一位小数）。
    for (n, want) in [
        (0u64, "0"),
        (950, "950"),
        (999, "999"),
        (1_000, "1k"),
        (1_234, "1.2k"),
        (9_000, "9k"),
        (44_500, "44.5k"),
        (99_500, "99.5k"),
        (100_000, "100k"),
        (200_000, "200k"),
    ] {
        assert_eq!(format_tokens(n), want, "{n} 的水位写法");
    }
}

#[test]
fn an_unknown_trigger_word_shows_itself_rather_than_a_stale_label() {
    assert_eq!(trigger_label("threshold"), "自动整理");
    assert_eq!(trigger_label("tool"), "模型主动");
    assert_eq!(trigger_label("retry"), "手动重试");
    // 协议加了新档位而词表没跟上时，界面上看得见（原样透出）而不是被旧词盖住。
    assert_eq!(trigger_label("brand_new"), "brand_new");
}
