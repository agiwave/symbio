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
//! 压根不会被派发。执行体拿不到可信摘要（`model.assist` 失败 / 空转写）⇒ **失败是产物**
//! （D-402）：落一条带原因分类的 `failed` 事实，读面据此折出 `Failed` 压缩叶，重试入口
//! 因此可达；不再返回成功、不再让压缩节点恒 `Completed`。
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
    ChatNode, MessageContent, NodeKind, NodeStatus, CHAT_STYLE_TAG_COMPRESSION,
};
use symbio_plugin_api::port::{
    PortErrorClass, PortExecContext, PortExecutor, PortFact, PortInput, PortOutcome, PortProvider,
};
use symbio_plugin_api::{AssistReply, Entity, FactDraft, Verb, EVENT_FAILED, EVENT_RESPONSE};

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
        let after_id = str_of(&input.payload, "after_id");
        let Some(summary) = summary_of(&facts, &final_id, &after_port) else {
            // 摘要那笔没交回正文（失败 / 空转写）：失败是产物（D-402），不拿成功冒充。
            return summary_failed(
                &input.payload,
                &facts,
                &root_id,
                &final_id,
                &after_port,
                &after_id,
            );
        };
        let key = {
            let fingerprint = str_of(&input.payload, "fingerprint");
            if fingerprint.is_empty() {
                root_id.clone()
            } else {
                fingerprint
            }
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
        let mut payload = json!({ "root_id": root_id, "messages": messages, "split": split });
        if let Some(keep_from) = &keep_from {
            payload["keep_from"] = json!(keep_from);
        }
        ctx.emit
            .emit(vec![FactDraft {
                key: format!("{SESSION_CONSOLIDATED}:{key}"),
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

/// 拿不到摘要时的产物：一条带原因分类的失败（D-402 ①）。
///
/// 载荷必须带上**读出与重试要用的全部材料**：失败压缩节点的身份（`compression_id`）、
/// 原因分类（`failure_class`）、给展示面的正文（`text`），以及失败时那段切片的边界
/// （`from` / `to` / `truncate` / `keep_from` / `split`）——重试不能拿「此刻的转写」
/// 重新规划，否则压的就不是失败过的那一段。
fn summary_failed(
    input: &Value,
    facts: &[PortFact],
    root_id: &str,
    final_id: &str,
    after_port: &str,
    after_id: &str,
) -> PortOutcome {
    let upstream = failure_of(facts, after_id, after_port);
    let class = classify_failure(
        upstream.as_ref().map(|(class, _)| class.as_str()),
        upstream.as_ref().map(|(_, reason)| reason.as_str()),
    );
    let detail = upstream
        .map(|(_, reason)| reason)
        .unwrap_or_else(|| "摘要服务没有交回正文".to_string());
    // 失败节点的身份按**前缀指纹**（含重试纪元）派生：读面把 `message.removed`
    // 当作永久隐藏，重试若沿用旧身份，重建出的节点永远不可见（D-402 ③）。
    let fingerprint = str_of(input, "fingerprint");
    let compression_id = if fingerprint.is_empty() {
        format!("compression:{final_id}")
    } else {
        format!("compression:{fingerprint}")
    };
    let mut outcome =
        PortOutcome::failed(PortErrorClass::NonRetryable, format!("{class}: {detail}"));
    outcome.facts = vec![json!({
        "root_id": root_id,
        "final_id": final_id,
        "compression_id": compression_id,
        "failure_class": class,
        "detail": detail,
        "text": failure_text(&class, &detail),
        "from": str_of(input, "from"),
        "to": str_of(input, "to"),
        "truncate": input.get("truncate").cloned().unwrap_or_else(|| json!({})),
        "keep_from": input.get("keep_from").cloned().unwrap_or(Value::Null),
        "split": input.get("split").and_then(Value::as_bool).unwrap_or(false),
        "fingerprint": fingerprint,
    })];
    outcome
}

/// 上游摘要那笔的失败事实：按债 id 前缀与端口双匹配（`failed` 载荷不带 `final_id`）。
fn failure_of(facts: &[PortFact], after_id: &str, port: &str) -> Option<(String, String)> {
    let prefix = format!("ring:{port}:{after_id}:");
    let fact = facts
        .iter()
        .rev()
        .find(|f| f.port == port && f.kind == EVENT_FAILED && f.id.starts_with(&prefix))?;
    let class = fact
        .payload
        .get("class")
        .and_then(Value::as_str)
        .unwrap_or("non_retryable")
        .to_string();
    let reason = fact
        .payload
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    Some((class, reason))
}

/// 失败原因分类：provider 的错误文本只按**已登记特征**归类，其余如实落到 `*_provider`。
///
/// 超窗（`non_retryable_context_length`）单独成一类，因为它是「换个小转写 / 换大窗口」
/// 才能解的；认证、配额等与瞬时故障分开，读面与债源都能据此解释。
fn classify_failure(class: Option<&str>, reason: Option<&str>) -> String {
    let reason = reason.unwrap_or_default().to_ascii_lowercase();
    const CONTEXT_LENGTH: &[&str] = &[
        "context length",
        "context_length",
        "context window",
        "maximum context",
        "prompt is too long",
        "too many tokens",
    ];
    if CONTEXT_LENGTH.iter().any(|needle| reason.contains(needle)) {
        return "non_retryable_context_length".to_string();
    }
    match class {
        // 没有上游失败事实：摘要服务就是没交回正文（空转写 / 空摘要）。
        None => "non_retryable_empty_summary".to_string(),
        Some("retryable") => "retryable_provider".to_string(),
        Some("needs_human") => "needs_human".to_string(),
        _ => "non_retryable_provider".to_string(),
    }
}

/// 失败节点的正文：用户能读懂的交代（分类 + 原因）。
fn failure_text(class: &str, detail: &str) -> String {
    format!("上下文压缩失败：{detail}（{class}）。可在此重试；持续失败时换用窗口更大的模型。")
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
