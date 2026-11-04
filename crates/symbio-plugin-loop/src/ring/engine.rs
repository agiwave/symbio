//! 环本体：选债 → 派发 → 等待 → 配对 → 记账 → 预算与租约 → 重放。
//!
//! 入口只有 [`drive`]；环不持有账本、预算、执行体注册表或任何跨步可变状态——
//! 每一步都从 [`DriveCtx`] 提供的只读投影重算。

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use futures::future::join_all;
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

    /// 落一条事实；重复投递的去重规则归宿主（按事实身份）。
    fn append(&self, fact: Fact);

    /// 债表数据源（应用侧的行数据）。
    fn source(&self) -> &dyn DebtSource;

    /// 按端口名解析执行体——清单发现归宿主，环不持有注册表。
    fn executor(&self, port: &str) -> Option<Arc<dyn PortExecutor>>;

    /// 当前时刻（毫秒，宿主给的相对时间轴）。
    fn now_ms(&self) -> u64;

    /// 同轮并发派发的上限（宿主给的配置数据；环不写死常数）。
    fn concurrency(&self) -> usize;

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

/// 选债结果：这一圈的全部可动作。
#[derive(Debug, Clone, Default)]
struct Batch {
    /// 可派发（可能先补 `request`）。
    dispatch: Vec<(DebtRow, u32)>,
    /// 租约已过：推进尝试或判死。
    expire: Vec<(DebtRow, u32, Option<u32>)>,
    /// 已发未配，租约未过。
    waiting: Vec<DebtKey>,
}

/// 驱动一个可见域到没有可还的债、只剩在飞。
pub async fn drive(ctx: &dyn DriveCtx, scope: u64) -> RunReport {
    let mut dispatched = 0usize;
    loop {
        let facts = ctx.facts();
        let view = FactView::new(&facts);
        let source = ctx.source();
        let rows = source.rows(&view, scope);
        let now_ms = ctx.now_ms();
        let batch = plan(&view, source, &rows, scope, now_ms);

        if !batch.dispatch.is_empty() {
            let cap = ctx.concurrency().max(1);
            for chunk in batch.dispatch.chunks(cap) {
                let run = chunk
                    .iter()
                    .map(|(row, attempt)| dispatch(ctx, scope, row.clone(), *attempt, now_ms));
                join_all(run).await;
                dispatched += chunk.len();
            }
            continue;
        }
        if !batch.expire.is_empty() {
            for (row, attempt, next) in batch.expire {
                expire(ctx, scope, row, attempt, next, now_ms);
            }
            continue;
        }
        let outcome = if batch.waiting.is_empty() {
            DriveOutcome::NoDebt
        } else {
            DriveOutcome::Waiting(batch.waiting)
        };
        return RunReport {
            outcome,
            dispatched,
        };
    }
}

/// 选这一圈的全部可还之债：可派发的进批，租约已过的排进过期，其余在飞收进等待。
fn plan(
    view: &FactView<'_>,
    source: &dyn DebtSource,
    rows: &[DebtRow],
    scope: u64,
    now_ms: u64,
) -> Batch {
    let mut batch = Batch::default();
    let mut seen = HashSet::new();
    for row in rows {
        if let Some(after) = &row.after {
            if view.closing(after).is_none() {
                continue;
            }
        }
        // 触发条件是**种类**的存在性：环只问宿主「这个可见域有没有这种事实」，
        // 不解释它是什么（字符串透传，归属：选债）。
        if let Some(when) = &row.when {
            if !source.exists(when, scope) {
                continue;
            }
        }
        if let Some(unless) = &row.unless {
            if view.has_id(scope, unless) {
                continue;
            }
        }
        let attempt = view.attempt(scope, row.port, &row.id).max(1);
        let key = DebtKey::new(scope, row.port, row.id.clone(), attempt);
        if view.closing(&key).is_some() {
            continue;
        }
        // 幂等底线：同一 (port, id, attempt) 一圈只入批一次。
        if !seen.insert(key.clone()) {
            continue;
        }
        let Some(sent) = view.last_sent(&key) else {
            batch.dispatch.push((row.clone(), attempt));
            continue;
        };
        let deadline = sent.at_ms.saturating_add(row.lease_ms);
        if now_ms < deadline {
            batch.waiting.push(key);
            continue;
        }
        let next = (attempt < row.max_attempts).then(|| attempt + 1);
        batch.expire.push((row.clone(), attempt, next));
    }
    batch
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
            let payloads = response_payloads(&drafts, &outcome);
            let last = payloads.len().saturating_sub(1);
            for (idx, payload) in payloads.into_iter().enumerate() {
                let mut fact = Fact::new(FactKind::Response, key.clone(), payload, now_ms);
                if idx == last {
                    fact = fact.with_reply(outcome.reply.clone());
                }
                ctx.append(fact);
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

/// 收束载荷：执行体 emit 的每条 `response` 草稿按序各成一条事实（分片只是载荷），
/// 没有草稿时退回返回体的唯一事实。环不合并、不解释分片。
fn response_payloads(drafts: &[FactDraft], outcome: &PortOutcome) -> Vec<Value> {
    let emitted: Vec<Value> = drafts
        .iter()
        .filter(|d| d.kind == FactKind::Response.as_str())
        .map(|d| d.payload.clone())
        .collect();
    if !emitted.is_empty() {
        return emitted;
    }
    match outcome.facts.as_slice() {
        [] => vec![json!({})],
        [only] => vec![only.clone()],
        many => vec![Value::Array(many.to_vec())],
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
