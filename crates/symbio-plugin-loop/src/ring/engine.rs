//! 环本体：选债 → 派发 → 等待 → 配对 → 记账 → 预算与租约 → 重放。
//!
//! 入口只有 [`drive`]；环不持有账本、预算、执行体注册表或任何跨步可变状态——
//! 每一步都从 [`DriveCtx`] 提供的只读投影重算。

use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use symbio_plugin_api::effects::FactDraft;
use symbio_plugin_api::port::{
    PortErrorClass, PortExecContext, PortExecutor, PortFactEmit, PortFactWriter, PortInput,
    PortOutcome,
};
use symbio_plugin_api::{ExecAbortSignal, ExecEventSink};

use crate::ring::facts::{DebtKey, Fact, FactKind, FactView};
use crate::ring::table::{DebtRow, DebtSource};

/// 宿主交给环的全部输入：事实投影的读写、债表、执行体解析与时钟。
///
/// 缓存（若有）由宿主自己持有；环每一步重新读一遍，跨步不保留任何东西。
pub trait DriveCtx: Send + Sync {
    /// 当前可见域的事实快照（只读投影）。
    fn facts(&self) -> Vec<Fact>;

    /// 落一条事实；重复投递由宿主按 `(kind, key)` 吸收。
    fn append(&self, fact: Fact);

    /// 债表数据源（应用侧的行数据）。
    fn source(&self) -> &dyn DebtSource;

    /// 按端口名解析执行体——清单发现归宿主，环不持有注册表。
    fn executor(&self, port: &str) -> Option<Arc<dyn PortExecutor>>;

    /// 当前时刻（毫秒，宿主给的相对时间轴）。
    fn now_ms(&self) -> u64;

    /// 非五类事实的去处（世界事实留在应用侧写口）。
    fn outlet(&self) -> PortFactEmit {
        PortFactEmit::silent()
    }
}

/// 驱动出口。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriveOutcome {
    /// 没有可还的债。
    NoDebt,
    /// 只剩在飞的债（租约未过）。
    Waiting(Vec<DebtKey>),
}

/// 一次驱动报告。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunReport {
    /// 出口。
    pub outcome: DriveOutcome,
    /// 本轮实际派发笔数。
    pub dispatched: usize,
}

/// 选债结果。
#[derive(Debug, Clone)]
enum Action {
    /// 派发一笔（可能先补 `request`）。
    Dispatch { row: DebtRow, attempt: u32 },
    /// 租约已过：推进尝试或判死。
    Expire {
        row: DebtRow,
        attempt: u32,
        next: Option<u32>,
    },
    /// 已发未配，租约未过。
    Wait { key: DebtKey },
}

/// 驱动一个可见域到没有可还的债、只剩在飞。
pub async fn drive(ctx: &dyn DriveCtx, scope: u64) -> RunReport {
    let mut dispatched = 0usize;
    loop {
        let facts = ctx.facts();
        let view = FactView::new(&facts);
        let rows = ctx.source().rows(&view, scope);
        let now_ms = ctx.now_ms();
        let Some(action) = plan(&view, &rows, scope, now_ms) else {
            return RunReport {
                outcome: DriveOutcome::NoDebt,
                dispatched,
            };
        };
        match action {
            Action::Wait { key } => {
                return RunReport {
                    outcome: DriveOutcome::Waiting(vec![key]),
                    dispatched,
                }
            }
            Action::Expire {
                row,
                attempt,
                next,
            } => expire(ctx, scope, row, attempt, next, now_ms),
            Action::Dispatch { row, attempt } => {
                dispatch(ctx, scope, row, attempt, now_ms).await;
                dispatched += 1;
            }
        }
    }
}

/// 选一笔最可还的债：先找可派发的，再找在飞的。
fn plan(view: &FactView<'_>, rows: &[DebtRow], scope: u64, now_ms: u64) -> Option<Action> {
    let mut waiting: Option<Action> = None;
    for row in rows {
        if let Some(after) = &row.after {
            if view.closing(after).is_none() {
                continue;
            }
        }
        let attempt = view.attempt(scope, row.port, &row.id).max(1);
        let key = DebtKey::new(scope, row.port, row.id.clone(), attempt);
        if view.closing(&key).is_some() {
            continue;
        }
        let Some(sent) = view.last_sent(&key) else {
            return Some(Action::Dispatch {
                row: row.clone(),
                attempt,
            });
        };
        let deadline = sent.at_ms.saturating_add(row.lease_ms);
        if now_ms < deadline {
            if waiting.is_none() {
                waiting = Some(Action::Wait { key });
            }
            continue;
        }
        let next = (attempt < row.max_attempts).then(|| attempt + 1);
        return Some(Action::Expire {
            row: row.clone(),
            attempt,
            next,
        });
    }
    waiting
}

/// 租约已过：有下一笔就记账并再举，没有就判死。
fn expire(
    ctx: &dyn DriveCtx,
    scope: u64,
    row: DebtRow,
    attempt: u32,
    next: Option<u32>,
    now_ms: u64,
) {
    let key = DebtKey::new(scope, row.port, row.id.clone(), attempt);
    match next {
        Some(next) => {
            let next_key = DebtKey::new(scope, row.port, row.id.clone(), next);
            ctx.append(Fact::new(
                FactKind::Retry,
                next_key,
                json!({"from": attempt, "class": "retryable", "reason": "lease-expired"}),
                now_ms,
            ));
        }
        None => {
            ctx.append(Fact::new(
                FactKind::Failed,
                key,
                json!({"class": "non_retryable", "reason": "lease-expired"}),
                now_ms,
            ));
        }
    }
}

/// 派发一笔：补 `request` → 落 `sent` → 调执行体 → 配对落账。
async fn dispatch(ctx: &dyn DriveCtx, scope: u64, row: DebtRow, attempt: u32, now_ms: u64) {
    let key = DebtKey::new(scope, row.port, row.id.clone(), attempt);
    ctx.append(Fact::new(
        FactKind::Request,
        key.clone(),
        row.payload.clone(),
        now_ms,
    ));
    ctx.append(Fact::new(
        FactKind::Sent,
        key.clone(),
        json!({"deadline_ms": now_ms.saturating_add(row.lease_ms)}),
        now_ms,
    ));

    let Some(executor) = ctx.executor(row.port) else {
        ctx.append(Fact::new(
            FactKind::Failed,
            key,
            json!({
                "class": "non_retryable",
                "reason": format!("no provider for port {}", row.port),
            }),
            now_ms,
        ));
        return;
    };

    let writer = Arc::new(CollectingWriter::new(ctx.outlet()));
    let exec = PortExecContext {
        deadline_ms: Some(now_ms.saturating_add(row.lease_ms)),
        abort: ExecAbortSignal::new(),
        idempotency_key: format!("{}:{}:{}:{}", key.scope, row.port, row.id, attempt),
        sink: ExecEventSink::silent(),
        emit: PortFactEmit::new(writer.clone()),
        ctx: None,
        subject: key.scope,
        group: String::new(),
        origin: None,
        permitted: true,
        deny_reason: String::new(),
        scratch_dir: None,
    };
    let outcome = executor
        .execute(
            exec,
            PortInput {
                port: row.port,
                payload: row.payload,
            },
        )
        .await;
    let drafts = writer.take();

    match outcome.error {
        Some((class, why)) => {
            if class == PortErrorClass::Retryable && attempt < row.max_attempts {
                let next_key = DebtKey::new(scope, row.port, row.id, attempt + 1);
                ctx.append(Fact::new(
                    FactKind::Retry,
                    next_key,
                    json!({"from": attempt, "class": class_name(class), "reason": why}),
                    now_ms,
                ));
            } else {
                ctx.append(Fact::new(
                    FactKind::Failed,
                    key,
                    json!({"class": class_name(class), "reason": why, "facts": outcome.facts}),
                    now_ms,
                ));
            }
        }
        None => {
            let payload = response_payload(&drafts, &outcome);
            let fact = Fact::new(FactKind::Response, key, payload, now_ms)
                .with_reply(outcome.reply);
            ctx.append(fact);
        }
    }
}

fn class_name(class: PortErrorClass) -> &'static str {
    match class {
        PortErrorClass::Retryable => "retryable",
        PortErrorClass::NonRetryable => "non_retryable",
        PortErrorClass::NeedsHuman => "needs_human",
    }
}

/// 收束载荷：优先用执行体 emit 的 `response` 草稿，其次用返回体的唯一事实。
fn response_payload(drafts: &[FactDraft], outcome: &PortOutcome) -> Value {
    if let Some(draft) = drafts.iter().find(|d| d.kind == FactKind::Response.as_str()) {
        return draft.payload.clone();
    }
    match outcome.facts.as_slice() {
        [] => json!({}),
        [only] => only.clone(),
        many => Value::Array(many.to_vec()),
    }
}

/// 执行体 emit 的收集器：五类事实进账本，其余原样转交应用出口。
struct CollectingWriter {
    outlet: PortFactEmit,
    collected: Mutex<Vec<FactDraft>>,
}

impl CollectingWriter {
    fn new(outlet: PortFactEmit) -> Self {
        Self {
            outlet,
            collected: Mutex::new(Vec::new()),
        }
    }

    fn take(&self) -> Vec<FactDraft> {
        let mut guard = self
            .collected
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        std::mem::take(&mut *guard)
    }
}

#[async_trait::async_trait]
impl PortFactWriter for CollectingWriter {
    async fn emit(&self, facts: Vec<FactDraft>) {
        let mut protocol = Vec::new();
        for fact in facts {
            if FactKind::parse(&fact.kind).is_some() {
                protocol.push(fact);
            } else {
                self.outlet.emit(vec![fact]).await;
            }
        }
        if !protocol.is_empty() {
            self.collected
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .extend(protocol);
        }
    }
}

#[cfg(test)]
#[path = "engine.test.rs"]
mod tests;
