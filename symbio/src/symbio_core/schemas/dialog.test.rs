//! 对话面契约的单元测试 —— 理由码的**跨端取值**。

use super::*;

/// 理由码的**字面值**是契约的一部分，改名要跨栈同步。
///
/// 这三个字不进类型系统（`reason: String`），所以没有任何东西会因为把它改成
/// `hi` 而变红；但 `session` 把它原样写进转写节点的 `meta.reason`
/// （`chat_loop/compose.rs` 的 `dialog_node`），磁盘上的老节点带着旧字面量——
/// 改了值，老对话就再也读不回它当时的理由。
#[test]
fn reason_code_literals_are_the_wire_contract() {
    assert_eq!(REASON_GREETING, "greeting");
    assert_eq!(REASON_THANKS, "thanks");
    assert_eq!(REASON_ACK, "ack");
    assert_eq!(REASON_EMPTY, "empty");
    assert_eq!(REASON_FROM_CONTEXT, "from_context");
    assert_eq!(REASON_CLARIFY, "clarify");
    assert_eq!(REASON_REFUSE, "refuse");
    assert_eq!(REASON_NEEDS_WORK, "needs_work");
    assert_eq!(REASON_UNCLASSIFIED, "unclassified");
}

/// **判决是闭集，措辞是文本**：`Verdict` 的 serde 形状决定外部调用方能发什么，
/// 而 `reason` 一侧必须**照原样**收——把它收紧成枚举就是本模块文档否掉的那件事。
#[test]
fn verdict_wire_shape_carries_the_reason_verbatim() {
    let v: Verdict = serde_json::from_str(r#"{"verdict":"answered","reason":"greeting"}"#).unwrap();
    assert_eq!(
        v,
        Verdict::Answered {
            reason: "greeting".into()
        }
    );

    let escalated: Verdict =
        serde_json::from_str(r#"{"verdict":"escalate","reason":"任何词"}"#).unwrap();
    assert_eq!(
        escalated,
        Verdict::Escalate {
            reason: "任何词".into()
        },
        "未知理由码必须原样收下：外部调用方（网关）会发来自本仓词表之外的值"
    );
}
