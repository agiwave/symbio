//! 宿主：发现全部 port 行、按会话起一次驱动、把债表当数据读。
//!
//! 宿主不是插件：它依赖契约层，但插件不依赖它；插件之间互不依赖仍成立。
//! **宿主不认识环的任何类型**：驱动执行者经既有的**能力收集机制**（`drive` 族的一行）
//! 取回，能力束是**调用参数**，不是注册面（`DE-051` 机制形状）。
//! 端口名与债行都不在这里构造：执行体由 `port` 族的行解析，债行由插件经 `debt` 族自报。
//! 收束谓词由读方自算，宿主不落业务事实。
//!
//! **宿主运行时也在这里**：驱动入口（`HostDrive` / 每会话重入位 / 按根绑定表）住在
//! [`drive`]，与它服务的 `Host` 同一个 crate——`symbio` 只剩装配登记（P1 批一）。

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
use symbio_plugin_api::termination::{
    TerminationPayload, TerminationView, TerminationWriter, TERMINATION_FAMILY,
};
use symbio_plugin_api::SymbioKey;
use symbio_plugin_api::{ExecEventSink, FactDraft};
use symbio_sdk::capability::CAPABILITY_VISITOR;
use symbio_sdk::keys::{PATH, PROVIDER_ID, SESSION_ID, WORKDIR};
use symbio_sdk::plugin::TRAVERSE_AVAILABLE_TOOLS;

pub use symbio_plugin_api::drive::{DriveKey, DriveOutcome, DriveReport};

/// 宿主运行时：驱动入口与每会话推进状态（`P1` 批一：状态归宿主）。
pub mod drive;

/// 载荷里的分片号（宿主吸收重复时的附加身份位；环不知道它的存在）。
const SHARD_KEY: &str = "shard";

fn shard_of(fact: &DriveFact) -> Option<u64> {
    fact.payload.get(SHARD_KEY).and_then(|v| v.as_u64())
}

/// 进程内宿主：根插件 + 事实源。
pub struct Host {
    root: Arc<dyn Plugin>,
    /// 事实源（唯一写入口）：世界事实与环协议事实都落这里。
    /// 读面（`facts` / `exists` / `reader`）只从这里投影——**不问写方是谁**。
    source: Arc<dyn FactSource>,
}

impl Host {
    /// 由根插件造宿主。事实在进程内，重启恢复由后续持久化接入。
    pub fn new(root: Arc<dyn Plugin>) -> Self {
        Self::with_store(root, Arc::new(MemoryStore::<Event>::new()))
    }

    /// 指定事实源（持久化实现替换时调用方零改动）。
    ///
    /// 同轮派发固定串行：唯一调用方是驱动入口，它按会话一次一轮推进；宿主不提供
    /// 「并发上限」参数（并行债的挑选判据在单 provider 场景不存在）。
    pub fn with_store(root: Arc<dyn Plugin>, source: Arc<dyn FactSource>) -> Self {
        Self { root, source }
    }

    /// 本会话的全量事实投影（**从事实源读**，含执行体写口）。
    pub fn world_facts(&self) -> Vec<PortFact> {
        self.source.facts().into_iter().map(event_to_fact).collect()
    }

    /// 本会话事实的只读投影（读方自算谓词用）。
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
        // 会话主体只从开面时唯一设置的 `AGENT_ID` 载体来（D-339）。载体不在就显式失败，
        // 不用 `principal_of(None)` 默认派生 `session/main`；载体在但值为空 = 会话
        // **确实没有绑定智能体**，其主体定义就是 `session/main`（`principal_of` 的语义）。
        let principal = {
            let Some(agent_id) = gathered.ctx.get(symbio_plugin_api::AGENT_ID) else {
                return Err(
                    "驱动缺少会话主体载体（AGENT_ID）；拒绝由缺失值派生 session/main".to_string(),
                );
            };
            symbio_plugin_api::principal_of(Some(agent_id.trim()))
        };
        let workdir = base.get(WORKDIR).unwrap_or_default();
        // 出口**同一句柄两处共持**：执行体经 `ExecEventSink::of(ctx)`（键 `EVENT_SINK`）
        // 取出口，这里把同一份 `sink` 写进它拿到的上下文，再放进 `handles.sink`。
        // 不新建第二个出口、不在端口侧兜底——`Null` 只留给显式静默的只读开面。
        gathered
            .ctx
            .set(symbio_kernel::exec::EVENT_SINK, sink.clone());
        // 入站来源主体（D-067）：宿主从本轮 ctx 取好——`metadata.source_principal` 由
        // 网关落 ctx、收件箱写口随会话轮装配挂回来。缺席 = 本机用户发起，效果面
        // 回落会话身份；这一格的解析是宿主的事，不往下推给扩展插件。
        let source = gathered
            .ctx
            .get(symbio_sdk::keys::SOURCE_PRINCIPAL)
            .unwrap_or_default();
        let handles = PortExecHandles {
            ctx: Some(Arc::clone(&gathered.ctx)),
            scratch_dir: (!workdir.is_empty()).then(|| PathBuf::from(&workdir)),
            // 出口由装配层按**本部署根**交来（会话插件给真实转写句柄，
            // 内部请求显式静默）；宿主不再写死静默。
            sink,
            session_id: session_id.to_string(),
            source,
        };
        let support: Arc<dyn DriveSupport> = Arc::new(HostDriveSupport {
            // 每驱动现造、从 0 起的投影游标：事实源是唯一输入，投影只缓存
            // 「已读过的那一段」，不与任何写侧账本合并。
            projection: Mutex::new(Projection::default()),
            plan: Mutex::new(None),
            source: Arc::clone(&self.source),
            executors: gathered.executors,
            sources: gathered.sources,
            session_id: session_id.to_string(),
            provider: provider.map(str::to_string),
            principal,
            handles,
        });
        // 簿记写方：驱动**前后**各给同一份只读投影（D-367）。前一次让「开轮前已落
        // 的事实」有机会成立谓词；后一次带宿主的 `NoDebt` 结论。终止事实落进同一
        // 事实源，环下一圈经同一投影读回——环零知识，宿主不替它判。
        let support_handle = Arc::clone(&support);
        observe_termination(&gathered.writers, &*support_handle, &self.source, false).await;
        let report = gathered.runner.drive(support).await?;
        if matches!(&report.outcome, DriveOutcome::NoDebt) {
            observe_termination(&gathered.writers, &*support_handle, &self.source, true).await;
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
                executors.insert(port, Arc::clone(&executor));
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

        // 簿记写方是按行收集的**数据**：宿主不认插件名，也不给某个内置写方开旁路
        // （D-367）。一个写方都没有时，前后两次观察都是空转。
        let mut writers: Vec<Arc<dyn TerminationWriter>> = Vec::new();
        for row in manager.list(TERMINATION_FAMILY).await {
            let Some(payload) = row.payload::<TerminationPayload>() else {
                continue;
            };
            writers.push(payload.writer());
        }

        Ok(Gathered {
            executors,
            sources,
            runner,
            writers,
            ctx,
        })
    }
}

struct Gathered {
    executors: HashMap<String, Arc<dyn PortExecutor>>,
    sources: Vec<Arc<dyn DeclaredDebtSource>>,
    runner: Arc<dyn DriveRunner>,
    /// 宿主侧簿记写方（`termination` 族）：驱动前后各观察一次。
    writers: Vec<Arc<dyn TerminationWriter>>,
    /// 装配好的调用方上下文（含能力表）；原样交给执行体，宿主不解释。
    ctx: Arc<dyn PluginInvokeRequest>,
}

/// 宿主实现的中性能力束：事实读写、执行体解析、存在性、时钟与限额。
struct HostDriveSupport {
    /// 事实源 → 中性事实的**投影**（游标增量；不是第二本账）。
    projection: Mutex<Projection>,
    /// 本轮计划的事实快照：`rows()` 刷新一次后缓存，`exists*` 复用同一份。
    ///
    /// 计划的一致性读面（`D-343`）：`rows()` 与 `exists*` 若各刷一次，两次读之间
    /// 落格的事实会让判据分叉（如 `session.config` 让债表判「没有」而选债判「有」）。
    plan: Mutex<Option<Arc<Vec<DriveFact>>>>,
    source: Arc<dyn FactSource>,
    executors: HashMap<String, Arc<dyn PortExecutor>>,
    sources: Vec<Arc<dyn DeclaredDebtSource>>,
    session_id: String,
    provider: Option<String>,
    /// 本会话主体（开面时从 `agent_id` 解析，随债表请求下传；D-339）。
    principal: String,
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

impl HostDriveSupport {
    /// 本轮计划的事实快照：`rows()` 已刷新过就复用，否则刷新一次并缓存。
    fn plan_facts(&self) -> Arc<Vec<DriveFact>> {
        let cached = self
            .plan
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        if let Some(facts) = cached {
            return facts;
        }
        let facts = Arc::new(self.facts());
        *self
            .plan
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Arc::clone(&facts));
        facts
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
        //
        // **拒收必须留痕**（D-396）：`Duplicate` 是幂等吸收，不算拒收；其余失败
        //（`OffGrid` / `Invariant` / 非写者 / I/O）若静默吞掉，环会拿着一条假「已落」
        // 继续派发，债务永不收束——t46 的哑故障正是从这里开始的。
        let kind = fact.kind.clone();
        let port = fact.key.port.clone();
        let id = fact.key.id.clone();
        let draft = FactDraft {
            key: ring_event_id(&fact),
            kind: fact.kind.clone(),
            entity: Entity::Scope,
            verb: Verb::Asserted,
            payload: fact.payload.clone(),
            produced_by: provenance_at(&*self.source),
        };
        if let Err(err) = self.source.append("loop".to_string(), draft) {
            // 契约层把失败压成字符串（`AppendError` 的 `Debug` 形态），幂等命中即
            // 裸 `Duplicate`——正常路径，不按错误报。
            if err != "Duplicate" {
                symbio_plugin_api::plugin_error!(
                    "drive",
                    "[Loop] 协议事实落格被拒：kind={} port={} id={} err={}",
                    kind,
                    port,
                    id,
                    err
                );
            }
        }
    }

    fn rows(&self) -> Vec<DriveRow> {
        let request = DriveRequest {
            session_id: self.session_id.clone(),
            provider: self.provider.clone(),
            principal: self.principal.clone(),
        };
        // 一轮计划只刷新一次：这份快照同时喂给债表与随后的 `exists*`（`D-343`）。
        let snapshot = Arc::new(self.facts());
        *self
            .plan
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Arc::clone(&snapshot));
        let declared = declared_facts(&snapshot);
        let mut rows = Vec::new();
        for source in &self.sources {
            for spec in source.rows(&request, &declared) {
                rows.push(DriveRow {
                    name: spec.name,
                    port: spec.port.to_string(),
                    id: spec.id,
                    payload: spec.payload,
                    after: spec.after,
                    when: spec.when,
                    when_id: spec.when_id,
                    unless: spec.unless,
                    when_no_open: spec.when_no_open,
                    after_port: spec.after_port,
                    lease_ms: spec.lease_ms,
                    max_attempts: spec.max_attempts,
                    role: spec.role,
                    accounts_usage: spec.accounts_usage,
                    accounts_spend: spec.accounts_spend,
                    failure: spec.failure,
                });
            }
        }
        rows
    }

    fn executor(&self, port: &str) -> Option<Arc<dyn PortExecutor>> {
        self.executors.get(port).cloned()
    }

    /// 读**本轮计划**的快照（`D-343`）：与 `rows()` 同一份，不再各自刷新。
    fn exists(&self, kind: &str) -> bool {
        self.plan_facts().iter().any(|fact| fact.kind == kind)
    }

    /// 与 [`Self::exists`] 同一条读面，只多看一位身份。
    ///
    /// 世界事实的投影身份就是入库 `event_id`（协议事实则是债 id）。
    fn exists_id(&self, kind: &str, id: &str) -> bool {
        self.plan_facts()
            .iter()
            .any(|fact| fact.kind == kind && fact.key.id == id)
    }

    /// 与 [`Self::exists`] 同一条读面，只多问一句位置。
    fn exists_since(&self, kind: &str, since_ms: u64) -> bool {
        self.plan_facts()
            .iter()
            .any(|fact| fact.kind == kind && fact.at_ms > since_ms)
    }

    /// 同轮派发上限：**恒为串行**。
    ///
    /// 环按轮推进，同一会话不存在两笔可并行的债（`Waiting` 在飞时本轮就结束）；
    /// 因此这个上限是常量，不是配置项——宿主不持有它，调用方也设不了。
    fn concurrency(&self) -> usize {
        1
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

    /// 事件原样（含入库 `seq`）：行执行体的 `RowRequest` 与区间判断读它（D-338）。
    fn events(&self) -> Vec<symbio_kernel::event::Event> {
        self.source.facts()
    }
}

/// 事实源的写口：草稿按草稿的幂等键落行；重复投递被事实源吸收。
struct WorldWriter {
    source: Arc<dyn FactSource>,
}

/// 事件声明给出的落格归属主体（D-339）。
///
/// 先查冻结表；只链接了部分插件的用例再查共享词汇表。两处都没有说明这条事实
/// 在声明期就没有归属——写口**拒绝在落账时猜一个默认值**。
fn declared_owner(kind: &str) -> &'static str {
    symbio_plugin_api::sealed_declarations()
        .ok()
        .and_then(|frozen| frozen.event_writer(kind))
        .or_else(|| {
            symbio_plugin_api::events::SHARED_EVENTS
                .iter()
                .find(|event| event.name == kind)
                .map(|event| event.owner)
        })
        .unwrap_or_else(|| {
            // panic-allow crates/symbio-host/src/lib.rs::declared_owner: 写口拒绝在落账时猜归属——声明表里查不到的 kind 说明声明期就漏了，静默落一条无主事实比崩掉更难查（D-339）。
            panic!("事实 `{kind}` 没有声明归属主体：写口拒绝在落账时猜默认值（D-339）")
        })
}

#[async_trait::async_trait]
impl PortFactWriter for WorldWriter {
    async fn emit(&self, facts: Vec<FactDraft>) {
        // 无归属的通用出口（工具 / 模型端口）：归属仍只从声明表来，不再沿用宿主身份。
        for fact in facts {
            let owner = declared_owner(&fact.kind);
            self.emit_owned(owner, vec![fact]).await;
        }
    }

    /// 归属位由机制侧从**声明表**取得后交来（`Frozen::event_writer` 按草稿的 kind 查
    /// 声明主体），写口只按它落行——**不读草稿里的任何归属暗示**：`FactDraft` 没有
    /// `actor` 字段，行为也无权指定归属（DE-026 D-252）。
    async fn emit_owned(&self, owner: &str, facts: Vec<FactDraft>) {
        for fact in facts {
            let mut draft = fact;
            // 草稿自带溯源时用它；否则指向本会话里刚落地的那条事实（I2 的声明类
            // 不接受无源）。空会话不填——那是真正的无源声明，交给事实源拒绝。
            draft.produced_by = draft.produced_by.or_else(|| provenance_at(&*self.source));
            // 世界事实的拒收同样不许静默：落不进格 = 写口拒了，不是「写成功了但看不见」。
            let kind = draft.kind.clone();
            let key = draft.key.clone();
            if let Err(err) = self.source.append(owner.to_string(), draft) {
                if err != "Duplicate" {
                    symbio_plugin_api::plugin_error!(
                        "host",
                        "[Writer] 世界事实落格被拒：owner={} kind={} key={} err={}",
                        owner,
                        kind,
                        key,
                        err
                    );
                }
            }
        }
    }
}

/// 把同一份只读投影交给全部簿记写方，并按声明表落回它们交回的草稿（D-367）。
///
/// 这里只有两件事：**读投影**、**按声明写**。宿主不解释谓词，也不给任何写方
/// 开「直接落一条业务事实」的后门——草稿的 owner 仍由 [`declared_owner`] 从事件
/// 声明解析，与通用世界写口同一条路径。
async fn observe_termination(
    writers: &[Arc<dyn TerminationWriter>],
    support: &dyn DriveSupport,
    source: &Arc<dyn FactSource>,
    exhausted: bool,
) {
    if writers.is_empty() {
        return;
    }
    let facts = support.facts();
    let view = TerminationView {
        facts: &facts,
        exhausted,
    };
    let outlet = WorldWriter {
        source: Arc::clone(source),
    };
    for writer in writers {
        for draft in writer.observe(&view) {
            let owner = declared_owner(&draft.kind);
            outlet.emit_owned(owner, vec![draft]).await;
        }
    }
}

/// 五事实与全种类事实身份 → 申报方读的窄视图（`D-343`：只读本轮计划那一份快照）。
fn declared_facts(facts: &[DriveFact]) -> DebtFacts {
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
        // 世界事实的投影端口位为空（协议事实的端口位由 `ring:` 身份还原）。
        if fact.key.port.is_empty() {
            out.note_fact(&fact.kind, &fact.key.id);
            // 载荷只读副本：申报方要拿「在等」事实里的提示正文构造请求（D-338）。
            out.note_fact_payload(&fact.kind, &fact.key.id, fact.payload.clone());
        }
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
    let Some((port, id, attempt)) = symbio_kernel::event::ring_parts(event_id, kind) else {
        return (String::new(), event_id.to_string(), 0);
    };
    let Ok(attempt) = attempt.parse::<u32>() else {
        return (String::new(), event_id.to_string(), 0);
    };
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
    // 端口位是 `rest` 的第一个段（`ring:{port}:{id}:…`），不是第二个；`id` 自身可含
    // `:`，只能从右定界（见 [`ring_key_of`]）。端口名含 `:` 会在注册期被拒绝
    // （`port_rows` 的保留字符校验，D-336），所以这里没有歧义。
    rest.split(':').next().unwrap_or_default().to_string()
}

/// 便利构造：一次驱动用的基上下文（前端只需补工作目录等）。
///
/// 带 `AGENT_ID` 载体（空值 = 未绑定智能体，主体按定义为 `session/main`）；
/// 驱动侧只认这个载体，不再从缺失值派生主体（D-339）。
pub fn base_context() -> Arc<dyn PluginInvokeRequest> {
    let meta = serde_json::Map::from_iter([(
        symbio_plugin_api::AGENT_ID.name().to_string(),
        serde_json::json!(""),
    )]);
    Arc::new(PluginSimpleRequest::from_parts(
        serde_json::json!({}),
        Some(&meta),
    ))
}

#[cfg(test)]
mod tests;
