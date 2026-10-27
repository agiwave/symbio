//! 行为名词汇的判据：已知名字唯一、判定按词不按前缀。

use super::*;

/// 三个已知名字各出现一次，且线格式是 `snake_case`。
#[test]
fn known_names_are_unique_and_snake_case() {
    assert_eq!(KNOWN.len(), 3);
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
