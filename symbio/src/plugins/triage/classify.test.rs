//! `triage/classify.rs` 的单元测试 —— 解析器的**全部行为**。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。
//!
//! `classify()` 本身要一个真实的 `ModelProvider`（属于 e2e 的范畴，见
//! `e2e/cases/t21-triage.mjs`）；本文件钉住它**不依赖模型**的那一半——
//! 「模型说了什么」到「判决是什么」的映射，以及「说不清时返回什么」。

use super::*;

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

/// 空对话线（仓外调用方直呼本路由）时，用这句话自身组一条 user 消息
#[test]
fn empty_context_falls_back_to_the_utterance_itself() {
    let msgs = build_messages("读一下 README", &[]);
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].role, Some(MessageRole::User));
    assert_eq!(text_of(&msgs[0]), "读一下 README");
}

/// 投影里已含本轮这句话时**不重复追加**——否则同一句话在请求里出现两次，
/// 分类器会以为用户说了两遍（真实症状：对「重复的诉求」判出更重的意图）
#[test]
fn tail_utterance_is_not_appended_twice() {
    let ctx = vec![
        user_message("你好"),
        ChatMessage {
            role: Some(MessageRole::Assistant),
            content: Some(MessageContent::Text("你好，有什么可以帮你？".to_string())),
            ..Default::default()
        },
        user_message("读一下 README"),
    ];
    let msgs = build_messages("读一下 README", &ctx);
    assert_eq!(msgs.len(), 3, "不得追加第四条");
    assert_eq!(text_of(msgs.last().unwrap()), "读一下 README");
}

/// 空正文节点不进请求包（它们对分类没有信息量，只占上下文）
#[test]
fn blank_nodes_are_dropped() {
    let ctx = vec![user_message("  "), user_message("读一下 README")];
    let msgs = build_messages("读一下 README", &ctx);
    assert_eq!(msgs.len(), 1);
    assert_eq!(text_of(&msgs[0]), "读一下 README");
}
