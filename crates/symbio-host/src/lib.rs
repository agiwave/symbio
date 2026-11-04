//! 宿主：发现全部 port 行、按 scope 起一次驱动、把债表当数据读、记机制状态 `idle`。
//!
//! 宿主不是插件：它依赖契约层，但插件不依赖它；插件之间互不依赖仍成立。
//! **宿主不认识环的任何类型**：驱动经注册机制取到环插件，能力束与可见域走
//! `plugin_api::drive` 的通用契约，报告也从同一上下文键取回（`DE-051` 机制形状）。
//! 端口名与债行都不在这里构造：执行体由 `port` 族的行解析，债行由插件经 `debt` 族自报。
//! `idle` 是机制事实（scope 无未还债），不是业务事实；收束谓词由读方自算，宿主不落业务事实。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use symbio_deploy::PLUGIN_ID_LOOP;
use symbio_kernel::capability::Registry;
use symbio_kernel::plugin::{Plugin, PluginInvokeRequest, PluginInvokeRequestExt, PluginSimpleRequest};
use symbio_plugin_api::creator_create_object;
use symbio_plugin_api::drive::{
    DriveFact, DriveRow, DriveSupport, DRIVE_PATH, DRIVE_REPORT, DRIVE_SCOPE, DRIVE_SUPPORT,
};
use symbio_plugin_api::port::{
    PortExecutor, PortFact, PortFactEmit, PortFactReader, PortFactWriter, PortPayload, PORT_FAMILY,
};
use symbio_plugin_api::FactDraft;
use symbio_sdk::capability::CAPABILITY_VISITOR;
use symbio_sdk::debt::{
    Closing, DebtFacts, DebtSource as DeclaredDebtSource, DebtTable, DriveRequest, DEBT_FAMILY,
};
use symbio_sdk::keys::{PATH, PROVIDER_ID, SESSION_ID, WORKDIR};
use symbio_sdk::plugin::TRAVERSE_AVAILABLE_TOOLS;

pub use symbio_plugin_api::drive::{DriveKey, DriveOutcome, DriveReport};

/// 载荷里的分片号（宿主吸收重复时的附加身份位；环不知道它的存在）。
const SHARD_KEY: &str = "shard";

fn shard_of(fact: &DriveFact) -> Option<u64> {
    fact.payload.get(SHARD_KEY).and_then(|v| v.as_u64())
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
    facts: Arc<Mutex<HashMap<u64, Vec<DriveFact>>>>,
    /// 世界事实（按可见域）：身份 + 种类 + 不透明载荷。环只按种类问存在性；
    /// 执行体经通用 `reader` 读载荷——宿主是唯一实现方。
    world: Arc<Mutex<HashMap<u64, Vec<PortFact>>>>,
    idle: Arc<Mutex<HashMap<u64, u64>>>,
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
            facts: Arc::new(Mutex::new(HashMap::new())),
            world: Arc::new(Mutex::new(HashMap::new())),
            idle: Arc::new(Mutex::new(HashMap::new())),
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
    pub fn facts(&self, scope: u64) -> Vec<DriveFact> {
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
        let support: Arc<dyn DriveSupport> = Arc::new(HostDriveSupport {
            facts: Arc::clone(&self.facts),
            world: Arc::clone(&self.world),
            executors: gathered.executors,
            sources: gathered.sources,
            session_id: session_id.to_string(),
            provider: provider.map(str::to_string),
            started: self.started,
            concurrency: self.concurrency,
        });

        let ctx = base.fork();
        ctx.set(PATH, DRIVE_PATH.to_string());
        ctx.set(DRIVE_SCOPE, scope);
        ctx.set(DRIVE_SUPPORT, support);
        let loop_plugin: Arc<dyn Plugin> =
            creator_create_object::<dyn Plugin>(PLUGIN_ID_LOOP, Arc::clone(&ctx))
                .ok_or_else(|| format!("环插件未注册: {PLUGIN_ID_LOOP}"))?;
        loop_plugin
            .route(ctx.clone())
            .await
            .map_err(|e| format!("驱动失败: {e:?}"))?;
        let report = ctx
            .get(DRIVE_REPORT)
            .ok_or_else(|| "环未交回驱动报告".to_string())?;

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
        Ok(report)
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

/// 宿主实现的中性能力束：事实读写、执行体解析、存在性、时钟与限额。
struct HostDriveSupport {
    facts: Arc<Mutex<HashMap<u64, Vec<DriveFact>>>>,
    world: Arc<Mutex<HashMap<u64, Vec<PortFact>>>>,
    executors: HashMap<String, Arc<dyn PortExecutor>>,
    sources: Vec<Arc<dyn DeclaredDebtSource>>,
    session_id: String,
    provider: Option<String>,
    started: Instant,
    concurrency: usize,
}

impl DriveSupport for HostDriveSupport {
    fn facts(&self, scope: u64) -> Vec<DriveFact> {
        self.facts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&scope)
            .cloned()
            .unwrap_or_default()
    }

    fn append(&self, fact: DriveFact) {
        let scope = fact.scope;
        let mut facts = self
            .facts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let bucket = facts.entry(scope).or_default();
        if bucket.iter().any(|known| {
            known.kind == fact.kind && known.key == fact.key && shard_of(known) == shard_of(&fact)
        }) {
            return;
        }
        bucket.push(fact);
    }

    fn rows(&self, scope: u64) -> Vec<DriveRow> {
        let request = DriveRequest {
            scope,
            session_id: self.session_id.clone(),
            provider: self.provider.clone(),
        };
        let declared = declared_facts(&self.facts(scope), scope);
        let mut rows = Vec::new();
        for source in &self.sources {
            for spec in source.rows(&request, &declared) {
                rows.push(DriveRow {
                    name: spec.name,
                    port: spec.port,
                    id: spec.id,
                    payload: spec.payload,
                    after: spec.after,
                    when: spec.when,
                    unless: spec.unless,
                    when_no_open: spec.when_no_open,
                    lease_ms: spec.lease_ms,
                    max_attempts: spec.max_attempts,
                });
            }
        }
        rows
    }

    fn executor(&self, port: &str) -> Option<Arc<dyn PortExecutor>> {
        self.executors.get(port).cloned()
    }

    fn exists(&self, kind: &str, scope: u64) -> bool {
        self.world
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&scope)
            .is_some_and(|facts| facts.iter().any(|known| known.kind == kind))
    }

    fn now_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    fn concurrency(&self) -> usize {
        self.concurrency
    }

    /// 非五类事实的去处：记下身份 / 种类 / 载荷，供环问存在性与执行体读事实。
    fn outlet(&self, scope: u64) -> PortFactEmit {
        PortFactEmit::new(Arc::new(WorldWriter {
            world: Arc::clone(&self.world),
            scope,
        }))
    }

    /// 交给执行体的事实读面（宿主实现；环只转交）。
    fn reader(&self, scope: u64) -> Option<Arc<dyn PortFactReader>> {
        Some(Arc::new(HostReader {
            world: Arc::clone(&self.world),
            scope,
        }))
    }
}

/// 只读事实面：按可见域交出世界事实的副本（载荷不透明）。
struct HostReader {
    world: Arc<Mutex<HashMap<u64, Vec<PortFact>>>>,
    scope: u64,
}

impl PortFactReader for HostReader {
    fn facts(&self, scope: u64) -> Vec<PortFact> {
        if scope != self.scope {
            return Vec::new();
        }
        self.world
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&scope)
            .cloned()
            .unwrap_or_default()
    }
}

/// 世界事实的写口：记身份 / 种类 / 载荷，不解释语义（环与宿主都不认识业务名）。
struct WorldWriter {
    world: Arc<Mutex<HashMap<u64, Vec<PortFact>>>>,
    scope: u64,
}

#[async_trait::async_trait]
impl PortFactWriter for WorldWriter {
    async fn emit(&self, facts: Vec<FactDraft>) {
        let mut world = self
            .world
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let bucket = world.entry(self.scope).or_default();
        for fact in facts {
            if bucket.iter().any(|known| known.id == fact.key) {
                continue;
            }
            bucket.push(PortFact {
                id: fact.key,
                kind: fact.kind,
                payload: fact.payload,
            });
        }
    }
}

/// 五事实 → 申报方读的按 id 归并视图（只取本可见域）。
fn declared_facts(facts: &[DriveFact], scope: u64) -> DebtFacts {
    let mut out = DebtFacts::default();
    for fact in facts {
        if fact.key.scope != scope {
            continue;
        }
        out.note_attempt(&fact.key.id, fact.key.attempt);
        match fact.closing() {
            Some(true) => {
                out.note_closing(&fact.key.id, Closing::Ok, fact.payload.clone());
            }
            Some(false) => {
                out.note_closing(&fact.key.id, Closing::Failed, fact.payload.clone());
            }
            None => {}
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
            "debts": keys.iter().map(|key: &DriveKey| serde_json::json!({
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
