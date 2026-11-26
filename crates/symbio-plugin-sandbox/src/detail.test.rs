//! `detail.rs` 的单元测试 —— 表单声明的形状（ / 裁决 B）。

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
fn threshold_is_maximum_auto_approved_risk() {
    let d = sandbox_detail_definition();
    let threshold = d
        .sections
        .iter()
        .flat_map(|s| s.fields.iter())
        .find(|f| f.key == "risk_threshold")
        .unwrap();
    let description = threshold.description.as_deref().unwrap();
    assert!(description.contains("最大自动放行风险"));
    assert!(description.contains("高于阈值需审批"));
    let labels: Vec<&str> = threshold.options.iter().map(|o| o.label.as_str()).collect();
    assert!(labels
        .iter()
        .any(|label| label.contains("仅低风险自动放行")));
    assert!(labels
        .iter()
        .any(|label| label.contains("中风险及以下自动放行")));
}

#[test]
fn rule_risk_is_described_as_a_floor() {
    let d = sandbox_detail_definition();
    let rules = d
        .sections
        .iter()
        .flat_map(|s| s.fields.iter())
        .find(|f| f.key == "rules")
        .unwrap();
    let description = rules.description.as_deref().unwrap();
    assert!(description.contains("下限"));
    assert!(description.contains("不得调低工具风险"));
    assert!(description.contains("无法判定的参数默认拒绝"));
    let risk = rules.columns.iter().find(|c| c.key == "risk").unwrap();
    assert_eq!(risk.label, "风险下限");
    assert!(risk
        .options
        .iter()
        .filter(|option| option.value != "inherit")
        .all(|option| option.label.contains("最低风险")));
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
    let kinds: Vec<&str> = rules.columns[0]
        .options
        .iter()
        .map(|option| option.value.as_str())
        .collect();
    for kind in [
        "shell_subcommand",
        "shell_executable",
        "shell_redirection_target",
        "shell_unresolved_argument",
        "shell_unparseable",
        "shell_expansion",
        "mcp_tool",
        "mcp_argument",
        "agent_id",
        "session_id",
        "heartbeat_action",
    ] {
        assert!(kinds.contains(&kind), "规则表缺少 {kind} 候选");
    }
    // 列只有通用控件词，不含任何沙箱/工具业务词（架构红线）。
    for c in &rules.columns {
        assert!(
            ["text", "select", "datalist", "number"].contains(&c.widget.as_str()),
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
    assert!(
        ids.contains(&ACTION_COPY),
        "内置档案必须可复制编辑：{ids:?}"
    );
    let copy = d
        .actions
        .iter()
        .find(|action| action.id == ACTION_COPY)
        .unwrap();
    assert_eq!(copy.disabled_when.as_ref().unwrap().key, "is_existing");
}

#[test]
fn sandbox_selector_makes_disabled_and_missing_states_explicit() {
    let archives = vec![("strict".to_string(), "严格".to_string())];
    let field = sandbox_option_field(&archives, Some("gone"), Some("新档案损坏"), true);
    assert_eq!(field.options[0].value, "");
    assert!(field.options[0].label.contains("禁用沙箱"));
    let missing = field
        .options
        .iter()
        .find(|option| option.value == "gone")
        .unwrap();
    assert!(missing.label.contains("缺失或不可用"));
    let description = field.description.unwrap();
    assert!(description.contains("新档案损坏"));
    assert!(description.contains("上一份良好快照"));

    let field = sandbox_option_field(&archives, Some("gone"), Some("首次加载失败"), false);
    let description = field.description.unwrap();
    assert!(description.contains("没有可用快照"));
    assert!(description.contains("失败关闭"));
    assert!(!description.contains("上一份良好快照"));
}

#[test]
fn rule_kind_is_editable_and_boundary_notice_is_permanent() {
    let d = sandbox_detail_definition();
    let rules = d
        .sections
        .iter()
        .flat_map(|s| &s.fields)
        .find(|f| f.key == "rules")
        .unwrap();
    assert_eq!(rules.columns[0].widget, "datalist");
    assert!(rules.columns[0]
        .placeholder
        .as_deref()
        .is_some_and(|p| p.contains("输入")));
    let boundary = d
        .sections
        .iter()
        .flat_map(|s| &s.fields)
        .find(|f| f.key == "boundary")
        .unwrap();
    assert_eq!(boundary.widget, "static");
    assert!(boundary.default.as_ref().is_some_and(|value| {
        value
            .as_str()
            .is_some_and(|text| text.contains("不提供容器、网络或进程隔离"))
    }));
}
