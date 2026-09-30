//! 能力目录折叠的单元测试：平凡值、分组、确定性、截断可见。
use super::*;
use crate::symbio_core::{CapabilityCategory, CapabilityMeta};
use serde_json::json;

fn cap(name: &str, cat: CapabilityCategory) -> CapabilityMeta {
    CapabilityMeta {
        name: name.to_string(),
        description: "d".to_string(),
        input_schema: json!({}),
        category: Some(cat),
        ..Default::default()
    }
}

/// 平凡值：无能力 ⇒ **空串**（调用方据此不注入空标题）。
#[test]
fn empty_caps_yield_empty_string() {
    assert_eq!(digest(&[], 40), "");
    let one = vec![cap("vdfs_read", CapabilityCategory::Resource)];
    assert_eq!(digest(&one, 0), "", "上限 0 ⇒ 不产出内容（不产出空标题）");
}

/// 分组按 **category 的线格式词**，组内名字有序；同组同名去重。
#[test]
fn groups_by_category_wire_and_sorts() {
    let caps = vec![
        cap("vdfs_write", CapabilityCategory::Resource),
        cap("vdfs_read", CapabilityCategory::Resource),
        cap("vdfs_read", CapabilityCategory::Resource), // 重复项
        cap("memory_recall", CapabilityCategory::Other),
    ];
    let out = digest(&caps, 40);
    assert!(out.starts_with(DIGEST_TITLE), "段首是标题：{out}");
    assert!(out.contains("- other: memory_recall"), "实际：{out}");
    assert!(
        out.contains("- resource: vdfs_read / vdfs_write"),
        "同组有序：{out}"
    );
    // 去重：vdfs_read 只出现一次
    assert_eq!(out.matches("vdfs_read").count(), 1, "{out}");
    assert!(!out.contains("…（共"), "未超限不标注截断");
}

/// 确定性（A4）：同一输入双跑逐字节相同。
#[test]
fn digest_is_deterministic() {
    let caps = vec![
        cap("b_tool", CapabilityCategory::FileOperation),
        cap("a_tool", CapabilityCategory::FileOperation),
    ];
    assert_eq!(digest(&caps, 40), digest(&caps, 40));
}

/// 截断**必须可见**：超出上限时列出几项并说明总数。
#[test]
fn truncation_is_visible() {
    let caps = vec![
        cap("t1", CapabilityCategory::FileOperation),
        cap("t2", CapabilityCategory::FileOperation),
        cap("t3", CapabilityCategory::FileOperation),
    ];
    let out = digest(&caps, 2);
    assert!(out.contains("t1") && out.contains("t2"), "{out}");
    assert!(!out.contains("t3"), "第三条被截断：{out}");
    assert!(out.contains("共 3 项"), "截断标注总数：{out}");
    assert!(out.contains("此处列出 2 项"), "{out}");
}

/// 无名能力被跳过（空名字进目录只会浪费 token）。
#[test]
fn unnamed_capability_is_skipped() {
    let caps = vec![
        cap("", CapabilityCategory::Other),
        cap("ok", CapabilityCategory::Other),
    ];
    let out = digest(&caps, 40);
    assert!(out.contains("ok"));
    assert_eq!(
        out.lines().filter(|l| l.starts_with("- ")).count(),
        1,
        "{out}"
    );
}
