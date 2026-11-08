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
//! 载荷 `{ "messages": [ChatNode…], "keep_from": <事实 id>, "split": bool }`：
//! 压缩节点是折叠叶（`NodeKind::Notice` + `style_tag = compression`，正文就是摘要）。
//! `keep_from` 是首条**未压**事实（切片只覆盖最旧前缀）：读面从这条事实起接着折，
//! 其余原样保留；缺席（整份都压掉了 / 旧快照）时读面回落「快照下标 + 1」，
//! 此时快照尾部补本轮终答（否则它不在任何一段里）。`split` 记录这一段被头尾截断过。

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
        let after_port = str_of(&input.payload, "after_port");
        let Some(summary) = summary_of(&facts, &final_id, &after_port) else {
            // 摘要那笔没交回正文（失败 / 空转写）：不拿占位冒充摘要。
            return PortOutcome::ok();
        };
        let keep_from = input
            .payload
            .get("keep_from")
            .and_then(Value::as_str)
            .map(str::to_string);
        let split = input
            .payload
            .get("split")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let messages = snapshot(&facts, &final_id, &summary, keep_from.as_deref(), split);
        if messages.is_empty() {
            return PortOutcome::ok();
        }
        let mut payload = json!({ "messages": messages, "split": split });
        if let Some(keep_from) = &keep_from {
            payload["keep_from"] = json!(keep_from);
        }
        ctx.emit
            .emit(vec![FactDraft {
                key: format!("{SESSION_CONSOLIDATED}:{root_id}"),
                kind: SESSION_CONSOLIDATED.to_string(),
                entity: Entity::Message,
                verb: Verb::Progressed,
                payload,
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

/// 本轮上游端口交回的摘要正文：按**债行声明的端口** + 终答身份双匹配，取最后一条。
fn summary_of(facts: &[PortFact], final_id: &str, port: &str) -> Option<String> {
    let fact = facts.iter().rev().find(|f| {
        f.port == port
            && f.kind == EVENT_RESPONSE
            && f.payload.get("final_id").and_then(Value::as_str) == Some(final_id)
    })?;
    let reply: AssistReply = serde_json::from_value(fact.payload.clone()).ok()?;
    (!reply.text.is_empty()).then_some(reply.text)
}

/// 压缩后的消息列表：折叠叶（正文 = 摘要）。
///
/// - 有 `keep_from`（只压了最旧前缀）⇒ **只**放折叠叶；后缀由读面从该事实起接着折，
///   本轮终答也在后缀里，放进快照会重复一份。
/// - 没有 `keep_from`（整份都压掉了）⇒ 补本轮助手终答，否则它不属于任何一段。
fn snapshot(
    facts: &[PortFact],
    final_id: &str,
    summary: &str,
    keep_from: Option<&str>,
    split: bool,
) -> Vec<Value> {
    let mut leaf = ChatNode {
        id: format!("compression:{final_id}"),
        msg_type: Some(NodeKind::Notice),
        content: Some(MessageContent::Text(summary.to_string())),
        status: Some(NodeStatus::Completed),
        ..Default::default()
    }
    .with_style_tag(CHAT_STYLE_TAG_COMPRESSION);
    if split {
        // 读面可见的「这一段被切开过」：载荷另有 `split`，节点上再留一份给展示面。
        let meta = leaf.meta.get_or_insert_with(|| json!({}));
        if let Some(map) = meta.as_object_mut() {
            map.insert("split".to_string(), Value::Bool(true));
        }
    }
    let mut out = vec![node(leaf)];
    if keep_from.is_some() {
        return out;
    }
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

#[cfg(test)]
#[path = "port.test.rs"]
mod tests;
