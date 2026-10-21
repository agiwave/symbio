//! v2 派生事实写入——承诺 / 任务表 / 熔断 + 记忆与学习。
//!
//! `turn_exec` 在轮末调用本模块的两个写方，把本轮的**派生副作用**落进同一份持久事实源
//! （<会话目录>/`v2-events.wal`，`paths::V2_WAL_FILE` 单源）。
//!
//! ## 为什么这两半是独立于「轮次事实」的
//!
//! 轮次事实（`u-{turn}` 开口 / 收束格）由 v2 运行器**原生**入格，运行器一处不缺。
//! 但下面这些运行器**一处都不写**——它们的出参在工具执行层（`Delegation` /
//! `TaskDeclaration` / 熔断理由）与读侧（召回视图 / 技能判定），由分发方经
//! `SessionDispatchPort::take_derived` 交回。不在这里补，代际立约（S08）/ 任务表（S7）/
//! 熔断（S9 §6 验收 2）/ 长期记忆（S06）/ 技能自我改进（S11）就整体失效。
//!
//! ## 锚点由调用方给，锚的**含义**不由调用方定
//!
//! `user_seq` 是本轮开口格的 seq，`anchor_id` 是事件号的词干。两条都由 `turn_exec`
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
//! 记忆侧另有顺序（编码[经验 + 判断] → 检索锚 → 巩固 → 技能观测 → 技能编译），
//! 见 [`record_learning`] 的文档——巩固要用**刷新后**的快照，它得看得见编码
//! 刚写下的那些。

use crate::symbio_core::{Budget, Entity, Event, Seq, Store, Verb, EVENT_MEMORY_CALIBRATED};
// 记录类型住 core 词汇表（[ADR-082](../../../../docs/010-adr/082.md "ADR-082") 裁决三 4/5 组）：
// 写侧（`note_*` 的门）随观察点留在工具执行层，读侧与落侧两头都从这里拿。
use crate::symbio_core::facts::{Delegation, TaskDeclaration};

/// 收束派生事实中的**承诺 / 任务表 / 熔断**半边（S08 §3 / S7 步 16–18 / S8 第 20 步）。
///
/// ## 为什么它与「轮次事实」分开成函数
///
/// 轮次事实（`u-{turn}` 开口 / 收束格）记的是「这一轮说了什么」——由 v2 运行器
/// 原生记账。但承诺 / 任务表 / 熔断是**本轮的派生事实**：数据来源都在工具执行层
/// （`Delegation` / `TaskDeclaration` / 熔断理由），运行器一处都不写。漏调本函数，
/// 代际立约（S08）、任务表（S7）、熔断（S9 §6 验收 2）就**静默全丢**
/// ——与记忆/学习同一个坑（见 [`record_learning`]）。
///
/// ## 锚点由调用方给，锚的**含义**不由调用方定
///
/// `user_seq` = 本轮 `user.message` 格的 seq（`u-{turn}` 开口格），`anchor_id` 是
/// 事件号的**词干**（`t{turn}`，一轮一个号）。两条都由 `turn_exec` 给，但
/// 「溯源锚必须是本轮的开口」这条不变（I2：承诺 / 任务 / 熔断溯源 100%）。
///
/// ## 失败口径（一条，三族共用）
///
/// 任一族入格失败 ⇒ **只记日志，不冒泡、不打断其余两族**。轮次已收束，派生事实
/// 失败不该把成功的一轮说成失败；而三族各自独立，一族写不进去不该连累另外两族的
/// 事实——它们之间没有数据依赖，只是共用一个锚点。
///
/// 此前承诺走的是另一条口径（`Err` 上抛），名义理由是「承诺是本轮的立约，签了就
/// 得算数」。但它上抛后**调用方也只记日志**，于是净行为与另两族相同；唯一的实际
/// 差别是 `?` 让函数**提前返回**——承诺写失败时，任务表与熔断事件跟着一起不写。
/// 那是 `?` 的副作用，不是设计意图。现在三条路都走同一个形状。
///
/// 熔断一轮只落**一格**：事件号按溯源锚定（`cb-{user_seq}`），两条就是同一个
/// 幂等键、第二条会被存储丢掉——那正是验收 2 要禁的「静默继续」，所以宁可在写
/// **之前**就只取第一条，而不是靠撞键兜底。逐工具的拒绝理由不在这里：它们已经在
/// 每条工具结果节点上（`Refused: …`），模型与用户都看得见；这里记的是**本轮被
/// 闸门拦过**这件事本身。
#[allow(clippy::too_many_arguments)]
pub(crate) fn record_derived(
    session_id: &str,
    store: &dyn Store<Event = Event>,
    snapshot: &[Event],
    turn: u64,
    user_seq: u64,
    principal: &str,
    to: &str,
    anchor_id: &str,
    delegations: &[Delegation],
    tasks: &[TaskDeclaration],
    breaks: &[&'static str],
) {
    // ── 承诺（04 §3.1 批⑧，S08 §3「加格子，不加机制」）──────────────────
    // 本轮代际立约随收束入格：立约与了结**同锚成对**，`check_all` 才看得见它们。
    // 承诺号带 `{anchor_id}` 前缀：tool_call id 只在一次模型响应内唯一，而 WAL 的
    // 幂等键是事件 id——跨轮复用同一个 id 会让第二次立约撞 `Duplicate`、把整轮
    // 收束拖失败。
    //
    // `to` = **来源主体**（ADR-067：从调用点常量变成数据）——本主体向「这条消息
    // 的出处」立约；缺省由取值点（`request_source_principal`）落本机用户。
    for d in delegations {
        let cid = format!("v2c-{anchor_id}-{}", d.id);
        for e in crate::symbio_core::commitment_events(
            &cid, principal, to, &d.promise, d.ok, &d.why, user_seq,
        ) {
            if let Err(e) = store.append(e) {
                crate::plugin_warn!(
                    "session",
                    "[v2] 承诺事件入格失败（会话 {session_id}，承诺 {cid}）：{e:?}"
                );
            }
        }
    }

    // ── 任务表（S7 步 16–18，04 §3.1 批⑨）────────────────────────────────
    // 与承诺**同锚**（`user_seq`）：任务是「这轮模型说该做什么」，出处是这一轮的
    // 开口。写方住 `task_events`，此处只负责把门推开——失败只记日志不冒泡。
    if let Err(why) = super::task_events::write(
        store,
        snapshot,
        tasks,
        turn,
        user_seq,
        principal,
        &format!("v2t-{anchor_id}"),
    ) {
        crate::plugin_warn!("session", "[task] 清单入格失败（会话 {session_id}）：{why}");
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
                "[v2] 熔断事件入格失败（会话 {session_id}）：{e:?}"
            );
        }
    }
}

/// 收束派生事实中的**淘汰回路**半边（S13 §3，T-073）：轮末按**刷新后**的声誉投影
/// 裁决「谁不再被授予 `assign.work`」，落成一条 `verdict.asserted`（可审计、可撤销
/// 的 grants 变更事件，`governance::GrantsOverlay` 的读侧半边）。
///
/// ## 为什么在轮末、为什么用刷新后的快照
///
/// 承诺（违约 ⇒ 声誉降）由 [`record_derived`] **同轮**落格——淘汰必须看得见本轮
/// 刚写下的那条违约，否则「委托失败 ⇒ 声誉 −1 ⇒ 收回派活权」要等到下一轮才发生，
/// 而那一轮里主体已经又派出去一轮了（判据滞后一个轮次 = 静默失效）。
/// 与巩固同一条纪律：口径是**完整史**（`range(0)`），且现读——不用调用方手里那份切片。
///
/// ## 判据的三个条件（缺一不淘汰）
///
/// 1. **有声誉条目**（立过约）——从没承诺过的主体没有可违约的东西，空条目
///    （score 0）不触发；
/// 2. **持 `assign.work`**（按**生产表**判，即登记行）——S13 §2 的字面机制只收回
///    这一项能力；仅持 `produce.artifact` 的对等主体（`agent:peer-b` 类）不在机制内
///    （它没有「派活」这个权威动作可收回）；
/// 3. **低于阈值且无既有淘汰 verdict**（后者经覆盖层判，last-wins）——同一条 verdict
///    不重复落格：事件号带溯源锚，重复写只会撞幂等键被静默丢掉，而那正是
///    「审计面少一条」的静默失效。
///
/// ## 结构性的诚实缺口：恢复不可能自动发生
///
/// 淘汰后唯一能涨声誉的路径是**成功委派**（`agent_run` 守约），而委派恰是被收回的
/// `assign.work` 那个动作——闸门（`tools/tool_executor` 的撤销闸）会拒掉它。于是
/// 恢复必须走**申诉 / 人工复位**（写一条 `change = "restore"` 的同格事件，机制半边
/// 在 `EliminationDecider::restoration_event` 与撤销回路单测里证过）。本步不造一个
/// 「假装能自动恢复」的回路；这条缺席已按**边界**登记在
/// [ADR-108](../../../../docs/010-adr/108.md "ADR-108")（含重开条件），任务层的缺口账
/// 在 [T-133](../../../../docs/009-task/133-多智能体社会.md "T-133-多智能体社会") §现状 ①。
///
/// ## 失败只记日志不冒泡
///
/// 淘汰是附加事实：写不进去不该把成功的一轮说成失败（与记忆三段同一条口径）。
pub(crate) fn record_elimination(store: &dyn Store<Event = Event>, turn: u64, user_seq: u64) {
    // 刷新后的快照：看得见 `record_derived` 同轮写下的承诺（见函数文档）。
    let snapshot = store.range(Seq::new(0));
    let view = crate::symbio_core::reputation()
        .apply(&snapshot, i64::MAX, Budget::generous())
        .value;
    // 覆盖层：判「有没有既有淘汰」——不复述载荷 ↔ 变更的映射（它在 core 的
    // `EliminationDecider::verdict_grants_changes`，单点定义）。
    let overlay = crate::symbio_core::GrantsOverlay::fold(
        crate::symbio_core::EliminationDecider::verdict_grants_changes(&snapshot),
    );
    let decider = crate::symbio_core::EliminationDecider;
    // 写侧闸（与 `inbox::commit_preemption` 的 `judge.intent` 检查同一条纪律）：
    // 落裁决事实需要仲裁者那一行持 `assert.verification`——缺能力 ⇒ 一条都不写
    // （fail-closed：没有资格的判定不该留下事实）。
    let spec = decider.spec();
    if !spec.can_assert_verification() {
        crate::plugin_warn!(
            "session",
            "[v2] 仲裁者缺 assert.verification，本轮淘汰判定不落格（turn={turn}）"
        );
        return;
    }
    let matrix = crate::symbio_core::authz::production_matrix();
    for (principal, entry) in &view.by_principal {
        // 条件 2：持 assign.work 的**部署**主体（淘汰面只认部署表：绑定登记
        // （[ADR-100]）是运行期身份事实，不构成「部署授予过 assign.work」——
        // 未部署的 `agent:<id>` 不在表里 ⇒ 不淘汰，边界同函数文档）。
        if !matrix.deployed_can_write_name(principal, crate::symbio_core::authz::CAP_ASSIGN_WORK) {
            continue;
        }
        if !decider.should_eliminate(entry) {
            continue;
        }
        // 条件 3：已有生效撤销 ⇒ 不重复落格（审计面不缺、幂等键不撞）。
        if overlay.is_revoked(principal, crate::symbio_core::authz::CAP_ASSIGN_WORK) {
            continue;
        }
        let ev = decider.elimination_event(principal, entry.score, user_seq);
        if let Err(e) = store.append(ev) {
            crate::plugin_warn!(
                "session",
                "[v2] 淘汰裁决入格失败（turn={turn}，subject={principal}）：{e:?}"
            );
        } else {
            crate::plugin_info!(
                "session",
                "[v2] 落格 {}：主体 {principal} 收回 assign.work（声誉分 {} 低于阈值 {}）",
                crate::symbio_core::EVENT_VERDICT_ASSERTED,
                entry.score,
                crate::symbio_core::EliminationDecider::THRESHOLD,
            );
        }
    }
}

/// 收束派生事实中的**记忆与学习**半边（步 11–13 + 步 22）。
///
/// ## 为什么它与「轮次事实」是两件事
///
/// 轮次事实（`u-{turn}` 开口 / 收束格）记的是**这一轮说了什么、答应了什么**，
/// 由 v2 运行器原生记账。但记忆与学习**不是轮次事实**，而是本轮的**派生副作用**：
/// `turn_exec` 一处都不写（它只写轮次事实）。漏调本函数，「编码 / 检索锚 / 巩固 /
/// 技能观测 / 技能编译」会**静默全丢**——S06 的长期记忆与 S11 的自我改进随之失效。
///
/// ## 锚点由调用方给，锚的**含义**不由调用方定
///
/// `user_seq` = 本轮 `user.message` 格的 seq（`u-{turn}` 开口格），`anchor_id` 是
/// 幂等键的**词干**（`t{turn}`，一轮一个号）。两条都由 `turn_exec` 给，但
/// 「溯源锚必须是本轮的开口」这条不变（I2：记忆溯源覆盖 100%）。
///
/// ## 失败只记日志不冒泡
///
/// 记忆是本轮的附加事实，它失败不该被说成整轮入格的失败。
///
/// ## 顺序是判据
///
/// 1. 编码（步 11 经验 + 步 11b 判断）先写——本轮新记忆要能进本轮的巩固；
/// 2. 检索事实（步 12）与它平级：溯源锚同为本轮开口；
/// 3. 巩固（步 13）最后，且用**刷新后的**快照——它要看得见 1 刚写下的那条；
/// 4. 技能观测在前、编译在后：观测记的是**本轮判没判、判成什么**，编译产出的是
///    **下一轮才可能被用的技能**。
#[allow(clippy::too_many_arguments)]
pub(crate) fn record_learning(
    session_id: &str,
    store: &dyn Store<Event = Event>,
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
            "[memory] {step} 失败（会话 {session_id}）：{why}"
        );
    };

    // ── 步 11 编码（S5，04 §3.1 批⑦）────────────────────────────────────
    // 经验（用户发言）= **人的认知**，可见域 `shared`——智能体读、本人写
    // （T-130 §3 的 grants 行语义，T-094 ①：内容侧产地）。单归属主部署下
    // 「这个人」= 本机用户，认知可供本部署的矩阵内主体借用（SC-010 判据 3）。
    if let Err(why) = super::memory_events::encode(
        store,
        snapshot,
        turn,
        user_seq,
        now,
        super::memory_events::EncodeContent::new(
            user_text,
            super::memory_events::MEMORY_TAG,
            format!("v2m-{anchor_id}"),
        )
        .with_vis_scope(crate::symbio_core::VisScope::Shared),
    ) {
        remember("步 11 编码", why);
    }
    // ── 步 11b 判断编码（T-085 批④，S10 七类认知内容的「判断」写方）────────
    // 本轮**成功收束**的正文（`response`）→ 一条 tag="判断" 的记忆，与用户发言
    // 那条同文去重 / 同截断 / 同溯源。兜底收束不记（`response = None`）——固化
    // 失败套路等于把没走通的路写成记忆（与技能编译同一条纪律）。
    // 判断 = 智能体自己的收束记录，可见域 `thread_private`（C10 缺省）——
    // 它是会话私有的工作记忆，不是可跨主体借用的认知（T-094 ①）。
    if let Some(response) = response {
        if let Err(why) = super::memory_events::encode(
            store,
            snapshot,
            turn,
            user_seq,
            now,
            super::memory_events::EncodeContent::new(
                response,
                super::memory_events::JUDGMENT_TAG,
                format!("v2j-{anchor_id}"),
            ),
        ) {
            remember("步 11b 判断编码", why);
        }
    }
    // ── 步 12 检索入格：本轮召回视图 → 一条 `memory.recalled` ─────────────
    if let Some(view) = recalled {
        if let Err(why) =
            super::memory_events::record_recalled(store, view, user_seq, principal, now)
        {
            remember("步 12 检索入格", why);
        }
    }
    // ── 步 13 巩固（用**刷新后**的快照）──────────────────────────────────
    // 巩固是**累计口径**（S2.7-b2）：它要把
    // doc-link-allow D-004: 「曾经记过什么」是记忆族的语义问句，非版本变更史
    // 「曾经记过什么」收成一族记忆。`range(0)` 给的是完整史，被快照取代而移入归档段
    // 的那些记忆也在里面 ⇒ 压缩前的记忆不会被**静默**丢掉。
    if let Err(why) = super::memory_events::consolidate(store, &store.range(Seq::new(0)), turn, now)
    {
        remember("步 13 巩固", why);
    }

    // ── 步 22 · 技能路由观测（S11，04 §3.1 批⑪ 子批 B）───────────────────
    //
    // 观测只带数据 `{ skill_id, fallback }`——`calibration` 投影的契约就是这两个
    // 字段（S11 §3「不加新格子」曾把它们复用在 `memory.recalled` 上；T-085 批①
    // 把它们搬进**自己的格子** `memory.calibrated`，读侧改按 `kind` 过滤——
    // 复用两张格子靠载荷字段判别，读侧就得赌「别的格子恰好不带这两个字段」）。
    // **没有这一步，回退永远不会发生**：零使用时置信度按 1.0 算，技能再错也一直
    // 被用，S11 §5 的「一直用错技能而自己不知道」当场应验。
    for (n, (skill_id, fallback)) in skill_obs.iter().enumerate() {
        let ev = Event::pending(
            format!("v2s-{anchor_id}-{n}"),
            EVENT_MEMORY_CALIBRATED,
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
    // 只编译**成功**收束（`response` = 收束格 `chat.assistant.final` 的正文）——兜底说明这条路
    // 自己没走通，固化它等于把失败写成套路。开关 `skill_compile_enabled` 默认 off
    // （S11 §4 平凡值「不编译，只检索」）：关着时读侧逐条跳过，整条
    // 编译 → 校准 → 回退链路原地待命、不产生任何事实。
    if skill_compile {
        if let Some(response) = response {
            if let Err(why) = super::skill_events::compile(
                store, snapshot, turn, user_seq, user_text, response, now,
            ) {
                remember("步 22 技能编译", why);
            }
        }
    }
}

#[cfg(test)]
#[path = "derived_facts.test.rs"]
mod tests;
