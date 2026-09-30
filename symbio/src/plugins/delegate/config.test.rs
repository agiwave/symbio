//! config 的单元测试：默认值、前缀归一、上限下界。
use super::*;

#[test]
fn defaults_are_documented_baseline() {
    let c = DelegateConfig::default();
    assert!(c.enabled, "出厂启用");
    assert_eq!(c.force_prefix, "/work ");
    assert!(c.keywords.is_empty(), "出厂只认前缀：宁可漏判，不要误判");
    assert_eq!(c.min_chars, 0, "出厂不按长度判定");
    assert_eq!(c.effective_digest_max(), 40);
}

/// 前缀归一：纯空白 = 不启用；**尾空格保留**（它是词边界护栏，不是排版）。
#[test]
fn force_prefix_is_optional_but_keeps_its_word_boundary() {
    let mut c = DelegateConfig::default();
    assert_eq!(
        c.effective_force_prefix(),
        Some("/work "),
        "尾空格必须留——trim 掉它会让 /worker 误命中 /work"
    );

    c.force_prefix = "   ".to_string();
    assert_eq!(c.effective_force_prefix(), None, "纯空白 = 不启用");

    c.force_prefix = String::new();
    assert_eq!(c.effective_force_prefix(), None, "空串 = 不启用");
}

/// 上限下界 1：配成 0 会让目录恒空，而下界保证"看得见至少一项"。
#[test]
fn digest_max_has_floor_of_one() {
    let c = DelegateConfig {
        digest_max: 0,
        ..Default::default()
    };
    assert_eq!(c.effective_digest_max(), 1);
}
