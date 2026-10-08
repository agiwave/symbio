//! v2 派生事实写入——承诺 / 任务表 / 熔断 + 记忆与学习。
//!
//! `v2_exec` 在轮末调用本模块的两个写方，把本轮的**派生副作用**落进同一份持久事实源
//! （<会话目录>/`v2-events.wal`，`paths::V2_WAL_FILE` 单源）。
//!
//! ## 为什么这两半是独立于「轮次事实」的
//!
//! 轮次事实（`u-{turn}` 开口 / 收束格）由 v2 运行器**原生**入格，运行器一处不缺。
//! 但下面这些运行器**一处都不写**——它们的出参在工具执行层（`Delegation` /
//! `TaskDeclaration` / 熔断理由）与读侧（召回视图 / 技能判定），由分发方经
//! `SessionDispatchPort::take_derived` 交回。不在这里补，代际立约（S08）/ 任务表（S7）/
//! 熔断（S9 §6 验收 2）/ 长期记忆（S06）/ 技能自我改进（S11）就整体失效，而档位名还自称
//! 「整体切换」。
//!
//! ## 锚点由调用方给，锚的**含义**不由调用方定
//!
//! `user_seq` 是本轮开口格的 seq，`anchor_id` 是事件号的词干。两条都由 `v2_exec`
//! 给（`t{turn}` 一族）。
//!
//! ## 失败口径（两种，不合并）
//!
//! - **承诺**：入格失败 ⇒ **上抛** `Err`——承诺是本轮的立约，签了就得算数；
//! - **任务表 / 熔断 / 记忆三段**：失败只记日志不冒泡（附加事实）。轮次已收束，
//!   附加事实失败不该把成功的一轮说成失败。
//!
//! ## 落笔顺序是判据，不是排版
//!
//! 收束发言先落、派生事实随后：`PreemptionDecider` 的「final 已发出 → 排队」边界比的是
//! 「任务开格之后有没有收束发言」。若 final 落在 `task.opened` 之后，宣告那个任务的发言
//! 自己就满足了该条件——抢占在生产里一次都不会发生（只有 fallback 收束例外），判定被
//! 接入却永远走不到 `Suspend`。
//!
//! 记忆侧另有顺序（编码 → 检索锚 → 巩固 → 技能观测 → 技能编译），见 [`record_learning`]
//! 的文档——巩固要用**刷新后**的快照，它得看得见编码刚写下的那条。

use crate::symbio_core::{Entity, Event, EventWalStore, Seq, Store, Verb, EVENT_MEMORY_RECALLED};

/// 收束派生事实中的**承诺 / 任务表 / 熔断**半边（S08 §3 / S7 步 16–18 / S8 第 20 步）。
///
/// ## 为什么它与「轮次事实」分开成函数
///
/// 轮次事实（`v2u-*` 用户格 / `v2f-*` 收束格）记的是「这一轮说了什么」——`full` 档
/// 由 v2 运行器原生记账，`chat_loop` 以 `TurnState::v2_executed` 把整段 `record` 拦下。
/// 但承诺 / 任务表 / 熔断是**本轮的派生事实**：数据来源都在工具执行层
/// （`Delegation` / `TaskDeclaration` / 熔断理由），运行器一处都不写。拦下轮次事实时
/// 若把它们一起拦掉，`full` 档的代际立约（S08）、任务表（S7）、熔断（S9 §6 验收 2）
/// 就**静默全丢**——与记忆/学习同一个坑（见 [`record_learning`]）。
///
/// 两档调的是**同一个**本函数，差别只在锚点（见下）——不是两份实现。
///
/// ## 锚点由调用方给，锚的**含义**不由调用方定
///
/// `user_seq` = 本轮 `user.message` 格的 seq。两档的锚点是**不同的事件**
/// （bridge = 转写写的 `v2u-*` 格；full = v2 原生写的 `u-{turn}` 格），但
/// 「溯源锚必须是本轮的开口」这条不变（I2：承诺 / 任务 / 熔断溯源 100%）。
/// 同理 `anchor_id` 是事件号的**词干**：bridge 用 `{v1 消息 id}-a{attempt}`
/// （同一句 v1 消息重试各成一格），full 用 `t{turn}`（v2 侧一轮一个号）。
///
/// ## 失败口径（承袭原实现，两种）
///
/// - **承诺**：入格失败 ⇒ **上抛**（`Err`）。bridge 档由 `record_to_wal` 的 `?`
///   接住（顶层 `record` 只记日志），full 档由调用方**只记日志不冒泡**——轮次已
///   收束，派生事实失败不该把成功的一轮说成失败；
/// - **任务表 / 熔断**：失败只记日志不冒泡（与记忆三段同一条口径：附加事实）。
///
/// 熔断一轮只落**一格**：事件号按溯源锚定（`cb-{user_seq}`），两条就是同一个
/// 幂等键、第二条会被存储丢掉——那正是验收 2 要禁的「静默继续」，所以宁可在写
/// **之前**就只取第一条，而不是靠撞键兜底。逐工具的拒绝理由不在这里：它们已经在
/// 每条工具结果节点上（`Refused: …`），模型与用户都看得见；这里记的是**本轮被
/// 闸门拦过**这件事本身。
#[allow(clippy::too_many_arguments)]
pub(crate) fn record_derived(
    wal: &std::path::Path,
    store: &EventWalStore,
    snapshot: &[Event],
    turn: u64,
    user_seq: u64,
    principal: &str,
    anchor_id: &str,
    delegations: &[super::tools::Delegation],
    tasks: &[super::tools::TaskDeclaration],
    breaks: &[&'static str],
) -> Result<(), String> {
    // ── 承诺（04 §3.1 批⑧，S08 §3「加格子，不加机制」）──────────────────
    // 本轮代际立约随收束入格：立约与了结**同锚成对**，`check_all` 才看得见它们。
    // 承诺号带 `{anchor_id}` 前缀：tool_call id 只在一次模型响应内唯一，而 WAL 的
    // 幂等键是事件 id——跨轮复用同一个 id 会让第二次立约撞 `Duplicate`、把整轮
    // 转写拖失败。
    for d in delegations {
        let cid = format!("v2c-{anchor_id}-{}", d.id);
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
    // 与承诺**同锚**（`user_seq`）：任务是「这轮模型说该做什么」，出处是这一轮的
    // 开口。写方住 `v2_tasks`，此处只负责把门推开——失败只记日志不冒泡。
    if let Err(why) = super::v2_tasks::write(
        store,
        snapshot,
        tasks,
        turn,
        user_seq,
        principal,
        &format!("v2t-{anchor_id}"),
    ) {
        crate::plugin_warn!("session", "[task] 清单入格失败（{}）：{why}", wal.display());
    }

    // ── 熔断（S8 第 20 步，04 §3.1 批⑩）────────────────────────────────────
    // 与承诺 / 任务**同锚**（`user_seq`）：熔断说的是「这一轮不允许做」，出处同样
    // 是这一轮的开口。写方住 core 的 `CircuitBreaker`，此处只负责把门推开。
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

#[cfg(test)]
#[path = "v2_facts.test.rs"]
mod tests;
