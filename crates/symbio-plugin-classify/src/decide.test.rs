//! `classify/classify.rs` 的单元测试 —— 解析器的**全部行为**。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。
//!
//! `classify()` 本身要一个真实的 `ModelProvider`（属于 e2e 的范畴，见
//! `e2e/cases/t21-classify.mjs`）；本文件钉住它**不依赖模型**的那一半——
//! 「模型说了什么」到「判决是什么」的映射，以及「说不清时返回什么」。

use super::*;
use symbio_core::chat_node::MessageContent;

fn reason_of(v: &Verdict) -> &str {
    match v {
        Verdict::Answered { reason } | Verdict::Escalate { reason } => reason,
        other => panic!("分类器不应产出 {other:?}"),
    }
}

/// 提示词要求的正常路径：一个词
#[test]
fn single_word_maps_to_verdict() {
    assert_eq!(
        parse_verdict("direct"),
        Some(Verdict::Answered {
            reason: REASON_FROM_CONTEXT.to_string()
        })
    );
    assert_eq!(
        parse_verdict("clarify"),
        Some(Verdict::Answered {
            reason: REASON_CLARIFY.to_string()
        })
    );
    assert_eq!(
        parse_verdict("refuse"),
        Some(Verdict::Answered {
            reason: REASON_REFUSE.to_string()
        })
    );
    assert_eq!(
        parse_verdict("work"),
        Some(Verdict::Escalate {
            reason: REASON_NEEDS_WORK.to_string()
        })
    );
}

/// `work` 是**唯一**进工具循环的判决：三个 `Answered` 与它的分界由词表决定
#[test]
fn only_work_escalates() {
    let escalating: Vec<&str> = CHOICES
        .iter()
        .filter(|(_, r)| matches!(verdict_of(r), Verdict::Escalate { .. }))
        .map(|(w, _)| *w)
        .collect();
    assert_eq!(escalating, vec!["work"]);
}

/// 大小写 / 标点 / 前后空白都不该让一个正确的词失效
#[test]
fn noise_around_the_word_is_tolerated() {
    for raw in ["work\n", "  WORK  ", "work.", "\"work\"", "Work。"] {
        assert_eq!(
            reason_of(&parse_verdict(raw).expect("应能解析出判决")),
            REASON_NEEDS_WORK,
            "「{raw}」应解析为 work"
        );
    }
}

/// 模型多说了几句时退化到「全文最早出现的那个词」——救回可救的
#[test]
fn verbose_answer_falls_back_to_first_word_mention() {
    let v = parse_verdict("这需要读文件，所以我选 work。\n（理由：用户要求读 README）")
        .expect("应能解析出判决");
    assert_eq!(reason_of(&v), REASON_NEEDS_WORK);
}

/// **失败方向**：解析不出来时返回 `None`，由调用方落 `Escalate`。
///
/// 这条断言的意义不在「返回 None」，而在**绝不返回 `Answered`**：
/// 一次分类故障若被当成「不用干活」，用户拿到的是沉默——没有任何错误信号。
#[test]
fn unparsable_text_yields_none_not_answered() {
    for raw in ["", "   ", "\n\n", "我不知道", "42", r#"{"verdict":"zzz"}"#] {
        assert_eq!(parse_verdict(raw), None, "「{raw}」不该解析出判决");
    }
}

/// 身份写 `principal`（D-147）：夹具模仿的是**落库面**的消息，`role` 只是读侧残留格。
fn principal_for(role: NodeRole) -> symbio_core::principal::Principal {
    use symbio_core::principal::Principal;
    match role {
        NodeRole::User => Principal::from_legacy(symbio_core::PRINCIPAL_USER),
        NodeRole::Assistant => Principal::from_legacy("session/main"),
        NodeRole::Tool => Principal::from_legacy("legacy/unknown"),
        NodeRole::System => Principal::from_legacy("system/local"),
    }
}

fn node(role: NodeRole, text: &str) -> ChatNode {
    ChatNode {
        principal: Some(principal_for(role)),
        content: Some(MessageContent::Text(text.to_string())),
        ..Default::default()
    }
}

/// 空对话线（仓外调用方直呼本路由）时，用这句话自身组一条 user 消息
#[test]
fn empty_context_falls_back_to_the_utterance_itself() {
    let msgs = build_messages("读一下 README", &[]);
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].role, "user");
    assert_eq!(msgs[0].text, "读一下 README");
}

/// 投影里已含本轮这句话时**不重复追加**——否则同一句话在请求里出现两次，
/// 分类器会以为用户说了两遍（真实症状：对「重复的诉求」判出更重的意图）
#[test]
fn tail_utterance_is_not_appended_twice() {
    let ctx = vec![
        node(NodeRole::User, "你好"),
        node(NodeRole::Assistant, "你好，有什么可以帮你？"),
        node(NodeRole::User, "读一下 README"),
    ];
    let msgs = build_messages("读一下 README", &ctx);
    assert_eq!(msgs.len(), 3, "不得追加第四条");
    assert_eq!(msgs.last().unwrap().text, "读一下 README");
}

/// 空正文节点不进请求包（它们对分类没有信息量，只占上下文）
#[test]
fn blank_nodes_are_dropped() {
    let ctx = vec![
        node(NodeRole::User, "  "),
        node(NodeRole::User, "读一下 README"),
    ];
    let msgs = build_messages("读一下 README", &ctx);
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].text, "读一下 README");
}

/// 角色词与工具身份原样过端口：端口的消息形态是 `PromptMessage`，不是
/// `ChatNode`——映射在这里丢掉字段，分类请求就会把「工具结果」伪装成用户发言。
#[test]
fn roles_and_tool_identity_survive_the_port_shape() {
    let mut tool = node(NodeRole::Tool, "文件内容").with_tool_name(Some("vfs_read".to_string()));
    tool.tool_call_id = Some("call_r1".to_string());
    let ctx = vec![node(NodeRole::Assistant, "我先读一下"), tool];

    let msgs = build_messages("继续", &ctx);
    assert_eq!(msgs[0].role, "assistant");
    assert_eq!(msgs[1].role, "tool");
    assert_eq!(msgs[1].tool_call_id.as_deref(), Some("call_r1"));
    assert_eq!(msgs[1].tool.as_deref(), Some("vfs_read"));
}
