//! 任务能力：**会话任务清单 → 任务事实 → 三个消费方**。
//!
//! 三个消费方各走一条已存在的扩展面，本模块只提供判据与排版：
//!
//! | 消费方 | 形态 | 扩展面 |
//! |---|---|---|
//! | 读出口 `session/stats` 的 `readyset` 列 | 事实源读什么算什么 | 共享声明（`TASK_VOCAB`） |
//! | 调度段 [`prompt_section`] | 请求视图置顶一段 `role=user` 的提示 | 上下文 provider（`context`） |
//! | 轮末调度 | 就绪集 → `task.ready` / `task.assigned` | 轮末贡献（[`ScheduleContributor`]，D-250） |
//!
//! 任务不是另开一份存储：任务就是事实源里的 `task.*` 事件（D-044 同族纪律）。清单本身
//! （`local/todo_write`）是纯内存、会话作用域的；本模块只**转写观察到的声明**，
//! 不自己维护任务图。
//!
//! ## 六条口径（为什么是这样）
//!
//! - **只记声明，不记全量**：`merge = true` 是增量，`merge = false` 是整体替换——
//!   本模块两种模式**行为相同**，因为事实源只认被声明到的项。清单里没提到的任务
//!   一律不动。
//! - **整体替换移出清单的项保持未终态**：append-only 不替模型编一条「已验收」的
//!   假事实。代价写在明处——它仍留在事实源里、也就仍可能留在就绪集里，读出口会
//!   念出一个清单上已经没有的 id。要表达「不再做」得先有一格 cancel 语义的事件，
//!   而 `task.*` 词汇表里没有这一格 ⇒ 本模块不替模型决定那个 id 该算什么。
//! - **降级不产事实**（进行中 → 未开始）：`task.progress` 是不可撤销的推进记录，
//!   清单把它改回 `pending` 只是模型改了主意，没有新事实发生。既不写事件、也不改
//!   观察状态，跨轮因此稳定（不会每次声明都补一条）。
//! - **回退出终态 = 返工**：`completed → pending / in_progress` 写
//!   `task.rework_created` + 一个**新节点**的 `task.opened`（`{id}-r{n}`），
//!   旧节点仍是终态——返工是新增一条事实，不是回滚
//!   （[DE-033 §2](../../../docs/003-design/033-多步任务与返工.md "DE-033-多步任务与返工")）。
//!   这正是 `rework_bounded` 的**生产写方**：不是一条永远空转的断言。
//! - **溯源锚**：全部锚在本轮 `user.message` 格（`user_seq`）——任务是「这轮模型
//!   说该做什么」，出处是这一轮的开口；`task.asserted` 是断言类事件，无溯源即违规
//!   （I2）。事件号 `v2t-{轮锚}-{调用 id}-{n}`：轮锚挡跨轮撞键，调用 id 挡同一轮里
//!   两次声明互撞。
//! - **写方**：判断与成格都在本模块的 [`TaskTable`]（效果端口的行为实现），落格在
//!   宿主的唯一写入口——`turn_exec` 轮末把本轮全部效果草稿交给 `Store::append`。
//!   本模块**不写事实源**：写口唯一（I1）。
//!
//! ## 诚实划界
//!
//! - **悬空依赖是断言的本职**：模型声明了一个不存在的依赖，`readyset` 会诚实地
//!   认为它不就绪，`acyclic_deps` 会在 `invariants` 列报出来。`depends_on` 默认
//!   `[]`，日常调用不踩这条线。
//! - **返工节点继承被返工任务的依赖**（返工做的是同一件事，前置条件不变）；
//!   而**别的任务**对旧 id 的依赖看的是旧节点的终态——旧节点验收过一次是事实，
//!   返工不追溯推翻它（要追溯得有「撤销终态」这一格，append-only 没有）。

use std::collections::BTreeMap;

use symbio_kernel::actors::{ActorSpec, Pattern, Scope};

use symbio_core::ReadySetView;
use symbio_core::PRINCIPAL_SCHEDULER;
use symbio_core::{actor_can_assign_work, readyset_for, Entity, Event, Verb};
use symbio_plugin_api::behaviors;
use symbio_plugin_api::effects::{
    BeforeDecision, BeforeRefusal, BeforeRequest, EffectCall, FactDraft, ToolBehavior,
};
use symbio_plugin_api::turn::{TurnContributor, TurnDraft, TurnEnd, TurnFace};

use crate::decl::{
    EVENT_TASK_ASSERTED, EVENT_TASK_ASSIGNED, EVENT_TASK_HELD, EVENT_TASK_OPENED,
    EVENT_TASK_PROGRESS, EVENT_TASK_READY, EVENT_TASK_REWORK_CREATED, TASK_VOCAB,
};

// ==================== 声明态：任务表的输入形状 ====================
//
// 写侧（[`TaskTable`]）与落/读侧（本模块的折叠）同属本插件，没有第二个消费方。
// `todo_write` 只声明行为名，不交记录类型——声明随效果端口走（DE-025 D-207 / D-214）。

/// 一项任务的声明态（一次 `todo_write` 调用里的一项）。
#[derive(Debug, Clone)]
pub(crate) struct TaskItem {
    /// 任务 id。**缺席时按 `task_{i+1}` 兜底**，与 `local/todo_write` 的默认同一条
    /// 格式——两处各写一次是跨插件契约（E-009 禁止本插件直引 local，所以没有
    /// 共享函数可用）：改名必须两处一起改，否则事实源的 id 会与工具清单对不上，
    /// 调度段就会念出模型不认识的 id。
    pub id: String,
    /// 任务内容（入 `task.opened` 的 `goal`）。
    pub goal: String,
    /// 依赖的任务 id。任务图是**数据不是机制**（`-多步任务与返工` §2），
    /// 声明在清单里，成环 / 悬空由读侧 `acyclic_deps` 报。
    pub depends_on: Vec<String>,
    /// 声明状态。
    pub status: TaskStatus,
}

/// 任务状态（与 `local/todo_write` 的 `status` 枚举同集）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum TaskStatus {
    /// 未开始。
    #[default]
    Pending,
    /// 进行中。
    InProgress,
    /// 已完成（`task.asserted` 的触发态）。
    Completed,
}

/// 一次要补进事实源的事件（算完再写：状态推进与写入分开，便于单测断言「该补几条」）。
struct Emitted {
    kind: &'static str,
    verb: Verb,
    payload: serde_json::Value,
}

/// 清单 id → 观察状态（**从事件序列重建**，不跨轮持有内存记账——同一份切片
/// 必须永远算出同一套状态，否则单测与生产会各走一套）。
#[derive(Debug, Default, Clone)]
struct Entry {
    /// 该清单 id 当前对应的**存活节点**（返工后是 `{id}-r{n}`）。
    live: String,
    /// 存活节点的状态。
    status: TaskStatus,
    /// 存活节点的依赖（返工节点继承它）。
    depends_on: Vec<String>,
    /// 存活节点的任务内容。
    goal: String,
    /// 已发生的返工轮数。
    rounds: u32,
}

/// 载荷里的字符串字段（缺省 = 空串 / 空表，与 `readyset` 的读法同口径）。
fn payload_str(payload: &serde_json::Value, key: &str) -> String {
    payload
        .get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn payload_deps(payload: &serde_json::Value, key: &str) -> Vec<String> {
    payload
        .get(key)
        .and_then(serde_json::Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(serde_json::Value::as_str)
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

/// 从已有事实重建观察状态。
///
/// 返工节点用 `task.rework_created` 载荷的 `replaces` 归到其清单项上：只有那一格
/// 携带 `replaces`，所以先登记「节点 id → 清单 id」，此后落在返工节点上的
/// opened / progressed / asserted 都按这张表归位——否则返工会在表里开出第二个条目，
/// 同一个清单项被记成两件事。
///
/// 收的是**迭代器**而不是切片：判据要看得见两段历史（轮初的快照 + 本轮已落格的
/// 草稿，见 [`symbio_plugin_api::effects::EffectEnv`]），合成一段要复制整份历史。
fn rebuild<'a>(events: impl Iterator<Item = &'a Event>) -> BTreeMap<String, Entry> {
    let mut map: BTreeMap<String, Entry> = BTreeMap::new();
    let mut nodes: BTreeMap<String, String> = BTreeMap::new();
    for e in events {
        if e.entity != Entity::Task {
            continue;
        }
        let Some(id) = e.payload.get("task_id").and_then(serde_json::Value::as_str) else {
            continue;
        };
        if e.kind == EVENT_TASK_REWORK_CREATED {
            let replaces = payload_str(&e.payload, "replaces");
            if replaces.is_empty() {
                continue;
            }
            nodes.insert(id.to_string(), replaces.clone());
            let round = e
                .payload
                .get("round")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(1) as u32;
            let entry = map.entry(replaces).or_default();
            entry.rounds = entry.rounds.max(round);
            continue;
        }
        let key = nodes.get(id).map(String::as_str).unwrap_or(id);
        match e.kind.as_str() {
            k if k == EVENT_TASK_OPENED => {
                let entry = map.entry(key.to_string()).or_default();
                entry.live = id.to_string();
                entry.status = TaskStatus::Pending;
                entry.depends_on = payload_deps(&e.payload, "depends_on");
                entry.goal = payload_str(&e.payload, "goal");
            }
            k if k == EVENT_TASK_PROGRESS => {
                let entry = map.entry(key.to_string()).or_default();
                entry.live = id.to_string();
                entry.status = TaskStatus::InProgress;
            }
            k if k == EVENT_TASK_ASSERTED => {
                let entry = map.entry(key.to_string()).or_default();
                entry.live = id.to_string();
                entry.status = TaskStatus::Completed;
            }
            _ => {}
        }
    }
    map
}

/// 一次声明该补的事件（状态推进的**判定方**：只看观察状态 + 声明态，不看清单全量）。
fn step(entry: &mut Entry, list_id: &str, item: &TaskItem) -> Vec<Emitted> {
    let mut out = Vec::new();
    if entry.live.is_empty() {
        // 第一次见到这个清单 id：先开格（`readyset` 只在 `task.opened` 建节点，
        // 没有这一格的任务根本不在调度的候选集里）。
        entry.live = list_id.to_string();
        entry.status = TaskStatus::Pending;
        entry.depends_on = item.depends_on.clone();
        entry.goal = item.goal.clone();
        out.push(Emitted {
            kind: EVENT_TASK_OPENED,
            verb: Verb::Opened,
            payload: serde_json::json!({
                "task_id": list_id,
                "depends_on": item.depends_on,
                "goal": item.goal,
            }),
        });
    }

    // 降级（进行中 → 未开始）不产事实：见模块头「降级不产事实」。
    if item.status == TaskStatus::Pending {
        return out;
    }

    if entry.status == TaskStatus::Completed && item.status != TaskStatus::Completed {
        // 回退出终态 = 返工：新增一条判定事实 + 一个新节点，旧节点仍是终态。
        entry.rounds += 1;
        let node = format!("{list_id}-r{}", entry.rounds);
        out.push(Emitted {
            kind: EVENT_TASK_REWORK_CREATED,
            verb: Verb::Asserted,
            payload: serde_json::json!({
                "task_id": node,
                "replaces": list_id,
                "round": entry.rounds,
            }),
        });
        out.push(Emitted {
            kind: EVENT_TASK_OPENED,
            verb: Verb::Opened,
            payload: serde_json::json!({
                "task_id": node,
                "depends_on": entry.depends_on,
                "goal": entry.goal,
            }),
        });
        entry.live = node;
        entry.status = TaskStatus::Pending;
    }

    if entry.status != item.status {
        match item.status {
            TaskStatus::InProgress => {
                out.push(Emitted {
                    kind: EVENT_TASK_PROGRESS,
                    verb: Verb::Progressed,
                    payload: serde_json::json!({ "task_id": entry.live }),
                });
                entry.status = TaskStatus::InProgress;
            }
            TaskStatus::Completed => {
                out.push(Emitted {
                    kind: EVENT_TASK_ASSERTED,
                    verb: Verb::Asserted,
                    payload: serde_json::json!({ "task_id": entry.live }),
                });
                entry.status = TaskStatus::Completed;
            }
            // `Pending` 已在上面提前返回。
            TaskStatus::Pending => {}
        }
    }
    out
}

/// 把一次 `todo_write` 声明折成**待落格**的 `task.*` 草稿。
///
/// - `produced_by` = 本轮 `user.message` 的 seq（溯源锚，I2）；
/// - 事件号 `v2t-{轮锚}-{调用 id}-{n}`：轮锚保证跨轮不撞，调用 id 保证**同一轮里
///   两次 `todo_write` 不撞**（各算各的 `n`），`n` 是本次声明内的序号；
/// - 只产草稿，**不写事实源**：写口仍是宿主的唯一写入口（I1）。
///
/// 观察状态从**两段历史**折叠：轮初快照 + 本轮已落格的草稿。少了后者，同一轮里
/// 第二个 `todo_write` 会把第一个刚开的格再开一次——那是「两条声明各说各的」，
/// 不是「后一条覆盖前一条」。
fn drafts(call: &EffectCall<'_>, items: &[TaskItem]) -> Vec<FactDraft> {
    let mut map = rebuild(call.facts());
    let mut out = Vec::new();
    let mut n = 0usize;
    for item in items {
        for e in step(map.entry(item.id.clone()).or_default(), &item.id, item) {
            out.push(FactDraft {
                key: format!("v2t-{}-{}-{n}", call.anchor(), call.call_id),
                kind: e.kind.to_string(),
                entity: Entity::Task,
                verb: e.verb,
                payload: e.payload,
                produced_by: Some(call.user_seq),
            });
            n += 1;
        }
    }
    out
}

/// 从 `todo_write` 的参数里读出声明项（形状是跨插件契约，见 [`behaviors::TASK_TABLE`]）。
///
/// 只认 `todos[]` 里的 `id` / `content` / `status` / `depends_on`：其余字段（`priority`
/// 之类）是给模型看的提示，不落事实。`content` 为空的项跳过——没有目标的任务开出来
/// 也只是噪声；`id` 缺席按位次兜底，与工具侧同一格式。
fn declared_items(args: &serde_json::Value) -> Vec<TaskItem> {
    let Some(todos) = args.get("todos").and_then(serde_json::Value::as_array) else {
        return Vec::new();
    };
    todos
        .iter()
        .enumerate()
        .filter_map(|(i, t)| {
            let goal = t
                .get("content")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            if goal.is_empty() {
                return None;
            }
            Some(TaskItem {
                id: t
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .map(String::from)
                    .unwrap_or_else(|| format!("task_{}", i + 1)),
                goal: goal.to_string(),
                depends_on: t
                    .get("depends_on")
                    .and_then(serde_json::Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(serde_json::Value::as_str)
                            .map(String::from)
                            .collect()
                    })
                    .unwrap_or_default(),
                status: match t.get("status").and_then(serde_json::Value::as_str) {
                    Some("in_progress") => TaskStatus::InProgress,
                    Some("completed") => TaskStatus::Completed,
                    _ => TaskStatus::Pending,
                },
            })
        })
        .collect()
}

/// 任务表的**效果与写侧判断**：一次成功的 `todo_write` 声明 → `task.*` 草稿
/// （DE-025 D-207）；不持权威的主体连写都不许写（DE-033 §3 / D-219）。
///
/// ## 为什么实现住这里，而不是工具所在的那个插件
///
/// 行为名 [`behaviors::TASK_TABLE`] 的 owner 是**任务词族**的 owner：格位、角色与
/// 折叠（[`step`] / [`rebuild`] / [`TASK_VOCAB`]）都在本插件。工具那边只声明「我写
/// 任务表」，不需要也不应该知道 `task.*` 有几格。两侧各自声明，中间没有中心映射表。
///
/// ## 参数形状是契约的一部分
///
/// 「写任务表」这个名字包含了 `todos[]` 的形状（[`declared_items`]）。换一个工具来
/// 写任务表，它要交同一个形状——那正是行为名跨进程的意义；而**改名**（`todo_write`
/// → 别的名字）不影响本模块一个字。
pub(crate) struct TaskTable;

impl ToolBehavior for TaskTable {
    fn owner(&self) -> &'static str {
        // 与 `PLUGIN.yml` 的 `plugin_id` 同一串：owner 序即装配序。
        "task"
    }

    fn behavior(&self) -> &'static str {
        behaviors::TASK_TABLE
    }

    /// 写侧资格（[DE-033 §3](../../../docs/003-design/033-多步任务与返工.md "DE-033-多步任务与返工")）：
    /// **立任务（`define.work`）或独立判定（`assert.verification`）二居其一**。
    ///
    /// 两项**互不可写**（C15：`PermissionMatrix::new` 拒绝同持）⇒ 没有任何主体能
    /// 同时「自己立、自己验」——「产出者不得自我认证」在任务表这一层是结构性的。
    /// 纯产出主体（工人 / 对等体）与纯派活主体（市场）因此写不了任务表：交付物是
    /// 产物格，立与验归工作权威。
    ///
    /// 拒止**不记承诺**：工具没跑，「声明过什么」无从说起。判断不产事件——写资格
    /// 是执行窗口的资格，不是一项事实。
    fn before(&self, req: &BeforeRequest<'_>) -> BeforeDecision {
        let matrix = symbio_core::production_matrix();
        let may_write = matrix.can_write_name(req.principal, symbio_core::CAP_DEFINE_WORK)
            || matrix.can_write_name(req.principal, symbio_core::CAP_ASSERT_VERIFICATION);
        if may_write {
            return BeforeDecision::Allow;
        }
        BeforeDecision::Refuse(
            BeforeRefusal::new(
                "任务表",
                format!(
                    "主体 {} 不持 define.work / assert.verification，不得写任务表（todo_write）",
                    req.principal
                ),
                symbio_core::failure_kind::PERMISSION_DENIED,
            )
            .without_delegation_note(),
        )
    }

    fn after(&self, call: &EffectCall<'_>) -> Vec<FactDraft> {
        // 失败的调用没有声明：`todo_write` 自己会拒掉坏清单，此时「模型声明了什么」
        // 只是它试过什么，不是清单状态。
        if !call.success {
            return Vec::new();
        }
        let items = declared_items(call.args);
        if items.is_empty() {
            return Vec::new();
        }
        drafts(call, &items)
    }
}

symbio_plugin_api::submit_behavior!(TaskTable);

/// 调度段：就绪任务集渲染成一段提示，请求视图**置顶**注入。
///
/// 只读不写（与 `memory_events::recall_view` 同款的只读视图），因此：
/// - 没有事实源 / 读不到 ⇒ `None`——请求里就少一段，不存在「空占位」；
/// - 就绪集空 ⇒ `None`——依赖没闭合时给模型一个空的候选集只会诱导它瞎猜。
///
/// `snapshot` 是本会话事实的**值**（[D-084](../../../docs/003-design/001-核心架构.md#d-084-能力模块公用基础协议上下文只递值事实只经引擎abcde-五类需要的唯一取得方式 "DE-001-核心架构")）：
/// 读方不收路径、不开文件——枚举与读完在存储层，这里只投影。
pub(crate) fn prompt_section(snapshot: &[Event], cap: usize) -> Option<String> {
    let ready = readyset_for(TASK_VOCAB, cap).read_all(snapshot).value.ready;
    if ready.is_empty() {
        return None;
    }
    let ids = ready
        .iter()
        .map(|t| t.task_id.as_str())
        .collect::<Vec<_>>()
        .join("、");
    Some(format!(
        "【任务调度】就绪任务（依赖已全部完成）：{ids}。请从就绪任务中推进一项；\
         其余任务的依赖尚未完成，暂不可开始。"
    ))
}

// ==================== 调度策略（D-189：唯一生产消费方在本文件） ====================

/// 调度者的时延预算：反射档（`LatencyTier::Reflex`）——它是一次纯计算 + 几次落格。
pub(crate) const SCHEDULE_BUDGET_MS: u64 = 80;

/// 一笔租约的时长（ms）。
///
/// 取值理由与端口租约同一条：**比最慢的可接受持有长，比「用户已经以为坏了」短**。
/// 任务是模型驱动的一步工作，分钟级足够；到期不续即放回候选池（不是重试——重试语义
/// 归端口，见 `crates/symbio-plugin-loop/src/port`）。
const LEASE_MS: i64 = 300_000;

/// 调度者要落的一笔事实（**只描述，不构造 `Event`**）。
///
/// 事件信封（`turn` / `ts` / `produced_by` / 归属）由轮末贡献端口统一盖章
/// ——调度者只回答「该有什么」。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScheduleFact {
    /// 事件种类：`task.ready` 或 `task.assigned`。
    pub kind: &'static str,
    /// 载荷（含 `task_id`——任务格的**身份字段**，每一格都必须带，投影按它归户）。
    pub payload: serde_json::Value,
}

/// 调度者（DE-034 §3 的 `Decider` 行，[DE-034](../../../docs/003-design/034-并发调度与租约.md "DE-034-并发调度与租约")）：
/// 就绪集 → 发放 / 到期判死。
///
/// 它是一个 `Decider`，不是组件：输入 `ReadySet` 视图，输出 `task.assigned` 事件；
/// 判据是规则（确定性、毫秒级），不调模型、不持有任何句柄。
///
/// ## 三件事，按这个顺序
///
/// 1. **判死到期的租约**：`task.assigned` 的 `until_ms <= now` 且任务非终态 ⇒ 补一条
///    `task.ready{reason: "lease_expired"}`。持有状态由投影算（`readyset` 只认时间戳），
///    但「那笔租约死了」这件事**要留下事实**——否则回放里它只是「突然又能派了」。
/// 2. **记下这次算过就绪**：候选集里每一笔落 `task.ready`（同一开启周期内只落一次）。
/// 3. **发放**：按 `(opened_seq, task_id)` 升序，给候选集的每一笔落
///    `task.assigned{owner, lease_ms, until_ms}`。
///
/// ## 写侧闸：`assign.work`
///
/// 落 `task.assigned` 需要资格。[`actor_can_assign_work`] 读的是**规格自己身上**的
/// 能力集（构造时来自授权表行）。缺该能力 ⇒ **一条事实都不写**（fail-closed）。
///
/// ## 它不做什么
///
/// 不做优先级与公平性（`readyset` 的打分参数）、不做抢占（那是
/// `crates/symbio-plugin-session/src/transcript/inbox.rs::PreemptionDecider` 的活）、不做验收
/// （`task.asserted` 归验证者）。它只回答一个问题：**这一刻该把哪几笔活交出去，
/// 交给谁，租到什么时候**。
pub(crate) struct ScheduleDecider;

impl ScheduleDecider {
    /// 生产构造点：调度者的 `ActorSpec`（[`Pattern::Decider`] / 80ms / 根作用域）。
    ///
    /// 能力集从**授权表的行**取（`grant_names_of`），不在这里列名字。
    pub(crate) fn spec() -> ActorSpec {
        ActorSpec {
            capabilities: symbio_core::production_matrix().grant_names_of(PRINCIPAL_SCHEDULER),
            pattern: Pattern::Decider,
            budget_ms: SCHEDULE_BUDGET_MS,
            scope: Scope::Root,
            principal: PRINCIPAL_SCHEDULER.to_string(),
        }
    }

    /// 算一次调度：给 `view` 里的候选集**记就绪 + 发放租约**，并判死已到期的租约。
    ///
    /// - `events`：完整史（判「这笔租约是不是已经死过一次」「这一次就绪记过没有」）；
    /// - `owner`：租约持有者（当前执行主体——调度者只派活，不假装自己干活）；
    /// - `now`：当前时刻（ms）。
    ///
    /// 返回按**落格顺序**排好的事实：先判死、再记就绪、最后发放。
    pub(crate) fn decide(
        &self,
        spec: &ActorSpec,
        view: &ReadySetView,
        events: &[Event],
        owner: &str,
        now: i64,
    ) -> Vec<ScheduleFact> {
        // 写侧闸：没有 `assign.work` ⇒ 什么都不写（fail-closed）。
        if !actor_can_assign_work(spec) {
            return Vec::new();
        }
        let mut out = Vec::new();

        // ── 1. 判死到期的租约 ────────────────────────────────────────────────
        for (task_id, expired) in expired_leases(events, now) {
            out.push(ScheduleFact {
                kind: EVENT_TASK_READY,
                payload: serde_json::json!({
                    "task_id": task_id,
                    "reason": "lease_expired",
                    "expired_owner": expired.owner,
                    "until_ms": expired.until_ms,
                }),
            });
        }

        // ── 2 / 3. 记就绪 + 发放 ─────────────────────────────────────────────
        for task in &view.ready {
            if !ready_recorded(events, &task.task_id) {
                out.push(ScheduleFact {
                    kind: EVENT_TASK_READY,
                    payload: serde_json::json!({
                        "task_id": task.task_id,
                        "opened_seq": task.seq,
                    }),
                });
            }
            out.push(ScheduleFact {
                kind: EVENT_TASK_ASSIGNED,
                payload: serde_json::json!({
                    "task_id": task.task_id,
                    "owner": owner,
                    "lease_ms": LEASE_MS,
                    "until_ms": now + LEASE_MS,
                }),
            });
        }
        out
    }
}

/// 一笔已到期的租约（`until_ms <= now` 且任务未终态、判死还没记过）。
struct Expired {
    owner: String,
    until_ms: i64,
}

/// 扫一遍史，找出**该判死**的租约：最新一条「发放」晚于最新一条「就绪或终态」，
/// 且它的 `until_ms` 已经过去。
///
/// 判据只用 seq 与时间戳（不含任何内存状态）：同一份切片永远算出同一批结果。
fn expired_leases(events: &[Event], now: i64) -> Vec<(String, Expired)> {
    // task_id → (最新 assigned 的 seq, owner, until_ms)
    let mut latest_assign: BTreeMap<&str, (u64, &str, i64)> = BTreeMap::new();
    // task_id → 最新「把这笔租约了掉」的 seq（`task.ready` 的补记 / 终态 / 挂起）
    let mut settled: BTreeMap<&str, u64> = BTreeMap::new();
    for e in events.iter().filter(|e| e.ts <= now) {
        if e.entity != Entity::Task {
            continue;
        }
        let Some(id) = e.payload.get("task_id").and_then(|v| v.as_str()) else {
            continue;
        };
        let seq = e.seq.map(|s| s.value()).unwrap_or(u64::MAX);
        match e.kind.as_str() {
            EVENT_TASK_ASSIGNED => {
                let owner = e
                    .payload
                    .get("owner")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let until_ms = e
                    .payload
                    .get("until_ms")
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or(i64::MIN);
                latest_assign.insert(id, (seq, owner, until_ms));
            }
            EVENT_TASK_READY | EVENT_TASK_ASSERTED | EVENT_TASK_HELD => {
                settled.insert(id, seq);
            }
            _ => {}
        }
    }
    let mut out: Vec<(String, Expired)> = latest_assign
        .into_iter()
        .filter(|(id, (seq, _, until_ms))| {
            let alive = settled.get(*id).is_none_or(|s| s < seq);
            alive && *until_ms <= now
        })
        .map(|(id, (_, owner, until_ms))| {
            (
                id.to_string(),
                Expired {
                    owner: owner.to_string(),
                    until_ms,
                },
            )
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// 这笔任务的**本轮就绪周期**里记过就绪没有：最新一条 `task.ready` 晚于最新一条
/// `task.opened`（返工会开新节点 ⇒ 新 id，旧的就绪记录不沾新节点）。
fn ready_recorded(events: &[Event], task_id: &str) -> bool {
    let mut opened: Option<u64> = None;
    let mut ready: Option<u64> = None;
    for e in events {
        if e.entity != Entity::Task {
            continue;
        }
        if e.payload.get("task_id").and_then(|v| v.as_str()) != Some(task_id) {
            continue;
        }
        let seq = e.seq.map(|s| s.value()).unwrap_or(u64::MAX);
        match e.kind.as_str() {
            EVENT_TASK_OPENED => opened = Some(seq),
            EVENT_TASK_READY => ready = Some(seq),
            _ => {}
        }
    }
    match (opened, ready) {
        (Some(o), Some(r)) => r > o,
        _ => false,
    }
}

/// 轮末贡献：把就绪集交给调度者，落成 `task.ready` / `task.assigned`（D-250）。
///
/// ## 为什么是贡献而不是环上的债
///
/// 没有就绪任务时它什么都不写——那是合法结局，不是「欠一条行」。写成轮末贡献，
/// 环不必为它编一个天然空转的 `owes`。
///
/// ## 为什么住在轮末而不是在效果之后另开一个调用点
///
/// 调度的输入是**本轮刚写完的任务表**：任务表的格在效果端口落完之后才在史里。
/// 轮末贡献端口按 `prio` 逐个询问，并且**每问一个之前重取一次史**（D-250），
/// 于是这里读到的 `history` 一定包含本轮新开的任务——不需要宿主为调度多写一行
/// 分支，也不需要本插件持有 store。
///
/// ## 只走生成轮
///
/// 定稿答话轮（[`TurnFace::Answered`]）没有工具交换，也就没有新的任务声明；
/// 调度在同一份史上是幂等的（已发放的不会再发），但多算一次没有意义。
/// 用 [`TurnFace`] 显式划界，而不是从 `history` 里猜。
///
/// ## 预算（`I3` 的反射档）
///
/// **算完再判**——能拦的只有落格这一半；超预算就一笔都不交，而不是交一半：
/// 半批租约没人认账（下一轮它们既不在候选集里、也没人持有）。
pub(crate) struct ScheduleContributor;

impl TurnContributor for ScheduleContributor {
    fn owner(&self) -> &'static str {
        "task"
    }

    fn contribute(&self, end: &TurnEnd<'_>) -> Vec<TurnDraft> {
        if end.face != TurnFace::Generated {
            return Vec::new();
        }
        let spec = ScheduleDecider::spec();
        let started = symbio_core::clock_now_ms();
        let view = readyset_for(TASK_VOCAB, end.task_cap)
            .read_all(end.history)
            .value;
        let facts = ScheduleDecider.decide(&spec, &view, end.history, end.principal, end.now_ms);
        let elapsed = symbio_core::clock_now_ms() - started;
        if elapsed > SCHEDULE_BUDGET_MS as i64 {
            symbio_core::plugin_warn!(
                "session",
                "[task] 调度超预算（{elapsed}ms > {SCHEDULE_BUDGET_MS}ms）⇒ 本轮不派活"
            );
            return Vec::new();
        }
        facts
            .into_iter()
            .enumerate()
            .map(|(n, fact)| {
                let verb = if fact.kind == EVENT_TASK_READY {
                    Verb::Progressed
                } else {
                    Verb::Asserted
                };
                TurnDraft::new(FactDraft {
                    key: format!("{}-s{n}", end.anchor),
                    kind: fact.kind.to_string(),
                    entity: Entity::Task,
                    verb,
                    payload: fact.payload,
                    produced_by: Some(end.user_seq),
                })
                // 派活的主体是调度者，不是跑这一轮的主体：归属由能力声明（D-250）。
                .with_actor(PRINCIPAL_SCHEDULER)
            })
            .collect()
    }
}

symbio_plugin_api::submit_turn_contributor!(ScheduleContributor);

#[cfg(test)]
#[path = "task.test.rs"]
mod tests;
