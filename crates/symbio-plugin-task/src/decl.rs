//! 任务族的事件声明（DE-024 D-197 / D-204 / D-206）。
//!
//! 名字、格位与角色都在这里：写方（[`crate::task::TaskTable`] / 调度者）与读方
//! （core 的 `readyset` / `autonomy` / `rework_bounded`，以及 `session` 的两个读数
//! 列）都从同一张声明表取；跨插件消费的名字经 `PLUGIN.yml` 的 `shared_events:` 投影
//! 进 `symbio_plugin_api::events`，本插件**不要求别的插件链进来**。
//!
//! 本插件不认识任何兄弟插件：`readyset_for` 在构造期收 [`TASK_VOCAB`]，
//! core 的折叠只按这张表认名字——所以任务族的 owner 从 `loop` 移到本插件时，
//! 消费方一个字都不用改。

use symbio_core::{Entity, EventDecl, FactRole, PluginDeclarations, TaskVocab, Verb};
// 跨插件共享的五条走清单生成物（`PLUGIN.yml` 的 `shared_events` → `events.rs`）。
// `pub(crate) use` 是**转手**不是复制：本插件的写方 / 读方从 `crate::decl` 取名字，
// 拼写的唯一出处仍是生成物。
pub(crate) use symbio_plugin_api::events::{
    EVENT_TASK_ASSERTED, EVENT_TASK_ASSIGNED, EVENT_TASK_HELD, EVENT_TASK_OPENED,
    EVENT_TASK_PROGRESS,
};

/// 就绪判死（调度者写，`reason: "lease_expired"`）：名字随 owner 声明入库——写闸
/// 只认声明过的名字，没声明的格位一律 `OffGrid`。
///
/// 单消费方（调度者自己判死、自己读），不进共享表。
pub(crate) const EVENT_TASK_READY: &str = "task.ready";

/// 返工：判定不合格 ⇒ **新增一条事件**（重开一个返工节点），不是回滚。落在
/// `task × asserted`（返工本身是一次质量判定的事实）。
///
/// 写方 [`crate::task`]，读方是 core 的终止性判据——它经 `CountsRework` 角色
/// 取这一格，不抄名字（DE-024 D-203）。
pub(crate) const EVENT_TASK_REWORK_CREATED: &str = "task.rework_created";

/// 本插件交出**任务族**的拼写（DE-024 D-206）：core 的 `readyset_for` 在构造期收这张表，
/// 字段是它们在算法里的角色——终态与租约同落 `Task × Asserted`，所以按格子命不出这五个名字。
pub(crate) const TASK_VOCAB: TaskVocab<'static> = TaskVocab {
    opened: EVENT_TASK_OPENED,
    progressed: EVENT_TASK_PROGRESS,
    held: EVENT_TASK_HELD,
    terminal: EVENT_TASK_ASSERTED,
    lease: EVENT_TASK_ASSIGNED,
};

/// owner —— 与目录名同形（`crates/symbio-plugin-task`），装配表按它排序与报缺口。
pub(crate) const OWNER: &str = "crates/symbio-plugin-task";

pub(crate) const EVENTS: &[EventDecl] = &[
    // ── 任务族 ──
    //
    // 这一族的写方与调度者都在本插件，读数被 `session` 的 `stats` 列消费——名字随
    // owner 登记进清单（DE-024 D-204），折叠在构造期收 [`TASK_VOCAB`] 的拼写
    // （D-206）。领域形状：任务图是 `depends_on` 载荷字段（数据不是机制，C14 判无环），
    // 返工是新增一条事件而不是改历史（append-only 已提供撤销语义）。
    // 开任务：载荷 `{ task_id, depends_on, goal }`。
    EventDecl::new(EVENT_TASK_OPENED, (Entity::Task, Verb::Opened)),
    // 推进：执行中的普通事实。在 `readyset` 里它同时是**解除挂起**——挂起是一条事件，
    // 恢复也是一条。
    EventDecl::new(EVENT_TASK_PROGRESS, (Entity::Task, Verb::Progressed)),
    // 暂挂（插话抢占 / 等外部资源；DE-037 里这一格叫 `task.blocked`，
    // 本仓库以 `task.held` 单点定义）。写方是 `session` 的收件箱。
    EventDecl::new(EVENT_TASK_HELD, (Entity::Task, Verb::Held)),
    // 终态：验证通过（验收通过才终态）。
    EventDecl::new(EVENT_TASK_ASSERTED, (Entity::Task, Verb::Asserted)),
    // 就绪判死（调度者写，`reason: "lease_expired"`）。
    EventDecl::new(EVENT_TASK_READY, (Entity::Task, Verb::Progressed)),
    // 发放（租约）：载荷 `{ task_id, owner, lease_ms, until_ms }`，与终态同落
    // `Task × Asserted`。**唯一写方是调度者**（`ScheduleDecider`），且写侧闸读
    // `assign.work`（`ActorSpec::can_assign_work`）——没有该能力的主体零事件
    // （fail-closed）。租约不是锁原语：持有状态由投影从这格算出，到期由调度者判死。
    EventDecl::new(EVENT_TASK_ASSIGNED, (Entity::Task, Verb::Asserted)),
    // 返工轮：终止前提 2（`rework_bounded`）的计数对象。写方 [`crate::task`]，读方
    // core 判据经角色取它（DE-024 D-203）。
    EventDecl::with_roles(
        EVENT_TASK_REWORK_CREATED,
        (Entity::Task, Verb::Asserted),
        &[FactRole::CountsRework],
    ),
];

// 装配口的提交：宿主链接本 crate 即生效。本插件不认识也不依赖任何兄弟插件。
symbio_plugin_api::submit_declarations!(OWNER, submitted_declarations);

fn submitted_declarations() -> PluginDeclarations<'static> {
    PluginDeclarations {
        events: EVENTS,
        facts: &[],
        views: &[],
        ports: &[],
        actors: Vec::new(),
        rows: &[],
    }
}

#[cfg(test)]
#[path = "decl.test.rs"]
mod tests;
