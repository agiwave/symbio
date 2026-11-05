//! `compact` 端口的执行面：读本轮终答的水位，超阈值时落一条压缩快照。
//!
//! ## 水位从哪来
//!
//! 终答载荷的 `context` 格（`port.rs` 模型侧写入）：
//! `request_tokens`（provider 实测输入用量）/ `limit_tokens`（本 provider 的有效
//! 上下文上限）。缺 `request_tokens` ⇒ 按历史事实估算已用量；缺 `limit_tokens`
//! ⇒ 取最近一条带该格的事实（上限是 provider 属性，不随轮次变）；都没有 ⇒
//! 没有可信水位，**不做**（返回成功、不落事实，环照常还债）。
//!
//! ## 未超阈值不落痕
//!
//! 「没压」不是一条领域事实，只是没做。执行体返回 `PortOutcome::ok()`。
//!
//! ## 压出来的快照
//!
//! 载荷 `{ "messages": [ChatNode…] }`：压缩节点是折叠叶
//! （`NodeKind::Notice` + `style_tag = compression`）＋本轮的助手终答。重建面
//! 从这份列表开始折，被压掉的前缀不再参与。

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use symbio_plugin_api::chat_node::{
    ChatNode, MessageContent, NodeKind, NodeStatus, CHAT_STYLE_TAG_COMPRESSION,
};
use symbio_plugin_api::events::{EVENT_CHAT_ASSISTANT_FINAL, EVENT_USER_MESSAGE};
use symbio_plugin_api::port::{
    PortExecContext, PortExecutor, PortFact, PortInput, PortOutcome, PortProvider,
};
use symbio_plugin_api::{Entity, FactDraft, Verb};

/// 端口名（唯一来源：本插件）。
pub(crate) const COMPACT_PORT: &str = "compact";

/// 事实种类：**已有种类**，声明 owner 是
/// `crates/symbio-plugin-session/src/declarations.rs:27`。本 crate 只写、不声明，
/// 也不往 `plugin-api` 加常量——owner 与写方分离是已知项（待 E-009 / CS-007 判）。
pub(crate) const SESSION_CONSOLIDATED: &str = "session.consolidated";

/// 触发阈值（水位占比）。**住插件内**：不下发 `session.config`，不进契约。
const COMPACT_AT: f64 = 0.7;

/// 估算口径：字符数 / 每 token 字符数（保守偏小，宁可少压）。
const CHARS_PER_TOKEN: usize = 4;

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
        let Some((used, limit)) = watermark(&facts, &final_id) else {
            return PortOutcome::ok();
        };
        if limit == 0 || (used as f64) < limit as f64 * COMPACT_AT {
            return PortOutcome::ok();
        }
        let messages = snapshot(&facts, &final_id);
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

/// 本轮的水位：`(已用, 上限)`；缺可信值 ⇒ `None`（不做）。
fn watermark(facts: &[PortFact], final_id: &str) -> Option<(u64, u64)> {
    let limit = facts
        .iter()
        .rev()
        .filter(|f| f.kind == EVENT_CHAT_ASSISTANT_FINAL)
        .find_map(|f| u64_of_context(f, "limit_tokens"));
    let limit = limit?;
    let used = facts
        .iter()
        .find(|f| f.id == final_id)
        .and_then(|f| u64_of_context(f, "request_tokens"))
        .unwrap_or_else(|| estimate(facts));
    Some((used, limit))
}

fn u64_of_context(fact: &PortFact, key: &str) -> Option<u64> {
    fact.payload.get("context")?.get(key)?.as_u64()
}

/// 缺实测用量时按历史事实估算：消息正文总字符数 / 每 token 字符数。
fn estimate(facts: &[PortFact]) -> u64 {
    let chars: usize = facts
        .iter()
        .filter(|f| f.kind == EVENT_USER_MESSAGE || f.kind == EVENT_CHAT_ASSISTANT_FINAL)
        .filter_map(|f| f.payload.get("text").and_then(Value::as_str))
        .map(|text| text.chars().count())
        .sum();
    (chars / CHARS_PER_TOKEN) as u64
}

/// 压缩后的整份消息列表：折叠叶（压缩记录）+ 本轮助手终答。
fn snapshot(facts: &[PortFact], final_id: &str) -> Vec<Value> {
    let mut out = vec![node(
        ChatNode {
            id: format!("compression:{final_id}"),
            msg_type: Some(NodeKind::Notice),
            content: Some(MessageContent::Text("上下文已压缩".to_string())),
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
