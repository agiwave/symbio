//! 消息节点读数的判据（[D-111](README.md#d-111-消息读数由交付出口装配压缩与记忆的事实行离开前端 "crates/symbio-plugin-session/src/README")）：
//! 那几行「压到哪儿了 / 什么触发的 / 压掉几条」由后端排好，前端只逐行排版。
//!
//! 这批判据钉的是**口径搬回来之后不再漂移**的三条：
//! 1. **逐字对上旧前端的呈现**——搬口径不改行为，行序、单位、词表都与前端删掉的那四条一致；
//! 2. **缺数字就少一行，绝不补 0**——「水位已降到 0」是最坏的一种谎报；
//! 3. **呈现只在出口出现**——没有数字的节点不占 `stats` 格（`skip_serializing` 因此有效）。
//!
//! 同一族纪律还管着会话级的**上下文水位**（`attributes.context_watermark`）：
//! 四个 token 数是事实（住在最近一轮骨架的 `meta.context`），怎么说、比率怎么除
//! 只在本文件出现一次。

use super::{
    context_reading_of, format_money, format_tokens, notice_stats_of, present_message,
    trigger_label,
};
use serde_json::json;
use symbio_core::{chat_node as cm, ReadingSection, ReadingTone};

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
fn the_money_unit_is_formatted_in_exactly_one_place() {
    // 纳单位 → 至少两位、最多四位小数；非零但小于半格写 `<0.0001`（不抹成 0）。
    for (nano, want) in [
        (0u64, "0.00"),
        (49_999, "<0.0001"),
        (50_000, "0.0001"),
        (100_000, "0.0001"),
        (1_000_000, "0.0010"),
        (10_000_000, "0.01"),
        (100_000_000, "0.10"),
        (1_000_000_000, "1.00"),
        (1_234_567_890, "1.2346"),
    ] {
        assert_eq!(format_money(nano), want, "{nano} 纳的金额写法");
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

// ==================== 会话级上下文水位（attributes.context_watermark） ====================

/// 逐行读回（与 [`labels`] 同族——水位这一格与消息上的 `stats` **是同一个方言**，
/// 所以连读法都不需要第二个实现）。
fn water_rows(section: &Option<ReadingSection>) -> Vec<(String, String)> {
    section
        .as_ref()
        .map(|s| {
            s.rows
                .iter()
                .map(|r| (r.label.clone(), r.value.clone()))
                .collect()
        })
        .unwrap_or_default()
}

/// 交付出口的**线上形状**（前端读的就是这个）。
fn wire(section: &Option<ReadingSection>) -> serde_json::Value {
    serde_json::to_value(section).unwrap_or_default()
}

#[test]
fn the_watermark_reads_used_against_limit_and_marks_the_auto_line() {
    let got = context_reading_of(&json!({
        "content_tokens": 42_000,
        "request_tokens": 45_000,
        "limit_tokens": 128_000,
        "compress_tokens": 89_600,
    }));
    assert_eq!(
        water_rows(&got),
        vec![
            ("已用".to_string(), "45k / 128k".to_string()),
            ("自动整理".to_string(), "89.6k".to_string()),
        ]
    );
    // 请求级开销单独说出来：换一批工具定义也会挪水位，那不该看起来像历史变长了。
    assert_eq!(
        got.as_ref().unwrap().rows[0].hint.as_deref(),
        Some("本轮请求开头的估算（内容 42k + 请求级 3k）")
    );
    assert_eq!(got.as_ref().unwrap().rows[1].tone, ReadingTone::Normal);
    // 比率在这里除完（前端不再除第二次）。
    let bar = got.as_ref().unwrap().bar.as_ref().expect("有分母就该有条");
    assert!(bar.level.unwrap() < 0.36);
    assert!((bar.marks[0] - 0.7).abs() < 0.001);

    // 线上形状就是宿主方言：`rows` + `bar{level,marks,tone}`，**没有**为水位自造的键。
    // 自造键（`used_ratio` / `compress_ratio` 那一类）会把业务名写进渲染器——
    // 换一个产者就得改前端，那正是 `present/reading` 模块文档立起来要挡的事。
    let v = wire(&got);
    assert_eq!(
        v["bar"],
        json!({ "level": bar.level, "marks": bar.marks, "tone": "normal" })
    );
    assert_eq!(v["rows"].as_array().unwrap().len(), 2);
    for invented in ["used_ratio", "compress_ratio", "watermark", "percent"] {
        assert!(v.get(invented).is_none(), "方言之外多了自造键 {invented}");
    }
    // 分区没有标题：这条读数自己就是那一句，再配一个「上下文水位」是排版噪声。
    assert!(v.get("title").is_none());
}

#[test]
fn crossing_the_auto_line_is_the_producers_call_not_the_frontends() {
    let got = context_reading_of(&json!({
        "request_tokens": 96_000,
        "limit_tokens": 128_000,
        "compress_tokens": 89_600,
    }));
    // 行与条**同时**转色：语气由产者判一次，渲染器不解析值的语义再决定颜色。
    assert_eq!(got.as_ref().unwrap().rows[1].tone, ReadingTone::Warn);
    assert_eq!(
        got.as_ref().unwrap().bar.as_ref().unwrap().tone,
        ReadingTone::Warn
    );
}

#[test]
fn a_missing_number_loses_its_row_instead_of_becoming_zero() {
    // 没有上限 ⇒ 「已用」报绝对值，而条**没有分母可画**：整格 `bar` 不出现，
    // 而不是画一条宽度 0 的条冒充「水位是 0」。
    let got = context_reading_of(&json!({ "request_tokens": 45_000 }));
    assert_eq!(
        water_rows(&got),
        vec![("已用".to_string(), "45k".to_string())]
    );
    assert!(got.as_ref().unwrap().bar.is_none());

    // 一个数都没有 ⇒ 整条读数不存在（前端因此不渲染水位条）。
    assert!(context_reading_of(&json!({})).is_none());
    // `limit` 为 0 不能变成除零：那一格按「没有上限」处置。
    let got = context_reading_of(&json!({ "request_tokens": 1_000, "limit_tokens": 0 }));
    assert_eq!(
        water_rows(&got),
        vec![("已用".to_string(), "1k".to_string())]
    );
    assert!(got.as_ref().unwrap().bar.is_none());
}

#[test]
fn over_the_limit_the_bar_is_full_but_the_text_stays_the_measured_number() {
    // 真实失败形态（输入超限）：条形截到满，文本仍报实测的 150k——
    // 把数也截成 128k 就是把失败读成正常。
    let got = context_reading_of(&json!({
        "request_tokens": 150_000,
        "limit_tokens": 128_000,
        "compress_tokens": 89_600,
    }));
    assert_eq!(water_rows(&got)[0].1, "150k / 128k");
    assert_eq!(got.as_ref().unwrap().bar.as_ref().unwrap().level, Some(1.0));
}
