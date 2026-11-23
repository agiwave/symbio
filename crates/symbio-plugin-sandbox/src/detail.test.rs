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

#[test]
fn constraint_fields_use_json_widget() {
    // 两个能力约束字段用通用 json widget 直接编辑结构化值——不再要求用户手写
    // `_json` 字符串（那条绕路是表单机制没有 json widget 时的历史产物）。
    let d = sandbox_detail_definition();
    let json_keys: Vec<&str> = d
        .sections
        .iter()
        .flat_map(|s| s.fields.iter())
        .filter(|f| f.widget == "json")
        .map(|f| f.key.as_str())
        .collect();
    assert!(
        json_keys.contains(&"allowed_facts"),
        "应有 allowed_facts json 字段"
    );
    assert!(
        json_keys.contains(&"blocked_tools"),
        "应有 blocked_tools json 字段"
    );
    // 旧的 `_json` 字符串字段必须彻底消失。
    let all_keys: Vec<&str> = d
        .sections
        .iter()
        .flat_map(|s| s.fields.iter())
        .map(|f| f.key.as_str())
        .collect();
    assert!(!all_keys.contains(&"allowed_facts_json"));
    assert!(!all_keys.contains(&"blocked_tools_json"));
}

#[test]
fn detail_declares_save_and_delete_actions() {
    // 回归：早期这里漏了 actions，沙箱编辑页没有「保存」按钮。与 mcp / skill
    // 同形——动作必须由定义显式声明，渲染器才据此显示按钮。
    let d = sandbox_detail_definition();
    let ids: Vec<&str> = d.actions.iter().map(|a| a.id.as_str()).collect();
    assert!(ids.contains(&"save"), "详情定义必须声明保存动作：{ids:?}");
    assert!(ids.contains(&"delete"), "详情定义必须声明删除动作：{ids:?}");
}
