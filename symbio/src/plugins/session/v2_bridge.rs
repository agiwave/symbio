//! v2 事实桥——把 v1 会话轮次**转写**进事件网格（ADR-044：实测与判据同源）。
//!
//! 定位：chat_loop 切到 v2 链路之前的过渡件。v1 管线行为**零变化**，只是在
//! 每轮收束时把「用户发言 / 助手答复 / 实测耗时」转写为 v2 事件，落进
//! per-session 的持久事实源（`<会话目录>/v2-events.wal`）——P99 / 兜底率
//! 等口径从此有真实流量可扫。
//!
//! 为什么这不是「旁路遥测」：ADR-044 反对的是**不进事件网格的计数器**；
//! 本桥是把 v1 流量的既有事实**转写为网格事件**（同一事实落入事实源），
//! 与 v1 侧的对话存储是两份持久化、一份语义。
//!
//! 口径：
//! - 档位恒 `deep`——v1 chat_loop 的每一轮都装配完整模型 + 工具；
//! - `turn` 号 = WAL 内既有 user.message 计数（0 起）；
//! - 重试（resume 重跑同一用户消息）在 v2 网格里是**新的轮次**（attempt
//!   递增）——失败与成功都是真实发生的收束，各自成格，N3 仍由构造成立；
//! - 中止（Aborted）不转写：轮未收束，网格少一格是诚实的缺口，不是假象；
//! - 临时会话（不落盘）不转写：事实源本就不持久，转写无从安放；
//! - 总开关：`SessionConfig::v2_mode`（`off` / `bridge`，默认 `bridge`）——
//!   `off` 档不转写（用户关的是数据源，不是对话；v1 行为照旧）。

use std::path::PathBuf;

use crate::symbio_core::{
    Entity, Event, EventWalStore, PermissionMatrix, Seq, Store, Verb, EVENT_ASSISTANT_FALLBACK,
    EVENT_ASSISTANT_FINAL, EVENT_MEMORY_RECALLED, EVENT_USER_MESSAGE,
};

use super::chat_session::PersistentChatSession;
use crate::symbio_core::chat_message as cm;

/// 一轮的收束形态（转写的第二只脚；第一只脚是用户发言）。
pub(crate) enum V2Closure {
    /// 模型作答完成。
    Final { text: String, cost_ms: u64 },
    /// 兜底（I3：失败也是一句话——`why` 是用户实际看到的东西）。
    Fallback { why: String, cost_ms: u64 },
}

/// 把一轮事实转写进该会话的 v2 WAL。
///
/// 所有失败都只记日志不冒泡：桥的故障不得拖垮 v1 对话——但**必须被看见**
/// （plugin_warn），不允许静默吞。
///
/// `recalled` = 本轮开头召回的长期记忆视图（S5 步 12），非空且有条目时收束入格一条
/// `memory.recalled`。它从 [`super::chat_loop::state::TurnState`] 一路带到这里才落笔：
/// 溯源锚是本轮 `user.message` 格，只有轮末它才在事实源里。
///
/// `delegations` / `tasks` 同一形态：工具执行层的出参（批⑧ 的承诺 / 批⑨ 的任务表），
/// 一路带到这里入格——工具执行层没有事实源，谁负责入格谁负责开这扇门。
///
/// `skill_obs` 同一形态：读侧的出参（`v2_skills::route` 在 `prepare_turn_inputs` 里
/// 拿置信度闸判的逐条 `(skill_id, fallback)`），轮末落一条 `memory.recalled` 载荷——
/// `calibration` 投影只认带这两个字段的事件，**不落它，回退一次都不会发生**
/// （S11 §5 静默失效第 2 行）。技能编译不额外传参：开关在 `session` 上，本函数
/// 已经拿着它。
// 10 个参数：收束转写的全部输入各自独立（轮次上下文 / 档位 / 记忆 / 承诺 / 任务表 /
// 熔断 / 技能判定各一条出路），打包成 struct 只多一层间接；单一调用点（chat_loop）传入，
// 显式豁免参数数上限。
#[allow(clippy::too_many_arguments)]
pub(crate) fn record(
    session: &PersistentChatSession,
    principal: &str,
    user_id: &str,
    user_text: &str,
    closure: V2Closure,
    recalled: Option<&crate::symbio_core::RecallView>,
    delegations: &[super::tools::Delegation],
    tasks: &[super::tools::TaskDeclaration],
    breaks: &[&'static str],
    skill_obs: &[(String, bool)],
) {
    // 总开关（`v2_mode`，ADR-045 过渡期的切换档位）：`off` 档网格零增长
    // （用户关的是数据源，不是对话）。`bridge` 档：v1 轮次全部经此转写；
    // `full` 档：轮次由 v2 运行器原生记账，调用侧以 `TurnState::v2_executed`
    // 拦下，不经此转写——**但拦下的只是「轮次事实」**：记忆与学习是派生副作用，
    // 由 [`record_learning`] 承担、`v2_exec` 在轮末直接调用（同一函数、两个调用点，
    // 不是两份实现）。检查在取目录之前：关掉时连 WAL 的打开开销都不该有。
    if matches!(session.v2_mode(), super::config::V2Mode::Off) {
        return;
    }
    let Some(dir) = session.session_dir() else {
        return; // 临时会话：事实源不持久，转写无从安放（见模块文档口径）
    };
    let result = record_to_wal(
        dir.join(super::paths::V2_WAL_FILE),
        principal,
        user_id,
        user_text,
        closure,
        recalled,
        delegations,
        tasks,
        breaks,
        skill_obs,
        session.skill_compile_enabled(),
    );
    if let Err(why) = result {
        crate::plugin_warn!(
            "session",
            "[v2-bridge] 转写失败（{}）：{}",
            dir.display(),
            why
        );
    }
}

// 12 个参数：见 `record` 的同类豁免（本函数是它的落格体，形状随行）。
#[allow(clippy::too_many_arguments)]
fn record_to_wal(
    wal: PathBuf,
    principal: &str,
    user_id: &str,
    user_text: &str,
    closure: V2Closure,
    recalled: Option<&crate::symbio_core::RecallView>,
    delegations: &[super::tools::Delegation],
    tasks: &[super::tools::TaskDeclaration],
    breaks: &[&'static str],
    skill_obs: &[(String, bool)],
    skill_compile: bool,
) -> Result<(), String> {
    let store = EventWalStore::open(&wal).map_err(|e| format!("打开 WAL 失败：{e}"))?;
    let snapshot = store.range(Seq::new(0));

    // turn 号 = 既有 user.message 计数；attempt = 同一 v1 用户消息的转写次数
    //（重试各成一格，失败与成功都是真实发生的收束）。
    let mut turn = 0u64;
    let mut attempt = 0u64;
    for e in &snapshot {
        if e.kind == EVENT_USER_MESSAGE && e.entity == Entity::Turn {
            turn += 1;
            if e.payload.get("turn_ref").and_then(|v| v.as_str()) == Some(user_id) {
                attempt += 1;
            }
        }
    }

    let user = Event::pending(
        format!("v2u-{user_id}-a{attempt}"),
        EVENT_USER_MESSAGE,
        Entity::Turn,
        Verb::Opened,
        turn,
        crate::symbio_core::authz::PRINCIPAL_USER,
    )
    .with_payload(serde_json::json!({ "text": user_text, "tier": "deep", "turn_ref": user_id }));
    let user_seq = store
        .append(user)
        .map(|s| s.value())
        .map_err(|e| format!("用户消息入格失败：{e:?}"))?;

    // 第 5 个返回值是**成功收束的正文**（技能编译的输入）：`text` 在 json! 里被
    // 消费，兜底收束没有可固化的回答 ⇒ `None`（只编译走得通的路，S11 §2）。
    let (id, kind, payload, cost_ms, success_text) = match closure {
        V2Closure::Final { text, cost_ms } => (
            format!("v2f-{user_id}-a{attempt}"),
            EVENT_ASSISTANT_FINAL,
            serde_json::json!({ "text": text.clone(), "model": "v1-chat_loop" }),
            cost_ms,
            Some(text),
        ),
        V2Closure::Fallback { why, cost_ms } => (
            format!("v2fb-{user_id}-a{attempt}"),
            EVENT_ASSISTANT_FALLBACK,
            serde_json::json!({ "why": why }),
            cost_ms,
            None,
        ),
    };

    // 01 §7 写侧闸（[04 §3.1 批⑥](../../../docs/plan/04-工程落地.md)）：收束格是
    // 主智能体的**权威发言**，入格前判它是否持有对应能力——该轮首条 →
    // `reply.first`，后续（同轮追加 / 重试）→ `reply.append`。映射口径在 core 的
    // `can_reply`，此处不复述；表由 `crate::symbio_core::authz` 提供，全机只有那一张。
    // 拒绝 = **不入格**：用户发言已入格、本轮留在未收束态，`check_all` 会把它
    // 报进第五列（不变量）——拒绝可被看见，且 v1 对话行为不变（由 `record` 吞下）。
    let prior_closures = snapshot
        .iter()
        .filter(|e| e.entity == Entity::Turn && e.verb == Verb::Closed && e.turn == turn)
        .count() as u64;
    authorize_close(
        &crate::symbio_core::authz::matrix_for(principal),
        principal,
        prior_closures == 0,
    )?;

    // ── 收束发言先落，派生事实随后 ─────────────────────────────────────────
    // 顺序是判据，不是排版：`PreemptionDecider::decide` 的「final 已发出 → 排队」
    // 边界（04 §2.3 步 1）比的是**任务开格之后**有没有收束发言。若 final 落在
    // `task.opened` 之后，那么宣告这个任务的那条发言自己就满足了该条件——抢占
    // 在生产里一次都不会发生（只有 fallback 收束例外），判定被接入却永远走不到
    // `Suspend`，正是 S07 §5 要防的静默失效。改为先落这一轮说了什么、再落它派生
    // 出的承诺 / 任务 / 熔断：三者溯源锚都是 `user_seq`，彼此顺序不影响任何不变量。
    store
        .append(
            Event::pending(id, kind, Entity::Turn, Verb::Closed, turn, principal)
                .with_produced_by(user_seq)
                .with_cost_ms(cost_ms)
                .with_payload(payload),
        )
        .map_err(|e| format!("收束事件入格失败：{e:?}"))?;

    // ── 承诺（04 §3.1 批⑧，S08 §3「加格子，不加机制」）──────────────────
    // 本轮代际立约随收束入格：溯源锚是刚落的 `user.message`（`user_seq`）——
    // 承诺是「这轮我答应了什么」，锚必须落在这一轮的开口上，否则立约会漂在
    // 没有出处的 turn 0。立约与了结**同锚成对**，`check_all` 才看得见它们。
    // 承诺号带 `{user_id}-a{attempt}` 前缀：tool_call id 只在一次模型响应内唯一，
    // 而 WAL 的幂等键是事件 id——跨轮复用同一个 id 会让第二次立约撞 `Duplicate`、
    // 把整轮转写拖失败。
    for d in delegations {
        let cid = format!("v2c-{user_id}-a{attempt}-{}", d.id);
        for e in crate::symbio_core::commitment_events(
            &cid,
            principal,
            crate::symbio_core::authz::PRINCIPAL_USER,
            &d.promise,
            d.ok,
            &d.why,
            user_seq,
        ) {
            store
                .append(e)
                .map_err(|e| format!("承诺事件入格失败：{e:?}"))?;
        }
    }

    // ── 任务表（S7 步 16–18，04 §3.1 批⑨）────────────────────────────────
    // 本轮 `todo_write` 声明的清单状态随收束入格，与承诺**同锚**（`user_seq`）：
    // 任务是「这轮模型说该做什么」，出处是这一轮的开口。写方住 `v2_tasks`，
    // 此处只负责把门推开——失败**只记日志不冒泡**（任务格是本轮的附加事实，
    // 它失败不该被说成「转写失败」，与记忆三段同一条口径）。
    if let Err(why) = super::v2_tasks::write(
        &store,
        &snapshot,
        tasks,
        turn,
        user_seq,
        principal,
        &format!("v2t-{user_id}-a{attempt}"),
    ) {
        crate::plugin_warn!("session", "[task] 清单入格失败（{}）：{why}", wal.display());
    }

    // ── 熔断（S8 第 20 步，04 §3.1 批⑩）────────────────────────────────────
    // 本轮被外部执行闸门拦下的事实随收束入格，与承诺 / 任务**同锚**（`user_seq`）：
    // 熔断说的是「这一轮不允许做」，出处同样是这一轮的开口。写方住 core 的
    // `CircuitBreaker`，此处只负责把门推开——失败**只记日志不冒泡**（熔断格是
    // 本轮的附加事实，与任务格 / 记忆三段同一条口径）。
    //
    // 一轮只落**一格**：事件号按溯源锚定（`cb-{user_seq}`），两条就是同一个幂等键，
    // 第二条会被存储丢掉——那正是验收 2 要禁的「静默继续」，所以宁可在写**之前**
    // 就只取第一条，而不是靠撞键兜底。逐工具的拒绝理由不在这里：它们已经在每条
    // 工具结果节点上（`Refused: …`），模型与用户都看得见；这里记的是**本轮被
    // 闸门拦过**这件事本身。
    if let Some(&reason) = breaks.first() {
        if let Err(e) =
            store.append(crate::symbio_core::CircuitBreaker.break_event(reason, user_seq))
        {
            crate::plugin_warn!(
                "session",
                "[v2-bridge] 熔断事件入格失败（{}）：{e:?}",
                wal.display()
            );
        }
    }

    // ── 收束派生事实：记忆与学习（步 11–13 + 步 22）────────────────────────
    // 抽成独立函数是因为**它不是轮次事实**：`full` 档由 v2 运行器原生记账、
    // 轮次事实不经本函数，但记忆与学习两档都要写（见 [`record_learning`] 的文档）。
    record_learning(
        &wal,
        &store,
        &snapshot,
        turn,
        user_seq,
        user_text,
        principal,
        &format!("{user_id}-a{attempt}"),
        success_text.as_deref(),
        recalled,
        skill_obs,
        skill_compile,
        crate::symbio_core::clock_now_ms(),
    );
    Ok(())
}

/// 收束派生事实中的**记忆与学习**半边（步 11–13 + 步 22）。
///
/// ## 为什么它与「轮次事实转写」是两件事
///
/// 轮次事实（`v2u-*` 用户格 / `v2f-*` 收束格 / 承诺 / 任务表 / 熔断）记的是
/// **这一轮说了什么、答应了什么**。`v2_mode = full` 档由 v2 运行器原生记账，
/// 调用侧以 `TurnState::v2_executed` 把整段 `record` 拦下——同一轮两份记账是假象。
///
/// 但记忆与学习**不是轮次事实**，而是本轮的**派生副作用**：`v2_exec` 一处都不写
/// （它只写轮次事实）。若这一半也随轮次事实一起被拦下，`full` 档下
/// 「编码 / 检索锚 / 巩固 / 技能观测 / 技能编译」会**静默全丢**——S06 的长期记忆与
/// S11 的自我改进在这个档位整体失效，而档位名还自称「整体切换」。
/// （本文件曾写「记忆写方同此档位……见 `v2_exec` 侧的同批注记」，而那一侧既无注记
/// 也无实现——`full` 档的写侧因此空了一整块。）
///
/// ## 锚点由调用方给，锚的**含义**不由调用方定
///
/// `user_seq` = 本轮 `user.message` 格的 seq。两档的锚点是**不同的事件**
/// （bridge = 转写写的 `v2u-*` 格；full = v2 原生写的 `u-{turn}` 格），但
/// 「溯源锚必须是本轮的开口」这条不变（I2：记忆溯源覆盖 100%）。
/// 同理 `anchor_id` 是幂等键的**词干**：bridge 用 `{v1 消息 id}-a{attempt}`
/// （同一句 v1 消息重试各成一格），full 用 `t{turn}`（v2 侧一轮一个号）。
///
/// ## 失败只记日志不冒泡
///
/// 记忆是本轮的附加事实，它失败不该被说成「转写失败」（那会把桥的健康度算错）。
///
/// ## 顺序是判据
///
/// 1. 编码（步 11）先写——本轮新记忆要能进本轮的巩固；
/// 2. 检索事实（步 12）与它平级：溯源锚同为本轮开口；
/// 3. 巩固（步 13）最后，且用**刷新后的**快照——它要看得见 1 刚写下的那条；
/// 4. 技能观测在前、编译在后：观测记的是**本轮判没判、判成什么**，编译产出的是
///    **下一轮才可能被用的技能**。
#[allow(clippy::too_many_arguments)]
pub(crate) fn record_learning(
    wal: &std::path::Path,
    store: &EventWalStore,
    snapshot: &[Event],
    turn: u64,
    user_seq: u64,
    user_text: &str,
    principal: &str,
    anchor_id: &str,
    response: Option<&str>,
    recalled: Option<&crate::symbio_core::RecallView>,
    skill_obs: &[(String, bool)],
    skill_compile: bool,
    now: i64,
) {
    let remember = |step: &str, why: String| {
        crate::plugin_warn!(
            "session",
            "[memory] {step} 失败（{}）：{why}",
            wal.display()
        );
    };

    // ── 步 11 编码（S5，04 §3.1 批⑦）────────────────────────────────────
    if let Err(why) = super::v2_memory::encode(
        store,
        snapshot,
        turn,
        user_seq,
        user_text,
        &format!("v2m-{anchor_id}"),
        now,
    ) {
        remember("步 11 编码", why);
    }
    // ── 步 12 检索入格：本轮召回视图 → 一条 `memory.recalled` ─────────────
    if let Some(view) = recalled {
        if let Err(why) = super::v2_memory::record_recalled(store, view, user_seq, principal, now) {
            remember("步 12 检索入格", why);
        }
    }
    // ── 步 13 巩固（用**刷新后**的快照）──────────────────────────────────
    if let Err(why) = super::v2_memory::consolidate(store, &store.range(Seq::new(0)), turn, now) {
        remember("步 13 巩固", why);
    }

    // ── 步 22 · 技能路由观测（S11，04 §3.1 批⑪ 子批 B）───────────────────
    //
    // 观测只带数据 `{ skill_id, fallback }`——`calibration` 投影的契约就是这两个
    // 字段（S11 §3「不加新格子」：复用 `memory.recalled`，它的 `payload` 由写方
    // 决定）。**没有这一步，回退永远不会发生**：零使用时置信度按 1.0 算，技能
    // 再错也一直被用，S11 §5 的「一直用错技能而自己不知道」当场应验。
    for (n, (skill_id, fallback)) in skill_obs.iter().enumerate() {
        let ev = Event::pending(
            format!("v2s-{anchor_id}-{n}"),
            EVENT_MEMORY_RECALLED,
            Entity::Memory,
            Verb::Asserted,
            turn,
            principal,
        )
        .with_produced_by(user_seq)
        .with_ts(now)
        .with_payload(serde_json::json!({ "skill_id": skill_id, "fallback": fallback }));
        if let Err(e) = store.append(ev) {
            remember("步 22 路由观测", format!("{e:?}"));
        }
    }

    // ── 步 22 · 技能编译 ────────────────────────────────────────────────
    // 只编译**成功**收束（`response` = `V2Closure::Final` 的正文）——兜底说明这条路
    // 自己没走通，固化它等于把失败写成套路。开关 `skill_compile_enabled` 默认 off
    // （S11 §4 平凡值「不编译，只检索」）：关着时读侧逐条跳过，整条
    // 编译 → 校准 → 回退链路原地待命、不产生任何事实。
    if skill_compile {
        if let Some(response) = response {
            if let Err(why) =
                super::v2_skills::compile(store, snapshot, turn, user_seq, user_text, response, now)
            {
                remember("步 22 技能编译", why);
            }
        }
    }
}

/// 收束入格前的写侧授权闸（[plan/01 §7](../../../docs/plan/01-核心架构.md) 写侧）。
///
/// `first` = 该轮尚无收束格。拒绝的语义是**不入格**，不是「照写但记一笔」——
/// 记一笔会把授权判定降级成一个可以忽略的日志项。
fn authorize_close(matrix: &PermissionMatrix, principal: &str, first: bool) -> Result<(), String> {
    if matrix.can_reply(principal, first) {
        return Ok(());
    }
    Err(format!(
        "收束被授权拒绝（{} 缺 {}）",
        principal,
        if first { "reply.first" } else { "reply.append" }
    ))
}

/// 本轮用户发言的**兜底取法**（消息 id + 文本）：历史里**第一条已提交**的用户 Text 节点。
///
/// 正路是 `chat_loop::state::TurnState::input_utterance`——循环前在 `single_message`
/// 上锚定的那一份（判决与转写共用同一份锚）。本函数只服务**没有锚**的请求
/// （`resume` 重跑等，那时「本轮」无从界定），退而取历史首条。**有锚时不得用它**：
/// `context.messages` 在 `load_history = true` 下装着整段历史，取「第一条」会逐轮
/// 指回首轮那句——转写的 `user.message` 文本 / `attempt` 判据与记忆编码会一起记错。
///
/// 判别式 = `role` + `msg_type` 两条**构造即成立**的字段，再加 `status` 只用来
/// 排除**显式在途**：`None` 与 `Completed` 都算已提交，`Streaming` / `Pending` /
/// `WaitingUserAction` 不算。
///
/// 为什么 `None` 算数：用户消息不是流式产物，而它的写入方有好几个——`chat/send`
/// 的调用方（CLI / 前端）、收件箱入口（`plugin/nodes.rs::parse_inbox_message`）、
/// 心跳——各写各的形状，`chat/send` 那一支就**不填** `status`
/// （`ChatMessage::default()` → `None`）。把「填了且 = `Completed`」当判据，
/// 转写就在不填的那一支上**静默不发生**：不报错、连 `plugin_warn!` 都没有，
/// 事实源默默少一格，P99 与兜底率默默缺一轮——比转写失败更难发现。
///
/// [`last_assistant_text`] 要求 `status == Completed`：助手消息**是**流式的，
/// 未收束的那条不该被当成终稿。两条判据因此不对称，这不是笔误。
pub(crate) fn first_user_utterance(messages: &[cm::ChatMessage]) -> Option<(String, String)> {
    messages
        .iter()
        .find(|m| {
            m.role == Some(cm::MessageRole::User)
                && m.msg_type == Some(cm::MessageType::Text)
                && matches!(m.status, None | Some(cm::MessageStatus::Completed))
        })
        .and_then(|m| {
            let text = m.content.as_ref()?.to_text();
            Some((m.id.clone(), text))
        })
}

/// 本轮助手答复文本：**最后一条**已完成的助手 Text 节点。
pub(crate) fn last_assistant_text(messages: &[cm::ChatMessage]) -> Option<String> {
    messages
        .iter()
        .rev()
        .find(|m| {
            m.role == Some(cm::MessageRole::Assistant)
                && m.msg_type == Some(cm::MessageType::Text)
                && m.status == Some(cm::MessageStatus::Completed)
        })
        .and_then(|m| m.content.as_ref().map(|c| c.to_text()))
}

#[cfg(test)]
#[path = "v2_bridge.test.rs"]
mod tests;
