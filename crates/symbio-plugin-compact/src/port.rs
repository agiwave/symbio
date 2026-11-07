//! `compact` 端口的执行面：**读回摘要，落压缩快照**（D-336 的第二行）。
//!
//! 摘要不由本端口取——那是 `model.assist` 的事（执行体归端口提供方）。本端口只做
//! 世界侧的那一半：
//!
//! 1. 从读面找本轮那笔 `model.assist` 的 `response`（`port` + `final_id` 双匹配），
//!    取回 [`AssistReply::text`]；
//! 2. 落 `session.consolidated`，折叠叶正文 = 摘要。
//!
//! ## 欠不欠由债源决定，不在本执行体判
//!
//! 水位（阈值 0.7 × provider 上限）在 `debt.rs` 申报时判：没超就不欠这两行，本执行体
//! 压根不会被派发。执行体拿不到可信摘要（`model.assist` 失败 / 空转写）⇒ 返回成功、
//! 不落事实——「没压」不是一条领域事实。
//!
//! ## 压出来的快照
//!
//! 载荷 `{ "messages": [ChatNode…] }`：压缩节点是折叠叶
//! （`NodeKind::Notice` + `style_tag = compression`，正文就是摘要）＋本轮的助手终答。
//! 重建面从这份列表开始折，被压掉的前缀不再参与。

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use symbio_plugin_api::chat_node::{
    AssistReply, ChatNode, MessageContent, NodeKind, NodeStatus, CHAT_STYLE_TAG_COMPRESSION,
};
use symbio_plugin_api::port::{
    PortExecContext, PortExecutor, PortFact, PortInput, PortOutcome, PortProvider,
};
use symbio_plugin_api::{Entity, FactDraft, Verb, EVENT_RESPONSE};

use crate::debt::ASSIST_PORT;

/// 端口名（唯一来源：本插件）。
pub(crate) const COMPACT_PORT: &str = "compact";

/// 事实种类：**已有种类**，声明 owner 是
/// `crates/symbio-plugin-session/src/declarations.rs`。本 crate 只写、不声明；
/// 名字由 owner 的清单生成到 `plugin-api`，本 crate import 同一常量（D-364）。
pub(crate) use symbio_plugin_api::events::EVENT_SESSION_CONSOLIDATED as SESSION_CONSOLIDATED;

pub(crate) struct CompactPort;

impl PortProvider for CompactPort {
    fn ports(&self) -> &'static [&'static str] {
        &[COMPACT_PORT]
    }

    fn provider_id(&self) -> &str {
        COMPACT_PORT
    }

    fn executor(&self) -> Option<Arc<dyn PortExecutor>> {
        Some(Arc::new(CompactExecutor))
    }
}

struct CompactExecutor;

#[async_trait]
impl PortExecutor for CompactExecutor {
    async fn execute(&self, ctx: PortExecContext, input: PortInput) -> PortOutcome {
        let root_id = str_of(&input.payload, "root_id");
        let final_id = str_of(&input.payload, "final_id");
        let Some(reader) = ctx.reader.clone() else {
            return PortOutcome::ok();
        };
        let facts = reader.facts();
        let Some(summary) = summary_of(&facts, &final_id) else {
            // 摘要那笔没交回正文（失败 / 空转写）：不拿占位冒充摘要。
            return PortOutcome::ok();
        };
        let messages = snapshot(&facts, &final_id, &summary);
        if messages.is_empty() {
            return PortOutcome::ok();
        }
        ctx.emit
            .emit(vec![FactDraft {
                key: format!("{SESSION_CONSOLIDATED}:{root_id}"),
                kind: SESSION_CONSOLIDATED.to_string(),
                entity: Entity::Message,
                verb: Verb::Progressed,
                payload: json!({ "messages": messages }),
                produced_by: None,
            }])
            .await;
        PortOutcome::ok()
    }
}

fn str_of(payload: &Value, key: &str) -> String {
    payload
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// 本轮 `model.assist` 交回的摘要正文：按端口 + 终答身份双匹配，取最后一条。
fn summary_of(facts: &[PortFact], final_id: &str) -> Option<String> {
    let fact = facts.iter().rev().find(|f| {
        f.port == ASSIST_PORT
            && f.kind == EVENT_RESPONSE
            && f.payload.get("final_id").and_then(Value::as_str) == Some(final_id)
    })?;
    let reply: AssistReply = serde_json::from_value(fact.payload.clone()).ok()?;
    (!reply.text.is_empty()).then_some(reply.text)
}

/// 压缩后的整份消息列表：折叠叶（正文 = 摘要）+ 本轮助手终答。
fn snapshot(facts: &[PortFact], final_id: &str, summary: &str) -> Vec<Value> {
    let mut out = vec![node(
        ChatNode {
            id: format!("compression:{final_id}"),
            msg_type: Some(NodeKind::Notice),
            content: Some(MessageContent::Text(summary.to_string())),
            status: Some(NodeStatus::Completed),
            ..Default::default()
        }
        .with_style_tag(CHAT_STYLE_TAG_COMPRESSION),
    )];
    if let Some(final_fact) = facts.iter().find(|f| f.id == final_id) {
        if let Some(text) = final_fact
            .payload
            .get("text")
            .or_else(|| final_fact.payload.get("answer"))
            .and_then(Value::as_str)
        {
            out.push(node(ChatNode {
                id: final_id.to_string(),
                msg_type: Some(NodeKind::Body),
                content: Some(MessageContent::Text(text.to_string())),
                status: Some(NodeStatus::Completed),
                ..Default::default()
            }));
        }
    }
    out
}

fn node(node: ChatNode) -> Value {
    serde_json::to_value(node).unwrap_or(Value::Null)
}
