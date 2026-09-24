//! `symbio/src/symbio_core/schemas/detail.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

use serde_json::json;

fn field(key: &str, widget: &str) -> DetailField {
    DetailField {
        key: key.into(),
        label: key.into(),
        widget: widget.into(),
        ..Default::default()
    }
}

fn def_of(fields: Vec<DetailField>) -> DetailDefinition {
    DetailDefinition {
        sections: vec![DetailSection {
            title: None,
            collapsed: false,
            fields,
        }],
        ..Default::default()
    }
}

#[test]
fn required_missing_and_blank_are_reported_per_field() {
    let mut f = field("host", "text");
    f.required = true;
    let def = def_of(vec![f]);

    assert_eq!(
        def.validate(&json!({})).unwrap_err().fields[0].field,
        "host"
    );
    assert_eq!(
        def.validate(&json!({ "host": "   " })).unwrap_err().fields[0].field,
        "host"
    );
    // `false` / `0` 是有效值，不是「没填」
    assert!(def.validate(&json!({ "host": "127.0.0.1" })).is_ok());
}

#[test]
fn non_object_payload_is_rejected() {
    assert!(def_of(vec![]).validate(&json!("nope")).is_err());
}

/// 隐藏字段（`visible_when` 不成立）与只读展示字段都不参与校验：
/// 没显示的字段不该拦住提交。
#[test]
fn hidden_and_static_fields_are_exempt() {
    let mut hidden = field("port", "number");
    hidden.required = true;
    hidden.visible_when = Some(DetailCondition {
        key: "enabled".into(),
        equals: Some(json!(true)),
        ..Default::default()
    });
    let mut ro = field("status", "static");
    ro.required = true;

    let def = def_of(vec![hidden, ro]);

    // 未启用 → 端口整行不渲染 → 缺失也不算错
    assert!(def.validate(&json!({ "enabled": false })).is_ok());
    // 启用 → 端口回归校验
    assert_eq!(
        def.validate(&json!({ "enabled": true }))
            .unwrap_err()
            .fields[0]
            .field,
        "port"
    );
}

/// 空条件恒成立（`when` / `visible_when` 缺省即显示）。
#[test]
fn empty_condition_always_holds() {
    assert!(DetailCondition::default().holds(&json!({})));
}

#[test]
fn field_check_covers_shape_and_range() {
    let mut n = field("port", "number");
    n.min = Some(1.0);
    n.max = Some(65535.0);
    assert!(n.check(&json!(80)).is_none());
    assert!(n.check(&json!(0)).is_some());
    assert!(n.check(&json!(70000)).is_some());
    assert!(n.check(&json!("abc")).is_some());

    let mut s = field("proto", "select");
    s.options = vec![
        DetailOption {
            value: "http".into(),
            label: "HTTP".into(),
            description: None,
        },
        DetailOption {
            value: "https".into(),
            label: "HTTPS".into(),
            description: None,
        },
    ];
    assert!(s.check(&json!("http")).is_none());
    assert!(s.check(&json!("carrier-pigeon")).is_some());

    assert!(field("flag", "toggle").check(&json!(true)).is_none());
    assert!(field("flag", "toggle").check(&json!("yes")).is_some());
    assert!(field("hosts", "list").check(&json!(["a"])).is_none());
    assert!(field("hosts", "list").check(&json!("a")).is_some());
    assert!(field("env", "map").check(&json!({ "K": "V" })).is_none());
    assert!(field("env", "map").check(&json!([])).is_some());
    assert!(field("dir", "path").check(&json!("/tmp")).is_none());
    assert!(field("dir", "path").check(&json!(1)).is_some());
    assert!(field("hb", "form").check(&json!({})).is_none());
    assert!(field("hb", "form").check(&json!("nope")).is_some());
}

/// 结构化子对象（`widget = "form"`）用子定义**递归**校验，错误字段带父前缀。
#[test]
fn nested_form_field_validates_against_sub_definition() {
    let mut interval = field("interval_seconds", "number");
    interval.min = Some(10.0);
    let mut hb = field("heartbeat", "form");
    hb.form = Some(Box::new(def_of(vec![interval])));
    let def = def_of(vec![hb]);

    assert!(def
        .validate(&json!({ "heartbeat": { "interval_seconds": 30 } }))
        .is_ok());
    // 错误字段名带父前缀，前端仍能逐字段高亮
    assert_eq!(
        def.validate(&json!({ "heartbeat": { "interval_seconds": 1 } }))
            .unwrap_err()
            .fields[0]
            .field,
        "heartbeat.interval_seconds"
    );
}

/// `disabled_when` 成立**不**豁免校验：字段仍然在提交值里（只是用户改不动），
/// 与 `visible_when` 不成立的「没显示就不该拦提交」是两回事。
#[test]
fn disabled_field_is_still_validated() {
    let mut f = field("port", "number");
    f.required = true;
    f.disabled_when = Some(DetailCondition {
        key: "locked".into(),
        equals: Some(json!(true)),
        ..Default::default()
    });
    let def = def_of(vec![f]);
    assert_eq!(
        def.validate(&json!({ "locked": true })).unwrap_err().fields[0].field,
        "port"
    );
}
