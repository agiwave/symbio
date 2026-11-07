//! 环本体：选债 → 派发 → 等待 → 配对 → 记账 → 预算与租约 → 重放。
//!
//! 入口只有 [`drive`]；环不持有账本、预算、执行体注册表或任何跨步可变状态——
//! 每一步都从 [`DriveCtx`] 提供的只读投影重算。

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use futures::future::join_all;
use serde_json::{json, Value};
use symbio_kernel::decl::ring::owed;
use symbio_plugin_api::drive::Unless;
use symbio_plugin_api::effects::FactDraft;
use symbio_plugin_api::port::{
    PortErrorClass, PortExecContext, PortExecHandles, PortExecutor, PortFact, PortFactEmit,
    PortFactReader, PortFactWriter, PortInput, PortOutcome,
};
use symbio_plugin_api::ExecAbortSignal;
use tokio::task::JoinHandle;

use crate::ring::facts::{DebtKey, Fact, FactKind, FactView};
use crate::ring::table::{DebtRow, DebtSource};

/// 单次驱动的步数上界（安全阀，不是宿主配置）：一圈一次 `plan`。
///
/// 取 128 的理由：正常一轮的步数 ≈ 债的「波次」——同一圈的批量派发只算一步，
/// 十几轮工具往返也只有几十步。再高就不是现实负载，而是环在自旋；上界是环对
/// 自身推进的硬约束，不该由宿主配置调没（所以不放 `DriveSupport` 的配置面）。
const STEP_BUDGET: usize = 128;

/// 宿主交给环的全部输入：事实投影的读写、债表与执行体解析。
///
/// 缓存（若有）由宿主自己持有；环每一步重新读一遍，跨步不保留任何东西。
///
/// **时钟不在这里**：`now` 只取事实的 `at_ms` 最大值（`D-376`）。要推进时间就落一条
/// 带更晚 `at_ms` 的事实——读墙钟会让同一份事实日志重放两次得到不同结果。
pub(crate) trait DriveCtx: Send + Sync {
    /// 当前可见域的事实快照（只读投影）。
    fn facts(&self) -> Vec<Fact>;

    /// 落一条事实；重复投递的去重规则归宿主（按事实身份）。
    fn append(&self, fact: Fact);

    /// 债表数据源（应用侧的行数据）。
    fn source(&self) -> &dyn DebtSource;

    /// 按端口名解析执行体——清单发现归宿主，环不持有注册表。
    fn executor(&self, port: &str) -> Option<Arc<dyn PortExecutor>>;

    /// 同一份上下文的可持有句柄：派发出去的执行体在后台完成时要靠它回写。
    ///
    /// 返回的是**同一份事实账**的共享视图（实现方克隆的是句柄，不是账本）。
    fn shared(&self) -> Arc<dyn DriveCtx>;

    /// 环的「现在」：事实时间轴上最晚的一笔（`D-376`），不读墙钟。
    ///
    /// 默认只从五类事实取。宿主实现应覆盖成**全部**事实（含世界事实）：触发一次
    /// 驱动的往往是世界事实（用户消息 / 工具回执），它的 `ts` 就是这一圈的起点；
    /// 只看五类事实会在首圈得到 0，把刚派发的租约算成「早已过期」。
    fn now_ms(&self) -> u64 {
        latest_ts(&self.facts())
    }

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
    /// 只剩**在飞的债**（租约未过）：环自己还能推进（应答落格 / 租约到期都会再踢），
    /// 宿主**不得**据此记 `idle`，也不得据此投影。
    Waiting(Vec<DebtKey>),
    /// 只剩**需要外部事实才能推进**的债（挂起：等答复 / 授权 / 超时唤醒）。环自己
    /// 已无可动作，把控制权交还调用方——这是正常的一轮结束，不是停摆。
    NeedsHuman(Vec<DebtKey>),
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
    /// 已发未配，租约未过：**在飞**（环自己还能推进）。
    in_flight: Vec<DebtKey>,
    /// 挂起（`sent{deadline_ms:null}`）：只有挂起点之后的触发事实能重新点燃。
    /// 这一格不是「在飞」——没有执行体在跑，只有外部事实能推进。
    suspended: Vec<DebtKey>,
    /// 被 `after_port` 前提挡住的行：该端口这一圈有人认领，所以本行不进计划。
    ///
    /// 只用于出口：一圈没有动作、账上也没有未收束债，但这里非空 ⇒ 有人在等一笔
    /// 没人能还的前提。必须报 `Stalled`，**不能报 `NoDebt`**（否则就是静默停摆）。
    blocked: Vec<DebtKey>,
}

/// 驱动本会话到没有可还的债、只剩在飞或只剩等外部事实（或明确告诉调用方跑不完）。
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
        // `now` 只来自事实（`D-376`）：同一份日志重放两次，租约的判定逐字节相同。
        let now_ms = ctx.now_ms();
        let work = plan(&view, source, &rows, now_ms);

        if !work.dispatch.is_empty() {
            let cap = ctx.concurrency().max(1);
            // 执行体的读面固定在**这一圈计划时**的快照上（`D-376`）：同一批里别的
            // 执行体随后落下的事实不进本笔的提示词 / 行输入。否则「收尾轮读到同轮
            // 摘要」这类先后就成了调度器的机缘——同一份事实日志重放两次，请求未必
            // 逐字节相同。计划本来就只用一次快照（`D-343`），读面与之对齐。
            let reader = ctx.fact_reader().map(|reader| {
                Arc::new(SnapshotReader::snapshot(&*reader)) as Arc<dyn PortFactReader>
            });
            let mut handles = Vec::new();
            for chunk in work.dispatch.chunks(cap) {
                let run = chunk.iter().map(|(row, attempt)| {
                    dispatch(ctx, row.clone(), *attempt, now_ms, reader.clone())
                });
                for handle in join_all(run).await.into_iter().flatten() {
                    handles.push(handle);
                }
                dispatched += chunk.len();
            }
            // 已经跑完的执行体在本圈内收账（保持「派发即完成」的既有语义）；
            // 挂死的那个不阻塞环：几轮让出后仍未完的留在后台，完成时自己回写并再踢。
            wait_ready(&mut handles).await;
            if !advanced(ctx, &facts) {
                return stalled(ctx, dispatched, work.blocked.clone());
            }
            continue;
        }
        if !work.expire.is_empty() {
            for (row, attempt, next) in work.expire {
                expire(ctx, row, attempt, next, now_ms);
            }
            if !advanced(ctx, &facts) {
                return stalled(ctx, dispatched, work.blocked);
            }
            continue;
        }
        // `NoDebt` 只许在**没有任何未收束债**时用：行被 `when_no_open` 之类挡住、
        // 又没有别的行可派时，账上仍开着债。此时报 `NoDebt` 会让宿主把 `idle`
        // 记成「正常完成」，轮静默停摆（无终答、无失败格）。开着的债即使暂时
        // 没人能还，也必须被调用方看见。
        //
        // 两种「等」必须分开（t29）：在飞 = 环自己还能推进，继续等；挂起 = 只有
        // 外部事实能推进，这才是交还控制权的时刻。混成一个出口会让调用方在
        // 「应答还没落到库」的空档去投影 / 收口。
        let in_flight_pending = !work.in_flight.is_empty();
        let mut outstanding = work.in_flight;
        for key in work.suspended {
            if !outstanding.contains(&key) {
                outstanding.push(key);
            }
        }
        for key in view.unresolved() {
            if !outstanding.contains(&key) {
                outstanding.push(key);
            }
        }
        let outcome = if outstanding.is_empty() {
            if work.blocked.is_empty() {
                DriveOutcome::NoDebt
            } else {
                // 依赖方被前提挡住，前提端口却没有任何本圈动作（与未收束债）：
                // 没有人能还、也没有人能认领下来。报 `NoDebt` 会让宿主把 `idle`
                // 记成正常完成——这里必须可见。
                DriveOutcome::Stalled(work.blocked)
            }
        } else if in_flight_pending {
            DriveOutcome::Waiting(outstanding)
        } else {
            DriveOutcome::NeedsHuman(outstanding)
        };
        return RunReport {
            outcome,
            dispatched,
        };
    }
}

/// 环的「现在」：事实时间轴上最晚的一笔（`D-376`）。
///
/// 不读墙钟：同一份事实日志重放两次，租约到期与否逐字节相同。要让时间前进，
/// 就落一条带更晚 `at_ms` 的事实——时间是账上的数据，不是进程的环境。
fn latest_ts(facts: &[Fact]) -> u64 {
    facts.iter().map(|f| f.at_ms).max().unwrap_or(0)
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

/// 让本圈已经跑完的后台执行体先收账；挂死的那个不挡路。
///
/// 既有语义是「派发即完成」，所以给刚 spawn 的任务几轮让出机会；仍未完成的
/// （真的挂住或等外部 I/O）留在后台，完成时经 [`finish`] 回写并再踢一次驱动。
/// 这里只等 `is_finished` 的句柄；不 `abort`，因为取消会让「迟归」不可验证，
/// 也可能把已经产生副作用的外部调用切成半截。
async fn wait_ready(handles: &mut Vec<JoinHandle<()>>) {
    for _ in 0..4 {
        if handles.iter().all(JoinHandle::is_finished) {
            break;
        }
        tokio::task::yield_now().await;
    }
    if handles.iter().all(JoinHandle::is_finished) {
        for handle in handles.drain(..) {
            // 已完成的执行体：`Err` 只可能是 panic / abort，出声不吞（S-002）。
            if let Err(err) = handle.await {
                symbio_plugin_api::plugin_warn!("loop", "执行体任务异常结束：{err}");
            }
        }
    }
}

/// 无进展出口：交回当前账上仍未收束的债，外加被 `after_port` 挡住的依赖方。
///
/// 后者虽然不是「已派发」，却仍然是**这一轮还欠着**的债：前提没走完，依赖方就
/// 不该被当作「没事」。把它交回调用方，才能让「门永不收束」停在可见的 `Stalled`。
fn stalled(ctx: &dyn DriveCtx, dispatched: usize, blocked: Vec<DebtKey>) -> RunReport {
    let facts = ctx.facts();
    let view = FactView::new(&facts);
    let mut outstanding = view.unresolved();
    for key in blocked {
        if !outstanding.contains(&key) {
            outstanding.push(key);
        }
    }
    RunReport {
        outcome: DriveOutcome::Stalled(outstanding),
        dispatched,
    }
}

/// 选这一圈的全部可还之债：可派发的进批，租约已过的排进过期，其余按「在飞」与
/// 「挂起」分表收进等待（两类的区别见 [`DriveOutcome::Waiting`] / [`DriveOutcome::NeedsHuman`]）。
fn plan(view: &FactView<'_>, source: &dyn DebtSource, rows: &[DebtRow], now_ms: u64) -> Work {
    // 声明行的求解**只在这里**做（`TC-1b`）：`owed` 是选债口的调用，数据源只交
    // 「冻结声明表 + 事实切片」，不交「欠哪几笔」的结论。放在数据源里先求一遍，
    // 就是派发器之外的第二个选债口。
    let mut all_rows: Vec<DebtRow> = rows.to_vec();
    if let Some(input) = source.declarations() {
        let debts = owed(
            input.frozen.rows(),
            &input.frozen,
            &input.events,
            now_ms as i64,
        );
        all_rows.extend(crate::plugin_rows::rows_from_debts(&debts, &input.events));
    }
    let rows = all_rows.as_slice();

    // 第一步只算「这一行这一圈是什么动作」，**不看** `after_port`：前提端口有没有
    // 认领，取决于同一份行集合里别的行——那是第二步用本圈算出的动作集合回答的。
    // 整个判断只用 `rows`（已由宿主按同一次事实快照求出），不第二次读事实（D-343）。
    let mut candidates: Vec<Candidate> = Vec::new();
    let mut seen = HashSet::new();
    // 前提端口：这一圈**有人认领**的端口。两处来源合起来才是完整的「认领」：
    // 1. 申报方这一轮报了这一行、且它那笔债还开着（已派发未收束）——**不看 `when`
    //    的判定**：`when` 读的是本圈计划快照（D-343），可能还没看见本圈刚落下的回执
    //    （例如分类刚收到 `response`、还没等到下一圈 parse），此时这笔债仍然是有人
    //    在做的活，依赖方不能抢跑；
    // 2. 下文选出的可动作行所在端口（尤其首派发的行还没有 `sent`）。
    //
    // 上一轮遗留的未收束债不算认领：债表按当轮事实现算，遗留债不会再被任何源申报，
    // 由它自己的租约判死收束，不该挡住新一轮（t49 恢复轮）。
    let mut claimed: HashSet<String> = HashSet::new();
    for row in rows {
        let attempt = view.attempt(row.port, &row.id).max(1);
        let key = DebtKey::new(row.port, row.id.clone(), attempt);
        if view.closing(&key).is_none() && view.last_sent(&key).is_some() {
            claimed.insert(row.port.to_string());
        }
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
        if view.closing(&key).is_some() {
            continue;
        }
        let Some(sent) = view.last_sent(&key) else {
            // 幂等底线：同一 (port, id, attempt) 一圈只入批一次。
            if seen.insert(key.clone()) {
                candidates.push(Candidate {
                    row: row.clone(),
                    key,
                    pick: Pick::Dispatch,
                });
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
                    candidates.push(Candidate {
                        row: row.clone(),
                        key,
                        pick: Pick::Dispatch,
                    });
                }
            } else if !candidates.iter().any(|c| c.key == key) {
                candidates.push(Candidate {
                    row: row.clone(),
                    key,
                    pick: Pick::Suspended,
                });
            }
            continue;
        }
        // 五类配对账：同一 (port, id, attempt) 一圈只入批一次。
        if !seen.insert(key.clone()) {
            continue;
        }
        let deadline = sent.at_ms.saturating_add(row.lease_ms);
        if now_ms < deadline {
            candidates.push(Candidate {
                row: row.clone(),
                key,
                pick: Pick::InFlight,
            });
            continue;
        }
        let next = (attempt < row.max_attempts).then(|| attempt + 1);
        candidates.push(Candidate {
            row: row.clone(),
            key,
            pick: Pick::Expire(next),
        });
    }

    // 入批顺序只由**行名与债 id** 决定，不继承装配顺序。
    //
    // `rows` 是宿主按能力收集顺序拼出来的表（`IndexMap` 插入序 = 插件挂载序），
    // 同一组能力换个装配顺序，派发顺序与落格次序就会跟着换——行为因此依赖一个
    // 与语义无关的偶然量。这里在分桶之前排一次全序，把顺序变成环自己的数据：
    // 同一组债、任意装配顺序，入批序列一致。
    //
    // 键取 `(行名, 债 id, 端口)`：前两位就是「名字与 id」，端口只用于不同端口恰好
    // 同名的两行；三者相同即同一笔债，谁先谁后没有语义差别。
    //
    // **排序不承担时序**：真的有时序要求的债必须用 `after` / `after_port` /
    // `when_no_open` 表达（谓词，见 DE-051 §1）。排序只消除并列者之间的偶然顺序，
    // 不会让任何一行提前——需要「先 A 后 B」的地方，顺序碰巧对不算成立。
    candidates.sort_by(|a, b| {
        (a.row.name, &a.row.id, a.row.port).cmp(&(b.row.name, &b.row.id, b.row.port))
    });

    // 第二处来源：这一圈**有动作**的行所在端口（首派发的行还没有 `sent`）。
    for candidate in &candidates {
        claimed.insert(candidate.row.port.to_string());
    }

    let mut work = Work::default();
    for candidate in candidates {
        if let Some(port) = candidate.row.after_port {
            if claimed.contains(port) {
                // 前提还没了：本行不进计划。记下来，出口据此证明「有人在等」。
                work.blocked.push(candidate.key);
                continue;
            }
        }
        match candidate.pick {
            Pick::Dispatch => work.dispatch.push((candidate.row, candidate.key.attempt)),
            Pick::Expire(next) => work
                .expire
                .push((candidate.row, candidate.key.attempt, next)),
            Pick::InFlight => work.in_flight.push(candidate.key),
            Pick::Suspended => work.suspended.push(candidate.key),
        }
    }
    work
}

/// 一行这一圈的动作（`after` / `when` / `when_id` / `unless` / `when_no_open` 已判过）。
struct Candidate {
    row: DebtRow,
    key: DebtKey,
    pick: Pick,
}

/// 动作类别；`after_port` 只决定这行「进不进计划」，不改变动作本身。
enum Pick {
    /// 可派发（可能先补 `request`）。
    Dispatch,
    /// 租约已过：推进尝试或判死。
    Expire(Option<u32>),
    /// 已发未配，租约未过：在飞。
    InFlight,
    /// 挂起：只有外部事实能推进。
    Suspended,
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

/// 派发一笔：补 `request` → 落 `sent` → 把执行体交给后台 → 按结局收账。
///
/// `sent` 是 `before` 事实，**先于执行体**落格（`DE-022`）：执行体挂死时租约已经在
/// 账上，下一圈 `plan` 看得到它，`expire` 能收束（`D-376`）。执行体因此不能挡在
/// `drive` 的路径上——它跑在后台任务里，完成时经 [`finish`] 回写并再踢一次。
///
/// 挂起不再改写 `sent`（事实源按 `(kind, key)` 吸收重复）：同 attempt 再落**第二条**
/// `sent`，载荷带 `shard` 区分、`deadline_ms: null`；计划层取最后一条，债继续开着。
async fn dispatch(
    ctx: &dyn DriveCtx,
    row: DebtRow,
    attempt: u32,
    now_ms: u64,
    reader: Option<Arc<dyn PortFactReader>>,
) -> Option<JoinHandle<()>> {
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
        return None;
    }
    let Some(executor) = ctx.executor(row.port) else {
        ctx.append(Fact::new(
            FactKind::Sent,
            key.clone(),
            json!({"deadline_ms": now_ms.saturating_add(row.lease_ms)}),
            now_ms,
        ));
        ctx.append(Fact::new(
            FactKind::Request,
            key.clone(),
            row.payload.clone(),
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
        return None;
    };

    // 租约先落，执行体后跑（`DE-022` 的 `before`）：挂死时 `expire` 才有东西可判。
    // `request` 紧贴在它之后、执行体之前落：执行期间产生的世界事实以 `request` 为直接
    // 前件（I2 的溯源锚），租约只是基础设施，不顶替「这次请求」的因果位（`D-376`）。
    ctx.append(Fact::new(
        FactKind::Sent,
        key.clone(),
        json!({"deadline_ms": now_ms.saturating_add(row.lease_ms)}),
        now_ms,
    ));
    ctx.append(Fact::new(
        FactKind::Request,
        key.clone(),
        row.payload.clone(),
        now_ms,
    ));

    let writer = Arc::new(CollectingWriter::new());
    let exec = PortExecContext {
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
        // 读面是这一圈计划时的快照（`D-376`），不是执行体跑起来时的活账本。
        reader,
    };
    let input = PortInput {
        port: row.port,
        payload: row.payload.clone(),
    };
    let shared = ctx.shared();
    Some(tokio::spawn(async move {
        let outcome = executor.execute(exec, input).await;
        finish(shared, row, attempt, key, writer, outcome).await;
    }))
}

/// 后台执行体返回后的收账（`D-376`）。
///
/// **迟归整笔丢弃**：完成时先看 `(port, id, attempt)` 这一笔还开不开——账上 attempt
/// 已经推进（`retry` 落在 N+1），或同键已有收束（`failed` / `response`），或租约
/// 已过（`now` 取自事实，不读墙钟），都说明这一笔已经被 `expire` 判死。此时连同
/// 执行体产生的草稿一起丢弃：迟到的一次往返不许再落格，更不许长出新债。
///
/// **产物一次性提交**：执行体 `emit` 的草稿（五类协议事实与它写出的世界事实）都
/// 缓冲到此处，只有这一笔还开着才一起落格。世界事实若在执行期间就写出去，工具产物
/// 会先于工具债收束落账，把消费它的能力行提前叫醒——同一份事实日志的阅读顺序
/// 因此取决于后台任务的完成时刻（`D-376`）。
async fn finish(
    ctx: Arc<dyn DriveCtx>,
    row: DebtRow,
    attempt: u32,
    key: DebtKey,
    writer: Arc<CollectingWriter>,
    outcome: PortOutcome,
) {
    let facts = ctx.facts();
    let view = FactView::new(&facts);
    let now_ms = ctx.now_ms();
    let superseded = view.attempt(&key.port, &key.id) > attempt;
    let closed = view.closing(&key).is_some();
    let lease_over = view
        .last_sent(&key)
        .and_then(|s| s.payload.get("deadline_ms").and_then(Value::as_u64))
        .is_some_and(|deadline| now_ms >= deadline);
    if superseded || closed || lease_over {
        return;
    }
    // 五类事实归环记账，其余原样交出：两类都在这里、且只在这里提交。
    let mut protocol = Vec::new();
    let mut world = Vec::new();
    for draft in writer.take() {
        if FactKind::parse(&draft.kind).is_some() {
            protocol.push(draft);
        } else {
            world.push(draft);
        }
    }
    if !world.is_empty() {
        ctx.outlet().emit(world).await;
    }

    match outcome.error {
        // 第三态「挂起」：带租约的 `sent` 已在派发前落过，这里再落**第二条** `sent`。
        // `deadline_ms: null` 同时表达「已挂起」与「租约已让出」；`shard` 让它不被
        // `(kind, key)` 吸收。计划层取最后一条，读到 null ⇒ 只等新触发，债继续开着。
        Some((PortErrorClass::NeedsHuman, _why)) => {
            ctx.append(Fact::new(
                FactKind::Sent,
                key,
                json!({"deadline_ms": Value::Null, "shard": 1}),
                now_ms,
            ));
        }
        Some((class, why)) => {
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
                    failure_payload(&outcome.facts, row.port, class, &why),
                    now_ms,
                ));
            }
        }
        None => {
            let port = row.port;
            let payloads = response_payloads(&protocol, &outcome, port);
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

/// 执行体读面的**派发时刻快照**（`D-376`）。
///
/// 同一圈派发的每一笔都看到同一份事实：计划已经只用一次快照（`D-343`），执行体若
/// 跑起来再读活账本，就会看见同批兄弟刚落下的事实——「哪一笔先完成」于是决定请求
/// 正文，重放不再逐字节相同。快照在 `drive` 里取一次，克隆句柄分给整批。
struct SnapshotReader {
    facts: Vec<PortFact>,
    events: Vec<symbio_kernel::event::Event>,
}

impl SnapshotReader {
    fn snapshot(reader: &dyn PortFactReader) -> Self {
        Self {
            facts: reader.facts(),
            events: reader.events(),
        }
    }
}

impl PortFactReader for SnapshotReader {
    fn facts(&self) -> Vec<PortFact> {
        self.facts.clone()
    }

    fn events(&self) -> Vec<symbio_kernel::event::Event> {
        self.events.clone()
    }
}

/// 执行体 emit 的收集器：**全部**草稿先缓冲，`finish` 时一次性提交（`D-376`）。
///
/// 五类事实进环的账本、其余走应用出口——分派只在 `finish` 做一次，执行期间不写
/// 任何东西：迟归的那一笔连同它的世界事实一起丢弃，半成品不会先落账。
struct CollectingWriter {
    collected: Mutex<Vec<FactDraft>>,
}

impl CollectingWriter {
    fn new() -> Self {
        Self {
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
        self.collected
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .extend(facts);
    }
}

#[cfg(test)]
#[path = "engine.test.rs"]
mod tests;
