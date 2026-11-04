//! 环本体：选债 → 派发 → 等待 → 配对 → 记账 → 预算与租约 → 重放。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use symbio_plugin_api::effects::FactDraft;
use symbio_plugin_api::port::{
    PortErrorClass, PortExecContext, PortExecutor, PortFactEmit, PortFactWriter, PortInput,
    PortOutcome,
};
use symbio_plugin_api::{ExecAbortSignal, ExecEventSink};

use crate::ring::facts::{DebtKey, Fact, FactKind};
use crate::ring::ledger::Ledger;
use crate::ring::table::{DebtRow, DebtSource};

/// 预算：派发笔数上限（按已 `sent` 的笔数读，不读序号）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// 最多派发多少笔。
    pub max_dispatches: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self { max_dispatches: 64 }
    }
}

/// 端口执行体注册表：端口名（数据）→ 通用执行体。
#[derive(Default, Clone)]
pub struct Registry {
    executors: HashMap<&'static str, Arc<dyn PortExecutor>>,
}

impl Registry {
    /// 空表。
    pub fn new() -> Self {
        Self::default()
    }

    /// 登记一个端口的执行体。
    pub fn register(&mut self, port: &'static str, executor: Arc<dyn PortExecutor>) -> &mut Self {
        self.executors.insert(port, executor);
        self
    }

    /// 按端口名取执行体（缺席 ⇒ `None`）。
    pub fn executor(&self, port: &str) -> Option<Arc<dyn PortExecutor>> {
        self.executors.get(port).cloned()
    }

    /// 已登记端口数。
    pub fn len(&self) -> usize {
        self.executors.len()
    }

    /// 是否没有登记。
    pub fn is_empty(&self) -> bool {
        self.executors.is_empty()
    }
}

/// 停机原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// 债表上没有可还的债，且没有在飞。
    Settled,
    /// 预算用完。
    Budget,
}

/// 驱动出口。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriveOutcome {
    /// 没有可还的债。
    NoDebt(StopReason),
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
pub(crate) enum Action {
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

/// 纯机制环。
pub struct Ring {
    ledger: Ledger,
    registry: Registry,
    limits: Limits,
    outlet: PortFactEmit,
}

impl Ring {
    /// 空账本起步。
    pub fn new(registry: Registry, limits: Limits) -> Self {
        Self {
            ledger: Ledger::new(),
            registry,
            limits,
            outlet: PortFactEmit::silent(),
        }
    }

    /// 带既有账本起步（重放）。
    pub fn with_ledger(ledger: Ledger, registry: Registry, limits: Limits) -> Self {
        Self {
            ledger,
            registry,
            limits,
            outlet: PortFactEmit::silent(),
        }
    }

    /// 指定非五类事实的去处（世界事实留在应用侧写口）。
    pub fn with_outlet(mut self, outlet: PortFactEmit) -> Self {
        self.outlet = outlet;
        self
    }

    /// 账本只读视图。
    pub fn ledger(&self) -> &Ledger {
        &self.ledger
    }

    /// 取回账本。
    pub fn into_ledger(self) -> Ledger {
        self.ledger
    }

    /// 驱动到没有可还的债、只剩在飞、或预算用完。
    pub async fn run(&mut self, source: &dyn DebtSource, now_ms: u64) -> RunReport {
        let mut dispatched = 0usize;
        loop {
            let rows = source.rows(self.ledger.facts());
            let action = self.plan(&rows, now_ms);
            let Some(action) = action else {
                return RunReport {
                    outcome: DriveOutcome::NoDebt(StopReason::Settled),
                    dispatched,
                };
            };
            if dispatched >= self.limits.max_dispatches {
                return RunReport {
                    outcome: DriveOutcome::NoDebt(StopReason::Budget),
                    dispatched,
                };
            }
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
                } => self.expire(row, attempt, next, now_ms),
                Action::Dispatch { row, attempt } => {
                    self.dispatch(row, attempt, now_ms).await;
                    dispatched += 1;
                }
            }
        }
    }

    /// 选一笔最可还的债：先找可派发的，再找在飞的。
    pub(crate) fn plan(&self, rows: &[DebtRow], now_ms: u64) -> Option<Action> {
        let mut waiting: Option<Action> = None;
        for row in rows {
            if let Some(after) = &row.after {
                if self.ledger.closing(after).is_none() {
                    continue;
                }
            }
            let attempt = self.ledger.attempt(row.port, &row.id).max(1);
            let key = DebtKey::new(row.port, row.id.clone(), attempt);
            if self.ledger.closing(&key).is_some() {
                continue;
            }
            let sent = self
                .ledger
                .facts()
                .iter()
                .rev()
                .find(|f| f.kind == FactKind::Sent && f.key == key);
            let Some(sent) = sent else {
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
    fn expire(&mut self, row: DebtRow, attempt: u32, next: Option<u32>, now_ms: u64) {
        let key = DebtKey::new(row.port, row.id.clone(), attempt);
        match next {
            Some(next) => {
                let next_key = DebtKey::new(row.port, row.id.clone(), next);
                let _ = self.ledger.append(Fact::new(
                    FactKind::Retry,
                    next_key,
                    json!({"from": attempt, "class": "retryable", "reason": "lease-expired"}),
                    now_ms,
                ));
            }
            None => {
                let _ = self.ledger.append(Fact::new(
                    FactKind::Failed,
                    key,
                    json!({"class": "non_retryable", "reason": "lease-expired"}),
                    now_ms,
                ));
            }
        }
    }

    /// 派发一笔：补 `request` → 落 `sent` → 调执行体 → 配对落账。
    async fn dispatch(&mut self, row: DebtRow, attempt: u32, now_ms: u64) {
        let key = DebtKey::new(row.port, row.id.clone(), attempt);
        let _ = self.ledger.append(Fact::new(
            FactKind::Request,
            key.clone(),
            row.payload.clone(),
            now_ms,
        ));
        let _ = self.ledger.append(Fact::new(
            FactKind::Sent,
            key.clone(),
            json!({"deadline_ms": now_ms.saturating_add(row.lease_ms)}),
            now_ms,
        ));

        let Some(executor) = self.registry.executor(row.port) else {
            let _ = self.ledger.append(Fact::new(
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

        let writer = Arc::new(CollectingWriter::new(self.outlet.clone()));
        let ctx = PortExecContext {
            deadline_ms: Some(now_ms.saturating_add(row.lease_ms)),
            abort: ExecAbortSignal::new(),
            idempotency_key: format!("{}:{}:{}", row.port, row.id, attempt),
            sink: ExecEventSink::silent(),
            emit: PortFactEmit::new(writer.clone()),
            ctx: None,
            subject: 0,
            group: String::new(),
            origin: None,
            permitted: true,
            deny_reason: String::new(),
            scratch_dir: None,
        };
        let outcome = executor
            .execute(
                ctx,
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
                    let next_key = DebtKey::new(row.port, row.id, attempt + 1);
                    let _ = self.ledger.append(Fact::new(
                        FactKind::Retry,
                        next_key,
                        json!({"from": attempt, "class": class_name(class), "reason": why}),
                        now_ms,
                    ));
                } else {
                    let _ = self.ledger.append(Fact::new(
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
                let _ = self.ledger.append(fact);
            }
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
