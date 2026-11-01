//! `config.rs` 的单元测试 —— 「空」的三种写法同义。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。
//!
//! 钉住的是**平凡值**：这个人格没被填过时，本插件必须交不出任何段。若三种「空」
//! 里有任意一种漏判，系统提示词里就会多出一段空标题——而空标题是**看不见的
//! 失效**：模型照常回答，没有任何报错。

use super::*;

#[test]
fn absent_key_has_no_text() {
    let c: PersonaConfig = serde_json::from_str("{}").expect("空配置应可反序列化");
    assert_eq!(c.text(), None);
}

#[test]
fn empty_and_blank_are_the_same_as_absent() {
    // 设置页清空输入框 ⇒ `""`；手打几个空格 ⇒ `"   "`。两者都不是人格。
    for raw in [r#"{"persona": ""}"#, r#"{"persona": "   \n\t "}"#] {
        let c: PersonaConfig = serde_json::from_str(raw).expect("空串应可反序列化");
        assert_eq!(c.text(), None, "「{raw}」应与缺席同义");
    }
}

#[test]
fn text_is_trimmed() {
    let c: PersonaConfig =
        serde_json::from_str(r#"{"persona": "  你说话简短。  "}"#).expect("正文应可反序列化");
    assert_eq!(c.text(), Some("你说话简短。"), "首尾空白不该进系统提示词");
}

/// 装配期写进 `PLUGIN.yml` 的身份键（`plugin_provider` / `plugin_order` …）必须
/// **不影响**配置解析：它们由装配方投影，不是本插件的业务键。
#[test]
fn assembly_keys_do_not_break_parsing() {
    let raw = r#"{"plugin_provider": "persona", "plugin_order": 55, "persona": "你说话简短。"}"#;
    let c: PersonaConfig = serde_json::from_str(raw).expect("带装配键的清单应可解析");
    assert_eq!(c.text(), Some("你说话简短。"));
}
