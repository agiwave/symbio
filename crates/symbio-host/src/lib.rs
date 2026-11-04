//! 宿主：发现全部 port 行、按 scope 起一次驱动、把债表当数据读、记机制状态 `idle`。
//!
//! 宿主不是插件：它依赖环 crate 与契约层，但插件不依赖它；插件之间互不依赖仍成立。
//! 端口名与债行都不在这里构造：执行体由 `port` 族的行解析，债行由插件经 `debt` 族自报。
//! `idle` 是机制事实（scope 无未还债），不是业务事实；收束谓词由读方自算，宿主不落业务事实。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use symbio_kernel::capability::Registry;
use symbio_kernel::plugin::{
    Plugin, PluginInvokeRequest, PluginInvokeRequestExt, PluginSimpleRequest,
};
use symbio_plugin_api::port::{PortExecutor, PortPayload, PORT_FAMILY};
use symbio_plugin_loop::ring::{
    drive, DebtKey, DebtRow, DebtSource, DriveCtx, DriveOutcome, Fact, FactKind, FactView,
};
use symbio_sdk::capability::CAPABILITY_VISITOR;
use symbio_sdk::debt::{Closing, DebtFacts, DebtSource as DeclaredDebtSource, DebtTable, DriveRequest, DEBT_FAMILY};
use symbio_sdk::keys::{PATH, PROVIDER_ID, SESSION_ID, WORKDIR};
use symbio_sdk::plugin::TRAVERSE_AVAILABLE_TOOLS;

/// 载荷里的分片号（宿主吸收重复时的附加身份位；环不知道它的存在）。
const SHARD_KEY: &str = "shard";

fn shard_of(fact: &Fact) -> Option<u64> {
    fact.payload.get(SHARD_KEY).and_then(|v| v.as_u64())
}

/// 一次驱动结果（宿主对外）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriveReport {
    /// 环的出口。
    pub outcome: DriveOutcome,
    /// 本次实际派发笔数。
    pub dispatched: usize,
    /// 当前可见域。
    pub scope: u64,
}

/// 可见域身份：会话 id 的稳定 FNV-1a 64 位；0 保留给「没有可见域」，不拿来占位。
pub fn scope_of(session_id: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in session_id.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    if hash == 0 {
        1
    } else {
        hash
    }
}

/// 进程内宿主：根插件 + 每个可见域的五事实。
pub struct Host {
    root: Arc<dyn Plugin>,
    facts: Mutex<HashMap<u64, Vec<Fact>>>,
    idle: Mutex<HashMap<u64, u64>>,
    started: Instant,
    concurrency: usize,
}

impl Host {
    /// 由根插件造宿主。事实在进程内，重启恢复由后续持久化接入；并发缺省串行。
    pub fn new(root: Arc<dyn Plugin>) -> Self {
        Self::with_concurrency(root, 1)
    }

    /// 指定同轮并发派发上限（配置数据，来自调用方的限额；环不写死）。
    pub fn with_concurrency(root: Arc<dyn Plugin>, concurrency: usize) -> Self {
        Self {
            root,
            facts: Mutex::new(HashMap::new()),
            idle: Mutex::new(HashMap::new()),
            started: Instant::now(),
            concurrency: concurrency.max(1),
        }
    }

    /// 该 scope 最近一次「无未还债」的落账时刻；`None` = 尚有未还债。
    ///
    /// 机制状态，不是业务：收束谓词由读方拿它与终答事实自算。
    pub fn idle_at(&self, scope: u64) -> Option<u64> {
        self.idle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&scope)
            .copied()
    }

    /// 该 scope 的五事实只读投影（读方自算谓词用；`idle` 是机制状态，走 [`Self::idle_at`]）。
    pub fn facts(&self, scope: u64) -> Vec<Fact> {
        self.facts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&scope)
            .cloned()
            .unwrap_or_default()
    }

    /// 按一个会话起一次驱动。`base` 提供工作目录等上下文字段。
    pub async fn drive(
        &self,
        base: &Arc<dyn PluginInvokeRequest>,
        session_id: &str,
        provider: Option<&str>,
    ) -> Result<DriveReport, String> {
        let scope = scope_of(session_id);
        let gathered = self.gather(base, session_id, provider).await?;
        let request = DriveRequest {
            scope,
            session_id: session_id.to_string(),
            provider: provider.map(str::to_string),
        };
        let source = MultiSource {
            sources: gathered.sources,
            request,
        };
        let ctx = HostCtx {
            host: self,
            scope,
            executors: gathered.executors,
            source,
        };
        let report = drive(&ctx, scope).await;
        let at = self.started.elapsed().as_millis() as u64;
        {
            let mut idle = self
                .idle
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if matches!(&report.outcome, DriveOutcome::NoDebt) {
                idle.insert(scope, at);
            } else {
                idle.remove(&scope);
            }
        }
        Ok(DriveReport {
            outcome: report.outcome,
            dispatched: report.dispatched,
            scope,
        })
    }

    /// 遍历根插件，收集全部 port 行与全部债务申报。
    async fn gather(
        &self,
        base: &Arc<dyn PluginInvokeRequest>,
        session_id: &str,
        provider: Option<&str>,
    ) -> Result<Gathered, String> {
        let manager: Arc<dyn Registry> = symbio_providers::new_registry();
        let ctx = base.fork();
        ctx.set(PATH, TRAVERSE_AVAILABLE_TOOLS.to_string());
        ctx.set(SESSION_ID, session_id.to_string());
        ctx.set(WORKDIR, base.get(WORKDIR).unwrap_or_default());
        if let Some(provider) = provider {
            ctx.set(PROVIDER_ID, provider.to_string());
        }
        ctx.set(CAPABILITY_VISITOR, manager.clone());
        symbio_sdk::capability::capability_init_error_bucket(&ctx);

        Arc::clone(&self.root)
            .traverse(String::new(), ctx)
            .await
            .map_err(|e| format!("能力收集失败: {e:?}"))?;

        let mut executors: HashMap<String, Arc<dyn PortExecutor>> = HashMap::new();
        for row in manager.list(PORT_FAMILY).await {
            let Some(payload) = row.payload::<PortPayload>() else {
                continue;
            };
            let provider = payload.provider();
            let Some(executor) = provider.executor() else {
                continue;
            };
            for port in provider.ports() {
                executors.insert(port.to_string(), Arc::clone(&executor));
            }
        }

        let mut sources: Vec<Arc<dyn DeclaredDebtSource>> = Vec::new();
        for row in manager.list(DEBT_FAMILY).await {
            if let Some(table) = row.payload::<DebtTable>() {
                sources.push(Arc::clone(&table.0));
            }
        }

        Ok(Gathered { executors, sources })
    }
}

struct Gathered {
    executors: HashMap<String, Arc<dyn PortExecutor>>,
    sources: Vec<Arc<dyn DeclaredDebtSource>>,
}

/// 宿主驱动上下文：事实读写、执行体解析与债表全部在这里，环不持有它们。
struct HostCtx<'a> {
    host: &'a Host,
    scope: u64,
    executors: HashMap<String, Arc<dyn PortExecutor>>,
    source: MultiSource,
}

impl DriveCtx for HostCtx<'_> {
    fn facts(&self) -> Vec<Fact> {
        self.host
            .facts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&self.scope)
            .cloned()
            .unwrap_or_default()
    }

    fn append(&self, fact: Fact) {
        let mut facts = self
            .host
            .facts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let bucket = facts.entry(self.scope).or_default();
        if bucket.iter().any(|known| {
            known.kind == fact.kind && known.key == fact.key && shard_of(known) == shard_of(&fact)
        }) {
            return;
        }
        bucket.push(fact);
    }

    fn source(&self) -> &dyn DebtSource {
        &self.source
    }

    fn executor(&self, port: &str) -> Option<Arc<dyn PortExecutor>> {
        self.executors.get(port).cloned()
    }

    fn now_ms(&self) -> u64 {
        self.host.started.elapsed().as_millis() as u64
    }

    fn concurrency(&self) -> usize {
        self.host.concurrency
    }
}

/// 多个插件申报的债表合成一个数据源；行数据原样转给环。
struct MultiSource {
    sources: Vec<Arc<dyn DeclaredDebtSource>>,
    request: DriveRequest,
}

impl DebtSource for MultiSource {
    fn rows(&self, facts: &FactView<'_>, scope: u64) -> Vec<DebtRow> {
        let view = declared_facts(facts, scope);
        let mut rows = Vec::new();
        for source in &self.sources {
            for spec in source.rows(&self.request, &view) {
                let after = spec.after.as_ref().and_then(|id| {
                    facts
                        .all()
                        .iter()
                        .rev()
                        .find(|fact| fact.kind.closes() && fact.key.scope == scope && &fact.key.id == id)
                        .map(|fact| fact.key.clone())
                });
                if spec.after.is_some() && after.is_none() {
                    // 前置尚未落账：申报方本不该给出这一行，宁可少还也不越过。
                    continue;
                }
                let mut row = DebtRow::new(spec.name, spec.port, spec.id.clone(), spec.payload)
                    .lease(spec.lease_ms, spec.max_attempts);
                if let Some(after) = after {
                    row = row.after(after);
                }
                if let Some(unless) = &spec.unless {
                    row = row.unless(unless.clone());
                }
                rows.push(row);
            }
        }
        rows
    }
}

/// 环的五事实 → 申报方读的按 id 归并视图（只取本可见域）。
fn declared_facts(facts: &FactView<'_>, scope: u64) -> DebtFacts {
    let mut out = DebtFacts::default();
    for fact in facts.all() {
        if fact.key.scope != scope {
            continue;
        }
        out.note_attempt(&fact.key.id, fact.key.attempt);
        match fact.kind {
            FactKind::Response => {
                out.note_closing(&fact.key.id, Closing::Ok, fact.payload.clone());
            }
            FactKind::Failed => {
                out.note_closing(&fact.key.id, Closing::Failed, fact.payload.clone());
            }
            _ => {}
        }
    }
    out
}

/// 便利构造：一次驱动用的基上下文（前端只需补工作目录等）。
pub fn base_context() -> Arc<dyn PluginInvokeRequest> {
    Arc::new(PluginSimpleRequest::new(None, None))
}

/// 便于前端把驱动结果当 JSON 读懂。
pub fn report_json(report: &DriveReport) -> serde_json::Value {
    let outcome = match &report.outcome {
        DriveOutcome::NoDebt => serde_json::json!({"kind": "no_debt"}),
        DriveOutcome::Waiting(keys) => serde_json::json!({
            "kind": "waiting",
            "debts": keys.iter().map(|key: &DebtKey| serde_json::json!({
                "port": key.port,
                "id": key.id,
                "attempt": key.attempt,
            })).collect::<Vec<_>>(),
        }),
    };
    serde_json::json!({
        "scope": report.scope,
        "dispatched": report.dispatched,
        "outcome": outcome,
    })
}

#[cfg(test)]
#[path = "host.test.rs"]
mod tests;


