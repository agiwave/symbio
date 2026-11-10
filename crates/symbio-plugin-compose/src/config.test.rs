//! `compose/config.rs` 的单元测试 —— 平凡值与"没填"的两种写法。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

/// 存量 `PLUGIN.yml`（装配期刚补出身份键、还没有本插件业务键）必须能读成缺省
#[test]
fn missing_keys_fall_back_to_defaults() {
    let c: ComposeConfig = serde_json::from_str("{}").unwrap();
    assert!(
        c.instruction_override().is_none(),
        "缺省指令段 ⇒ 用内置那份"
    );
    assert_eq!(ComposeConfig::default().instruction_override(), None);
}

/// D-427：自持开关出厂为开，且关掉是一条可写进配置的取值。
/// 出厂值必须与迁移前 `SessionConfig::compose_enabled` 一致，否则卸载或缺键会
/// 把「默认开」静默变成「默认关」。
#[test]
fn compose_enabled_defaults_to_on_and_stays_reachable() {
    assert!(ComposeConfig::default().compose_enabled);
    assert!(default_compose_enabled());
    let c: ComposeConfig = serde_json::from_str(r#"{"compose_enabled": false}"#).unwrap();
    assert!(!c.compose_enabled);
}

/// 指令段覆盖能从配置里写进来
#[test]
fn instruction_comes_from_config() {
    let c: ComposeConfig = serde_json::from_str(r#"{"instruction": "只说一句。"}"#).unwrap();
    assert_eq!(c.instruction_override(), Some("只说一句。"));
}

/// **空串与缺席同义**：设置页把输入框清空得到的是 `""`，不是"删掉这个键"。
///
/// 空指令段**不是**"没有指令"，而是「用一段空文本当指令」——生成的答话失去全部
/// 约束（不复述、不编、不用工具），而日志里一切正常。两种写法必须在**同一处**归一。
#[test]
fn blank_strings_are_same_as_absent() {
    let c: ComposeConfig = serde_json::from_str(r#"{"instruction": "   "}"#).unwrap();
    assert!(c.instruction_override().is_none());
}
