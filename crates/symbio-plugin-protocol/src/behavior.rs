//! 效果行为：一次成功的提案 / 加入各 ⇒ 一条领域事实草稿。
//!
//! 实现只交 [`FactDraft`]，**不碰事实源**：唯一的写口（`loop` 的 `record_derived`）
//! 补上宿主面再落格（D-213）。行为与工具通过 `CapabilityMeta.behaviors` 的**名字**相连。
//!
//! 幂等键用 `EffectCall::anchor` + 调用 id + 被写的键：同一次重放不会记两次，而跨请求的
//! 再提案 / 再加入会各自成键（清单取最后一条提案，参与者按不同值累积）。
//!
//! ## 为什么没有 `before`
//!
//! 同名再提案是调整（目的或消息序列变了），不是错误；重复加入会被视图按参与者去重，
//! 不需要在执行前拒掉。去重只在幂等键那一层（同一次调用重放）。

use symbio_plugin_api::behaviors::Kind;
use symbio_plugin_api::{EffectCall, FactDraft, ToolBehavior};
use symbio_plugin_api::{Entity, Verb};

use super::decl::{PROTOCOL_JOINED, PROTOCOL_PROPOSED};
use super::tool::{messages_of, valid_name};

/// 提案行为名。由本 crate 自报（D-363）。
pub(crate) const PROTOCOL_PROPOSED_BEHAVIOR: &str = "protocol_proposed";

/// 加入行为名。由本 crate 自报（D-363）。
pub(crate) const PROTOCOL_JOINED_BEHAVIOR: &str = "protocol_joined";

/// 一次成功提案 = 一条协议定义。
pub(crate) struct ProtocolProposeCapture;

impl ToolBehavior for ProtocolProposeCapture {
    fn owner(&self) -> &'static str {
        "protocol"
    }

    fn behavior(&self) -> &'static str {
        PROTOCOL_PROPOSED_BEHAVIOR
    }

    fn kind(&self) -> Kind {
        Kind::LandsFacts
    }

    fn after(&self, call: &EffectCall<'_>) -> Vec<FactDraft> {
        if !call.success {
            return Vec::new();
        }
        let Some((name, title, purpose, messages)) = proposal(call.args) else {
            return Vec::new();
        };
        vec![FactDraft {
            key: format!("pp-{}-{}-{name}", call.anchor, call.call_id),
            kind: PROTOCOL_PROPOSED.to_string(),
            entity: Entity::Domain,
            verb: Verb::Asserted,
            payload: serde_json::json!({
                "name": name,
                "title": title,
                "purpose": purpose,
                "messages": messages,
            }),
            produced_by: Some(call.user_seq),
        }]
    }
}

/// 一次成功加入 = 一条参与者断言。
pub(crate) struct ProtocolJoinCapture;

impl ToolBehavior for ProtocolJoinCapture {
    fn owner(&self) -> &'static str {
        "protocol"
    }

    fn behavior(&self) -> &'static str {
        PROTOCOL_JOINED_BEHAVIOR
    }

    fn kind(&self) -> Kind {
        Kind::LandsFacts
    }

    fn after(&self, call: &EffectCall<'_>) -> Vec<FactDraft> {
        if !call.success {
            return Vec::new();
        }
        let Some((name, participant)) = join(call.args) else {
            return Vec::new();
        };
        vec![FactDraft {
            key: format!("pj-{}-{}-{name}.{participant}", call.anchor, call.call_id),
            kind: PROTOCOL_JOINED.to_string(),
            entity: Entity::Domain,
            verb: Verb::Asserted,
            payload: serde_json::json!({
                "name": name,
                "participant": participant,
            }),
            produced_by: Some(call.user_seq),
        }]
    }
}

fn proposal(value: &serde_json::Value) -> Option<(&str, &str, &str, Vec<String>)> {
    let name = value.get("name")?.as_str()?.trim();
    let title = value.get("title")?.as_str()?.trim();
    let purpose = value.get("purpose")?.as_str()?.trim();
    if !valid_name(name) || title.is_empty() || purpose.is_empty() {
        return None;
    }
    let messages = messages_of(value).ok()?;
    Some((name, title, purpose, messages))
}

fn join(value: &serde_json::Value) -> Option<(&str, &str)> {
    let name = value.get("name")?.as_str()?.trim();
    let participant = value.get("participant")?.as_str()?.trim();
    if !valid_name(name) || participant.is_empty() || participant.chars().count() > 64 {
        return None;
    }
    Some((name, participant))
}

// 链接期提交：两个实现随本 crate 进入二进制，宿主按声明选它们。
symbio_plugin_api::submit_behavior!(ProtocolProposeCapture);
symbio_plugin_api::submit_behavior!(ProtocolJoinCapture);

#[cfg(test)]
#[path = "behavior.test.rs"]
mod tests;
