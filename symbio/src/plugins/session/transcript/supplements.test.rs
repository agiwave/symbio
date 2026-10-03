//! `transcript/supplements.rs` 的单元测试 —— 合并规则的形状与无损性。
//!
//! 与实现同级分文件（约定：`X.rs` + `X.test.rs`）。
//!
//! 本文件只回答"合并后那条消息长什么样"；"抽干几个""什么时候抽"由
//! `inbox.test.rs` 与 e2e（`e2e/cases/t19-supplements-merge.mjs`）负责。

use super::*;
use crate::symbio_core::chat_message as cm;
use crate::symbio_core::session_chat;

/// 造一条收件箱条目（params 与本模块无关，取默认）。
fn item(id: &str, text: &str) -> InboxItem {
    InboxItem {
        id: id.to_string(),
        message: cm::ChatMessage {
            id: id.to_string(),
            role: Some(cm::MessageRole::User),
            msg_type: Some(cm::MessageType::Text),
            content: Some(cm::MessageContent::Text(text.to_string())),
            status: Some(cm::MessageStatus::Completed),
            ..Default::default()
        },
        params: session_chat::Request::default(),
        workdir: None,
    }
}

/// 空批次没有可合并的东西 —— 返回 `None`，由调用方决定这是"没有补充"。
#[test]
fn empty_batch_has_nothing_to_merge() {
    assert!(merge_supplements(&[]).is_none());
}

/// **平凡值**：只有一条时原样返回，**一个字段都不多**。
///
/// 这条钉住的是"开启补充整合不会改变只发一条时的任何字节"——最常见的情况
/// 必须与今天逐字一致，否则整个特性从第一秒起就在改行为。
#[test]
fn single_item_is_returned_verbatim() {
    let only = item("m1", "只有一条");
    let merged = merge_supplements(std::slice::from_ref(&only)).expect("一条也应有结果");

    assert_eq!(merged.id, "m1");
    assert_eq!(
        merged.content.as_ref().map(|c| c.to_text()).unwrap(),
        "只有一条"
    );
    assert!(
        merged.meta.is_none(),
        "一条不算补充整合，不得加任何 meta 标记（实得 {:?}）",
        merged.meta
    );
    assert_eq!(merged.parent_id, None);
}

/// 多条按**入队顺序**拼接，连接符是空行；id 取第一条。
#[test]
fn many_are_joined_in_order_with_blank_line() {
    let merged = merge_supplements(&[
        item("a", "第一句"),
        item("b", "第二句"),
        item("c", "第三句"),
    ])
    .expect("三条应有结果");

    assert_eq!(merged.id, "a", "id 取第一条（地址末段即身份）");
    assert_eq!(
        merged.content.as_ref().map(|c| c.to_text()).unwrap(),
        "第一句\n\n第二句\n\n第三句"
    );
    assert_eq!(merged.role, Some(cm::MessageRole::User));
    assert_eq!(merged.msg_type, Some(cm::MessageType::Text));
    assert_eq!(merged.status, Some(cm::MessageStatus::Completed));
    assert_eq!(
        merged.parent_id, None,
        "顶层：find_turn_user_split_idx 只认根级 User"
    );
}

/// 标记必须**同时**给出可追溯性（全部原始 id）与条数。
///
/// 合并后原始条目只此一处留痕：丢了 `supplement_ids` 就再也说不清用户发了几条。
#[test]
fn markers_carry_all_ids_and_the_count() {
    let merged = merge_supplements(&[item("a", "甲"), item("b", "乙")]).expect("应有结果");
    let meta = merged.meta.expect("多条必须带 meta");

    assert_eq!(meta[META_SUPPLEMENT], serde_json::json!(true));
    assert_eq!(
        meta[META_SUPPLEMENT_IDS],
        serde_json::json!(["a", "b"]),
        "按入队顺序列出全部原始条目 id"
    );
    assert_eq!(meta[META_SUPPLEMENT_COUNT], serde_json::json!(2));
}

/// 正文里的空行、换行、markdown 一律**原样保留**（不做摘要、不做改写）。
#[test]
fn user_text_is_not_rewritten() {
    let merged = merge_supplements(&[item("a", "# 标题\n\n- 项 1"), item("b", "  缩进 保留  ")])
        .expect("应有结果");

    assert_eq!(
        merged.content.as_ref().map(|c| c.to_text()).unwrap(),
        "# 标题\n\n- 项 1\n\n  缩进 保留  "
    );
}

/// 缺 `content` 的条目按空串参与拼接（不 panic、不吞掉连接符）。
#[test]
fn missing_content_participates_as_empty() {
    let mut empty = item("b", "");
    empty.message.content = None;
    let merged = merge_supplements(&[item("a", "甲"), empty]).expect("应有结果");

    assert_eq!(
        merged.content.as_ref().map(|c| c.to_text()).unwrap(),
        "甲\n\n"
    );
}

/// **无损性**：只要有一条带非文本片段，图片就必须留在合并结果里。
///
/// 用 `to_text()` 一把梭会把图片悄悄吃掉——"静默丢内容"是最坏的一类失败，
/// 因此这里钉住形态升级为 `Parts` 的那条路径。
#[test]
fn non_text_parts_survive_the_merge() {
    let mut with_image = item("b", "");
    with_image.message.content = Some(cm::MessageContent::Parts(vec![
        cm::ContentPart::Text {
            text: "看这张图".to_string(),
        },
        cm::ContentPart::ImageUrl {
            image_url: cm::ImageUrl {
                url: "https://example.com/x.png".to_string(),
                detail: None,
            },
        },
    ]));

    let merged = merge_supplements(&[item("a", "甲"), with_image]).expect("应有结果");
    match merged.content.expect("必须有正文") {
        cm::MessageContent::Parts(parts) => {
            assert_eq!(parts.len(), 4, "甲 + 连接符 + 文本 + 图片");
            assert!(
                matches!(parts[3], cm::ContentPart::ImageUrl { .. }),
                "图片必须原样保留，实得 {:?}",
                parts[3]
            );
        }
        cm::MessageContent::Text(t) => panic!("含图片时不得退化为纯文本（会丢图）：{t}"),
    }
}

/// 纯文本路径**不**升级成 `Parts`（最常见路径保持最简形态）。
#[test]
fn text_only_stays_text() {
    let merged = merge_supplements(&[item("a", "甲"), item("b", "乙")]).expect("应有结果");
    assert!(
        matches!(merged.content, Some(cm::MessageContent::Text(_))),
        "纯文本不必升级为 Parts"
    );
}
