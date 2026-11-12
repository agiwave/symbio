//! 一次性问答契约的单元测试 —— 载荷键名与地址段的**跨端取值**。

use super::*;

/// 载荷的**键名是契约的一部分**：这些键进 `model.assist` 行的线格式，前端（Tauri
/// 命令参数）与外部网关都按它们拼 JSON——改名 = 跨栈同步，改值没有任何东西会红。
#[test]
fn assist_payload_keys_are_the_wire_contract() {
    let req = AssistRequest {
        system: "s".into(),
        user: "u".into(),
        provider: Some("m".into()),
    };
    let json = serde_json::to_value(&req).expect("序列化是契约自身的义务");
    for key in ["system", "user", "provider"] {
        assert!(json.get(key).is_some(), "请求键 `{key}` 是线格式的一部分");
    }

    let reply = AssistReply {
        text: "t".into(),
        reasoning: Some("r".into()),
        reasoning_id: Some("id".into()),
        cost_ms: 7,
    };
    let json = serde_json::to_value(&reply).expect("序列化是契约自身的义务");
    for key in ["text", "reasoning", "reasoning_id", "cost_ms"] {
        assert!(json.get(key).is_some(), "回复键 `{key}` 是线格式的一部分");
    }
}

/// `serde(default)` 是兼容性承诺：旧调用方少带键不报错——缺省语义属于契约。
#[test]
fn assist_payload_defaults_keep_old_callers_alive() {
    let req: AssistRequest = serde_json::from_str("{}").expect("空载荷 = 全缺省");
    assert_eq!(req, AssistRequest::default());

    let reply: AssistReply = serde_json::from_str("{}").expect("空载荷 = 全缺省");
    assert_eq!(reply, AssistReply::default());
}

/// 转写地址段的**字面值**是契约的一部分：session 写面与 agent 读面各拼一份地址，
/// 两边不一致 = 读不到对方的转写，且没有类型系统兜底。
#[test]
fn seg_messages_literal_is_the_address_contract() {
    assert_eq!(SEG_MESSAGES, "message");
}
