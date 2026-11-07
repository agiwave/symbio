//! `plugin_row` 行执行体（机制侧，D-241 / D-304 / D-338）。
//!
//! 本模块补的正是旧 `loop_rows` 的位置：**只认 `Decl` 与 `RowHandler` 的通用形状**，
//! 不认识任何世界名（不 `use` notes / task 的任何类型）。
//!
//! ```text
//! 环选债口（`ring::plan`）──owed + 事实投影──▶ 债行（by == plugin_row）
//! 环派发 ──row + key + attempt──▶ RowHandler::run
//! RowStep::Ask ──▶ 落「在等」事实，执行体挂起
//! 模型回执落在同一 id 上 ──▶ 行重新触发，RowHandler::parse 关账
//! ```
//!
//! **本模块不选债**：`owed` 由环的选债口调用（`TC-1b`），这里只把已求出的 `Debt`
//! 投影成环认得的行形状。在派发器之外先问求解器再看结果，就是第二个选债口。
//!
//! `Ask` 不在这里调模型：那是 D-304 被否的那条。执行体只落一条事实并挂起，
//! 往返由申报方声明的那笔债走端口（D-338）。

use std::sync::Arc;

use serde_json::{json, Value};
use symbio_kernel::decl::ring::Debt;
use symbio_kernel::event::{Entity, Event, Verb};
use symbio_plugin_api::drive::DriveFact;
use symbio_plugin_api::effects::EffectDraft;
use symbio_plugin_api::events::EVENT_MESSAGE_ADDED;
use symbio_plugin_api::node_build::dialog_node;
use symbio_plugin_api::port::{
    PortErrorClass, PortExecContext, PortExecutor, PortInput, PortOutcome,
};
use symbio_plugin_api::principal::Principal;
use symbio_plugin_api::rows::{
    ask_id, handler_for, ModelReply, RowRequest, RowStep, ASK_KIND, PLUGIN_ROW,
};
use symbio_plugin_api::FactDraft;
use symbio_plugin_api::PluginInvokeRequestExt;

use crate::ring::table::DebtRow;

/// 行债的租约与重试上界（机制侧给值；行本身不声明策略）。
const ROW_LEASE_MS: u64 = 60_000;
const ROW_MAX_ATTEMPTS: u32 = 1;

/// 选债口求出的 `Debt` → 环认得的债行（`by == plugin_row` 的全部未结债）。
///
/// `Frozen::rows()` 参与选债是 D-304 裁决六：谁执行由 `by` 判别，环只认债行。
/// **本函数不调 `owed`**：求解发生在环的选债口里（`TC-1b`），这里只做形状投影。
pub(crate) fn rows_from_debts(debts: &[Debt<'_>], events: &[Event]) -> Vec<DebtRow> {
    debts
        .iter()
        .filter(|debt| !debt.blocked && debt.decl.by == Some(PLUGIN_ROW))
        .map(|debt| {
            let row = debt.decl.name;
            let trace = debt.key.trace();
            let waiting = ask_since(events, row, &trace, 0).is_some();
            DebtRow {
                name: row,
                port: PLUGIN_ROW,
                id: format!("{row}:{trace}"),
                payload: json!({ "row": row, "key": debt.key.0.clone() }),
                after: None,
                // Ask 之后这笔债要等回执：`response` 落在 sent 之后，行重新触发。
                when: waiting.then(|| symbio_plugin_api::EVENT_RESPONSE.to_string()),
                when_id: None,
                unless: None,
                when_no_open: None,
                // 消费端口产物的行必须等该端口收束：派发不再等执行体之后，
                // 「产物落格」与「响应落账」这两笔谁先到不能由调度机缘决定（D-376）。
                after_port: symbio_plugin_api::rows::port_of_fact(debt.decl.owes.source),
                lease_ms: ROW_LEASE_MS,
                max_attempts: ROW_MAX_ATTEMPTS,
            }
        })
        .collect()
}

/// 中性事实 → 事件（入库 `seq` 不伪造：没有就留空，读方按「无区间」自处）。
fn neutral_event(fact: DriveFact) -> Event {
    let mut event = Event::pending(
        fact.key.id.clone(),
        fact.kind,
        Entity::Scope,
        Verb::Asserted,
        "user",
    );
    event.payload = fact.payload;
    event.ts = fact.at_ms as i64;
    event
}

/// 由能力束的完整事实投影造声明求值输入。
pub(crate) fn decl_events(facts: Vec<DriveFact>) -> Vec<Event> {
    facts.into_iter().map(neutral_event).collect()
}

/// 该行此刻「在等」的那条事实（按 `(row, key)` 前缀；attempt 不参与「是否存在」）。
fn ask_since<'a>(events: &'a [Event], row: &str, trace: &str, _attempt: u32) -> Option<&'a Event> {
    let prefix = format!("{row}:{trace}:");
    events.iter().rev().find(|e| {
        e.kind == ASK_KIND
            && e.payload.get("row").and_then(Value::as_str) == Some(row)
            && e.event_id.starts_with(&prefix)
    })
}

/// 回执事件里的环债 id（`ring:<port>:<id>:<attempt>:<kind>`）。
fn ring_id(event_id: &str, kind: &str) -> Option<String> {
    let rest = event_id.strip_prefix("ring:")?;
    let parts: Vec<&str> = rest.split(':').collect();
    let at = parts.iter().rposition(|p| *p == kind)?;
    if at < 2 {
        return None;
    }
    Some(parts[1..at - 1].join(":"))
}

/// 与这笔「在等」配对回执：先找 `response`，再找 `failed`。
fn reply_of<'a>(events: &'a [Event], ask: &str) -> Option<ModelReply<'a>> {
    let mut found = None;
    for event in events.iter().rev() {
        if event.kind != symbio_plugin_api::EVENT_RESPONSE
            && event.kind != symbio_plugin_api::EVENT_FAILED
        {
            continue;
        }
        if ring_id(&event.event_id, &event.kind).as_deref() != Some(ask) {
            continue;
        }
        if event.kind == symbio_plugin_api::EVENT_RESPONSE {
            let text = event
                .payload
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default();
            found = Some(ModelReply::Text(text));
        } else {
            let why = event
                .payload
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("端口失败");
            found = Some(ModelReply::Failed(why));
        }
        break;
    }
    found
}

/// 落格时按**声明表**给每份草稿定归属。
///
/// `owner` 只从 `Frozen::event_writer(kind)` 来——机制侧知道这一格的归属主体；
/// **不从草稿来**：`FactDraft` 没有 `actor` 字段，行为无权指定归属（DE-026 D-252）。
/// 表上没有这一格的归属主体就显式失败：写口不猜默认值，也不静默丢（D-339）。
async fn emit_attributed(
    exec: &PortExecContext,
    drafts: Vec<FactDraft>,
) -> Result<(), PortOutcome> {
    let Ok(frozen) = symbio_plugin_api::sealed_declarations() else {
        return Err(PortOutcome::failed(
            PortErrorClass::NonRetryable,
            "声明表未装配：行事实没有归属主体",
        ));
    };
    let mut attributed = Vec::with_capacity(drafts.len());
    for draft in drafts {
        let owner = frozen.event_writer(&draft.kind).or_else(|| {
            symbio_plugin_api::events::SHARED_EVENTS
                .iter()
                .find(|event| event.name == draft.kind)
                .map(|event| event.owner)
        });
        let Some(owner) = owner else {
            return Err(PortOutcome::failed(
                PortErrorClass::NonRetryable,
                format!("事实 `{}` 在声明表上没有归属主体", draft.kind),
            ));
        };
        attributed.push((owner, draft));
    }
    for (owner, draft) in attributed {
        exec.emit.emit_owned(owner, vec![draft]).await;
    }
    Ok(())
}

/// 通用行执行体：按 `Decl.by == plugin_row` 派发到 [`RowHandler`]。
pub(crate) struct PluginRowExecutor;

#[async_trait::async_trait]
impl PortExecutor for PluginRowExecutor {
    async fn execute(&self, exec: PortExecContext, input: PortInput) -> PortOutcome {
        let Some(row) = input.payload.get("row").and_then(Value::as_str) else {
            return PortOutcome::failed(PortErrorClass::NonRetryable, "行债载荷缺 row");
        };
        let key: Vec<(String, String)> = input
            .payload
            .get("key")
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        let Some(handler) = handler_for(row) else {
            return PortOutcome::failed(
                PortErrorClass::NonRetryable,
                format!("没有 `{row}` 的行实现"),
            );
        };
        // 行执行体的主体只从会话载体来（`AGENT_ID` 由开面时唯一设置）。取不到就
        // 显式失败——不回落 `session/main`，也不回落 `"user"`（D-339）。
        let Some(principal) = session_principal(&exec) else {
            return PortOutcome::failed(
                PortErrorClass::NonRetryable,
                "行执行体缺少会话主体（拒绝回落 session/main）",
            );
        };
        let attempt = exec
            .idempotency_key
            .rsplit(':')
            .next()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(1);
        let ask = ask_id(row, &key, attempt);
        let events = exec
            .reader
            .as_ref()
            .map(|reader| reader.events())
            .unwrap_or_default();

        if let Some(reply) = reply_of(&events, &ask) {
            let request = row_request(handler.row(), &key, &exec, &events, &principal);
            let drafts = handler.parse(&request, reply);
            if drafts.is_empty() {
                return PortOutcome::failed(
                    PortErrorClass::NonRetryable,
                    format!("`{row}` 拿到回执却一笔都不落"),
                );
            }
            if let Err(outcome) = emit_attributed(&exec, drafts).await {
                return outcome;
            }
            return PortOutcome::ok();
        }

        let trace = key
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(",");
        if ask_since(&events, row, &trace, attempt).is_some() {
            // 已经 Ask、回执未到：留在原地等，不重发提示。
            return paused();
        }

        let request = row_request(handler.row(), &key, &exec, &events, &principal);
        match handler.run_ctx(&request, exec.ctx.clone()).await {
            RowStep::Facts(facts) => {
                if facts.is_empty() {
                    return PortOutcome::failed(
                        PortErrorClass::NonRetryable,
                        format!("`{row}` 这一步一笔都不落"),
                    );
                }
                if let Err(outcome) = emit_attributed(&exec, facts).await {
                    return outcome;
                }
                PortOutcome::ok()
            }
            RowStep::FactsWithEffects(facts, effects) => {
                // 对话面效果（D-293）：能力只交纯值草稿，节点 id 与轮号由宿主盖。
                // 节点身份由 `(owner, anchor, key)` 派生 ⇒ 重复投递撞同一事件 id，
                // 由写口的 Duplicate 吸收（不在实时面上多出一张卡）。
                let mut drafts = facts;
                for effect in effects {
                    match effect {
                        EffectDraft::Dialog(draft) => {
                            let id = format!(
                                "dlg-{}-{}-{}",
                                handler.owner(),
                                exec.idempotency_key,
                                draft.key
                            );
                            let who: &str = match draft.role {
                                "user" => "user",
                                "system" => "system",
                                _ => principal.as_str(),
                            };
                            let mut node = dialog_node(
                                &draft.text,
                                "effect",
                                false,
                                &Principal::from_legacy(who),
                            );
                            node.id = id.clone();
                            if let Some(card) = &draft.card {
                                if let Some(meta) = node.meta.as_mut() {
                                    meta["card"] = card.clone();
                                }
                            }
                            drafts.push(FactDraft {
                                key: id,
                                kind: EVENT_MESSAGE_ADDED.to_string(),
                                entity: Entity::Message,
                                verb: Verb::Opened,
                                payload: json!({ "messages": [node] }),
                                produced_by: exec.origin,
                            });
                        }
                        // 实时上线面还没有生产草稿（本仓此刻没有能力交 `LiveDraft`）：
                        // 不在这里私造帧。
                        EffectDraft::Live(_) => {}
                    }
                }
                if let Err(outcome) = emit_attributed(&exec, drafts).await {
                    return outcome;
                }
                PortOutcome::ok()
            }
            RowStep::Ask(ask) => {
                let draft = FactDraft {
                    key: ask_id(handler.row(), &key, attempt),
                    kind: ASK_KIND.to_string(),
                    entity: Entity::Scope,
                    verb: Verb::Asserted,
                    // 行名与债键都在载荷里：种类只有一个名字（逐行造名会落进 OffGrid），
                    // 申报方按 `row` 挑自己那行、按 `key` 认出自己的输入（D-338）。
                    payload: json!({ "row": handler.row(), "key": key, "prompt": ask.prompt }),
                    produced_by: exec.origin,
                };
                exec.emit.emit(vec![draft]).await;
                paused()
            }
        }
    }
}

/// 挂起：回执未到，不给租约上界（D-338）。
fn paused() -> PortOutcome {
    PortOutcome::failed(PortErrorClass::NeedsHuman, "等待模型回执")
}

/// 行执行体的会话主体：只读开面时唯一设置的 `AGENT_ID` 载体。
///
/// 载体不在（`ctx` 缺失或键未设）就返回 `None`，由调用方显式失败——不交给
/// `principal_of(None)` 去默认派生 `session/main`。载体在而值为空 = 会话确实
/// 没有绑定智能体，其主体定义就是 `session/main`（`principal_of` 的语义）。
fn session_principal(exec: &PortExecContext) -> Option<String> {
    let agent_id = exec
        .ctx
        .as_deref()
        .and_then(|ctx| ctx.get(symbio_plugin_api::AGENT_ID))?;
    Some(symbio_plugin_api::principal_of(Some(agent_id.trim())))
}

fn row_request<'a>(
    row: &'static str,
    key: &'a [(String, String)],
    exec: &'a PortExecContext,
    events: &'a [Event],
    principal: &'a str,
) -> RowRequest<'a> {
    let now_ms = events.iter().map(|e| e.ts).max().unwrap_or(0);
    let user_seq = events
        .iter()
        .rev()
        .find_map(|e| e.seq.as_ref().map(|seq| seq.value()))
        .unwrap_or(0);
    RowRequest {
        row,
        key,
        anchor: &exec.idempotency_key,
        now_ms,
        principal,
        user_seq,
        facts: events,
        history: events,
    }
}

/// 供 `drive.rs` 拦截端口名：这一族由机制自己执行。
pub(crate) fn is_row_port(port: &str) -> bool {
    port == PLUGIN_ROW
}

/// 造一个执行体（无状态；环按端口名解析）。
pub(crate) fn executor() -> Arc<dyn PortExecutor> {
    Arc::new(PluginRowExecutor)
}
