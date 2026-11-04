//! 轮内阶段的世界事实（`port=session`，P2-6 段 2 的写方）。
//!
//! ## 为什么这些事实住会话插件
//!
//! 判决、召回、压缩、装配、汇报说的是「这个世界此刻知道什么 / 说了什么 / 装成了什么」，
//! 不是「环走到第几步」。它们的写方因此必须是**会话执行体**：环自己不落这些格，
//! 环只欠一笔 `request(port=session)` 并在 `response` 上说这笔债还了（`D-323`）。
//!
//! 事实名按**载荷分类**取，不出现任何序列词根：
//!
//! | 行 | 事实 | 载荷 |
//! |---|---|---|
//! | `decide` | `chat.decided` | `reason` / `source` / `model_calls` / `result` / `answer` / `opening` |
//! | `recall` | `session.recalled` | `entries`（逐条 `{seq, tag, content}`）/ `resumed` / `degraded` / `result` |
//! | `compress` | `session.consolidated` | `messages` / `turns` / `model_calls` |
//! | `prepare` | `session.prepared` | `kind` / `result` / `replaces` |
//! | `report` | `chat.assistant.reported` | `text` / `tool_rounds` / `quiet_ms` / `node` |
//!
//! ## 幂等键由环按事实派生
//!
//! 每行载荷的 `id` 是环从**产生这笔债的那条事实**派生的串（端口名 + 事实 id，禁计数）。
//! 执行体原样用它当事件 id：重放撞上同一个键 ⇒ `Duplicate` 被 `emit` 吸收，
//! 不重复落格（`D-323` 的单写者纪律）。
//!
//! ## 载荷信封
//!
//! 所有行共用三格：`id`（世界事实的幂等键）、`anchor`（本幕锚的 seq）、
//! `user_seq`（溯源格）。世界载荷在 `data` 里——信封与内容分开，执行体不解析环的行名。

use serde_json::{json, Value};

use symbio_kernel::event::{Entity, Verb};
use symbio_plugin_api::events::{EVENT_CHAT_ASSISTANT_REPORTED, EVENT_SESSION_CONSOLIDATED};
use symbio_plugin_api::port::PortExecContext;
use symbio_plugin_api::FactDraft;

use crate::declarations::{EVENT_CHAT_DECIDED, EVENT_SESSION_PREPARED, EVENT_SESSION_RECALLED};

/// 载荷信封：`(id, anchor, produced_by)`。
fn envelope(payload: &Value) -> Result<(String, u64, Option<u64>), String> {
    let id = payload
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| "载荷缺非空 `id`（世界事实的幂等键由环按事实派生）".to_string())?
        .to_string();
    let anchor = payload.get("anchor").and_then(Value::as_u64).unwrap_or(0);
    let produced_by = payload.get("user_seq").and_then(Value::as_u64);
    Ok((id, anchor, produced_by))
}

/// 世界载荷：必须是对象（执行体不替调用方造形状）。
fn data_of(payload: &Value) -> Result<Value, String> {
    payload
        .get("data")
        .cloned()
        .filter(Value::is_object)
        .ok_or_else(|| "载荷缺对象 `data`".to_string())
}

/// 一格待落的世界事实。
struct WorldFact<'a> {
    id: String,
    kind: &'a str,
    entity: Entity,
    verb: Verb,
    body: Value,
    anchor: u64,
    produced_by: Option<u64>,
}

/// 把一格世界事实落进事实源；`produced_by` 缺席时不编锚（`I2` 不要求 `asserted` 带溯源）。
///
/// 幂等键就是 `id`：重放撞上同一个键由 `emit` 吸收，不重复落格。
async fn land(ctx: &PortExecContext, fact: WorldFact<'_>) -> String {
    let mut body = fact.body;
    if let Some(object) = body.as_object_mut() {
        object.insert("anchor".into(), json!(fact.anchor));
    }
    ctx.emit
        .emit(vec![FactDraft {
            key: fact.id.clone(),
            kind: fact.kind.into(),
            entity: fact.entity,
            verb: fact.verb,
            payload: body,
            produced_by: fact.produced_by,
        }])
        .await;
    fact.id
}

/// `decide`：轮首判决的结论（理由码 / 来源 / 模型往返笔数 / 答话或首响）。
pub(crate) async fn decide(ctx: &PortExecContext, payload: &Value) -> Result<Value, String> {
    let (id, anchor, produced_by) = envelope(payload)?;
    let body = data_of(payload)?;
    let key = land(
        ctx,
        WorldFact {
            id,
            kind: EVENT_CHAT_DECIDED,
            entity: Entity::Scope,
            verb: Verb::Asserted,
            body,
            anchor,
            produced_by,
        },
    )
    .await;
    Ok(json!({ "id": key }))
}

/// `recall`：本幕召回的逐条结果（能力包的行按 `tag` 取自己那一批）。
pub(crate) async fn recall(ctx: &PortExecContext, payload: &Value) -> Result<Value, String> {
    let (id, anchor, produced_by) = envelope(payload)?;
    let mut data = data_of(payload)?;
    if !data.get("entries").map(Value::is_array).unwrap_or(false) {
        return Err("召回载荷缺数组 `entries`".to_string());
    }
    if let Some(object) = data.as_object_mut() {
        object.entry("result").or_insert(Value::Null);
    }
    let key = land(
        ctx,
        WorldFact {
            id,
            kind: EVENT_SESSION_RECALLED,
            entity: Entity::Scope,
            verb: Verb::Asserted,
            body: data,
            anchor,
            produced_by,
        },
    )
    .await;
    Ok(json!({ "id": key }))
}

/// `compress`：语义压缩的产物与这一次压缩花掉的模型往返笔数。
///
/// 事实名复用 `session.consolidated`（压缩这件事已经有一条世界事实，不为阶段另造名）。
/// 消息快照的另一个写方是 `orchestrator::consolidate`；本行写的是**这一次压缩的阶段
/// 载荷**（同一把幂等键由环派生，重放不重复落格）。
pub(crate) async fn compress(ctx: &PortExecContext, payload: &Value) -> Result<Value, String> {
    let (id, anchor, produced_by) = envelope(payload)?;
    let mut data = data_of(payload)?;
    if !data.get("messages").map(Value::is_array).unwrap_or(false) {
        return Err("压缩载荷缺数组 `messages`".to_string());
    }
    if let Some(object) = data.as_object_mut() {
        object.entry("model_calls").or_insert(json!(0));
    }
    let key = land(
        ctx,
        WorldFact {
            id,
            kind: EVENT_SESSION_CONSOLIDATED,
            entity: Entity::Message,
            verb: Verb::Progressed,
            body: data,
            anchor,
            produced_by,
        },
    )
    .await;
    Ok(json!({ "id": key }))
}

/// `prepare`：一次装配的结论（备输入 / 续写恢复 / 同一轮再接一幕）。
pub(crate) async fn prepare(ctx: &PortExecContext, payload: &Value) -> Result<Value, String> {
    let (id, anchor, produced_by) = envelope(payload)?;
    let data = data_of(payload)?;
    if data
        .get("kind")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|kind| !kind.is_empty())
        .is_none()
    {
        return Err("装配载荷缺 `kind`".to_string());
    }
    let key = land(
        ctx,
        WorldFact {
            id,
            kind: EVENT_SESSION_PREPARED,
            entity: Entity::Scope,
            verb: Verb::Asserted,
            body: data,
            anchor,
            produced_by,
        },
    )
    .await;
    Ok(json!({ "id": key }))
}

/// `report`：一次中途汇报（或「这一段静默试过、没话」）。
///
/// 事实名复用 `chat.assistant.reported`：它是 `reported_anchors` 的关账格，也是
/// `report_due` 的配额输入。`text` 为空表示 `silent`（试过、没话），与有话说同一条
/// 事实面——读者按 `text` 判，不按事实名判。
pub(crate) async fn report(ctx: &PortExecContext, payload: &Value) -> Result<Value, String> {
    let (id, anchor, produced_by) = envelope(payload)?;
    let data = data_of(payload)?;
    let key = land(
        ctx,
        WorldFact {
            id,
            kind: EVENT_CHAT_ASSISTANT_REPORTED,
            entity: Entity::Scope,
            verb: Verb::Asserted,
            body: data,
            anchor,
            produced_by,
        },
    )
    .await;
    Ok(json!({ "id": key }))
}
