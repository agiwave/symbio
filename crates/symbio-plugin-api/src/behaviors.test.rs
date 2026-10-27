//! 行为名词汇的判据：已知名字唯一、判定按词不按前缀。

use super::*;

/// 已知名字逐个点名：拼错、重复、漏登记都会红。
///
/// **不数个数**（DE-025 D-210：不为「声明数 / 变体数」再加计数器）。逐名列出比数字
/// 多一层信息——加一个行为名必须在这里写一行，等于「我知道我在扩宿主词汇表」，
/// 而不是「某个计数从 3 变成 4」。
#[test]
fn known_names_are_unique_and_snake_case() {
    let mut names: Vec<&str> = KNOWN.to_vec();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            ASSIGN_WORK,
            DELEGATION_PROMISE,
            FILE_CHANGE,
            NOTE_CAPTURE,
            TASK_TABLE,
        ],
    );
    for name in KNOWN {
        assert!(
            name.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
            "{name:?} 不是 snake_case 线格式"
        );
        assert_eq!(
            KNOWN.iter().filter(|other| *other == name).count(),
            1,
            "{name:?} 重复登记"
        );
    }
}

/// `is_known` 只认整词：前缀 / 大小写变体都不是已知行为。
///
/// 闸门按单个常量判，校验面按本函数判；两处若一处写宽，未知行为就会被当成
/// 「已登记」静默放行。
#[test]
fn is_known_matches_whole_words_only() {
    for name in KNOWN {
        assert!(is_known(name));
    }
    for raw in [
        "assign",
        "assign_work_extra",
        "ASSIGN_WORK",
        "",
        " task_table",
    ] {
        assert!(!is_known(raw), "{raw:?} 不应被判为已知行为");
    }
}
