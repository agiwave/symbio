//! `classify/rules.rs` 的单元测试 —— 规则表的**正例与反例**。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。
//!
//! 本文件的重点是**反例**：规则短路是全仓唯一一处「不调用模型就替用户下判决」的
//! 地方，它误判的代价是**静默丢请求**（见 `rules.rs` 的模块文档）。因此
//! 「哪些话**不能**命中」与「哪些话命中」同等重要，且必须逐条钉住。

use super::*;

/// 问候：归一化后全等即命中（含大小写 / 标点 / 空白变体）
#[test]
fn greetings_hit() {
    for raw in [
        "你好",
        " 你好 ",
        "你好！",
        "您好。",
        "Hi",
        "HELLO!",
        "嗨",
        "在吗？",
    ] {
        assert_eq!(
            classify_by_rule(raw),
            Some(REASON_GREETING),
            "「{raw}」应命中问候"
        );
    }
}

/// 致谢
#[test]
fn thanks_hit() {
    for raw in [
        "谢谢",
        "谢谢你！",
        "多谢",
        "感谢",
        "Thanks",
        "THANK YOU",
        "辛苦了",
    ] {
        assert_eq!(
            classify_by_rule(raw),
            Some(REASON_THANKS),
            "「{raw}」应命中致谢"
        );
    }
}

/// 确认 / 收到
#[test]
fn acks_hit() {
    for raw in ["好", "好的", "嗯", "OK", "okay", "收到", "明白", "知道了"] {
        assert_eq!(
            classify_by_rule(raw),
            Some(REASON_ACK),
            "「{raw}」应命中确认"
        );
    }
}

/// 空输入：空串 / 纯空白 / 纯标点 —— 「用户什么都没说」，不是「要干活」
#[test]
fn empty_inputs_hit() {
    for raw in ["", "   ", "\t\n", "？", "。。。", "  ！  "] {
        assert_eq!(
            classify_by_rule(raw),
            Some(REASON_EMPTY),
            "「{raw}」应命中空输入"
        );
    }
}

/// **反例（本文件最重要的一条）**：带真实请求的问候**不得**被短路。
///
/// 若这条红了，说明匹配从「全等」退化成了「包含」——那时用户说
/// 「你好，帮我读一下 README」会拿到一个 `Answered`，而**没有任何人干活**，
/// 且不产生任何错误信号。
#[test]
fn greetings_with_a_real_request_do_not_hit() {
    for raw in [
        "你好，帮我读一下 README",
        "你好 我想问个问题",
        "谢谢，不过我还是想改一下",
        "好的，那就这样做吧，另外帮我看看日志",
        "嗯，那你继续",
    ] {
        assert_eq!(
            classify_by_rule(raw),
            None,
            "「{raw}」含真实请求，不得被反射档短路"
        );
    }
}

/// 普通请求一律不命中（表只覆盖礼节，不覆盖业务）
#[test]
fn ordinary_requests_do_not_hit() {
    for raw in [
        "我们刚才聊了什么",
        "读一下 README",
        "帮我写一个脚本",
        "现在几点",
        "把上次那个 bug 修一下",
    ] {
        assert_eq!(classify_by_rule(raw), None, "「{raw}」不该命中规则表");
    }
}

/// 内部标点**保留**：带补充说明的致谢是一条真实补充，不该归成纯致谢
#[test]
fn internal_punctuation_is_preserved() {
    assert_eq!(classify_by_rule("谢谢，不用了"), None);
}

/// 表本身没有空项、没有重复（表是数据，数据也有不变量）
#[test]
fn table_has_no_blank_or_duplicate_entries() {
    for entry in TABLE {
        let (reason, patterns): (&str, &[&str]) = *entry;
        assert!(!reason.is_empty(), "理由码不得为空");
        assert!(!patterns.is_empty(), "理由码 {reason} 的模式集不得为空");
        for p in patterns {
            let p: &str = p;
            assert!(!p.is_empty(), "理由码 {reason} 下有空模式");
            assert_eq!(
                normalize(p),
                p,
                "模式「{p}」未归一化——用户输入的归一化结果永远匹配不上它"
            );
        }
        let mut sorted: Vec<&str> = patterns.to_vec();
        sorted.sort_unstable();
        let n = sorted.len();
        sorted.dedup();
        assert_eq!(sorted.len(), n, "理由码 {reason} 的模式集有重复项");
    }
}
