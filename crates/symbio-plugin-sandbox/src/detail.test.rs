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
fn constraint_field_is_a_generic_rule_table() {
    // 能力约束用通用 table widget 编辑规则列表——用户可逐行配命令/路径的
    // 匹配模式、动作与风险档，不再手写 JSON（json widget 已被否定为终态）。
    let d = sandbox_detail_definition();
    let rules = d
        .sections
        .iter()
        .flat_map(|s| s.fields.iter())
        .find(|f| f.key == "rules")
        .expect("应有 rules 表格字段");
    assert_eq!(rules.widget, "table");
    let col_keys: Vec<&str> = rules.columns.iter().map(|c| c.key.as_str()).collect();
    assert_eq!(col_keys, vec!["kind", "match", "value", "action", "risk"]);
    // 列只有通用控件词，不含任何沙箱/工具业务词（架构红线）。
    for c in &rules.columns {
        assert!(
            ["text", "select", "number"].contains(&c.widget.as_str()),
            "列控件必须是通用标量 widget：{}",
            c.widget
        );
        assert!(!c.label.contains("shell"), "列标签不得含业务词");
    }
    // 旧 json 字段与 `_json` 字符串字段都必须彻底消失。
    let all_keys: Vec<&str> = d
        .sections
        .iter()
        .flat_map(|s| s.fields.iter())
        .map(|f| f.key.as_str())
        .collect();
    assert!(!all_keys.contains(&"allowed_facts"));
    assert!(!all_keys.contains(&"blocked_tools"));
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
