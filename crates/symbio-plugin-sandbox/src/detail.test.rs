//! `detail.rs` 的单元测试 —— 表单声明的形状（D-531 / 裁决 B）。

use super::*;

#[test]
fn detail_declares_approval_fields_instead_of_session_options() {
    let d = sandbox_detail_definition();
    // 标题从 name 派生（选中已有沙箱时不再是「新建沙箱」）。
    assert_eq!(d.title_from, vec!["name".to_string()]);
    // mode / risk_level 已下沉为沙箱内部字段，会话层不再单独选择。
    let keys: Vec<&str> = d
        .sections
        .iter()
        .flat_map(|s| s.fields.iter())
        .map(|f| f.key.as_str())
        .collect();
    assert!(keys.contains(&"approval_mode"));
    assert!(keys.contains(&"risk_threshold"));
    assert!(keys.contains(&"name"));
}

#[test]
fn select_fields_carry_explicit_options() {
    let d = sandbox_detail_definition();
    for section in &d.sections {
        for f in &section.fields {
            if f.key == "approval_mode" || f.key == "risk_threshold" {
                assert!(f.widget == "select");
                assert!(!f.options.is_empty(), "{} 必须带候选", f.key);
            }
        }
    }
}
