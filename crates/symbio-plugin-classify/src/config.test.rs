//! `classify/config.rs` 的单元测试 —— 平凡值与存量配置的兼容。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

/// 缺省即出厂行为：规则短路**开**
#[test]
fn rule_shortcut_defaults_to_on() {
    assert!(ClassifyConfig::default().rule_shortcut);
    assert_eq!(
        ClassifyConfig::default().rule_shortcut,
        default_rule_shortcut()
    );
}

/// 存量 `PLUGIN.yml`（装配期刚补出身份键、还没有本插件业务键）必须能读成缺省
#[test]
fn missing_business_keys_fall_back_to_defaults() {
    let c: ClassifyConfig = serde_json::from_str("{}").unwrap();
    assert_eq!(c.rule_shortcut, default_rule_shortcut());
    assert!(c.model_id().is_none(), "缺省模型 ⇒ 用会话选定值");
    assert!(c.prompt_override().is_none(), "缺省提示词 ⇒ 用内置那份");
}

/// 平凡值可达：`rule_shortcut: false` 是一条能被写进配置的取值
#[test]
fn plain_value_is_reachable_from_config() {
    let c: ClassifyConfig = serde_json::from_str(r#"{"rule_shortcut": false}"#).unwrap();
    assert!(!c.rule_shortcut);
}

/// 「独立模型」与「提示词覆盖」都能从配置里写进来（S6 的两条产线）
#[test]
fn model_and_prompt_come_from_config() {
    let c: ClassifyConfig =
        serde_json::from_str(r#"{"model": "cheap-classifier", "system_prompt": "只输出一个词。"}"#)
            .unwrap();
    assert_eq!(c.model_id(), Some("cheap-classifier"));
    assert_eq!(c.prompt_override(), Some("只输出一个词。"));
}

/// **空串与缺席同义**：设置页把输入框清空得到的是 `""`，不是"删掉这个键"。
///
/// 把它当成一个 id 去查只会查不到 ⇒ 落回会话模型（行为上恰好也对），但
/// `prompt_override` 若认空串就会把内置提示词换成一段空文本——那是一条**没有
/// 任何错误信号**的失效：分类器收到空系统提示词，四选一的准确率崩掉，而日志里
/// 一切正常。两种写法必须在**同一处**归一。
#[test]
fn blank_strings_are_same_as_absent() {
    let c: ClassifyConfig =
        serde_json::from_str(r#"{"model": "  ", "system_prompt": ""}"#).unwrap();
    assert!(c.model_id().is_none());
    assert!(c.prompt_override().is_none());
}
