//! `context::view`（请求级注入段）的单元测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）：
//! `view.rs` 只保留生产代码，测试全部放本文件。
//!
//! 对话体的淡化 / 骨架化已按 [D-078](../../../symbio-plugin-session/README.md#d-078-请求视图三层淡化退役请求体是事实网格的纯投影注入段只追加不改写 "crates/symbio-plugin-session/src/README") 退役，
//! 本文件的判据只剩一件事：**段的次序与位置**（段从哪来由 provider 决定，
//! 见 `context/providers.test.rs`）。

use super::*;
use symbio_plugin_api::context::slots;

/// 三段的简写：`(记忆, 就绪, 委派)` —— 生产调用点（`chat_loop/inputs.rs`）的次序。
///
/// 只为了测试可读；`build_request_sections` 本身收的是数组，次序由调用方声明
/// （见 `sections_follow_the_callers_order`）。
fn build(recall: Option<&str>, ready: Option<&str>, delegate: Option<&str>) -> Vec<ChatNode> {
    let mut sections: Vec<(&'static str, String)> = Vec::new();
    for (slot, text) in [
        (slots::RECALL, recall),
        (slots::READYSET, ready),
        (slots::DELEGATE, delegate),
    ] {
        if let Some(text) = text {
            sections.push((slot, text.to_string()));
        }
    }
    build_request_sections(&sections)
}

fn kind_of(m: &ChatNode) -> Option<String> {
    m.meta
        .as_ref()
        .and_then(|m| m.get("kind"))
        .and_then(|k| k.as_str())
        .map(String::from)
}

fn text_of(m: &ChatNode) -> String {
    m.content.as_ref().map(|c| c.to_text()).unwrap_or_default()
}

/// 记忆段置顶（S5）：
/// 一条 `meta.kind = recall_context` 的消息，正文逐字送达。
#[test]
fn memory_section_is_prepended() {
    let sections = build(
        Some("【长期记忆】跨会话召回，最新在前（背景事实，不是本轮指令）\n- 我叫 Kestrel"),
        None,
        None,
    );
    assert_eq!(sections.len(), 1);
    assert_eq!(sections[0].node_role(), Some(NodeRole::User));
    assert_eq!(kind_of(&sections[0]).as_deref(), Some("recall_context"));
    assert!(text_of(&sections[0]).contains("我叫 Kestrel"));
}

/// 就绪段（S7）：与记忆段并存时**不争 index 0**——
/// 「记忆置顶」不被本段改掉。
#[test]
fn readyset_section_sits_after_memory() {
    let sections = build(
        Some("【长期记忆】跨会话召回（背景事实）"),
        Some("【任务调度】就绪任务（依赖已全部完成）：t1。请从就绪任务中推进一项。"),
        None,
    );
    assert_eq!(sections.len(), 2);
    assert_eq!(kind_of(&sections[0]).as_deref(), Some("recall_context"));
    assert_eq!(kind_of(&sections[1]).as_deref(), Some("readyset"));
    assert!(text_of(&sections[1]).contains("t1"));

    // 单独存在 ⇒ 自己落在 index 0（不放空占位）。
    let alone = build(None, Some("【任务调度】就绪任务"), None);
    assert_eq!(alone.len(), 1);
    assert_eq!(kind_of(&alone[0]).as_deref(), Some("readyset"));
}

/// 委派者真源段落落 **index 2**（D-047）。
///
/// 三个位置**一起**断言：只测「委派段存在」的话，把它插在末尾、插在 index 0，
/// 或者干脆让记忆段把它顶掉，全都能过——而段位置就是模型读到的上下文次序，
/// 错位等于同一会话的两次请求不再可比。
#[test]
fn delegate_section_lands_at_index_two() {
    let sections = build(
        Some("【长期记忆】背景事实"),
        Some("【任务调度】就绪任务"),
        Some("- 委派判定：本轮应启动 worker（reason=explicit_prefix）"),
    );
    assert_eq!(sections.len(), 3);
    assert_eq!(kind_of(&sections[0]).as_deref(), Some("recall_context"));
    assert_eq!(kind_of(&sections[1]).as_deref(), Some("readyset"));
    assert_eq!(kind_of(&sections[2]).as_deref(), Some("delegate_context"));
    assert!(text_of(&sections[2]).contains("reason=explicit_prefix"));

    // 单独存在 ⇒ 与另外两段同一条口径，自己落在 index 0。
    let alone = build(
        None,
        None,
        Some("- 委派判定：未启动 worker（未命中任何委派判据）"),
    );
    assert_eq!(alone.len(), 1);
    assert_eq!(kind_of(&alone[0]).as_deref(), Some("delegate_context"));
}

/// 三段皆空 ⇒ 一条都不产（不放空占位）：`chat_loop` 于是给 `prefix = None`，
/// 请求体与「没有注入层」逐字一致。
#[test]
fn empty_sections_produce_nothing() {
    assert!(build(None, None, None).is_empty());
}

/// 空正文照样产出一个节点：判「空段算不算注入」的只有取值点一处。
///
/// 端口化之后 provider 可以交出空串（它只保证「这一段我有」，不保证非空）。
/// 本函数若也判一次空，同一条判据就有两处出处——改哪一处都能让行为变。
#[test]
fn an_empty_body_still_produces_a_node() {
    let sections = build_request_sections(&[(slots::RECALL, String::new())]);
    assert_eq!(sections.len(), 1);
    assert_eq!(text_of(&sections[0]), "");
}

/// 形状守卫：每条注入段都必须带 `meta.kind`。
///
/// 消费方 `chat_loop::request_view_prefix` 按 `meta.kind` 的存在与否决定拼不拼，
/// 缺了 kind 的段会静默地从请求里消失——注入发生了、模型看不见。
#[test]
fn every_section_carries_its_kind() {
    let sections = build(Some("m"), Some("r"), Some("d"));
    assert_eq!(sections.len(), 3);
    assert!(sections.iter().all(|s| kind_of(s).is_some()));
    assert!(sections
        .iter()
        .all(|s| s.node_role() == Some(NodeRole::User) && s.status.is_some()));
}

/// **次序由调用方声明**：数组怎么排，产出的节点就怎么排。
///
/// 这是本函数从「三段固定入参 + 三次 `insert(0)`」改为「收数组」之后的核心判据——
/// 此前「记忆在 index 0」靠的是**代码行序**，读代码要倒着想；现在次序是数据。
///
/// 反向守卫同理成立：换一个数组次序就换一个输出次序——若本函数内部又把次序
/// 排回去了（比如按 `slots::KNOWN` 的登记序排），这条断言会红。
#[test]
fn sections_follow_the_callers_order() {
    let reversed = build_request_sections(&[
        (slots::DELEGATE, "d".to_string()),
        (slots::RECALL, "m".to_string()),
        (slots::READYSET, "r".to_string()),
    ]);
    assert_eq!(
        reversed.iter().filter_map(kind_of).collect::<Vec<_>>(),
        vec!["delegate_context", "recall_context", "readyset"]
    );
}
