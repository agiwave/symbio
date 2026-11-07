//! 环本体：选债 → 派发 → 等待 → 配对 → 记账 → 预算与租约 → 重放。
//!
//! 入口只有 [`drive`]；环不持有账本、预算、执行体注册表或任何跨步可变状态——
//! 每一步都从 [`DriveCtx`] 提供的只读投影重算。

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use futures::future::join_all;
use serde_json::{json, Value};
use symbio_plugin_api::drive::Unless;
use symbio_plugin_api::effects::FactDraft;
use symbio_plugin_api::port::{
    PortErrorClass, PortExecContext, PortExecHandles, PortExecutor, PortFactEmit, PortFactReader,
    PortFactWriter, PortInput, PortOutcome,
};
use symbio_plugin_api::ExecAbortSignal;

use crate::ring::facts::{DebtKey, Fact, FactKind, FactView};
use crate::ring::table::{DebtRow, DebtSource};

/// 单次驱动的步数上界（安全阀，不是宿主配置）：一圈一次 `plan`。
///
/// 取 128 的理由：正常一轮的步数 ≈ 债的「波次」——同一圈的批量派发只算一步，
/// 十几轮工具往返也只有几十步。再高就不是现实负载，而是环在自旋；上界是环对
/// 自身推进的硬约束，不该由宿主配置调没（所以不放 `DriveSupport` 的配置面）。
const STEP_BUDGET: usize = 128;

/// 宿主交给环的全部输入：事实投影的读写、债表、执行体解析与时钟。
///
/// 缓存（若有）由宿主自己持有；环每一步重新读一遍，跨步不保留任何东西。
pub(crate) trait DriveCtx: Send + Sync {
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

    /// 单次驱动的步数上界（安全阀；不是宿主配置）。默认 [`STEP_BUDGET`]；
    /// 测试用它把预算压小以覆盖出口。
    fn step_budget(&self) -> usize {
        STEP_BUDGET
    }

    /// 非五类事实的去处（世界事实留在应用侧写口）。
    fn outlet(&self) -> PortFactEmit {
        PortFactEmit::silent()
    }

    /// 交给执行体的事实读面（宿主实现）。环只**原样转交**，不调用、不解释。
    fn fact_reader(&self) -> Option<Arc<dyn PortFactReader>> {
        None
    }

    /// 交给执行体的不透明句柄束（宿主实现）。环只**原样转交**，不读取其中任何字段。
    fn exec_handles(&self) -> PortExecHandles {
        PortExecHandles::default()
    }
}

/// 驱动出口。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DriveOutcome {
    /// 没有可还的债，且账上没有未收束的债。
    NoDebt,
    /// 仍有未收束的债，但本轮没有可动作的：在飞（租约未过）、挂起，或开着却
    /// 没人能还（如被 `when_no_open` 挡住、又无别的行可派）。宿主不得据此记 `idle`。
    Waiting(Vec<DebtKey>),
    /// 步数预算用尽：还欠着债，本轮不再继续（调用方必须看见，不能静默）。
    Exhausted {
        /// 实际走完的步数（一圈一次 `plan`）。
        steps: usize,
        /// 仍未收束的债。
        outstanding: Vec<DebtKey>,
    },
    /// 一圈没有新增任何事实：继续跑只会自旋，因此停下并交回未收束的债。
    Stalled(Vec<DebtKey>),
}

/// 一次驱动报告。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RunReport {
    /// 出口。
    pub(crate) outcome: DriveOutcome,
    /// 本轮实际派发笔数。
    pub(crate) dispatched: usize,
}

/// 选债结果：这一圈的全部可动作。
#[derive(Debug, Clone, Default)]
struct Work {
    /// 可派发（可能先补 `request`）。
    dispatch: Vec<(DebtRow, u32)>,
    /// 租约已过：推进尝试或判死。
    expire: Vec<(DebtRow, u32, Option<u32>)>,
    /// 已发未配，租约未过。
    waiting: Vec<DebtKey>,
}

/// 驱动本会话到没有可还的债、只剩在飞（或明确告诉调用方跑不完）。
pub(crate) async fn drive(ctx: &dyn DriveCtx) -> RunReport {
    let mut dispatched = 0usize;
    let mut steps = 0usize;
    loop {
        let facts = ctx.facts();
        let view = FactView::new(&facts);
        // 步数上界：一圈一次 `plan`。超界不静默——把还开着的债交回调用方，
        // 由它当成「这一轮没跑完」处理（宿主因此不会记 `idle`）。
        if steps >= ctx.step_budget().max(1) {
            return RunReport {
                outcome: DriveOutcome::Exhausted {
                    steps,
                    outstanding: view.unresolved(),
                },
                dispatched,
            };
        }
        steps += 1;

        let source = ctx.source();
        let rows = source.rows(&view);
        let now_ms = ctx.now_ms();
        let work = plan(&view, source, &rows, now_ms);

        if !work.dispatch.is_empty() {
            let cap = ctx.concurrency().max(1);
            for chunk in work.dispatch.chunks(cap) {
                let run = chunk
                    .iter()
                    .map(|(row, attempt)| dispatch(ctx, row.clone(), *attempt, now_ms));
                join_all(run).await;
                dispatched += chunk.len();
            }
            if !advanced(ctx, &facts) {
                return stalled(ctx, dispatched);
            }
            continue;
        }
        if !work.expire.is_empty() {
            for (row, attempt, next) in work.expire {
                expire(ctx, row, attempt, next, now_ms);
            }
            if !advanced(ctx, &facts) {
                return stalled(ctx, dispatched);
            }
            continue;
        }
        // `NoDebt` 只许在**没有任何未收束债**时用：行被 `when_no_open` 之类挡住、
        // 又没有别的行可派时，账上仍开着债。此时报 `NoDebt` 会让宿主把 `idle`
        // 记成「正常完成」，轮静默停摆（无终答、无失败格）。开着的债即使暂时
        // 没人能还，也必须被调用方看见。
        let mut outstanding = work.waiting;
        for key in view.unresolved() {
            if !outstanding.contains(&key) {
                outstanding.push(key);
            }
        }
        let outcome = if outstanding.is_empty() {
            DriveOutcome::NoDebt
        } else {
            DriveOutcome::Waiting(outstanding)
        };
        return RunReport {
            outcome,
            dispatched,
        };
    }
}

/// 这一圈有没有新增事实（按事实身份 `(kind, key)` 判；五类事实按这一对吸收重复）。
///
/// 判据落在**事实层面**，不数「连续几圈没进展」——后者是在拍一个数字，且会把
/// 「一圈派发、下一圈收束」这种正常节律误判成停滞。一圈有动作却一条新事实都没
/// 落下，只可能是写口把全部投递都吸收了：再跑下一圈还是同一结果，必须停下。
fn advanced(ctx: &dyn DriveCtx, before: &[Fact]) -> bool {
    let known: HashSet<(FactKind, DebtKey)> =
        before.iter().map(|f| (f.kind, f.key.clone())).collect();
    ctx.facts()
        .iter()
        .any(|f| !known.contains(&(f.kind, f.key.clone())))
}

/// 无进展出口：交回当前账上仍未收束的债。
fn stalled(ctx: &dyn DriveCtx, dispatched: usize) -> RunReport {
    let facts = ctx.facts();
    let view = FactView::new(&facts);
    RunReport {
        outcome: DriveOutcome::Stalled(view.unresolved()),
        dispatched,
    }
}

/// 选这一圈的全部可还之债：可派发的进批，租约已过的排进过期，其余在飞收进等待。
fn plan(view: &FactView<'_>, source: &dyn DebtSource, rows: &[DebtRow], now_ms: u64) -> Work {
    let mut work = Work::default();
    let mut seen = HashSet::new();
    for row in rows {
        if let Some(after) = &row.after {
            if view.closing(after).is_none() {
                continue;
            }
        }
        // 触发条件是**种类**的存在性：环只问宿主「有没有这种事实」，
        // 不解释它是什么（字符串透传，归属：选债）。
        if let Some(when) = &row.when {
            if !source.exists(when) {
                continue;
            }
        }
        // 输入依赖的身份：执行体要读的那条事实必须**这一轮的**那条，不是旧的那条。
        // 与 `when` 走同一条读面（宿主的 `exists_id`），环仍不解释种类。
        if let Some((kind, id)) = &row.when_id {
            if !source.exists_id(kind, id) {
                continue;
            }
        }
        // 「该端口的债全部还清才欠」读的是债状态，不是事实状态（归属：选债）。
        if let Some(port) = row.when_no_open {
            if view
                .unresolved()
                .iter()
                .any(|key| key.port.as_str() == port)
            {
                continue;
            }
        }
        // 排除条件与 `when` 走同一条读面（世界事实，按种类读；归属：选债）。
        // 债表永不读载荷：需要按内容分叉时，由写方把结论写成一条具体事实。
        if let Some(unless) = &row.unless {
            let hit = match unless {
                Unless::Kind(kind) => source.exists(kind),
                Unless::Id { kind, id } => source.exists_id(kind, id),
            };
            if hit {
                continue;
            }
        }
        let attempt = view.attempt(row.port, &row.id).max(1);
        let key = DebtKey::new(row.port, row.id.clone(), attempt);
        if view.closing(&key).is_some() {
            continue;
        }
        let Some(sent) = view.last_sent(&key) else {
            // 幂等底线：同一 (port, id, attempt) 一圈只入批一次。
            if seen.insert(key.clone()) {
                work.dispatch.push((row.clone(), attempt));
            }
            continue;
        };
        // 挂起（派发结局第三态）：`sent` 的 `deadline_ms` 为 `null` —— 租约让出与
        // 「已挂起」是同一个字段。它既不到期重试（那会 `attempt+1`），也不立刻重派
        // （那会自旋）：只有**挂起点之后**出现的触发事实才能重新点燃这一笔。
        if sent.payload.get("deadline_ms").is_some_and(Value::is_null) {
            let retriggered = row
                .when
                .as_deref()
                .is_some_and(|kind| source.exists_since(kind, sent.at_ms));
            if retriggered {
                if seen.insert(key.clone()) {
                    work.dispatch.push((row.clone(), attempt));
                }
            } else if !work.waiting.contains(&key) {
                work.waiting.push(key);
            }
            continue;
        }
        // 五类配对账：同一 (port, id, attempt) 一圈只入批一次。
        if !seen.insert(key.clone()) {
            continue;
        }
        let deadline = sent.at_ms.saturating_add(row.lease_ms);
        if now_ms < deadline {
            work.waiting.push(key);
            continue;
        }
        let next = (attempt < row.max_attempts).then(|| attempt + 1);
        work.expire.push((row.clone(), attempt, next));
    }
    work
}

/// 租约已过：有下一笔就记账并再举，没有就判死。
fn expire(ctx: &dyn DriveCtx, row: DebtRow, attempt: u32, next: Option<u32>, now_ms: u64) {
    let key = DebtKey::new(row.port, row.id.clone(), attempt);
    match next {
        Some(next) => {
            let next_key = DebtKey::new(row.port, row.id.clone(), next);
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

/// 派发一笔：补 `request` → 调执行体 → 按结局落 `sent` 与配对账。
///
/// `sent` 落在执行体**之后**：挂起结局要让它与「租约让出」在同一个字段里表达
/// （`deadline_ms: null`），而事实源按 `(kind, key)` 吸收重复——先落一条带租约的
/// `sent` 就再也改不成挂起。三种结局：
/// - 正常 / 可重试 / 判死：`sent{deadline_ms}` + 对应配对账；
/// - 挂起：只落 `sent{deadline_ms: null}`，**不落收束**，债继续开着。
async fn dispatch(ctx: &dyn DriveCtx, row: DebtRow, attempt: u32, now_ms: u64) {
    // 端口名要跟着收束载荷走：读侧按 `port` 判「这一次往返算不算对读者说的话」。
    let port = row.port;
    let key = DebtKey::new(row.port, row.id.clone(), attempt);
    let handles = ctx.exec_handles();
    // 中止是**本轮**的（`DE-047` 驱动级信号）：置位后，每一笔还没起活的派发都先看
    // 它一眼。已取消 ⇒ 不再调执行体，按中止关账——否则「用户按下中止、下一笔工具
    // 照常跑」，中止退化成「只打断当前这一次调用」。
    let abort = handles
        .ctx
        .as_deref()
        .map(ExecAbortSignal::of)
        .unwrap_or_default();
    if abort.is_aborted() {
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
        ctx.append(Fact::new(
            FactKind::Failed,
            key,
            json!({
                // 产出身份跟着收束载荷走（`failed` 的声明字段含 `root_id`）：
                // 读侧按端口 + 本轮开端折正文节点，缺了它这一笔在重建面静默消失。
                "root_id": row.payload.get("root_id").cloned().unwrap_or(Value::Null),
                "port": port,
                "class": "non_retryable",
                "reason": "aborted",
                "status": "aborted",
            }),
            now_ms,
        ));
        return;
    }
    ctx.append(Fact::new(
        FactKind::Request,
        key.clone(),
        row.payload.clone(),
        now_ms,
    ));

    let Some(executor) = ctx.executor(row.port) else {
        ctx.append(Fact::new(
            FactKind::Sent,
            key.clone(),
            json!({"deadline_ms": now_ms.saturating_add(row.lease_ms)}),
            now_ms,
        ));
        ctx.append(Fact::new(
            FactKind::Failed,
            key,
            json!({
                "class": "non_retryable",
                "reason": format!("no provider for port {} (row {})", row.port, row.name),
            }),
            now_ms,
        ));
        return;
    };

    let writer = Arc::new(CollectingWriter::new(ctx.outlet()));
    let exec = PortExecContext {
        deadline_ms: Some(now_ms.saturating_add(row.lease_ms)),
        // 交给执行体的是**本轮同一个**信号（从上一条派发的 ctx 读回），不是现造的死信号。
        abort,
        idempotency_key: format!("{}:{}:{}", row.port, row.id, attempt),
        sink: handles.sink,
        emit: PortFactEmit::new(writer.clone()),
        ctx: handles.ctx,
        subject: format!("{}:{}:{}", row.port, row.id, attempt),
        group: String::new(),
        origin: None,
        permitted: true,
        deny_reason: String::new(),
        scratch_dir: handles.scratch_dir,
        session_id: handles.session_id,
        source: handles.source,
        reader: ctx.fact_reader(),
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
        // 第三态「挂起」：`request` + `sent` 之后不落收束。`deadline_ms: null`
        // 同时表达「已挂起」与「租约已让出」——计划层读到它只会等新触发。
        Some((PortErrorClass::NeedsHuman, _why)) => {
            ctx.append(Fact::new(
                FactKind::Sent,
                key,
                json!({"deadline_ms": Value::Null}),
                now_ms,
            ));
        }
        Some((class, why)) => {
            ctx.append(Fact::new(
                FactKind::Sent,
                key.clone(),
                json!({"deadline_ms": now_ms.saturating_add(row.lease_ms)}),
                now_ms,
            ));
            if class == PortErrorClass::Retryable && attempt < row.max_attempts {
                let next_key = DebtKey::new(row.port, row.id, attempt + 1);
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
                    failure_payload(&outcome.facts, port, class, &why),
                    now_ms,
                ));
            }
        }
        None => {
            ctx.append(Fact::new(
                FactKind::Sent,
                key.clone(),
                json!({"deadline_ms": now_ms.saturating_add(row.lease_ms)}),
                now_ms,
            ));
            let payloads = response_payloads(&drafts, &outcome, port);
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
///
/// 环把**端口名**补进载荷（执行体不自报，也不该自报「我是哪个端口」）：读侧
/// （`rebuild::output_nodes`）按 `port == "model"` 判这一次往返是否对读者说话，
/// 缺了它，正文 / 思考节点在重建面全部静默消失。
fn response_payloads(drafts: &[FactDraft], outcome: &PortOutcome, port: &str) -> Vec<Value> {
    let emitted: Vec<Value> = drafts
        .iter()
        .filter(|d| d.kind == FactKind::Response.as_str())
        .map(|d| d.payload.clone())
        .collect();
    let raw = if !emitted.is_empty() {
        emitted
    } else {
        match outcome.facts.as_slice() {
            [] => vec![json!({})],
            [only] => vec![only.clone()],
            many => vec![Value::Array(many.to_vec())],
        }
    };
    raw.into_iter()
        .map(|payload| stamp_port(payload, port))
        .collect()
}

/// 「端口」是环的身份位：对象载荷直接补键，非对象载荷包一层。
fn stamp_port(payload: Value, port: &str) -> Value {
    match payload {
        Value::Object(mut map) => {
            map.insert("port".to_string(), Value::String(port.to_string()));
            Value::Object(map)
        }
        other => json!({ "port": port, "facts": other }),
    }
}

/// 失败载荷：与 `response` **同一条产出形状**（环补的 `port` + 执行体交回的产出），
/// 只多 `class` / `reason`。
///
/// `failed` 的声明字段本来就含 `port` / `root_id` / `text` / `reasoning` / `answer`
/// 等（`symbio-kernel::decl::PROTOCOL_FACTS`），读侧（`rebuild::output_nodes`）正是
/// 按 `port == "model"` 折正文 / 思考节点。失败若把这些产出裹进 `facts` 数组，
/// 读侧只看得见顶层——被截断的思考会随失败一起消失（t77）；端口也必须与成功路径
/// （`response_payloads`）同一套补法，否则同一次往返在两种结局下是两个身份。
fn failure_payload(facts: &[Value], port: &str, class: PortErrorClass, why: &str) -> Value {
    let body = match facts {
        [] => json!({}),
        [only] => only.clone(),
        many => Value::Array(many.to_vec()),
    };
    let mut payload = stamp_port(body, port);
    if let Some(map) = payload.as_object_mut() {
        map.insert("class".to_string(), json!(class_name(class)));
        // 环侧读原因用 `reason`（notes 的 `reply_of` 与会话收束都读它）；执行体
        // 自带的 `why` 不覆盖，两个键同义并存。
        map.insert("reason".to_string(), json!(why));
    }
    payload
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
