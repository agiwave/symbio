//! `model_chat` 模块的单元测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）：
//! `model_chat.rs` 只保留生产代码，测试全部放本文件。

use super::*;
use crate::symbio_core::schemas::session::chat_message::ResumeAction;

/// 全字段显式赋值（`Some`）的样本，供键集合契约使用。
///
/// **刻意不使用 `..Default::default()`**：这样一旦 `Request` 新增字段，
/// 本函数直接编译失败，强制作者把它加进契约测试——避免"新字段带着
/// `skip_serializing_if` 悄悄加入、键集合契约被无声破坏"。
fn fully_populated() -> Request {
    Request {
        system_prompt: Some("p".into()),
        single_message: Some(ChatMessage::default()),
        stream: Some(false),
        max_tool_rounds: Some(3),
        tool_context_window: Some(4),
        auto_compress: Some(false),
        enable_compact_tool: Some(true),
        provider_id: Some("pid".into()),
        load_history: Some(false),
        resume: Some(ResumeRequest {
            target_id: "t1".into(),
            action: ResumeAction::RetryTurn,
            args: None,
            reason: None,
            answer: None,
        }),
    }
}

fn sorted_keys(v: &serde_json::Value) -> Vec<String> {
    let mut k: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
    k.sort();
    k
}

/// 跨语言契约：序列化输出的**键集合恒定**——`None` 一律输出 `null`，
/// 不随字段取值增删。历史上仅 `stream` 无 `skip_serializing_if`，其余字段有，
/// 同一协议在不同取值下键集不同，消费方无法稳定判别"未设置"。
#[test]
fn request_key_set_is_value_independent() {
    let empty = serde_json::to_value(Request::default()).unwrap();
    let full = serde_json::to_value(fully_populated()).unwrap();

    assert_eq!(
        sorted_keys(&empty),
        sorted_keys(&full),
        "键集合必须与取值无关"
    );
    // 且 None 字段以显式 null 存在（而非缺键）
    assert_eq!(empty["load_history"], serde_json::Value::Null);
}

/// 契约另一侧：`None`（未表态）**不得**与零值混淆——故字段保持 `Option` 且
/// 缺键/`null` 等价可反序列化（调用方省略字段时不翻转为 `false` / `0`）。
#[test]
fn absent_fields_deserialize_as_none_not_zero() {
    let r: Request = serde_json::from_str("{}").unwrap();
    assert_eq!(
        r.auto_compress, None,
        "缺键必须是 None（chat_loop 据此取默认 true）"
    );
    assert_eq!(
        r.max_tool_rounds, None,
        "缺键必须是 None（0 会被解释成 0 轮熔断）"
    );
    assert_eq!(r.load_history, None, "缺键必须是 None（默认加载历史）");

    let r2: Request = serde_json::from_str(
        r#"{"auto_compress":null,"max_tool_rounds":null,"load_history":null}"#,
    )
    .unwrap();
    // `null` 与缺键等价（Request 未派生 PartialEq，逐字段比对）
    assert_eq!(r2.auto_compress, r.auto_compress);
    assert_eq!(r2.max_tool_rounds, r.max_tool_rounds);
    assert_eq!(r2.load_history, r.load_history);
}

/// 旧调用方仍可能带已删除的 `thinking` 死字段：serde 默认忽略未知键，须保持兼容。
#[test]
fn unknown_fields_are_tolerated() {
    let r: Request =
        serde_json::from_str(r#"{"stream":true,"thinking":{"enabled":true}}"#).unwrap();
    assert_eq!(r.stream, Some(true));
}
