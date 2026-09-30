//! `reply/config.rs` 的单元测试 —— 平凡值与"没填"的两种写法。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

/// 存量 `PLUGIN.yml`（装配期刚补出身份键、还没有本插件业务键）必须能读成缺省
#[test]
fn missing_keys_fall_back_to_defaults() {
    let c: ReplyConfig = serde_json::from_str("{}").unwrap();
    assert!(c.model_id().is_none(), "缺省模型 ⇒ 用会话选定值");
    assert!(
        c.instruction_override().is_none(),
        "缺省指令段 ⇒ 用内置那份"
    );
    assert_eq!(ReplyConfig::default().model_id(), None);
}

/// 「独立模型」与「指令段覆盖」都能从配置里写进来（S6 的两条产线）
#[test]
fn model_and_instruction_come_from_config() {
    let c: ReplyConfig =
        serde_json::from_str(r#"{"model": "good-writer", "instruction": "只说一句。"}"#).unwrap();
    assert_eq!(c.model_id(), Some("good-writer"));
    assert_eq!(c.instruction_override(), Some("只说一句。"));
}

/// **空串与缺席同义**：设置页把输入框清空得到的是 `""`，不是"删掉这个键"。
///
/// `instruction` 认空串的后果比 `model` 严重：空指令段**不是**"没有指令"，而是
/// 「用一段空文本当指令」——生成的答话失去全部约束（不复述、不编、不用工具），
/// 而日志里一切正常。两种写法必须在**同一处**归一。
#[test]
fn blank_strings_are_same_as_absent() {
    let c: ReplyConfig = serde_json::from_str(r#"{"model": "", "instruction": "   "}"#).unwrap();
    assert!(c.model_id().is_none());
    assert!(c.instruction_override().is_none());
}
