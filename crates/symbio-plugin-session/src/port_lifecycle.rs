//! 幕的开启与收束两行（`port=session`，P2-6 段 1）。
//!
//! 这两行把「一幕从哪开始、到哪结束」从环的幕机里搬到会话世界：环只欠一笔
//! `request(port=session, row=open)` / `row=finalize`，由本插件落事实并回体。
//!
//! - `open`：落本幕的用户格 / 配置格 / 分支格（新轮）或只落的配置格（续写），
//!   回体给出锚值（根 id、用户发言、用户格序号、是否新开根、要替换的旧子）。
//! - `finalize`：落收束格（`chat.assistant.final`）或兜底格
//!   （`chat.assistant.fallback`）；闸门拒绝时落否定事实（`authz.denied`）。
//!
//! ## 载荷键
//!
//! 两行的载荷都是中性键（不出现环的世界名）：`mode` / `round` / `root` /
//! `replaces` / `branch_of` / `node` / `auto_compress` / `reason` / `payload`。
//! 轮号由宿主按债的作用域盖章，执行体不猜。
//!
//! ## 段 1 只加不接
//!
//! 这两行进表并可被单独执行（测试直接驱动执行体），但 `entry::open_stage` /
//! `close_stage` 在本段仍走旧路径；接线在 P2-6 段 2。

use serde_json::{json, Value};

use symbio_kernel::event::{Entity, Seq, Verb};
use symbio_kernel::store::Store;
use symbio_plugin_api::events::{
    EVENT_AUTHZ_DENIED, EVENT_CHAT_ASSISTANT_FALLBACK, EVENT_CHAT_ASSISTANT_FINAL,
    EVENT_SESSION_CONFIG, EVENT_USER_MESSAGE,
};
use symbio_plugin_api::port::PortExecContext;
use symbio_plugin_api::FactDraft;

use crate::port::dispatch_of;

/// `open`：落一幕的锚。
pub(crate) async fn open(ctx: &PortExecContext, payload: &Value) -> Result<Value, String> {
    let dispatch = dispatch_of(ctx)?;
    let facts = dispatch
        .facts
        .as_ref()
        .ok_or_else(|| "执行需要持久会话（临时会话无事实源）".to_string())?;
    let mode = payload.get("mode").and_then(Value::as_str).unwrap_or("new");
    let round = payload.get("round").and_then(Value::as_u64).unwrap_or(0);
    let text = payload
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let snapshot = facts.range(Seq::new(0));

    let (scope, user_seq) = match mode {
        // 新轮：用户格由本行落（键 `u-{scope}`，幂等），作用域取债的 `subject`。
        "new" => {
            let node = payload
                .get("node")
                .ok_or_else(|| "新幕缺 `node`（用户消息节点）".to_string())?;
            let mut body = json!({
                "text": node.get("text").and_then(Value::as_str).unwrap_or(&text),
                "id": node.get("id").and_then(Value::as_str).unwrap_or(""),
            });
            if let Some(meta) = node.get("meta").filter(|m| !m.is_null()) {
                body["meta"] = meta.clone();
            }
            let scope = ctx.subject;
            ctx.emit
                .emit(vec![FactDraft {
                    key: format!("u-{scope}"),
                    kind: EVENT_USER_MESSAGE.into(),
                    entity: Entity::Scope,
                    verb: Verb::Opened,
                    payload: body,
                    produced_by: None,
                }])
                .await;
            let seq = find_seq(facts.as_ref(), &format!("u-{scope}"))?;
            (scope, seq)
        }
        // 续写：锚是最近一条**已开未收束**的用户格，不新落用户格。
        "resume" | "continue" => {
            let (scope, seq) = last_open(&snapshot)
                .ok_or_else(|| "续写轮找不到未收束的轮次（事实源与恢复路径不一致）".to_string())?;
            (scope, seq)
        }
        other => return Err(format!("未知开模式：`{other}`")),
    };

    let (root_id, fresh_root) = match payload.get("root").and_then(Value::as_str) {
        Some(root) => (root.to_string(), false),
        None => (symbio_sdk::llm::llm_short_id(), true),
    };

    // 重试子支的分支格仍由环随 `resume-turn` 落（本段只加不接，不新增同名事实的第二写者）。

    // 配置格：生效设置 + 请求级覆盖（`auto_compress` 已在请求里折过）。
    let view = dispatch.spec.view();
    let settings = &view.settings;
    ctx.emit
        .emit(vec![FactDraft {
            key: format!("cfg-{scope}-{}", ctx.idempotency_key),
            kind: EVENT_SESSION_CONFIG.into(),
            entity: Entity::Scope,
            verb: Verb::Asserted,
            payload: json!({
                "classify_enabled": settings.classify_enabled,
                "auto_compress": payload.get("auto_compress").and_then(Value::as_bool).unwrap_or(false),
                "progress_enabled": settings.progress_enabled,
                "progress_interval_ms": settings.progress_interval_ms,
                "progress_min_rounds": settings.progress_min_rounds,
                "progress_max_per_turn": settings.progress_max_per_turn,
                "readyset_cap": settings.task_cap,
                "skill_compile_enabled": settings.skill_compile_enabled,
                "skill_fast_path": settings.skill_fast_path,
                "is_persistent": view.persistent,
            }),
            produced_by: Some(user_seq),
        }])
        .await;

    Ok(json!({
        "mode": mode,
        "user_text": text,
        "root_id": root_id,
        "fresh_root": fresh_root,
        "replaces": payload.get("replaces").cloned().unwrap_or(Value::Null),
        "scope": scope,
        "user_seq": user_seq,
        "round": round,
    }))
}

/// `finalize`：落收束格 / 兜底格。闸门拒绝时只落否定事实，不落收束格。
pub(crate) async fn finalize(ctx: &PortExecContext, payload: &Value) -> Result<Value, String> {
    let scope = payload
        .get("scope")
        .and_then(Value::as_u64)
        .unwrap_or(ctx.subject);
    let user_seq = payload.get("user_seq").and_then(Value::as_u64);
    let cost_ms = payload.get("cost_ms").and_then(Value::as_u64).unwrap_or(0);
    let reason = payload
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or("final");
    let body = payload.get("payload").cloned().unwrap_or_else(|| json!({}));

    if !ctx.permitted {
        // 收束格是发言格：被闸拒绝 ⇒ 落否定事实，本轮保持未收束。
        ctx.emit
            .emit(vec![FactDraft {
                key: format!("deny-{scope}-speech"),
                kind: EVENT_AUTHZ_DENIED.into(),
                entity: Entity::Scope,
                verb: Verb::Asserted,
                payload: json!({
                    "key": EVENT_CHAT_ASSISTANT_FINAL,
                    "kind": "speech",
                    "reason": "denied",
                    "detail": ctx.deny_reason,
                }),
                produced_by: user_seq,
            }])
            .await;
        return Ok(json!({ "denied": true }));
    }

    let (kind, key, payload) = match reason {
        "final" => (EVENT_CHAT_ASSISTANT_FINAL, format!("f-{scope}"), body),
        "fallback" | "exhausted" => {
            let mut body = body;
            body["status"] = json!("failed");
            body["reason"] = json!(reason);
            if let Some(detail) = payload.get("detail") {
                body["detail"] = detail.clone();
            }
            (EVENT_CHAT_ASSISTANT_FALLBACK, format!("fb-{scope}"), body)
        }
        "aborted" => {
            let mut body = body;
            body["status"] = json!("aborted");
            body["reason"] = json!("interrupted");
            (EVENT_CHAT_ASSISTANT_FINAL, format!("f-{scope}"), body)
        }
        other => return Err(format!("未知收束模式：`{other}`")),
    };
    ctx.emit
        .emit(vec![FactDraft {
            key,
            kind: kind.into(),
            entity: Entity::Scope,
            verb: Verb::Closed,
            payload,
            produced_by: user_seq,
        }])
        .await;
    Ok(json!({ "cost_ms": cost_ms }))
}

/// 幂等键 → 序号。`emit` 只落格不回序号，读回同一事实源即可（同一把锁）。
fn find_seq(
    store: &dyn Store<Event = symbio_kernel::event::Event>,
    key: &str,
) -> Result<u64, String> {
    store
        .range(Seq::new(0))
        .into_iter()
        .find(|e| e.event_id == key)
        .and_then(|e| e.seq)
        .map(|s| s.value())
        .ok_or_else(|| format!("事实 `{key}` 没有落进事实源"))
}

/// **已开未收束**的轮次：返回 `(scope, 用户格 seq)`（与 `entry` 同口径，完整史）。
fn last_open(events: &[symbio_kernel::event::Event]) -> Option<(u64, u64)> {
    let mut open: Vec<(u64, u64)> = Vec::new();
    for e in events {
        if e.entity == Entity::Scope && e.verb == Verb::Opened && e.kind == EVENT_USER_MESSAGE {
            if let Some(seq) = e.seq {
                open.push((e.scope, seq.value()));
            }
        }
        if e.entity == Entity::Scope
            && e.verb == Verb::Closed
            && matches!(
                e.kind.as_str(),
                EVENT_CHAT_ASSISTANT_FINAL | EVENT_CHAT_ASSISTANT_FALLBACK
            )
        {
            open.retain(|(stage, _)| *stage != e.scope);
        }
    }
    open.last().copied()
}
