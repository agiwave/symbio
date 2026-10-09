//! `reading` 方言（读数面板）的判据。
//!
//! ## 为什么值得单测
//!
//! 形状本身没有逻辑，但**三件"错了不报错"的事**只在形状上：
//! ① `tone` 的 serde 名（`WARN` 与 `warn` 在前端是两个字面量，错的那个静默
//!    落回默认 ⇒ 需留意的行全变平常色）；② 可选字段缺省（缺 `subtitle` 的面板
//!    必须还能反序列化——前端拿到的就是缺省那一份）；③ 访问器的行数统计
//!    （分区嵌套，漏一层就少数一半）。
//!
//! 反序列化用例刻意用**手写 JSON 字面量**而不是先序列化再反序列化：后者会
//! 自动对齐自己的默认值，证不了「外部（前端）按文档写的 JSON 读得进来」。

use crate::symbio_core::schemas::reading::{ReadingPanel, ReadingRow, ReadingSection, ReadingTone};

#[test]
fn tone_serializes_as_lowercase_so_the_frontend_literal_matches() {
    assert_eq!(
        serde_json::to_string(&ReadingTone::Warn).unwrap(),
        "\"warn\"",
        "前端写的是字面量 'warn'；这里改名它静默落回默认"
    );
    assert_eq!(
        serde_json::to_string(&ReadingTone::Normal).unwrap(),
        "\"normal\""
    );
}

#[test]
fn a_row_round_trips_with_optional_fields_absent() {
    // 前端按文档能写出的最小 JSON：只要 label + value。
    let json = r#"{"label":"兜底率","value":"0%"}"#;
    let row: ReadingRow = serde_json::from_str(json).expect("最小行必须读得进");
    assert_eq!(row.label, "兜底率");
    assert_eq!(row.hint, None, "缺省口径提示");
    assert_eq!(row.tone, ReadingTone::Normal, "缺省语气 = 平常");
}

#[test]
fn a_panel_round_trips_without_optional_sections() {
    let json = r#"{"title":"读数"}"#;
    let panel: ReadingPanel = serde_json::from_str(json).expect("最小面板必须读得进");
    assert_eq!(panel.title, "读数");
    assert!(panel.sections.is_empty(), "缺省分区 = 空（不是错误）");
    assert_eq!(panel.row_count(), 0);
    assert!(!panel.has_warn(), "空面板没有需留意的行");
}

#[test]
fn accessors_count_nested_rows_and_spot_warnings() {
    let panel = ReadingPanel {
        title: "t".into(),
        subtitle: None,
        sections: vec![
            ReadingSection {
                title: Some("a".into()),
                rows: vec![ReadingRow {
                    label: "x".into(),
                    value: "1".into(),
                    hint: None,
                    tone: ReadingTone::Normal,
                }],
            },
            ReadingSection {
                title: None,
                rows: vec![
                    ReadingRow {
                        label: "y".into(),
                        value: "2".into(),
                        hint: Some("口径".into()),
                        tone: ReadingTone::Warn,
                    },
                    ReadingRow {
                        label: "z".into(),
                        value: "3".into(),
                        hint: None,
                        tone: ReadingTone::Normal,
                    },
                ],
            },
        ],
    };
    assert_eq!(panel.row_count(), 3, "分区嵌套的行数要全数到");
    assert!(panel.has_warn(), "第二个分区有一行需留意");
}
