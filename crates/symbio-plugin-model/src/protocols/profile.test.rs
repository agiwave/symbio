//! 协议参数档案的判据。
//!
//! 这张表的价值全在「只写一遍」——所以判据也围绕这一点：值对、链序对、
//! 且解析代码里不再出现同类字面量（后者由 `grep-audit` 的守卫钉住，见
//! ）。

use super::*;
use serde_json::json;

/// 四个协议各一行，两个数都取**今天的实测值**。
///
/// 这条用例的作用不是「记住数字」，而是让任何一次改动都必须**显式**改表：
/// 探测值写小了会被网关拒（GLM 要求 `max_tokens > 2`），写大了每次探活都白花钱。
#[test]
fn every_protocol_row_keeps_its_measured_budget() {
    for (name, p) in [
        ("openai_chat", &OPENAI_CHAT),
        ("openai_responses", &OPENAI_RESPONSES),
        ("anthropic_messages", &ANTHROPIC_MESSAGES),
        ("gemini_api", &GEMINI_API),
    ] {
        assert_eq!(p.default, 8192, "{name} 的生成缺省");
        assert!(p.probe > 2, "{name} 的探测值必须过 GLM 那条 `> 2` 的下限");
    }
    assert_eq!(OPENAI_CHAT.probe, 256, "OpenAI 兼容生态的下限最严");
}

/// fallback 链按**表里的顺序**取，`null` 视为未命中。
///
/// MiMo 那个实测形态（首帧 `reasoning_content: null` + `reasoning: "…"`）正是这条
/// 要钉的：只认第一个键的实现会把思考内容静默丢掉。
#[test]
fn the_chain_skips_nulls_and_follows_the_table_order() {
    let mimo = json!({ "reasoning_content": null, "reasoning": "思考正文" });
    assert_eq!(
        first_str(&mimo, OPENAI_CHAT_REASONING_KEYS),
        Some("思考正文"),
        "`null` 不能挡住后面的键"
    );

    let deepseek = json!({ "reasoning_content": "第一", "reasoning": "第二" });
    assert_eq!(
        first_str(&deepseek, OPENAI_CHAT_REASONING_KEYS),
        Some("第一"),
        "顺序由表决定，不由对象的键序决定"
    );

    assert_eq!(first_str(&json!({}), OPENAI_CHAT_REASONING_KEYS), None);
}

/// Anthropic 那三组键也走同一个函数——两处 fallback 合成一处是这条的实质。
#[test]
fn the_anthropic_keys_use_the_same_extractor() {
    let typed = json!({ "thinking": null, "thought": "推理", "reasoning": "兜底" });
    assert_eq!(first_str(&typed, ANTHROPIC_THINKING_KEYS), Some("推理"));

    let inline = json!({ "text": "正文", "reasoning_content": "夹带的思考" });
    assert_eq!(
        first_str(&inline, ANTHROPIC_TEXT_DELTA_REASONING_KEYS),
        Some("夹带的思考")
    );

    assert_eq!(
        ANTHROPIC_THINKING_DELTA_TYPES,
        &["thinking_delta", "thought_delta", "reasoning_delta"],
        "带类型事件的类型名同样是数据"
    );
}
