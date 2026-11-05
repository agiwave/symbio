//! 宿主：发现全部 port 行、按会话起一次驱动、把债表当数据读、记机制状态 `idle`。
//!
//! 宿主不是插件：它依赖契约层，但插件不依赖它；插件之间互不依赖仍成立。
//! **宿主不认识环的任何类型**：驱动执行者经既有的**能力收集机制**（`drive` 族的一行）
//! 取回，能力束是**调用参数**，不是注册面（`DE-051` 机制形状）。
//! 端口名与债行都不在这里构造：执行体由 `port` 族的行解析，债行由插件经 `debt` 族自报。
//! `idle` 是机制事实（本会话无未还债），不是业务事实；收束谓词由读方自算，宿主不落业务事实。

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use symbio_kernel::capability::Registry;
use symbio_kernel::event::{Entity, Event, Verb};
use symbio_kernel::plugin::{
    Plugin, PluginInvokeRequest, PluginInvokeRequestExt, PluginSimpleRequest,
};
use symbio_kernel::store::MemoryStore;
use symbio_plugin_api::debt::{
    Closing, DebtFacts, DebtSource as DeclaredDebtSource, DebtTable, DriveRequest, DEBT_FAMILY,
};
use symbio_plugin_api::drive::{
    DriveFact, DrivePayload, DriveRow, DriveRunner, DriveSupport, DRIVE_FAMILY,
};
use symbio_plugin_api::fact_source::FactSource;
use symbio_plugin_api::port::{
    PortExecHandles, PortExecutor, PortFact, PortFactEmit, PortFactReader, PortFactWriter,
    PortPayload, PORT_FAMILY,
};
use symbio_plugin_api::{ExecEventSink, FactDraft};
use symbio_sdk::capability::CAPABILITY_VISITOR;
use symbio_sdk::clock::clock_now_ms;
use symbio_sdk::keys::{PATH, PROVIDER_ID, SESSION_ID, WORKDIR};
use symbio_sdk::plugin::TRAVERSE_AVAILABLE_TOOLS;

pub use symbio_plugin_api::drive::{DriveKey, DriveOutcome, DriveReport};

/// 载荷里的分片号（宿主吸收重复时的附加身份位；环不知道它的存在）。
const SHARD_KEY: &str = "shard";

fn shard_of(fact: &DriveFact) -> Option<u64> {
    fact.payload.get(SHARD_KEY).and_then(|v| v.as_u64())
}

/// 进程内宿主：根插件 + 事实源 + 机制状态 `idle`。
pub struct Host {
    root: Arc<dyn Plugin>,
    /// 事实源（唯一写入口）：世界事实与环协议事实都落这里。
    /// 读面（`facts` / `exists` / `reader`）只从这里投影——**不问写方是谁**。
    source: Arc<dyn FactSource>,
    idle: Arc<Mutex<Option<u64>>>,
    concurrency: usize,
}

impl Host {
    /// 由根插件造宿主。事实在进程内，重启恢复由后续持久化接入；并发缺省串行。
    pub fn new(root: Arc<dyn Plugin>) -> Self {
        Self::with_store(root, Arc::new(MemoryStore::<Event>::new()))
    }

    /// 指定同轮并发派发上限（配置数据，来自调用方的限额；环不写死）。
    pub fn with_concurrency(root: Arc<dyn Plugin>, concurrency: usize) -> Self {
        Self::with_store_and_concurrency(root, Arc::new(MemoryStore::<Event>::new()), concurrency)
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
            source,
            idle: Arc::new(Mutex::new(None)),
            concurrency: concurrency.max(1),
        }
    }

    /// 本会话的全量事实投影（**从事实源读**，含执行体写口）。
    pub fn world_facts(&self) -> Vec<PortFact> {
        self.source.facts().into_iter().map(event_to_fact).collect()
    }

    /// 本会话最近一次「无未还债」的落账时刻；`None` = 尚有未还债。
    ///
    /// 机制状态，不是业务：收束谓词由读方拿它与终答事实自算。
    pub fn idle_at(&self) -> Option<u64> {
        *self
            .idle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 本会话事实的只读投影（读方自算谓词用；`idle` 是机制状态，走 [`Self::idle_at`]）。
    ///
    /// 投影直接来自事实源——**没有第二本账**：环写下的协议事实与任何写方写下的
    /// 世界事实都从同一个读面出来。
    pub fn facts(&self) -> Vec<DriveFact> {
        self.source
            .facts()
            .into_iter()
            .map(event_to_drive_fact)
            .collect()
    }

    /// 按本会话起一次驱动。`base` 提供工作目录等上下文字段。
    ///
    /// 调用方**不得**在外层写 `loop { drive }`——环内部的轮次推进归环，
    /// 外层每看到一条新事实只触发一次。
    pub async fn drive(
        &self,
        base: &Arc<dyn PluginInvokeRequest>,
        session_id: &str,
        provider: Option<&str>,
        sink: ExecEventSink,
    ) -> Result<DriveReport, String> {
        let gathered = self.gather(base, session_id, provider).await?;
        let workdir = base.get(WORKDIR).unwrap_or_default();
        // 出口**同一句柄两处共持**：执行体经 `ExecEventSink::of(ctx)`（键 `EVENT_SINK`）
        // 取出口，这里把同一份 `sink` 写进它拿到的上下文，再放进 `handles.sink`。
        // 不新建第二个出口、不在端口侧兜底——`Null` 只留给显式静默的只读开面。
        gathered.ctx.set(symbio_kernel::exec::EVENT_SINK, sink.clone());
        let handles = PortExecHandles {
            ctx: Some(Arc::clone(&gathered.ctx)),
            scratch_dir: (!workdir.is_empty()).then(|| PathBuf::from(&workdir)),
            // 出口由装配层按**本部署根**交来（会话插件给真实转写句柄，
            // 内部请求显式静默）；宿主不再写死静默。
            sink,
            session_id: session_id.to_string(),
        };
        let support: Arc<dyn DriveSupport> = Arc::new(HostDriveSupport {
            // 每驱动现造、从 0 起的投影游标：事实源是唯一输入，投影只缓存
            // 「已读过的那一段」，不与任何写侧账本合并。
            projection: Mutex::new(Projection::default()),
            source: Arc::clone(&self.source),
            executors: gathered.executors,
            sources: gathered.sources,
            session_id: session_id.to_string(),
            provider: provider.map(str::to_string),
            concurrency: self.concurrency,
            handles,
        });
        let report = gathered.runner.drive(support).await?;

        let at = clock_now_ms().max(0) as u64;
        {
            let mut idle = self
                .idle
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if matches!(&report.outcome, DriveOutcome::NoDebt) {
                *idle = Some(at);
            } else {
                *idle = None;
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
            .traverse(String::new(), Arc::clone(&ctx))
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
            ctx,
        })
    }
}

struct Gathered {
    executors: HashMap<String, Arc<dyn PortExecutor>>,
    sources: Vec<Arc<dyn DeclaredDebtSource>>,
    runner: Arc<dyn DriveRunner>,
    /// 装配好的调用方上下文（含能力表）；原样交给执行体，宿主不解释。
    ctx: Arc<dyn PluginInvokeRequest>,
}

/// 宿主实现的中性能力束：事实读写、执行体解析、存在性、时钟与限额。
struct HostDriveSupport {
    /// 事实源 → 中性事实的**投影**（游标增量；不是第二本账）。
    projection: Mutex<Projection>,
    source: Arc<dyn FactSource>,
    executors: HashMap<String, Arc<dyn PortExecutor>>,
    sources: Vec<Arc<dyn DeclaredDebtSource>>,
    session_id: String,
    provider: Option<String>,
    concurrency: usize,
    /// 交给执行体的不透明句柄束（宿主装配，环原样转交）。
    handles: PortExecHandles,
}

/// 事实源到中性事实的投影。
///
/// 事实本身始终以事实源为唯一来源；这里只缓存**已读过事件**的解析结果，
/// 游标记「下一段从哪里读」。**没有写侧账本**：`append` 只落事实源，
/// 下一圈由游标增量读回来——因此上一圈写下的事实下一圈一定看得见。
#[derive(Default)]
struct Projection {
    /// 下一个要读的序号。
    cursor: u64,
    /// 已投影事件的入库身份（去重按 `event_id`，不按 `(kind, key)`）。
    seen: HashSet<String>,
    /// 已投影的事实，按 `seq` 升序（只按读回顺序追加，不重排）。
    facts: Vec<DriveFact>,
}

impl Projection {
    /// 读 `cursor` 之后的新事实，返回**全部**已投影事实（升序）。
    fn refresh(&mut self, source: &dyn FactSource) -> Vec<DriveFact> {
        let events = source.range(self.cursor);
        if let Some(last) = events.last() {
            if let Some(seq) = last.seq {
                self.cursor = seq.value().saturating_add(1);
            }
        }
        for event in events {
            if !self.seen.insert(event.event_id.clone()) {
                continue;
            }
            self.facts.push(event_to_drive_fact(event));
        }
        self.facts.clone()
    }
}

impl DriveSupport for HostDriveSupport {
    fn facts(&self) -> Vec<DriveFact> {
        self.projection
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .refresh(&*self.source)
    }

    fn append(&self, fact: DriveFact) {
        // 协议事实与世界事实同源：都落事实源（同一写口、同一读面）。
        // 不做旁路去重：重复投递由事实源按 `event_id` 幂等吸收。
        let draft = FactDraft {
            key: ring_event_id(&fact),
            kind: fact.kind.clone(),
            entity: Entity::Scope,
            verb: Verb::Asserted,
            payload: fact.payload.clone(),
            produced_by: provenance_at(&*self.source),
        };
        let _ = self.source.append("loop".to_string(), draft);
    }

    fn rows(&self) -> Vec<DriveRow> {
        let request = DriveRequest {
            session_id: self.session_id.clone(),
            provider: self.provider.clone(),
        };
        let declared = declared_facts(&self.facts(), &*self.source);
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
                    when_id: spec.when_id,
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

    /// 与 `facts()` **同一条读面**：世界事实也在投影里，不再回头另读事实源。
    fn exists(&self, kind: &str) -> bool {
        self.facts().iter().any(|fact| fact.kind == kind)
    }

    /// 与 [`Self::exists`] 同一条读面，只多看一位身份。
    ///
    /// 世界事实的投影身份就是入库 `event_id`（协议事实则是债 id）。
    fn exists_id(&self, kind: &str, id: &str) -> bool {
        self.facts()
            .iter()
            .any(|fact| fact.kind == kind && fact.key.id == id)
    }

    fn now_ms(&self) -> u64 {
        clock_now_ms().max(0) as u64
    }

    fn concurrency(&self) -> usize {
        self.concurrency
    }

    /// 非五类事实的去处：草稿落事实源（同一事实源、同一读面）。
    fn outlet(&self) -> PortFactEmit {
        PortFactEmit::new(Arc::new(WorldWriter {
            source: Arc::clone(&self.source),
        }))
    }

    /// 交给执行体的事实读面（宿主实现；环只转交）。
    fn reader(&self) -> Option<Arc<dyn PortFactReader>> {
        Some(Arc::new(HostReader {
            source: Arc::clone(&self.source),
        }))
    }

    /// 交给执行体的不透明句柄束（宿主装配；环只转交）。
    fn exec_handles(&self) -> PortExecHandles {
        self.handles.clone()
    }
}

/// 只读事实面：交出本会话事实源全种类的副本（载荷不透明）。
struct HostReader {
    source: Arc<dyn FactSource>,
}

impl PortFactReader for HostReader {
    fn facts(&self) -> Vec<PortFact> {
        self.source.facts().into_iter().map(event_to_fact).collect()
    }
}

/// 事实源的写口：草稿按草稿的幂等键落行；重复投递被事实源吸收。
struct WorldWriter {
    source: Arc<dyn FactSource>,
}

#[async_trait::async_trait]
impl PortFactWriter for WorldWriter {
    async fn emit(&self, facts: Vec<FactDraft>) {
        for fact in facts {
            let mut draft = fact;
            // 草稿自带溯源时用它；否则指向本会话里刚落地的那条事实（I2 的声明类
            // 不接受无源）。空会话不填——那是真正的无源声明，交给事实源拒绝。
            draft.produced_by = draft.produced_by.or_else(|| provenance_at(&*self.source));
            let _ = self.source.append("host".to_string(), draft);
        }
    }
}

/// 五事实与全种类事实身份 → 申报方读的窄视图。
fn declared_facts(facts: &[DriveFact], source: &dyn FactSource) -> DebtFacts {
    let mut out = DebtFacts::default();
    for fact in facts {
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
    for event in source.facts() {
        out.note_fact(&event.kind, &event.event_id);
    }
    out
}

/// 协议事实的入库身份：`ring:` 前缀 + 端口 / 债 id / 尝试号 / 种类
/// （分片另加一位）。前缀把协议事实与宿主 / 执行体写的世界事实分开——
/// 读面按它把端口位交回执行体，不用猜。
fn ring_event_id(fact: &DriveFact) -> String {
    let base = format!(
        "ring:{}:{}:{}:{}",
        fact.key.port, fact.key.id, fact.key.attempt, fact.kind
    );
    match shard_of(fact) {
        Some(shard) => format!("{base}:{shard}"),
        None => base,
    }
}

/// I2 的溯源位：本会话里**最近一条已入库事实**的 seq。
///
/// 协议事实（`turn × asserted`）与世界事实（端口执行体交的断言 / 收束）都在 I2 的
/// 「声明类」里，必须带 `produced_by`。宿主这一层拿不到执行体内心的因果，能指出的
/// 最准确上游就是**本会话刚落地的那条事实**——它们是同一义务链上的前件。
/// 空会话（没有任何前件）返回 `None`，由事实源按 I2 拒绝（那是真正的无源声明）。
fn provenance_at(source: &dyn FactSource) -> Option<u64> {
    source
        .facts()
        .into_iter()
        .rev()
        .find(|event| event.seq.is_some())
        .and_then(|event| event.seq.map(|seq| seq.value()))
}

/// 事件 → 中性事实（**全量映射**：任何世界事实也进投影，不止环的五类）。
///
/// 协议事实的债键从入库身份还原（见 [`ring_key_of`]）；世界事实的端口位为空、
/// 身份就是 `event_id`、尝试号为 0——它们不参与环的五事实视图，但
/// `exists` / `exists_id` 必须看得见。
fn event_to_drive_fact(event: Event) -> DriveFact {
    let (port, id, attempt) = ring_key_of(&event.event_id, &event.kind);
    DriveFact {
        kind: event.kind,
        key: DriveKey { port, id, attempt },
        payload: event.payload,
        at_ms: event.ts.max(0) as u64,
        reply: None,
    }
}

/// 从入库身份还原协议事实的债键。
///
/// 身份形如 `ring:{port}:{id}:{attempt}:{kind}[:{shard}]`，其中 `id` 自身可能
/// 含 `:`；因此从右往左定界：最右是可选分片号，往左依次是种类与尝试号，
/// 剩下的中段整体是 `id`。不是 `ring:` 抬起的事实 ⇒ 世界事实的三元组。
fn ring_key_of(event_id: &str, kind: &str) -> (String, String, u32) {
    let Some(rest) = event_id.strip_prefix("ring:") else {
        return (String::new(), event_id.to_string(), 0);
    };
    let parts: Vec<&str> = rest.split(':').collect();
    let Some(kind_at) = parts.iter().rposition(|part| *part == kind) else {
        return (String::new(), event_id.to_string(), 0);
    };
    // `kind` 左侧必须还有尝试号与端口位。
    if kind_at < 2 {
        return (String::new(), event_id.to_string(), 0);
    }
    let Ok(attempt) = parts[kind_at - 1].parse::<u32>() else {
        return (String::new(), event_id.to_string(), 0);
    };
    let port = parts[0].to_string();
    let id = parts[1..kind_at - 1].join(":");
    (port, id, attempt)
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
