//! 效果行为：一次成功的 `idempotent_record` ⇒ 一条 `idempotency.recorded` 草稿。
//!
//! 实现只交 [`FactDraft`]，**不碰事实源**：唯一的写口（`loop` 的 `record_derived`）
//! 补上宿主面再落格（D-213）。行为与工具通过 `CapabilityMeta.behaviors` 的**名字**
//! 相连——宿主两侧都不认识 `idempotent_record` 这个工具名。
//!
//! ## 幂等键：同轮同键由写口去重，跨轮由 `before` 拒
//!
//! - `after` 的键是 `i-{anchor}-{key}`：**不带调用 id**。同一轮里同一键的两次调用会
//!   产生同一个事件 id，第二条被存储丢掉——这正是「同轮重复只算一次」。
//! - `before` 只看本插件自己的 `idempotency.recorded` 事实：跨轮再来同一个键时，
//!   第一次的记录已在会话事实里，于是执行前就拒，模型看到的是「已经记过」。
//!
//! 重试**不在本片**：重放身份与续写幕的口径属后续片，这里不假装已经具备。

use symbio_plugin_api::behaviors::Kind;
use symbio_plugin_api::{
    BeforeDecision, BeforeRefusal, BeforeRequest, EffectCall, FactDraft, ToolBehavior,
};
use symbio_plugin_api::{Entity, Verb};

use super::decl::IDEMPOTENCY_RECORDED;

/// 本行为管辖的名字。**由本 crate 自报**（D-363）：工具在自己的
/// `CapabilityMeta.behaviors` 里声明同一个常量，宿主只做收集。
pub(crate) const IDEMPOTENCY_RECORD_BEHAVIOR: &str = "idempotency_record";

/// 参数 / 事实载荷里的二元组：键 + 值。三处（判方、写方、工具回执）共用这一个口径。
fn pair(value: &serde_json::Value) -> Option<(&str, &str)> {
    let key = value.get("key")?.as_str()?.trim();
    let value = value.get("value")?.as_str()?.trim();
    (!key.is_empty() && !value.is_empty()).then_some((key, value))
}

/// 一次成功的调用 = 一条幂等记录。
pub(crate) struct IdempotencyRecord;

impl ToolBehavior for IdempotencyRecord {
    fn owner(&self) -> &'static str {
        "idempotency"
    }

    fn behavior(&self) -> &'static str {
        IDEMPOTENCY_RECORD_BEHAVIOR
    }

    fn kind(&self) -> Kind {
        Kind::LandsFacts
    }

    /// 同一会话里已经记过**同一个键** ⇒ 拒绝这次写入。
    ///
    /// 判据只看 `req.facts` 里的 `idempotency.recorded`（本插件自己的事件名）：既不需要
    /// 知道工具名，也不需要任何跨插件约定。
    fn before(&self, req: &BeforeRequest<'_>) -> BeforeDecision {
        let Some((key, _)) = pair(req.args) else {
            return BeforeDecision::Allow;
        };
        let dup = req.facts.iter().any(|e| {
            e.kind == IDEMPOTENCY_RECORDED
                && e.payload.get("key").and_then(|v| v.as_str()) == Some(key)
        });
        if !dup {
            return BeforeDecision::Allow;
        }
        BeforeDecision::Refuse(BeforeRefusal::new(
            "幂等台账",
            format!("这个键已经记过（同一会话内同一 key）：{key}"),
            symbio_plugin_api::failure_kind::ERROR,
        ))
    }

    fn after(&self, call: &EffectCall<'_>) -> Vec<FactDraft> {
        if !call.success {
            return Vec::new();
        }
        let Some((key, value)) = pair(call.args) else {
            return Vec::new();
        };
        vec![FactDraft {
            // 不带调用 id：同一轮同一键的两条草稿必须撞成同一个事件 id（见模块文档）
            key: format!("i-{}-{key}", call.anchor),
            kind: IDEMPOTENCY_RECORDED.to_string(),
            entity: Entity::Domain,
            verb: Verb::Asserted,
            payload: serde_json::json!({ "key": key, "value": value }),
            produced_by: Some(call.user_seq),
        }]
    }
}

// 链接期提交：本实现随本 crate 进入二进制，宿主按声明选它。
symbio_plugin_api::submit_behavior!(IdempotencyRecord);

#[cfg(test)]
#[path = "behavior.test.rs"]
mod tests;
