//! 控制面声明集：义务环自己消费的事实与视图。
//!
//! 这些名字**不是**内核语法：内核只拥有协议五格（`request` / `sent` / `response` /
//! `failed` / `retry`），其余事实与全部视图由本插件声明。装载期 `seal` 把它们与
//! 内核协议格一起校验——新增一个视图、一个控制面事实、一处字段，改的只是本文件。
//!
//! 视图的求值函数签名仍是 `fn(&[Event], i64) -> Value`：拿不到写口、拿不到别的视图、
//! 不读时钟（D-115 决策 1）。本文件是它们的**所有者**，`symbio_core::decl` 只是类型与
//! 装载机制的所在。

use symbio_core::{
    ChatVocab, CommitmentVocab, Entity, Event, EventDecl, FactDecl, FactRole, MessageVocab,
    TaskVocab, Verb, ViewDecl, EVENT_FAILED, EVENT_REQUEST, EVENT_RESPONSE, EVENT_RETRY,
    EVENT_SENT,
};

// ── 本插件持有的事件名（DE-024 D-197 / D-201）──
//
// 名字、格位与声明在同一处（`EVENTS`），内核不再为它们保留常量。写方 / 读方都从
// 这里取名字：跨模块引用不复制字面量，改名只改这一行。
pub(crate) const EVENT_SESSION_CONFIG: &str = "session.config";
pub(crate) const EVENT_TURN_STAGE: &str = "turn.stage";
pub(crate) const EVENT_AUTHZ_GRANTED: &str = "authz.granted";
pub(crate) const EVENT_MEMORY_RECALLED: &str = "memory.recalled";
pub(crate) const EVENT_TASK_READY: &str = "task.ready";
/// 返工：判定不合格 ⇒ **新增一条事件**（重开一个返工节点），不是回滚。落在
/// `task × asserted`（返工本身是一次质量判定的事实）。
///
/// 写方 [`crate::task_events`]，读方是 core 的终止性判据——它经 `CountsRework` 角色
/// 取这一格，不抄名字（DE-024 D-203），所以这个名字的两个端面都在本插件，随本插件
/// 声明（D-197 / D-201）。
pub(crate) const EVENT_TASK_REWORK_CREATED: &str = "task.rework_created";
/// 巩固：压缩 + 反事实（不是逐帧回放）。落在 `memory × progressed`。
///
/// 写方是 `memory_events::consolidate`（过 `accept` 闸门才落格），读方是 [`crate::recall`]
/// ——两端都在本插件，名字因此随本插件走（D-201）。
pub(crate) const EVENT_MEMORY_CONSOLIDATED: &str = "memory.consolidated";
/// 遗忘：**不是物理删除**——Log 永不删，只是投影不再包含（可撤销、可审计）。
/// 落在 `memory × closed`；`produced_by` 指向被遗忘那条记忆的 seq。
pub(crate) const EVENT_MEMORY_FORGOTTEN: &str = "memory.forgotten";
// 跨插件共享的八条名字（DE-024 D-204 / D-205 / D-206）：常量的**事实源是本插件清单的
// `shared_events:` 行**，由 `scripts/facts/events-rs.mjs` 投影进
// `symbio-plugin-api::events`，读侧（`session` 的 `stats` / `inbox` / `conation`，以及它调
// `calibration_for` / `readyset_for` 时交进去的那张表）从这里取名字，而不是内核替它们命名。
pub(crate) use symbio_plugin_api::events::{
    EVENT_MEMORY_CALIBRATED, EVENT_MEMORY_ENCODED, EVENT_TASK_ASSERTED, EVENT_TASK_ASSIGNED,
    EVENT_TASK_CONTROLLED, EVENT_TASK_HELD, EVENT_TASK_OPENED, EVENT_TASK_PROGRESS,
    EVENT_TURN_DISPATCH,
};

/// 本插件交出**任务族**的拼写（DE-024 D-206）：core 的 `readyset_for` 在构造期收这张表，
/// 字段是它们在算法里的角色——终态与租约同落 `Task × Asserted`，所以按格子命不出这五个名字。
pub(crate) const TASK_VOCAB: TaskVocab<'static> = TaskVocab {
    opened: EVENT_TASK_OPENED,
    progressed: EVENT_TASK_PROGRESS,
    held: EVENT_TASK_HELD,
    terminal: EVENT_TASK_ASSERTED,
    lease: EVENT_TASK_ASSIGNED,
};

/// 对话面 / 消息面 / 承诺面的三张词汇表（DE-024 D-203 归零批）。
///
/// core 的折叠在构造期收这三张表。拼写全部取自清单生成物 `symbio_plugin_api::events`
/// ——本插件是这批名字的 **owner**（`PLUGIN.yml` 的 `shared_events` + 上面 `EVENTS` 的
/// 格位），但**取用一律走生成物**：同一份拼写若在 owner 里手写一遍、在消费方里生成一遍，
/// 改名那天就会有一半没跟上。
///
/// ## 为什么是三张而不是一张
///
/// 按**面**切开，每张表的大小就是那个折叠算法真正的依赖：`reputation` 只认承诺四条，
/// `changes` 只认产物一条。交一张 20 字段的合并表会把耦合面说宽——读代码的人会以为
/// 声誉折叠与发言格有关，那不是真的（DE-024 D-206 的同一形状）。
pub(crate) const CHAT_VOCAB: ChatVocab = ChatVocab {
    user_message: symbio_plugin_api::events::EVENT_USER_MESSAGE,
    dispatch: symbio_plugin_api::events::EVENT_TURN_DISPATCH,
    supplemented: symbio_plugin_api::events::EVENT_TURN_SUPPLEMENTED,
    branch: symbio_plugin_api::events::EVENT_TURN_BRANCH,
    branch_selected: symbio_plugin_api::events::EVENT_TURN_BRANCH_SELECTED,
    reported: symbio_plugin_api::events::EVENT_CHAT_ASSISTANT_REPORTED,
    opening: symbio_plugin_api::events::EVENT_CHAT_ASSISTANT_OPENING,
    final_reply: symbio_plugin_api::events::EVENT_CHAT_ASSISTANT_FINAL,
    fallback: symbio_plugin_api::events::EVENT_CHAT_ASSISTANT_FALLBACK,
    progress: symbio_plugin_api::events::EVENT_CHAT_ASSISTANT_PROGRESS,
    denied: symbio_plugin_api::events::EVENT_AUTHZ_DENIED,
};

/// 消息变更面（5 条）：`mutations` / `changes` / `rebuild` / `transcript` /
/// `prompt_fidelity` 折的就是这一片。
pub(crate) const MESSAGE_VOCAB: MessageVocab = MessageVocab {
    artifact_added: symbio_plugin_api::events::EVENT_ARTIFACT_ADDED,
    message_added: symbio_plugin_api::events::EVENT_MESSAGE_ADDED,
    message_edited: symbio_plugin_api::events::EVENT_MESSAGE_EDITED,
    message_removed: symbio_plugin_api::events::EVENT_MESSAGE_REMOVED,
    consolidated: symbio_plugin_api::events::EVENT_SESSION_CONSOLIDATED,
};

/// 承诺面（4 条）：`reputation` 与 `commitment_events` 是它仅有的两个消费方。
pub(crate) const COMMITMENT_VOCAB: CommitmentVocab = CommitmentVocab {
    opened: symbio_plugin_api::events::EVENT_COMMITMENT_OPENED,
    released: symbio_plugin_api::events::EVENT_COMMITMENT_RELEASED,
    broken: symbio_plugin_api::events::EVENT_COMMITMENT_BROKEN,
    asserted: symbio_plugin_api::events::EVENT_COMMITMENT_ASSERTED,
};

/// 淘汰裁决事件名（`verdict × asserted`）——**本插件持有**，不进共享清单。
///
/// 判定它不进共享表的理由是可判的（CS-007）：除本插件外没有别的插件在**生产代码**里
/// 引用这条名字。会话插件确实读裁决事实，但它读的是 `verdict_grants_changes` 的**格子**
/// （`verdict × asserted`），不认拼写（DE-024 D-203「判据读格子与角色、不读名字」）。
/// 一个只有一个名字消费者的事实，登记进共享表只会让清单说谎。
///
/// 单名字走形参不走词汇表：它没有同格的兄弟，收成一张只有一个字段的表只会让调用点
/// 多一次解引用。
pub(crate) const EVENT_VERDICT_ASSERTED: &str = "verdict.asserted";

/// 上面的短名字，给「裁决」这个动作的调用点用（与 `ChatVocab::denied` 的用法同形）。
pub(crate) const VERDICT: &str = EVENT_VERDICT_ASSERTED;

/// 控制面事件名与格位（DE-024 D-197）：这些名字不属于内核语法，格位随名字一起
/// 由本插件声明。内核 `CELLS` 里的同名行随迁移逐条退役；parity 用例只对仍在表上的
/// 名字判等，最后一批业务事件迁出后 `CELLS` 只剩协议五格。
pub(crate) const EVENTS: &[EventDecl] = &[
    // 定稿发言：收束格上两个名字（定稿 / 兜底）里被 `final_unique_per_turn` 数的那一个。
    // 角色替名字进判据（DE-024 D-203）——删掉这行不会让守卫静默失能，装配口报
    // `Gap::UnmannedRole`。
    EventDecl::with_roles(
        "chat.assistant.final",
        (Entity::Turn, Verb::Closed),
        &[FactRole::FinalizesTurn],
    ),
    EventDecl::new("chat.assistant.fallback", (Entity::Turn, Verb::Closed)),
    EventDecl::new("chat.assistant.reported", (Entity::Turn, Verb::Asserted)),
    EventDecl::new("chat.assistant.opening", (Entity::Turn, Verb::Asserted)),
    EventDecl::new("chat.assistant.progress", (Entity::Turn, Verb::Progressed)),
    EventDecl::new(EVENT_TURN_STAGE, (Entity::Turn, Verb::Asserted)),
    // 开轮：格位本身答得出「这是开轮」；它同时是**历史切片**的预算声明来源
    // （派发档落格之前的形态），所以带 `declares_budget`。
    EventDecl::with_roles(
        "user.message",
        (Entity::Turn, Verb::Opened),
        &[FactRole::DeclaresBudget],
    ),
    EventDecl::new("turn.supplemented", (Entity::Turn, Verb::Asserted)),
    EventDecl::new(EVENT_SESSION_CONFIG, (Entity::Turn, Verb::Asserted)),
    // 本轮派发档：现行预算声明（`turn × asserted` 上只有它带这一档角色）。
    EventDecl::with_roles(
        EVENT_TURN_DISPATCH,
        (Entity::Turn, Verb::Asserted),
        &[FactRole::DeclaresBudget],
    ),
    // 被拒不产生收束格，但它回答「这一轮为什么停」——C6 的停机角色。
    EventDecl::with_roles(
        "authz.denied",
        (Entity::Turn, Verb::Asserted),
        &[FactRole::StopsTurn],
    ),
    EventDecl::new(EVENT_AUTHZ_GRANTED, (Entity::Turn, Verb::Asserted)),
    // 记忆找回（S5）：行表与事实表已经在用这个名字。
    EventDecl::new(EVENT_MEMORY_RECALLED, (Entity::Memory, Verb::Asserted)),
    // 巩固与遗忘（S5 的两格）：写方 `memory_events::consolidate`、读方 [`crate::recall`]
    // 都在本插件，随 D-201 从内核 `CELLS` 迁出。同族的 `memory.encoded` / `memory.calibrated`
    // 走另一条：`session` 也读它们，名字因此登记进本插件清单的 `shared_events`，格位仍由
    // 本插件交出（DE-024 D-204 / D-205），不回内核。
    EventDecl::new(
        EVENT_MEMORY_CONSOLIDATED,
        (Entity::Memory, Verb::Progressed),
    ),
    EventDecl::new(EVENT_MEMORY_FORGOTTEN, (Entity::Memory, Verb::Closed)),
    // 编码进记忆（写方 `memory_events` / `skill_events`，读方还有 `session` 的 `stats`）：
    // 跨插件消费 ⇒ 名字登记进清单、格位由本插件交出（DE-024 D-204）。
    EventDecl::new(EVENT_MEMORY_ENCODED, (Entity::Memory, Verb::Opened)),
    // 校准观测：技能**被用过之后**的一次命中 / 回退记录（`payload: { skill_id, fallback }`）。
    //
    // 为什么不能继续复用 `memory.recalled`：那是 S11 §3 的省格决定，代价落在**读侧**——
    // 校准投影只能靠载荷有没有这两个字段认自己的输入，同一张格子两种语义，判别退化成
    // 「谁恰好带了这两个字段」，那不是闭集：任何将来带这两个字段的事件都会被算进置信度。
    // 给它一格后读侧按名字过滤，判别回到闭集；写侧不变（仍是轮末 `derived_facts` 收束族，
    // **不开第二写通道**——I1）。
    //
    // 读侧投影 `symbio_core::calibration_for` 住在 core（loop 的路由阈值与 session 的
    // 校准列同时消费它），而**名字由调用方在构造期交进来**（DE-024 D-205）：本插件交这个
    // 常量，`session` 交清单生成物里同名的那条——core 一行都不读它。
    EventDecl::new(EVENT_MEMORY_CALIBRATED, (Entity::Memory, Verb::Asserted)),
    // 控制面的一格（写方两个插件各一处：工具闸的熔断 / 拒因在本插件，打断处置在
    // `session` 的 inbox）：owner 是本插件，名字走清单生成，不回内核。
    EventDecl::new(EVENT_TASK_CONTROLLED, (Entity::Control, Verb::Opened)),
    // ── 任务族（S7 步 16–18 / S04 / S07 批⑩）──
    //
    // 这一族的五格此前由内核命名，而写方与调度者全在本插件、读数被 `session` 的
    // `stats` 列消费——名字随 owner 登记进清单（DE-024 D-204），折叠在构造期收这张表
    // 的拼写（D-206）。领域形状：任务图是 `depends_on` 载荷字段（数据不是机制，C14
    // 判无环），返工是新增一条事件而不是改历史（append-only 已提供撤销语义）。
    // 开任务：载荷 `{ task_id, depends_on, goal }`。
    EventDecl::new(EVENT_TASK_OPENED, (Entity::Task, Verb::Opened)),
    // 推进：执行中的普通事实。在 `readyset` 里它同时是**解除挂起**——挂起是一条事件，
    // 恢复也是一条。
    EventDecl::new(EVENT_TASK_PROGRESS, (Entity::Task, Verb::Progressed)),
    // 暂挂（插话抢占 / 等外部资源；roadmap/S07 里这一格叫 `task.blocked`，
    // 本仓库以 `task.held` 单点定义）。写方是 `session` 的收件箱（批⑩）。
    EventDecl::new(EVENT_TASK_HELD, (Entity::Task, Verb::Held)),
    // 终态：验证通过（验收通过才终态）。
    EventDecl::new(EVENT_TASK_ASSERTED, (Entity::Task, Verb::Asserted)),
    // 就绪判死（调度者写，`reason: "lease_expired"`）：此前内核 `CELLS` 根本没有这一行，
    // 落格只能被写闸拒掉；名字随 owner 声明之后它才真正可入库。
    EventDecl::new(EVENT_TASK_READY, (Entity::Task, Verb::Progressed)),
    // 发放（租约）：载荷 `{ task_id, owner, lease_ms, until_ms }`，与终态同落
    // `Task × Asserted`。**唯一写方是调度者**（`ScheduleDecider`），且写侧闸读
    // `assign.work`（`ActorSpec::can_assign_work`）——没有该能力的主体零事件
    // （fail-closed）。租约不是锁原语：持有状态由投影从这格算出，到期由调度者判死。
    EventDecl::new(EVENT_TASK_ASSIGNED, (Entity::Task, Verb::Asserted)),
    // 返工轮：终止前提 2（`rework_bounded`）的计数对象。写方 `task_events`，读方
    // core 判据经角色取它（DE-024 D-203）。
    EventDecl::with_roles(
        EVENT_TASK_REWORK_CREATED,
        (Entity::Task, Verb::Asserted),
        &[FactRole::CountsRework],
    ),
    // ── 对话面补登（DE-024 D-203 归零批）──
    //
    // 下面九条此前只有内核 `CELLS` 一行：写方在本插件、读方是 core 的折叠
    // （`rebuild` / `transcript` / `prompt_fidelity` / `branch` / `changes` /
    // `reputation` / `actors::commitment`），两个插件都调那些折叠 ⇒ 名字的 owner
    // 是**写方**（本插件），消费面经清单 `shared_events` 投影
    // （`symbio_plugin_api::events`，DE-024 D-204）。
    //
    // 落格理由逐条取自内核 `CELLS` 的同名行（那一份随本批退役，说明搬到本处）：
    EventDecl::new("artifact.added", (Entity::Artifact, Verb::Asserted)),
    // 分支族两支都**不删任何东西**，所以不是 `closed`（那一格是「这一轮的产出定稿」）；
    // 它们是「关于这一轮的断言」，且靠 `produced_by` 才能被验真（I2 的断言类）。
    EventDecl::new("turn.branch", (Entity::Turn, Verb::Asserted)),
    // 淘汰裁决：写方在本插件轮末，读侧翻译在 `session`（两个生产消费方）。
    EventDecl::new(EVENT_VERDICT_ASSERTED, (Entity::Verdict, Verb::Asserted)),
    // 承诺四格：写方是轮末的立约 / 履约步（`derived_facts`）与 `agent` 插件的
    // 立约效果，两侧都经 core 的成格函数 `commitment_events` 进出，名字由调用方在
    // 构造期交进去（`ChatVocab`），core 一行都不读它们。
    EventDecl::new("commitment.opened", (Entity::Commitment, Verb::Opened)),
    EventDecl::new("commitment.released", (Entity::Commitment, Verb::Closed)),
    EventDecl::new("commitment.broken", (Entity::Commitment, Verb::Closed)),
    EventDecl::new("commitment.asserted", (Entity::Commitment, Verb::Asserted)),
];

/// 判据的**词汇面**（DE-024 D-203）：本插件调用 core 守卫时交出去的那张表。
///
/// 取进程装配表——它含本插件 `EVENTS` 的角色声明，也含别的 owner 将来声明的停机 /
/// 定稿角色；本二进制没装出表时用清单生成物（`symbio_plugin_api::vocab`）。内核
/// `CELLS` 那一档已随名字归零删除（DE-024 D-203 裁决三）：内核不再替任何人记名字。
pub(crate) fn guard_vocab() -> &'static [EventDecl] {
    symbio_plugin_api::vocab()
}

pub(crate) const FACTS: &[FactDecl] = &[
    // 收束格：字段来自 `turn_runner::produced_payload` + 的 `status` / `reason` / `terminated`。
    //
    // 正文不在这里（模型轮写空串）：它归那一次 `response(port=model)`，这里只有
    // 轮级结局——`answer_id`（这一轮说了什么的节点身份）与 `responses`（本轮全部
    // 模型输出的逻辑 id）。`text` / `text_id` 只给**没有模型往返**的轮次（快路 /
    // 定稿答话）；`reasoning` 同理（措辞轮的那段思考没有可挂的模型事实）。
    FactDecl {
        name: "chat.assistant.final",
        port: None,
        key: &[],
        expand: &[],
        fields: &[
            "text",
            "model",
            "turn_id",
            "text_id",
            "answer_id",
            "responses",
            "reasoning_id",
            "reasoning",
            "status",
            "reason",
            "terminated",
        ],
    },
    // 助手文本事实的另一半（`report_due` 的「最近一句」锚点，）：
    // 兜底与收束同为 `Closed`，其余三格落 `turn × asserted / progressed`。
    FactDecl {
        name: "chat.assistant.fallback",
        port: None,
        key: &[],
        expand: &[],
        fields: &[
            "text",
            "model",
            "turn_id",
            "text_id",
            "answer_id",
            "responses",
            "reasoning_id",
            "reasoning",
            "status",
            "reason",
            "detail",
        ],
    },
    FactDecl {
        name: "chat.assistant.reported",
        port: None,
        key: &[],
        expand: &[],
        fields: &["text", "tool_rounds", "quiet_ms", "id"],
    },
    FactDecl {
        name: "chat.assistant.opening",
        port: None,
        key: &[],
        expand: &[],
        fields: &["text", "id", "reason", "exclude_from_context"],
    },
    // 进展格载荷与收束格共用产出形状（`produced_payload`）+ `status`，
    // 只声明本视图读得到的字段，不复制整份形状（形状的 owner 是 `facts.rs`）。
    FactDecl {
        name: "chat.assistant.progress",
        port: None,
        key: &[],
        expand: &[],
        fields: &["text", "status"],
    },
    // 阶段事实：`result` 是这一步的结论（召回命中 / 续写种类，）。
    //
    // `model_calls` 是**这一步打了几次模型**（D-122 决策 4 / 5）：`source` 记的是
    // 判决的**来源**，读不出「有没有打过模型」（模型调了而应答不可解析 ⇒
    // `source=fallback`）。这一格是那次模型往返在事实层唯一的**数**——判据侧
    // 拿它对账「观测 ⇔ 三格」。
    FactDecl {
        name: "turn.stage",
        port: None,
        key: &[],
        expand: &[],
        fields: &[
            "stage",
            "round",
            "source",
            "reason",
            "result",
            "model_calls",
        ],
    },
    // 用户格：`meta` 是可选的补充元数据（`prepare.rs` 只在非空时写）。
    //
    // `tier` / `budget_ms` 是**历史形态**： 起用户格先于任何判决落，档位那时
    // 还没定 ⇒ 新写入不再带它们。声明保留，读方（`declared_tier` / `declared_budget_ms`）
    // 因此仍能读旧 WAL；新事实的档位在 `turn.dispatch`。
    FactDecl {
        name: "user.message",
        port: None,
        key: &[],
        expand: &[],
        fields: &["text", "tier", "budget_ms", "id", "meta"],
    },
    // 用户补充格：静默锚点也会退到它（`report_due`），与用户格同族。
    FactDecl {
        name: "turn.supplemented",
        port: None,
        key: &[],
        expand: &[],
        fields: &["text", "count", "id", "ids"],
    },
    // 本轮配置快照：决策只读它，不读 `SessionConfig` 活值。
    FactDecl {
        name: "session.config",
        port: None,
        key: &[],
        expand: &[],
        fields: &[
            "classify_enabled",
            "auto_compress",
            "progress_enabled",
            "progress_interval_ms",
            "progress_min_rounds",
            "progress_max_per_turn",
            "is_persistent",
        ],
    },
    // 本轮派发档（`turn.dispatch`）**不在本表**：表里没有一行以它为前件或出口，
    // 也没有一条被行用到的视图读它；它的读方是审计投影
    // （`invariants::declared_budget_ms`）。按判据 7 的口径它是业务事实，
    // 名字与写方都照旧，只是不进控制面声明集。
    // 拒绝事实：`key` 是「主体 × 能力 × 资源」的编码。
    //
    // `attempt` 是**工具端口拒绝**的尝试号（D-138）：`authorize-tool` / `deny-tools`
    // 按 `(port, id, attempt)` 收口。收束路径那条只带 `key` 的拒绝没有端口三字段，
    // 按任何端口键都取不到实例——它本来就不该关一笔工具授权。
    FactDecl {
        name: "authz.denied",
        port: None,
        key: &["key"],
        expand: &[],
        fields: &[
            "key", "port", "id", "attempt", "tool", "kind", "reason", "detail",
        ],
    },
    // 放行事实（/ DE-015 §4.2）：授权级联的正向一半。
    //
    // 键取 `(port, id, attempt)`：级联的派发行要与 `sent` / `failed` 关同一
    // **次尝试**（D-138）；`key` 只是给人看的同一 id 副本。
    FactDecl {
        name: "authz.granted",
        port: None,
        key: &["port", "id", "attempt"],
        expand: &[],
        fields: &[
            "key", "port", "id", "attempt", "tool", "risk", "deferred", "basis", "grant",
        ],
    },
];

fn in_flight(events: &[Event], _now: i64) -> serde_json::Value {
    serde_json::Value::Array(
        open_calls(events)
            .into_iter()
            .map(|e| {
                serde_json::json!({
                    "id": str_field(e, "id"),
                    "port": str_field(e, "port"),
                    "attempt": u64_field(e, "attempt"),
                })
            })
            .collect(),
    )
}

/// **在途**协议调用的唯一口径：`sent` 中没有被 `response` / `failed` / `retry`
/// （同 id + attempt）关掉的。
///
/// 两个视图（`in_flight` / `stale_sent`）共用它，而不是各写一遍关账规则：
/// 「什么算在途」只能有一处口径，否则两个视图会在某个角落说不同的话。
///
/// 三个出口都只关**同一次尝试**（D-138）：`failed` 不再按逻辑 id 一次关掉所有
/// 尝试。重试链上旧的尝试已经由 `retry` 关掉，不会留下「没人收的旧在途」。
///
/// 可见范围放到 crate：义务环的 `waiting`（`loop_rows`）也要问同一个问题——
/// 「此刻有没有一笔已派出、没收到终局的往返」。D-222 订正 2 之后「在等」不再由
/// `by = None` 表达（`await-*` 是可以还的债），它只能按这条形状问，于是口径必须
/// 是**同一份**，不能各写一遍。
pub(crate) fn open_calls(events: &[Event]) -> Vec<&Event> {
    let mut open: Vec<&Event> = Vec::new();
    for e in events {
        match e.kind.as_str() {
            EVENT_SENT => open.push(e),
            EVENT_RESPONSE | EVENT_RETRY | EVENT_FAILED => {
                let id = str_field(e, "id");
                let attempt = u64_field(e, "attempt");
                open.retain(|s| !(str_field(s, "id") == id && u64_field(s, "attempt") == attempt));
            }
            _ => {}
        }
    }
    open
}

/// **卡住的在途**：`sent` 之后超过 `lease_ms` 仍无应答的那些。
///
/// `now` 是入参而不是时钟（D-115 决策 1）——同一份日志给定同一个 `now`，
/// 两次求值必须得到同一结果。
///
/// 字段：`stale` 是租约到期（租约行的开账条件），`exhausted` 是尝试已到顶
/// （判死而不是再重试）。两者分开而不是折成一个布尔：它们回答的是不同问题，
/// 而「到期但还能重试」与「到期且已到顶」是两种不同的收尾。
///
/// `lease_ms == 0`（旧日志 / 未带租约的派发）⇒ 永不过期。这不是默认值而是
/// 语义：没有租约的调用不能被看护者判成卡住。
fn stale_sent(events: &[Event], now: i64) -> serde_json::Value {
    serde_json::Value::Array(
        open_calls(events)
            .into_iter()
            .map(|e| {
                let attempt = u64_field(e, "attempt").unwrap_or(0);
                let lease_ms = u64_field(e, "lease_ms").unwrap_or(0);
                let cap = u64_field(e, "attempt_cap").unwrap_or(0);
                let age_ms = (now - e.ts).max(0) as u64;
                serde_json::json!({
                    "port": str_field(e, "port"),
                    "id": str_field(e, "id"),
                    "attempt": attempt,
                    "age_ms": age_ms,
                    "stale": lease_ms > 0 && age_ms >= lease_ms,
                    "exhausted": cap > 0 && attempt >= cap,
                })
            })
            .collect(),
    )
}

/// 这一轮**最新一份配置**（按事件序取最后一条；`None` = 这一轮没落配置事实）。
fn config_of(events: &[Event], turn: u64) -> Option<&Event> {
    events
        .iter()
        .rfind(|e| e.turn == turn && e.kind == EVENT_SESSION_CONFIG)
}

/// 当前这一幕的身份：轮号 + 幕锚（最新一条用户格 / 恢复格）。
///
/// 幕锚是「本次执行从哪开始」的唯一定义：同一轮号会被续写幕复用，按轮扫全史
/// 会把首幕的阶段误判成「已做」。
///
/// 轮号跟着**最新幕锚**走，而不是取切片里最大的轮号：恢复一条被后续轮次归档
/// 的旧轮时，切片从旧轮首条事件一直延伸到今天，最大轮号指向新轮——旧轮的恢复幕
/// 在表头里根本看不见（这正是 `resume_anchor` 旁路存在的原因， 步 5d）。
/// 恢复幕的 `turn.stage{stage:"resume"}` 是开演时刚落的，`ts` 比任何既有轮次都新，
/// 按 `(ts, seq)` 取最后一条幕锚即可让表头回到被续写的那一轮。
///
/// 没有幕锚的切片（测试夹具 / 只有配置事实）退回「最大轮号」——幕锚缺席时
/// 没有更诚实的轮号可给。
fn drive_of(events: &[Event]) -> Option<(u64, Option<&Event>)> {
    if let Some(anchor) = events
        .iter()
        .filter(|e| is_drive_anchor(e))
        .max_by_key(|e| (e.ts, seq_of(e)))
    {
        return Some((anchor.turn, Some(anchor)));
    }
    const HEAD_KINDS: &[&str] = &[
        "user.message",
        "turn.stage",
        "chat.assistant.final",
        "chat.assistant.fallback",
        "session.config",
    ];
    let turn = events
        .iter()
        .filter(|e| HEAD_KINDS.contains(&e.kind.as_str()))
        .map(|e| e.turn)
        .max()?;
    Some((turn, None))
}

fn is_drive_anchor(e: &Event) -> bool {
    e.kind == "user.message" || (e.kind == "turn.stage" && str_field(e, "stage") == Some("resume"))
}

fn seq_of(e: &Event) -> u64 {
    e.seq.map(|s| s.value()).unwrap_or(0)
}

/// 阶段「已做」的锚后判据：`seq` 必须存在且不早于幕锚。
///
/// `None` 的 `seq` 一律不算——未入库的事件不是事实（与 `stage_events` 的落格同判）。
fn after_anchor(e: &Event, anchor_seq: u64) -> bool {
    e.seq.is_some_and(|s| s.value() >= anchor_seq)
}

/// 轮头（升到 v5，步 5d 升到 v6）：五个策略行的开账条件全在这一格里，
/// 收束行与执行行也是。
///
/// `done` 三格按幕锚折叠（续写幕复用轮号，不能按轮扫全史）；`skill_hit` 取本幕
/// 召回阶段的结论，`resumed` 区分续写幕与用户新发言。
///
/// v5 加一格 `tools_open`：本幕最近一条模型应答交回了工具调用、且其中至少一笔
/// 还没被终局关掉。执行行的开账条件要它——工具批没关时模型不许再往下走
/// （b：`execute-turn` 从「跑下一步」退成「只跑手上那批」）。
/// v6 给它补了请求面（见该格的注释）：恢复幕的唤醒请求也在「有活没干完」之列。
///
/// v8（/ D-122 决策 2）：「本幕最近一次模型应答」与「模型没答成」两格都
/// 只看**这一轮自己的**模型往返（`via` 缺席的那些）——内部往返（判决 / 措辞 / 摘要）
/// 的应答 `calls` 为空，混进来会让一条判决往返被读成「这一轮答完了」。
///
/// v11（D-166）加 `model_open`：本幕已宣布、还没派出的模型往返。执行行与派发行
/// 因此按**事实**分工，而不是靠 `prio` 60 < 66 碰巧排对。
///
/// v14（`D-222` 订正 2）把 `model_open` 的关账格从 `sent` 挪到 `response` / `failed` /
/// `retry`：一次模型往返此后占**两个**环步（`dispatch-model` 落 `sent`、
/// `await-model` 落终局），`sent` 只说明「发出去了」，不说明「这一圈结束了」。
/// 按旧口径，派出一落 `sent` 本格就变假，执行行（60）立刻重新欠账、抢在 `await-model`
/// （71）之前再宣布一笔——同一步号的幂等键撞上，那一圈什么也没写，环读成
/// `exhausted{no_progress}`。关账口径与 [`open_calls`] 同源（`(id, attempt)` 上
/// 三条终局格），所以「在等」在派生面上只有一份定义。
///
/// v12 加 `prepared`：本幕落过 `inputs` 阶段格没有。它的读者（`prepare-turn` 行）
/// 已随 `turn` 端口退场（`D-222`），这一格因此无生产读者，去留由下一次行表归一决定。
///
/// v13（D-168 裁决 4）加 `closure`：本幕最近一条收束格的 `reason`。它是「还开不开
/// 下一幕」的唯一数据面——流向不再由 `close_turn` 的返回值决定。口径与 `closed`
/// 逐字同源（同一条格、同一个幕锚），因此**读面不变**。
fn turn_head(events: &[Event], _now: i64) -> serde_json::Value {
    let Some((turn, anchor)) = drive_of(events) else {
        return serde_json::Value::Null;
    };
    let anchor_seq = anchor.map(seq_of).unwrap_or(0);
    // 心跳等无用户节点的轮次也落用户格（`id: null`）——那不算「这一轮有发言」
    // （与 `first_utterance.is_some()` 同判）。
    let has_utterance = anchor.is_some_and(|e| {
        e.kind == "user.message" && e.payload.get("id").is_none_or(|v| !v.is_null())
    });
    let resumed =
        anchor.is_some_and(|e| e.kind == "turn.stage" && str_field(e, "stage") == Some("resume"));
    // 请求轮次以**本幕**落下的 `round` 格为准（不是全轮最大值：续写幕会复用轮号）。
    let round = events
        .iter()
        .filter(|e| {
            e.turn == turn
                && e.kind == "turn.stage"
                && str_field(e, "stage") == Some("round")
                && after_anchor(e, anchor_seq)
        })
        .max_by_key(|e| seq_of(e))
        .and_then(|e| u64_field(e, "round"))
        .unwrap_or(0);
    let config = config_of(events, turn);
    let flag = |key: &str| {
        config
            .and_then(|e| e.payload.get(key))
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    };
    let stage_since = |stage: &str| {
        events.iter().any(|e| {
            e.turn == turn
                && e.kind == "turn.stage"
                && str_field(e, "stage") == Some(stage)
                && u64_field(e, "round") == Some(round)
                && after_anchor(e, anchor_seq)
        })
    };
    let skill_hit = events.iter().any(|e| {
        e.turn == turn
            && e.kind == "turn.stage"
            && str_field(e, "stage") == Some("recall")
            && u64_field(e, "round") == Some(round)
            && after_anchor(e, anchor_seq)
            && str_field(e, "result") == Some("hit")
    });
    // 本幕是否已走快路作答（D-163）：`policy.reflex` 的出口只有这一格
    // （`stage:"reflex"`，`result` 是命中的技能 id）。收束行据此在**零模型往返**
    // 的轮上开账——快路不落 `response(port=model)`，`has_final` 折不出来。
    //
    // `result:"miss"` 不算「已答」：那是执行体在命中集为空时用来关账的结局
    // （正常不可达，开账条件与命中判定同源），这一轮该照常进模型往返。
    let reflex_answered = events.iter().any(|e| {
        e.turn == turn
            && e.kind == "turn.stage"
            && str_field(e, "stage") == Some("reflex")
            && u64_field(e, "round") == Some(round)
            && after_anchor(e, anchor_seq)
            && str_field(e, "result").is_some_and(|r| r != "miss")
    });
    // 本幕的判决是否把这一轮判成「不用再发请求」（D-162 裁决 1）。
    //
    // 折叠口径与 `stage_since` 逐字相同（同幕、同轮、幕锚之后），只是多比一格
    // `result`：判决那一步的**结论**说的就是「这一轮往下走还是就此答话」。
    // 压缩与召回的前件读它——定稿答话轮不进驱动循环（§5.1），这两行因此不该开账。
    let answered = events.iter().any(|e| {
        e.turn == turn
            && e.kind == "turn.stage"
            && str_field(e, "stage") == Some("decide")
            && u64_field(e, "round") == Some(round)
            && after_anchor(e, anchor_seq)
            && str_field(e, "result") == Some("answered")
    });
    // 本幕是否已收束（与 [`turn_closed`] 同一判法：幕锚之后出现过收束格）。
    let closed = events.iter().any(|e| {
        e.turn == turn
            && matches!(
                e.kind.as_str(),
                "chat.assistant.final" | "chat.assistant.fallback"
            )
            && after_anchor(e, anchor_seq)
    });
    // 本幕**为什么**停下（v13 / D-168 裁决 4）：本幕最近一条收束格的 `reason` 一格
    // （`ClosureReason` 的线格式）。未收束 ⇒ `null`。
    //
    // 这是「还开不开下一幕」的**唯一数据面**。流向此前是 `close_turn` 的返回值
    // （`TurnFlow`），由编排层 `match` 决定「再发一次请求」还是收工——那是按步骤号
    // 决定的判定点，而它在日志上早就有一条事实（收束格的 `reason`）：截断续写的
    // 幕与自然说完的幕在事实面上本就不同（`exhausted` / `completed`）。
    //
    // 取**最近一条**而不是「任意一条」：收束格一条就够（收束之后不再落），但
    // 兜底格与收束格可以先后出现（中止路径先兜底、收束行随后关门），口径统一
    // 按 seq 取末条。
    let closure = closed
        .then(|| {
            events
                .iter()
                .filter(|e| {
                    e.turn == turn
                        && matches!(
                            e.kind.as_str(),
                            "chat.assistant.final" | "chat.assistant.fallback"
                        )
                        && after_anchor(e, anchor_seq)
                })
                .max_by_key(|e| seq_of(e))
                .and_then(|e| str_field(e, "reason"))
        })
        .flatten();
    // 本幕最近一条 `response(port=model)`：收束格（`calls` 空）与工具格
    // （`calls` 非空且未关完）共用同一次选取，口径只在这里。
    //
    // **只取这一轮自己的模型往返**（`via` 缺席的那些，D-122 决策 2）：判决 / 措辞 /
    // 摘要那些内部往返的应答 `calls` 为空（它们不带工具），混进来会让一条判决往返
    // 被读成「这一轮答完了」⇒ 收束行提前开账。
    let model_calls = events
        .iter()
        .filter(|e| {
            e.turn == turn
                && e.kind == EVENT_RESPONSE
                && str_field(e, "port") == Some("model")
                && str_field(e, "via").is_none()
                && after_anchor(e, anchor_seq)
        })
        .max_by_key(|e| seq_of(e))
        .and_then(|e| e.payload.get("calls"))
        .and_then(|v| v.as_array());
    // 收束该不该发生（D-121 决策 2）：口径钉在日志上——**本幕最近一条
    // `response(port=model)` 的 `calls` 为空**（`expand: ["calls"]` 是「这条响应要不要
    // 转出工具债」的唯一数据面），且本幕未收束。执行体手里那份草稿不参与判定，
    // 否则崩溃恢复之后「答完了没」就成了假话。
    //
    // `calls` 取不到数组 ⇒ 不算收束（宁可让 `close-turn` 不开账，也不拿一个来路不明的
    // 形状把没答完的一轮判成答完）；没有响应（首步就中止 / 失败的轮）同理。
    let has_final = !closed && model_calls.is_some_and(|calls| calls.is_empty());
    // 本幕模型端口的**结局**（c）：本幕**派发过**的一次模型往返，最后落在
    // `failed(port=model)` 上——模型没答成，且之后没有更晚的成功应答。
    //
    // 这一格是「中止 / 适配器失败」那两种**收束**的前件：它们没有「`calls` 为空的
    // 模型应答」（答都没答成，哪来应答），`has_final` 因此不成立；但它们同样要把
    // 这一轮收掉（兜底格 / 中止格），而收束格只由 `close` 执行体写 ⇒ 行表得能
    // 判出「该收束了」，判据只能是日志上的东西。
    //
    // 「派发过」（同一 `(port, id)` 的 `request` 也在本幕锚之后）这一条不是装饰：
    // 崩溃恢复时租约看护会在**本幕锚之后**落一条 `failed(port=model)`——它收的是
    // 上一世那笔卡住的调用，而那笔的 `request` 在锚之前。少了这条，恢复幕一进门
    // 就会判「该收束了」，把「重跑一次模型往返」读成「这一轮已经答不上来了」。
    //
    // `via` 缺席的那些才是「这一轮自己的模型往返」（D-122 决策 2）：内部往返失败
    // （判决没打出来）退化成「判不出来 ⇒ 进工具循环」，**不是**「模型没答成 ⇒
    // 收束」——后者会让一条判决失败把整轮收掉。
    let model_outcome_failed = events
        .iter()
        .filter(|e| {
            e.turn == turn
                && matches!(e.kind.as_str(), EVENT_RESPONSE | EVENT_FAILED)
                && str_field(e, "port") == Some("model")
                && str_field(e, "via").is_none()
                && after_anchor(e, anchor_seq)
        })
        .max_by_key(|e| seq_of(e))
        .is_some_and(|e| {
            e.kind == EVENT_FAILED
                && events.iter().any(|r| {
                    r.turn == turn
                        && r.kind == EVENT_REQUEST
                        && str_field(r, "port") == Some("model")
                        && str_field(r, "via").is_none()
                        && after_anchor(r, anchor_seq)
                        && str_field(r, "id") == str_field(e, "id")
                })
        });
    // 该不该收束：`has_final`（答完了）∨ 模型没答成（不会再有应答）∨ 快路已作答
    // （零往返，D-163）。三条都要求本幕未收束——收束过就不欠了。
    let closeable = !closed && (has_final || model_outcome_failed || reflex_answered);
    // 工具批还开着（b）：交回的调用里至少有一笔没被 `response` / `failed`
    // （同 `port=tool` 同 id）关掉。两格互斥（`calls` 空 vs 非空），所以执行行只在
    // 「有活没干完」的时候让位。
    //
    // v6（d）再加一条**请求面**的判据：清点的是**本轮**（不是本幕）里
    // 落过、却还没有终局的 `request(port=tool)`。恢复幕交回的那一笔就靠它：模型
    // 响应是首幕的（本幕没有新的模型应答），旧批次也早已关账，唯一的「开着」是
    // 唤醒请求本身——不看请求面，执行行会在派发之前抢先跑一次模型往返。
    //
    // 判据按请求的 `seq` 取「之后有没有终局」，不按幕锚：唤醒请求落在幕锚
    // **之前**（恢复幕的锚是 `begin_drive` 才落的那条阶段事实）。
    let request_open = !closed
        && events.iter().any(|req| {
            req.turn == turn
                && req.kind == EVENT_REQUEST
                && str_field(req, "port") == Some("tool")
                && !events.iter().any(|e| {
                    e.turn == turn
                        && matches!(e.kind.as_str(), EVENT_RESPONSE | EVENT_FAILED)
                        && str_field(e, "port") == Some("tool")
                        && str_field(e, "id") == str_field(req, "id")
                        && seq_of(e) > seq_of(req)
                })
        });
    // 本幕有没有一笔**宣布之后还没收清**的模型往返（D-166，`D-222` 订正 2 改口径）。
    //
    // 一次模型往返现在占两个环步：`dispatch-model`（落 `sent`）与 `await-model`
    // （落 `response` / `failed`）。所以「这一圈过去了没有」的判据不是 `sent`，是
    // **终局格**——与 [`open_calls`] 逐字同源（`(id, attempt)` 上 `response` /
    // `failed` / `retry` 三条关账）。`sent` 只说明「发出去了」，此时应答还在路上，
    // 执行行若按 `sent` 让位就会在 `await-model` 之前再宣布一笔：同一步号的幂等键撞上，
    // 那一圈零写入，环读成 `exhausted{no_progress}`（`turn_exec` 的
    // `an_actor_that_writes_nothing_stops_as_no_progress` 与各轮用例都钉着它）。
    //
    // 执行行要这一格——一次模型往返的「宣布」与「派出」是两个环步，宣布之后
    // 派发行（66）还没轮到，执行行（60）按 `prio` 会抢先再宣布一笔；同一幕两条
    // `request(port=model)` 并存，模型就不再有「一次往返」这个粒度。
    //
    // 收窄按幕锚（`after_anchor`）：续写幕复用轮号，首幕那笔已关闭的请求不算「本幕
    // 还开着」；上一世遗留、本幕没宣布过的请求也不该把本幕的执行行堵住（它由派发
    // 行自己判死，见 `turn_exec::run_model`）。
    let model_open = events.iter().any(|req| {
        req.turn == turn
            && req.kind == EVENT_REQUEST
            && str_field(req, "port") == Some("model")
            && str_field(req, "via").is_none()
            && after_anchor(req, anchor_seq)
            && !events.iter().any(|e| {
                e.turn == turn
                    && matches!(e.kind.as_str(), EVENT_RESPONSE | EVENT_FAILED | EVENT_RETRY)
                    && str_field(e, "port") == Some("model")
                    && str_field(e, "id") == str_field(req, "id")
                    && u64_field(e, "attempt") == u64_field(req, "attempt")
            })
    });
    let tools_open = !closed
        && (request_open
            || model_calls.is_some_and(|calls| {
                calls.iter().any(|c| {
                    let id = c.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    !events.iter().any(|e| {
                        e.turn == turn
                            && matches!(e.kind.as_str(), EVENT_RESPONSE | EVENT_FAILED)
                            && str_field(e, "port") == Some("tool")
                            && str_field(e, "id") == Some(id)
                            && after_anchor(e, anchor_seq)
                    })
                })
            }));
    serde_json::json!({
        "turn": turn,
        "round": round,
        "anchor": anchor_seq,
        "has_utterance": has_utterance,
        "resumed": resumed,
        "classify_enabled": flag("classify_enabled"),
        "classify_done": stage_since("decide"),
        "answered": answered,
        "auto_compress": flag("auto_compress"),
        "compress_done": stage_since("compress"),
        "is_persistent": flag("is_persistent"),
        "recall_done": stage_since("recall"),
        "skill_hit": skill_hit,
        "reflex_answered": reflex_answered,
        // 本幕的请求输入已备好：`prepare-turn` 行的出口曾折在这里，那条行已删
        // （`D-222`：幕是直线，「备好没有」不再是一个前件）。这一格暂时无生产读者。
        "prepared": stage_since("inputs"),
        // 本幕的收束原因（D-168 裁决 4）：`exhausted` / `completed` / `null`（未收束）。
        // 读者只有幕间判据（`loop_rows::next_stage_round`）——「还开不开下一幕」。
        "closure": closure,
        "has_final": has_final,
        "closeable": closeable,
        "tools_open": tools_open,
        "model_open": model_open,
    })
}

/// 本幕落下的阶段事实（`turn.stage`）：一次性阶段的**出口**。
///
/// 按幕锚收窄，续写幕因此不会被首幕落在同一轮号上的 `recall` / `compress` 关账。
/// 数组的每个元素是一条阶段，键绑定读 `turn` 与 `round`。
fn turn_stages(events: &[Event], _now: i64) -> serde_json::Value {
    let Some((turn, anchor)) = drive_of(events) else {
        return serde_json::Value::Array(Vec::new());
    };
    let anchor_seq = anchor.map(seq_of).unwrap_or(0);
    serde_json::Value::Array(
        events
            .iter()
            .filter(|e| e.turn == turn && e.kind == "turn.stage" && after_anchor(e, anchor_seq))
            .map(|e| {
                serde_json::json!({
                    "turn": e.turn,
                    "stage": str_field(e, "stage"),
                    "round": u64_field(e, "round").unwrap_or(0),
                })
            })
            .collect(),
    )
}

/// 本幕是否已经收束（收束 / 兜底格出现在幕锚之后）。
///
/// `[]` = 还开着；一条 `{turn}` = 已关。执行行与反射行都拿它作出口。
fn turn_closed(events: &[Event], _now: i64) -> serde_json::Value {
    let Some((turn, anchor)) = drive_of(events) else {
        return serde_json::Value::Array(Vec::new());
    };
    let anchor_seq = anchor.map(seq_of).unwrap_or(0);
    let closed = events.iter().any(|e| {
        e.turn == turn
            && matches!(
                e.kind.as_str(),
                "chat.assistant.final" | "chat.assistant.fallback"
            )
            && after_anchor(e, anchor_seq)
    });
    if closed {
        serde_json::json!([{ "turn": turn }])
    } else {
        serde_json::Value::Array(Vec::new())
    }
}

/// **中途汇报该不该说**：只吃日志与 `now`，不碰内存计数 / 墙钟。
///
/// `quiet_ms` 的起点是本轮**最近一次用户看得见的动静**：助手说过的任何一句
/// （汇报 / 开场白 / 收束 / 等待）优先；一句都没说过时退到用户格 / 补充格。
///
/// `due` 的四条都由这份日志折出（v4）：开关 / 静默够久 / 配额未满 / **轮数够**。
/// 最后一条此前留在触发点（`round_report_hook` 自己数批次边界），因为当时的
/// 计数是内存里的 `AtomicUsize`；而「本幕完成了几轮工具调用」在日志上就是本幕
/// 落了几条**带 `calls` 的 `response(port=model)`**——一条批次边界 = 一次带工具
/// 调用的模型往返。折进视图之后，这一行在派发点上与其它行同形：判据只有数据。
fn report_due(events: &[Event], now: i64) -> serde_json::Value {
    let Some((turn, drive_anchor)) = drive_of(events) else {
        return serde_json::Value::Null;
    };
    let drive_seq = drive_anchor.map(seq_of).unwrap_or(0);
    let config = config_of(events, turn);
    let enabled = config
        .and_then(|e| e.payload.get("progress_enabled"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let interval_ms = config
        .and_then(|e| e.payload.get("progress_interval_ms"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let min_rounds = config
        .and_then(|e| e.payload.get("progress_min_rounds"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let quota = config
        .and_then(|e| e.payload.get("progress_max_per_turn"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    // 配额是**请求作用域**的量（旧 `RoundProgress` 随请求新建）：续写幕重新开账，
    // 故只数本幕锚之后的汇报。
    let reports = events
        .iter()
        .filter(|e| {
            e.turn == turn && e.kind == "chat.assistant.reported" && after_anchor(e, drive_seq)
        })
        .count() as u64;
    let anchor = events
        .iter()
        .filter(|e| e.turn == turn && is_report_anchor(e))
        .max_by_key(|e| (e.ts, seq_of(e)));
    let quiet_ms = anchor.map(|e| (now - e.ts).max(0)).unwrap_or(0);
    // 本幕**已完成几轮工具调用** = 幕锚之后带 `calls` 的模型应答条数。
    //
    // 口径与 `turn_head` 的 `model_calls` 同源：只看这一轮自己的往返（`via` 缺席），
    // 内部往返（判决 / 措辞 / 摘要）不带工具、也不该把「够不够轮」凑满。
    // `calls` 为空 / 取不到的应答不算一轮——那是答话或形状不明，不是「干过活」。
    let tool_rounds = events
        .iter()
        .filter(|e| {
            e.turn == turn
                && e.kind == EVENT_RESPONSE
                && str_field(e, "port") == Some("model")
                && str_field(e, "via").is_none()
                && after_anchor(e, drive_seq)
                && e.payload
                    .get("calls")
                    .and_then(|v| v.as_array())
                    .is_some_and(|calls| !calls.is_empty())
        })
        .count() as u64;
    // 轮数前提照旧是「轮数 > 0 ∧ 轮数 ≥ progress_min_rounds」：`max(1)` 就是
    // 「> 0」那一半——一轮都没跑完时汇报的是「还没开始」，那不是进度。
    let due =
        enabled && quiet_ms >= interval_ms && reports < quota && tool_rounds >= min_rounds.max(1);
    serde_json::json!({
        "turn": turn,
        // 静默区间的身份 = 区间起点那一格的 seq。汇报事实落格后，本视图的锚
        // 前移，下一段静默是新的一笔账（配额之外还要能逐段关账）。
        "anchor": anchor.map(seq_of).unwrap_or(0),
        "due": due,
        "quiet_ms": quiet_ms,
        // 本幕已完成的工具轮数：汇报句的「已经完成 N 轮」读它（同一份读数，
        // 不再由触发点自己数）。
        "tool_rounds": tool_rounds,
        "min_rounds": min_rounds,
        "reports": reports,
        "quota": quota,
    })
}

/// 中途汇报的**出口**：每一条汇报事实关掉它前一段静默区间的账。
///
/// 汇报自己会说一句话（成为新的静默起点），所以一条汇报只能关「它之前」那段：
/// 锚取**严格早于汇报 seq** 的最近一条动静。这样 `report_due.anchor` 与
/// 本视图的 `anchor` 在同一段区间上逐字相等，`(turn, anchor)` 就是区间的身份。
fn reported_anchors(events: &[Event], _now: i64) -> serde_json::Value {
    serde_json::Value::Array(
        events
            .iter()
            .filter(|e| e.kind == "chat.assistant.reported")
            .filter_map(|report| {
                let report_seq = report.seq?.value();
                let anchor = events
                    .iter()
                    .filter(|e| {
                        e.turn == report.turn
                            && e.seq.is_some_and(|s| s.value() < report_seq)
                            && is_report_anchor(e)
                    })
                    .max_by_key(|e| (e.ts, seq_of(e)));
                Some(serde_json::json!({
                    "turn": report.turn,
                    "anchor": anchor.map(seq_of).unwrap_or(0),
                }))
            })
            .collect(),
    )
}

/// 助手说过的每一句（汇报 / 开场白 / 收束 / 兜底 / 进展）。
const SPOKE_KINDS: &[&str] = &[
    "chat.assistant.reported",
    "chat.assistant.opening",
    "chat.assistant.final",
    "chat.assistant.fallback",
    "chat.assistant.progress",
];

/// 静默时钟的起点候选：用户看得见的任何一次动静。
///
/// 用户格 / 补充格 / 恢复幕都是「用户动了一下」，助手说过的每一句同理。恢复幕
/// 也算——审批 / 回填发生在请求开始时，那一幕的静默应当从此刻起算，而不是从
/// 几天前的那句用户话起算。
fn is_report_anchor(e: &Event) -> bool {
    is_drive_anchor(e) || e.kind == "turn.supplemented" || SPOKE_KINDS.contains(&e.kind.as_str())
}

pub(crate) const VIEWS: &[ViewDecl] = &[
    // 在途 = `sent` 中没有对应 `response` / `failed` / `retry`
    // （三者都是同 id + attempt）的那些。
    //
    // v2：加入 `retry` 一侧。没有它，被作废的那次尝试会永远留在「在途」里
    // ——而它已经不会再回来了。
    // v3：`failed` 也收窄到同一次尝试（D-138）；读面因此要多声明它的 `attempt`。
    ViewDecl {
        name: "in_flight",
        ver: 3,
        reads: &[
            (EVENT_SENT, "id"),
            (EVENT_SENT, "port"),
            (EVENT_SENT, "attempt"),
            (EVENT_RESPONSE, "id"),
            (EVENT_RESPONSE, "attempt"),
            (EVENT_FAILED, "id"),
            (EVENT_FAILED, "attempt"),
            (EVENT_RETRY, "id"),
            (EVENT_RETRY, "attempt"),
        ],
        // 输出的每一格就是「还开着的 `sent`」，四个协议名都在输出口上出现。
        evidence: &[EVENT_SENT, EVENT_RESPONSE, EVENT_FAILED, EVENT_RETRY],
        cost: 4096,
        eval: in_flight,
    },
    // 卡住：与 `in_flight` 同一口径的在途集合，加上租约与尝试上限两个判断。
    //
    // 时间不进欠账公式：`stale` 由本视图把 `now` 折成布尔，租约行只对 `true` 开账
    // （与 `report_due.due` 同一条纪律）。
    ViewDecl {
        name: "stale_sent",
        // v2：`failed` 的关账口径收窄到同一次尝试（D-138），读面多声明它的
        // `attempt`；`awaited` 不在这里折——它不是「卡住」的一部分，由看护者
        // 从 `sent` 事实上读。
        ver: 2,
        reads: &[
            (EVENT_SENT, "id"),
            (EVENT_SENT, "port"),
            (EVENT_SENT, "attempt"),
            (EVENT_SENT, "lease_ms"),
            (EVENT_SENT, "attempt_cap"),
            (EVENT_RESPONSE, "id"),
            (EVENT_RESPONSE, "attempt"),
            (EVENT_FAILED, "id"),
            (EVENT_FAILED, "attempt"),
            (EVENT_RETRY, "id"),
            (EVENT_RETRY, "attempt"),
        ],
        evidence: &[EVENT_SENT, EVENT_RESPONSE, EVENT_FAILED, EVENT_RETRY],
        cost: 4096,
        eval: stale_sent,
    },
    // 轮头（升到 v5，步 5d 升到 v6）：五个策略行的开账条件 +
    // `close-turn` / `execute-turn` 的开账条件。
    //
    // v3 → v4 只加一格：`has_final`（口径见 [`turn_head`]）。它是收束行的**前件**，
    // 因此读面要连收束格与模型响应的 `calls` 一起声明——视图声明的字段名是装载期
    // 的孤儿事实判据（`OrphanFact`）的输入，声明与算法不一致就是给表少报一条边。
    // v4 → v5 加 `tools_open`：它读 `failed` 的 `port` / `id`（工具批的另一种终局），
    // 其余读面不变。
    // v5 → v6 给 `tools_open` 补请求面：它读 `request(port=tool)` 的 `port` / `id`。
    // v6 → v7 加 `closeable`：收束行的前件从「答完了」扩到「答完了 **或** 模型没答成」
    // ——后者的读面是 `failed` 的 `port`（`failed(port=model)`）与 `request` 的
    // `port` / `id`（判「本幕派发过」），v4 / v5 起就已在读面里，故读面不变。
    // v7 → v8：口径收窄到「这一轮自己的模型往返」——`request` / `response` / `failed`
    // 三个协议名的 `via` 字段进读面（D-122 决策 2）。内部往返（判决 / 措辞 /
    // 摘要）带 `via`，这一轮自己的不带；不带的那批才是「答完了 / 没答成」的判据。
    // v8 → v9 加 `answered`（D-162 裁决 1）：判决那一步的结论折成这一格，定稿答话轮
    // 因此不再开压缩 / 召回的账。判据只能落在**同一个视图的一格**上：`owed` 把
    // `unless` 的键与债键逐字比对，跨键的 `unless` 是静默不命中（§3.4 第 2 条）。
    // 读面不变——`turn.stage` 的 `result` 自 v2（技能命中折在召回格里）就在里面。
    // v9 → v10 加 `reflex_answered`（D-163）：快路的出口折成这一格，收束行因此在
    // **零模型往返**的轮上也开得出账（`has_final` 折不出没有应答的轮）。读面不变。
    // v10 → v11 加 `model_open`（D-166）：模型端口的「已宣布、没派出」折成一格，
    // 执行行据此让位给派发行。读面补 `request` / `failed` 的 `attempt` 与 `sent` 的
    // `port` / `id` / `attempt`——判「这笔账关没关」要与派发行同一组键对。
    // v11 → v12 加 `prepared`（D-168 裁决 3）：读的是 `turn.stage` 的 `stage` / `round`，
    // v2 起就在读面里，故读面不变。
    // v12 → v13 加 `closure`（D-168 裁决 4）：读面补收束两格的 `reason`——这一格是
    // 「为什么停下」的原始事实，折进视图之后「还开不开下一幕」不再靠返回值。
    // v13 → v14（`D-222` 订正 2）：`model_open` 的关账格从 `sent` 挪到 `response` /
    // `failed` / `retry`（一次往返占两个环步）。读面补 `retry` 的 `port` / `id` /
    // `attempt`——判「这笔账关没关」的三个关账格里先前只有两个在面上。
    ViewDecl {
        name: "turn_head",
        ver: 14,
        reads: &[
            ("user.message", "id"),
            ("turn.stage", "stage"),
            ("turn.stage", "round"),
            ("turn.stage", "result"),
            ("session.config", "classify_enabled"),
            ("session.config", "auto_compress"),
            ("session.config", "is_persistent"),
            (EVENT_REQUEST, "port"),
            (EVENT_REQUEST, "id"),
            (EVENT_REQUEST, "attempt"),
            (EVENT_REQUEST, "via"),
            (EVENT_SENT, "port"),
            (EVENT_SENT, "id"),
            (EVENT_SENT, "attempt"),
            (EVENT_RESPONSE, "port"),
            (EVENT_RESPONSE, "id"),
            (EVENT_RESPONSE, "attempt"),
            (EVENT_RESPONSE, "calls"),
            (EVENT_RESPONSE, "via"),
            (EVENT_RETRY, "port"),
            (EVENT_RETRY, "id"),
            (EVENT_RETRY, "attempt"),
            (EVENT_FAILED, "port"),
            (EVENT_FAILED, "id"),
            (EVENT_FAILED, "attempt"),
            (EVENT_FAILED, "via"),
            ("chat.assistant.final", "text"),
            ("chat.assistant.fallback", "text"),
            ("chat.assistant.final", "reason"),
            ("chat.assistant.fallback", "reason"),
        ],
        // 表头是「本轮开账条件的全部输入」：每个读面的事实名都在输出里出现一次
        // （有没有发言 / 判过没有 / 召回过没有 / 答完没有…），故证据集 = 读面全集。
        evidence: &[
            "user.message",
            "turn.stage",
            "session.config",
            EVENT_REQUEST,
            EVENT_SENT,
            EVENT_RESPONSE,
            EVENT_FAILED,
            "chat.assistant.final",
            "chat.assistant.fallback",
        ],
        cost: 4096,
        eval: turn_head,
    },
    // 本幕落下的阶段事实：`decide` / `recall` / `compress` 三行的出口。
    ViewDecl {
        name: "turn_stages",
        ver: 1,
        reads: &[
            ("user.message", "id"),
            ("turn.stage", "stage"),
            ("turn.stage", "round"),
        ],
        // 输出就是「本幕落下的阶段格」；`user.message` 只用来定位幕锚，
        // 它出现与否不关这一行收口（假边的来源，见 DE-015 的 D-160）。
        evidence: &["turn.stage"],
        cost: 4096,
        eval: turn_stages,
    },
    // 本幕是否收束：执行行与反射行的出口。
    ViewDecl {
        name: "turn_closed",
        ver: 1,
        reads: &[
            ("user.message", "id"),
            ("turn.stage", "stage"),
            ("chat.assistant.final", "text"),
            ("chat.assistant.fallback", "text"),
        ],
        // 两条收束格才是「本幕收束了」的证据；`user.message` / `turn.stage` 只用来
        // 找幕锚——按读面全集配图，任何写 `turn.stage` 的执行体都成了执行行的关门方。
        evidence: &["chat.assistant.final", "chat.assistant.fallback"],
        cost: 4096,
        eval: turn_closed,
    },
    // 汇报到期：静默时长 = `now` − 最近一次用户看得见的动静的 `ts`；
    // 配额 = 本轮汇报数 vs `session.config.progress_max_per_turn`。
    //
    // v2 起输出 `anchor`（静默起点的 seq）：汇报行以 `(turn, anchor)` 标识一段
    // 静默区间，`reported_anchors` 逐条关账。v3 把 `quiet_ms` / `min_rounds`
    // 一并交给触发点——它们不再从进程配置里读，判定输入只剩事实与 `now`。
    //
    // v4 把**轮数前提**收进 `due`：`tool_rounds`（本幕带 `calls` 的模型应答数）
    // 与 `min_rounds` 的比较此前留在触发点（它自己数批次边界），于是这一行是
    // 「表判一半、代码判一半」；折进来之后派发点只剩选债。读面因此多声明
    // `response` 的 `port` / `via` / `calls` 三格（口径与 `turn_head` 同一条：
    // 只看这一轮自己的往返）。
    ViewDecl {
        name: "report_due",
        ver: 4,
        reads: &[
            ("session.config", "progress_enabled"),
            ("session.config", "progress_interval_ms"),
            ("session.config", "progress_min_rounds"),
            ("session.config", "progress_max_per_turn"),
            ("chat.assistant.reported", "tool_rounds"),
            ("chat.assistant.opening", "text"),
            ("chat.assistant.final", "text"),
            ("chat.assistant.fallback", "text"),
            ("chat.assistant.progress", "status"),
            ("user.message", "text"),
            ("turn.supplemented", "text"),
            ("turn.stage", "stage"),
            (EVENT_RESPONSE, "port"),
            (EVENT_RESPONSE, "via"),
            (EVENT_RESPONSE, "calls"),
        ],
        // 开账侧的视图：输出的每个字段都由这些格里的一格决定（静默起点、配额、
        // 本幕汇报数、本幕工具轮数），证据集取读面全集。它从不作行的出口，
        // 配图用不到这一格。
        evidence: &[
            "session.config",
            "chat.assistant.reported",
            "chat.assistant.opening",
            "chat.assistant.final",
            "chat.assistant.fallback",
            "chat.assistant.progress",
            "user.message",
            "turn.supplemented",
            "turn.stage",
            EVENT_RESPONSE,
        ],
        cost: 4096,
        eval: report_due,
    },
    // 汇报的出口：每条汇报事实关掉它之前那段静默区间。
    ViewDecl {
        name: "reported_anchors",
        ver: 1,
        reads: &[
            ("user.message", "text"),
            ("turn.supplemented", "text"),
            ("turn.stage", "stage"),
            ("chat.assistant.reported", "text"),
            ("chat.assistant.opening", "text"),
            ("chat.assistant.final", "text"),
            ("chat.assistant.fallback", "text"),
            ("chat.assistant.progress", "status"),
        ],
        // 出口侧只认汇报格：其余读面算的是「这条汇报关的是哪一段静默」的区间起点，
        // 它们出现得再多，也不会让某一笔汇报债关门。
        evidence: &["chat.assistant.reported"],
        cost: 4096,
        eval: reported_anchors,
    },
];

fn str_field<'a>(e: &'a Event, key: &str) -> Option<&'a str> {
    e.payload.get(key).and_then(|v| v.as_str())
}

fn u64_field(e: &Event, key: &str) -> Option<u64> {
    e.payload.get(key).and_then(|v| v.as_u64())
}

#[cfg(test)]
#[path = "control_decl.test.rs"]
mod tests;
