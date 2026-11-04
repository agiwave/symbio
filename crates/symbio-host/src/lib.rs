//! 宿主：发现全部 port 行、按 scope 起一次驱动、把债表当数据读、记机制状态 `idle`。
//!
//! 宿主不是插件：它依赖契约层，但插件不依赖它；插件之间互不依赖仍成立。
//! **宿主不认识环的任何类型**：驱动执行者经既有的**能力收集机制**（`drive` 族的一行）
//! 取回，能力束与可见域是**调用参数**，不是注册面（`DE-051` 机制形状）。
//! 端口名与债行都不在这里构造：执行体由 `port` 族的行解析，债行由插件经 `debt` 族自报。
//! `idle` 是机制事实（scope 无未还债），不是业务事实；收束谓词由读方自算，宿主不落业务事实。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use symbio_kernel::capability::Registry;
use symbio_kernel::event::{Entity, Event, Verb};
use symbio_kernel::plugin::{
    Plugin, PluginInvokeRequest, PluginInvokeRequestExt, PluginSimpleRequest,
};
use symbio_kernel::store::MemoryStore;
use symbio_plugin_api::drive::{
    DriveFact, DrivePayload, DriveRow, DriveRunner, DriveSupport, DRIVE_FAMILY,
};
use symbio_plugin_api::port::{
    PortExecutor, PortFact, PortFactEmit, PortFactReader, PortFactWriter, PortPayload, PORT_FAMILY,
};
use symbio_plugin_api::debt::{
    Closing, DebtFacts, DebtSource as DeclaredDebtSource, DebtTable, DriveRequest, DEBT_FAMILY,
};
use symbio_plugin_api::fact_source::FactSource;
use symbio_plugin_api::FactDraft;
use symbio_sdk::capability::CAPABILITY_VISITOR;
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

/// 进程内宿主：根插件 + 事实源 + 每个可见域的五事实账。
pub struct Host {
    root: Arc<dyn Plugin>,
    facts: Arc<Mutex<HashMap<u64, Vec<DriveFact>>>>,
    /// 事实源（唯一写入口）：世界事实与环协议事实都落这里。
    /// 读面（`exists` / `reader`）只从这里投影——**不问写方是谁**。
    source: Arc<dyn FactSource>,
    idle: Arc<Mutex<HashMap<u64, u64>>>,
    started: Instant,
    concurrency: usize,
}

impl Host {
    /// 由根插件造宿主。事实在进程内，重启恢复由后续持久化接入；并发缺省串行。
    pub fn new(root: Arc<dyn Plugin>) -> Self {
        Self::with_store(root, Arc::new(MemoryStore::<Event>::new()))
    }

    /// 指定同轮并发派发上限（配置数据，来自调用方的限额；环不写死）。
    pub fn with_concurrency(root: Arc<dyn Plugin>, concurrency: usize) -> Self {
        Self::with_store_and_concurrency(
            root,
            Arc::new(MemoryStore::<Event>::new()),
            concurrency,
        )
    }

    /// 指定事实源（持久化实现替换时调用方零改动）。
    pub fn with_store(root: Arc<dyn Plugin>, source: Arc<dyn FactSource>) -> Self {
        Self::with_store_and_concurrency(root, source, 1)
    }

    fn with_store_and_concurrency(
        root: Arc<dyn Plugin>,
        source: Arc<dyn FactSource>,
        concurrency: usize,
    ) -> Self {
        Self {
            root,
            facts: Arc::new(Mutex::new(HashMap::new())),
            source,
            idle: Arc::new(Mutex::new(HashMap::new())),
            started: Instant::now(),
            concurrency: concurrency.max(1),
        }
    }

    /// 该可见域的全量事实投影（**从事实源读**，含执行体写口）。
    pub fn world_facts(&self, scope: u64) -> Vec<PortFact> {
        self.source
            .facts(scope)
            .into_iter()
            .map(event_to_fact)
            .collect()
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
    ///
    /// scope 由会话 id 派生（旧入口）。生产驱动入口走 [`Self::drive_at`]：
    /// 事实源里的 scope 是**轮的同一性**，同一条会话的不同轮各是各的域。
    pub async fn drive(
        &self,
        base: &Arc<dyn PluginInvokeRequest>,
        session_id: &str,
        provider: Option<&str>,
    ) -> Result<DriveReport, String> {
        let scope = scope_of(session_id);
        self.drive_at(scope, base, session_id, provider).await
    }

    /// 按**事实信封里的 scope** 起一次驱动。`base` 提供工作目录等上下文字段。
    ///
    /// 与 [`Self::drive`] 的唯一差别是 scope 的来源：这里由调用方给出（一条新事实
    /// 自带的 scope），不再从会话 id 派生。调用方**不得**在外层写 `loop { drive }`
    /// ——环内部的轮次推进归环，外层每看到一条新事实只触发一次。
    pub async fn drive_at(
        &self,
        scope: u64,
        base: &Arc<dyn PluginInvokeRequest>,
        session_id: &str,
        provider: Option<&str>,
    ) -> Result<DriveReport, String> {
        let gathered = self.gather(base, session_id, provider).await?;
        let support: Arc<dyn DriveSupport> = Arc::new(HostDriveSupport {
            facts: Arc::clone(&self.facts),
            source: Arc::clone(&self.source),
            executors: gathered.executors,
            sources: gathered.sources,
            session_id: session_id.to_string(),
            provider: provider.map(str::to_string),
            started: self.started,
            concurrency: self.concurrency,
        });
        let report = gathered.runner.drive(support, scope).await?;

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

        let mut runner: Option<Arc<dyn DriveRunner>> = None;
        for row in manager.list(DRIVE_FAMILY).await {
            let Some(payload) = row.payload::<DrivePayload>() else {
                continue;
            };
            if runner.is_some() {
                return Err("drive 族出现多个提供者".to_string());
            }
            runner = Some(payload.runner());
        }
        let runner = runner.ok_or_else(|| "drive 族没有任何提供者".to_string())?;

        Ok(Gathered {
            executors,
            sources,
            runner,
        })
    }
}

struct Gathered {
    executors: HashMap<String, Arc<dyn PortExecutor>>,
    sources: Vec<Arc<dyn DeclaredDebtSource>>,
    runner: Arc<dyn DriveRunner>,
}

/// 宿主实现的中性能力束：事实读写、执行体解析、存在性、时钟与限额。
struct HostDriveSupport {
    facts: Arc<Mutex<HashMap<u64, Vec<DriveFact>>>>,
    source: Arc<dyn FactSource>,
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
        // 协议事实与世界事实同源：都落事实源（同一写口、同一读面）。
        let draft = FactDraft {
            key: ring_event_id(&fact),
            kind: fact.kind.to_string(),
            entity: Entity::Scope,
            verb: Verb::Asserted,
            payload: fact.payload.clone(),
            produced_by: provenance_at(&*self.source, scope),
        };
        let _ = self.source.append(scope, "loop".to_string(), draft);
        bucket.push(fact);
    }

    fn rows(&self, scope: u64) -> Vec<DriveRow> {
        let request = DriveRequest {
            scope,
            session_id: self.session_id.clone(),
            provider: self.provider.clone(),
        };
        let declared = declared_facts(&self.facts(scope), &*self.source, scope);
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
        self.source
            .facts(scope)
            .into_iter()
            .any(|event| event.kind == kind)
    }

    fn exists_id(&self, kind: &str, id: &str, scope: u64) -> bool {
        self.source
            .facts(scope)
            .into_iter()
            .any(|event| event.kind == kind && event.event_id == id)
    }

    fn now_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    fn concurrency(&self) -> usize {
        self.concurrency
    }

    /// 非五类事实的去处：草稿落事实源（同一事实源、同一读面）。
    fn outlet(&self, scope: u64) -> PortFactEmit {
        PortFactEmit::new(Arc::new(WorldWriter {
            source: Arc::clone(&self.source),
            scope,
        }))
    }

    /// 交给执行体的事实读面（宿主实现；环只转交）。
    fn reader(&self, scope: u64) -> Option<Arc<dyn PortFactReader>> {
        Some(Arc::new(HostReader {
            source: Arc::clone(&self.source),
            scope,
        }))
    }
}

/// 只读事实面：按可见域交出事实源全种类的副本（载荷不透明）。
struct HostReader {
    source: Arc<dyn FactSource>,
    scope: u64,
}

impl PortFactReader for HostReader {
    fn facts(&self, scope: u64) -> Vec<PortFact> {
        if scope != self.scope {
            return Vec::new();
        }
        self.source
            .facts(scope)
            .into_iter()
            .map(event_to_fact)
            .collect()
    }
}

/// 事实源的写口：草稿按草稿的幂等键落行；重复投递被事实源吸收。
struct WorldWriter {
    source: Arc<dyn FactSource>,
    scope: u64,
}

#[async_trait::async_trait]
impl PortFactWriter for WorldWriter {
    async fn emit(&self, facts: Vec<FactDraft>) {
        for fact in facts {
            let mut draft = fact;
            // 草稿自带溯源时用它；否则指向该域里刚落地的那条事实（I2 的声明类
            // 不接受无源）。空域不填——那是真正的无源声明，交给事实源拒绝。
            draft.produced_by = draft
                .produced_by
                .or_else(|| provenance_at(&*self.source, self.scope));
            let _ = self.source.append(self.scope, "host".to_string(), draft);
        }
    }
}

/// 五事实与全种类事实身份 → 申报方读的窄视图（只取本可见域）。
fn declared_facts(
    facts: &[DriveFact],
    source: &dyn FactSource,
    scope: u64,
) -> DebtFacts {
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
    for event in source.facts(scope) {
        out.note_fact(&event.kind, &event.event_id);
    }
    out
}

/// 协议事实的入库身份：`ring:` 前缀 + 可见域 / 端口 / 债 id / 尝试号 / 种类
/// （分片另加一位）。前缀把协议事实与宿主 / 执行体写的世界事实分开——
/// 读面按它把端口位交回执行体，不用猜。
fn ring_event_id(fact: &DriveFact) -> String {
    let base = format!(
        "ring:{}:{}:{}:{}:{}",
        fact.scope, fact.key.port, fact.key.id, fact.key.attempt, fact.kind
    );
    match shard_of(fact) {
        Some(shard) => format!("{base}:{shard}"),
        None => base,
    }
}

/// I2 的溯源位：该 `scope` 里**最近一条已入库事实**的 seq。
///
/// 协议事实（`turn × asserted`）与世界事实（端口执行体交的断言 / 收束）都在 I2 的
/// 「声明类」里，必须带 `produced_by`。宿主这一层拿不到执行体内心的因果，能指出的
/// 最准确上游就是**该域里刚落地的那条事实**——它们是同一轮义务链上的前件。
/// 空域（没有任何前件）返回 `None`，由事实源按 I2 拒绝（那是真正的无源声明）。
fn provenance_at(source: &dyn FactSource, scope: u64) -> Option<u64> {
    source
        .facts(scope)
        .into_iter()
        .rev()
        .find(|event| event.scope == scope && event.seq.is_some())
        .and_then(|event| event.seq.map(|seq| seq.value()))
}

/// 事件 → 读面的一行（身份 / 端口 / 种类 / 不透明载荷）。
fn event_to_fact(event: Event) -> PortFact {
    let port = ring_port_of(&event.event_id);
    PortFact {
        id: event.event_id,
        port,
        kind: event.kind,
        payload: event.payload,
    }
}

/// 从入库身份取端口位：协议事实的 id 以 `ring:` 开头（见 [`ring_event_id`]）；
/// 世界事实没有端口——空串表示「不属于任何端口」。
fn ring_port_of(event_id: &str) -> String {
    let Some(rest) = event_id.strip_prefix("ring:") else {
        return String::new();
    };
    rest.split(':').nth(1).unwrap_or_default().to_string()
}

/// 便利构造：一次驱动用的基上下文（前端只需补工作目录等）。
pub fn base_context() -> Arc<dyn PluginInvokeRequest> {
    Arc::new(PluginSimpleRequest::new(None, None))
}

#[cfg(test)]
mod tests;
