//! 委派判定的单元测试：每条判据各一正一反，外加平凡值与确定性。
use super::*;

fn cfg() -> DelegateConfig {
    DelegateConfig::default()
}

/// 平凡值：关掉开关 ⇒ 恒 `Chat`，且理由说得清楚（不是静默）。
#[test]
fn disabled_always_chats() {
    let mut c = cfg();
    c.enabled = false;
    let d = decide("/work 帮我重构整个模块", &c);
    assert_eq!(d.dispatch, Dispatch::Chat);
    assert!(d.reason.contains("已关闭"), "理由应指向开关：{}", d.reason);
}

/// 显式前缀：用户直接指挥 ⇒ 必开（哪怕同时命中其它判据也一样）。
#[test]
fn explicit_prefix_wins() {
    let d = decide("/work 帮我改一下", &cfg());
    assert!(d.should_delegate());
    assert!(
        d.reason.contains("/work"),
        "理由带上命中的前缀：{}",
        d.reason
    );
}

/// 前缀是**逐字**匹配：`/worker` 不该被 `/work` 命中（尾空格的作用）。
#[test]
fn prefix_does_not_match_longer_word() {
    let d = decide("/worker 是什么", &cfg());
    assert_eq!(
        d.dispatch,
        Dispatch::Chat,
        "/worker 不是指令（前缀 /work 不匹配它）"
    );
}

/// 关键词：子串、忽略大小写（出厂空集——这里显式配上再验）。
#[test]
fn keyword_hit_is_case_insensitive() {
    let mut c = cfg();
    c.keywords = vec!["重构".to_string(), "Deploy".to_string()];

    let hit = decide("帮我把这块重构一下", &c);
    assert!(hit.should_delegate(), "中文关键词命中");
    assert!(
        hit.reason.contains("重构"),
        "理由带上命中的词：{}",
        hit.reason
    );

    let upper = decide("please DEPLOY the service", &c);
    assert!(upper.should_delegate(), "英文关键词忽略大小写");

    let miss = decide("你好呀", &c);
    assert_eq!(miss.dispatch, Dispatch::Chat);
}

/// 空关键词项被跳过：配置里留空行不该让**一切**都命中（静默误判）。
#[test]
fn blank_keyword_never_matches() {
    let mut c = cfg();
    c.keywords = vec!["".to_string(), "   ".to_string()];
    let d = decide("随便一句", &c);
    assert_eq!(d.dispatch, Dispatch::Chat);
}

/// 长度阈值：`0` = 不启用；配了则按**字符数**（不是字节）。
#[test]
fn length_threshold_counts_chars() {
    let mut c = cfg();
    c.min_chars = 0;
    assert_eq!(
        decide("一二三四五六七八九十十", &c).dispatch,
        Dispatch::Chat
    );

    c.min_chars = 5;
    let d = decide("一二三四五", &c);
    assert!(
        d.should_delegate(),
        "5 个汉字 = 5 字符（按字节会误判为 15）"
    );
    assert!(d.reason.contains('5'), "理由带上实测字符数：{}", d.reason);
}

/// 空消息：不判"要动手"（没有内容就没有任务）。
#[test]
fn empty_message_chats() {
    assert_eq!(decide("", &cfg()).dispatch, Dispatch::Chat);
    assert_eq!(decide("   \n ", &cfg()).dispatch, Dispatch::Chat);
}

/// 确定性（A4）：同一输入双跑结论与理由逐字相同。
#[test]
fn decision_is_deterministic() {
    let c = cfg();
    let a = decide("帮我把这个重构一下", &c);
    let b = decide("帮我把这个重构一下", &c);
    assert_eq!(a, b);
}
