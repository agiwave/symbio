//! v2 派生事实写入——效果（承诺 / 任务表 / 文件变更…）+ 熔断 + 记忆。
//!
//! `turn_exec` 在轮末调用本模块的两个写方，把本轮的**派生副作用**落进同一份持久事实源
//! （<会话目录>/`v2-events.wal`，`paths::V2_WAL_FILE` 单源）。
//!
//! ## 为什么这两半是独立于「轮次事实」的
//!
//! 轮次事实（`u-{turn}` 开口 / 收束格）由 v2 运行器**原生**入格，运行器一处不缺。
//! 但下面这些运行器**一处都不写**——它们的出参在工具执行层（`EffectInput` /
//! 熔断理由）与读侧（召回视图），由分发方经 `SessionDispatchPort::take_derived`
//! 交回。不在这里补，代际立约/ 任务表/ 熔断（DE-039 §6 验收 2）/ 长期记忆就整体失效。
//! 技能编译与回退观测不在这里：它们是技能能力包的收束贡献（D-250 / D-262）。
//!
//! ## 效果只有一条通路
//!
//! 承诺、任务表、文件变更**都不在本模块**：它们各自是能力包里的一个 `ToolBehavior`
//! （`symbio-plugin-agent` / `symbio-plugin-task` / `symbio-plugin-vdfs`），本模块只把
//! `EffectInput` 补上宿主面、问出草稿、交给唯一写入口。加一个新效果不需要动这里
//! （DE-025 D-207）。
//!
//! ## 锚点由调用方给，锚的**含义**不由调用方定
//!
//! `user_seq` 是本轮开口格的 seq，由 `turn_exec` 给；事件号的词干（`t{turn}`）不在
//! 这里生成——它是 [`symbio_plugin_api::effects::EffectCall::anchor`] 的活，由行为自己
//! 取，因为「这笔效果属于哪一轮」是宿主面的一部分（DE-025 D-213）。
//!
//! ## 失败口径（两种，不合并）
//!
//! - **效果 / 熔断 / 记忆三段**：失败只记日志不冒泡（附加事实）。轮次已收束，
//!   附加事实失败不该把成功的一轮说成失败。
//!

//!
//! ## 落笔顺序是判据，不是排版
//!
//! 收束发言先落、派生事实随后：`PreemptionDecider` 的「final 已发出 → 排队」边界比的是
//! 「任务开格之后有没有收束发言」。若 final 落在 `task.opened` 之后，宣告那个任务的发言
//! 自己就满足了该条件——抢占在生产里一次都不会发生（只有 fallback 收束例外），判定被
//! 接入却永远走不到 `Suspend`。
//!
//! 记忆侧另有顺序（编码[经验 + 判断] → 检索锚 → 巩固），见 [`record_learning`]
//! 的文档——巩固要用**刷新后**的快照，它得看得见编码刚写下的那些。

use crate::control_decl::{COMMITMENT_VOCAB, VERDICT};
use symbio_core::{actor_can_assert_verification, Event, Seq, Store};
use symbio_plugin_api::effects::EffectInput;

/// 收束派生事实中的**效果 / 熔断**半边（DE-038 §3）。
///
/// ## 为什么它与「轮次事实」分开成函数
///
/// 轮次事实（`u-{turn}` 开口 / 收束格）记的是「这一轮说了什么」——由 v2 运行器
/// 原生记账。但效果与熔断是**本轮的派生事实**：数据来源都在工具执行层
/// （`EffectInput` / 熔断理由），运行器一处都不写。漏调本函数，
/// 代际立约、任务表、熔断（DE-039 §6 验收 2）就**静默全丢**
/// ——与记忆/学习同一个坑（见 [`record_learning`]）。
///
/// ## 锚点由调用方给，锚的**含义**不由调用方定
///
/// `user_seq` = 本轮 `user.message` 格的 seq（`u-{turn}` 开口格），由 `turn_exec` 给，
/// 「溯源锚必须是本轮的开口」这条不变（I2：承诺 / 任务 / 熔断溯源 100%）。
///
/// ## 失败口径（一条，两族共用）
///
/// 任一族入格失败 ⇒ **只记日志，不冒泡、不打断另一族**。轮次已收束，派生事实
/// 失败不该把成功的一轮说成失败；而两族各自独立，一族写不进去不该连累另一族的
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
    effects: &[EffectInput],
    breaks: &[&'static str],
) {
    // ── 效果（DE-025 D-207）──────────────────────────────────────────────
    // 工具执行层交出的是**纯值**（这次调用发生了什么），这里补上宿主面（词干 /
    // 溯源 / 两个主体 / 两片只读历史）再问能力包的行为要草稿，最后经唯一写入口落格。
    //
    // 三件事各自只有一份：**判断**（这次调用算不算立约、算不算一次任务声明）在
    // 能力包的行为实现里，**成格形状**（几格、载荷长什么样）在该判断种类的成格函数
    // 里，**写**在 `Store::append`。本处只负责把三者按顺序接起来——它不认识任何
    // 工具名，也不认识任何效果名。
    //
    // `to` = **来源主体**（D-067：从调用点常量变成数据）——本主体向「这条消息
    // 的出处」立约；缺省由取值点（`request_source_principal`）落本机用户。
    //
    // 失败只记日志不冒泡：轮次已收束，派生事实失败不该把成功的一轮说成失败
    // （与熔断 / 记忆三段同一条口径）。
    //
    // `applied` 是**本轮已落格的草稿**：同轮的后一个效果要看得见前一个。少了它，
    // 同一轮里两次 `todo_write` 会各自从轮初快照出发，把第一条刚开的格再开一次。
    //
    // 词干 `anchor` 在这里构造（`t{turn}`）——它是宿主的命名法，能力只拿它拼幂等键
    // （DE-026 D-252）；轮号也不交进端口，落格时由本处盖章。
    let anchor = format!("t{turn}");
    let mut applied: Vec<Event> = Vec::new();
    for input in effects {
        let env = symbio_plugin_api::effects::EffectEnv {
            anchor: &anchor,
            user_seq,
            principal,
            source: to,
            snapshot,
            applied: &applied,
        };
        for draft in symbio_plugin_api::apply_effects(&input.call(&env)) {
            let event_id = draft.key.clone();
            let mut event = draft.into_event(principal);
            event.turn = turn;
            match store.append(event.clone()) {
                Ok(_) => applied.push(event),
                Err(e) => symbio_plugin_api::plugin_warn!(
                    "session",
                    "[v2] 效果入格失败（会话 {session_id}，事件 {event_id}）：{e:?}"
                ),
            }
        }
    }

    // ── 熔断────────────────────────────────────
    // 与承诺 / 任务**同锚**（`user_seq`）：熔断说的是「这一轮不允许做」，出处同样
    // 是这一轮的开口。写方住本插件的闸门判据（`tools::dispatch::gates`，D-190），
    // 此处只负责把门推开。
    if let Some(&reason) = breaks.first() {
        if let Err(e) = store.append(super::tools::CircuitBreaker.break_event(reason, user_seq)) {
            symbio_plugin_api::plugin_warn!(
                "session",
                "[v2] 熔断事件入格失败（会话 {session_id}）：{e:?}"
            );
        }
    }
}

/// 收束派生事实中的**淘汰回路**半边（DE-043 §3）：轮末按**刷新后**的声誉投影
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
/// 2. **持 `assign.work`**（按**生产表**判，即登记行）——DE-043 §2 的字面机制只收回
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
/// [D-108](../../../docs/003-design/002-架构总览.md#d-108-淘汰是单向门恢复的生产触发器缺席记为边界自动恢复在本判据下不可能 "DE-002-架构总览")（含重开条件），任务层的缺口账
/// 在 [DE-043](../../../docs/003-design/043-多智能体社会.md "DE-043-多智能体社会") §现状 ①。
///
/// ## 失败只记日志不冒泡
///
/// 淘汰是附加事实：写不进去不该把成功的一轮说成失败（与记忆三段同一条口径）。
pub(crate) fn record_elimination(store: &dyn Store<Event = Event>, turn: u64, user_seq: u64) {
    // 刷新后的快照：看得见 `record_derived` 同轮写下的承诺（见函数文档）。
    let snapshot = store.range(Seq::new(0));
    let view = symbio_core::reputation(COMMITMENT_VOCAB)
        .read_all(&snapshot)
        .value;
    // 覆盖层：判「有没有既有淘汰」——不复述载荷 ↔ 变更的映射（它在 core 的
    // `EliminationDecider::verdict_grants_changes`，单点定义）。
    let overlay = symbio_core::GrantsOverlay::fold(
        symbio_core::EliminationDecider::verdict_grants_changes(&snapshot),
    );
    let decider = symbio_core::EliminationDecider;
    // 写侧闸（与 `inbox::commit_preemption` 的 `judge.intent` 检查同一条纪律）：
    // 落裁决事实需要仲裁者那一行持 `assert.verification`——缺能力 ⇒ 一条都不写
    // （fail-closed：没有资格的判定不该留下事实）。
    let spec = decider.spec();
    if !actor_can_assert_verification(&spec) {
        symbio_plugin_api::plugin_warn!(
            "session",
            "[v2] 仲裁者缺 assert.verification，本轮淘汰判定不落格（turn={turn}）"
        );
        return;
    }
    let matrix = symbio_core::production_matrix();
    for (principal, entry) in &view.by_principal {
        // 条件 2：持 assign.work 的**部署**主体（淘汰面只认部署表：绑定登记
        // （[D-100]）是运行期身份事实，不构成「部署授予过 assign.work」——
        // 未部署的 `agent:<id>` 不在表里 ⇒ 不淘汰，边界同函数文档）。
        if !matrix.deployed_can_write_name(principal, symbio_core::CAP_ASSIGN_WORK) {
            continue;
        }
        if !decider.should_eliminate(entry) {
            continue;
        }
        // 条件 3：已有生效撤销 ⇒ 不重复落格（审计面不缺、幂等键不撞）。
        if overlay.is_revoked(principal, symbio_core::CAP_ASSIGN_WORK) {
            continue;
        }
        let ev = decider.elimination_event(VERDICT, principal, entry.score, user_seq);
        if let Err(e) = store.append(ev) {
            symbio_plugin_api::plugin_warn!(
                "session",
                "[v2] 淘汰裁决入格失败（turn={turn}，subject={principal}）：{e:?}"
            );
        } else {
            symbio_plugin_api::plugin_info!(
                "session",
                "[v2] 落格 {}：主体 {principal} 收回 assign.work（声誉分 {} 低于阈值 {}）",
                crate::control_decl::EVENT_VERDICT_ASSERTED,
                entry.score,
                symbio_core::EliminationDecider::THRESHOLD,
            );
        }
    }
}

/// 收束派生事实中的**记忆与学习**半边（编码 / 检索 / 巩固 + 技能编译）。
///
/// ## 为什么它与「轮次事实」是两件事
///
/// 轮次事实（`u-{turn}` 开口 / 收束格）记的是**这一轮说了什么、答应了什么**，
/// 由 v2 运行器原生记账。但记忆与学习**不是轮次事实**，而是本轮的**派生副作用**：
/// `turn_exec` 一处都不写（它只写轮次事实）。漏调本函数，「编码 / 检索锚 / 巩固 /
/// 技能观测 / 技能编译」会**静默全丢**——长期记忆与自我改进随之失效。
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
/// 1. 编码（经验 + 判断）先写——本轮新记忆要能进本轮的巩固；
/// 2. 检索事实与它平级：溯源锚同为本轮开口；
/// 3. 巩固最后，且用**刷新后的**快照——它要看得见 1 刚写下的那条。
///
/// 技能观测与编译**不在本函数**：它们是技能能力包的收束贡献（`Contributor`，
/// D-250 / D-262），由 `settle` 在记忆之后按 `prio` 逐个询问。
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
    recalled: Option<&super::recall::RecallView>,
    now: i64,
) {
    let remember = |step: &str, why: String| {
        symbio_plugin_api::plugin_warn!(
            "session",
            "[memory] {step} 失败（会话 {session_id}）：{why}"
        );
    };

    // ── 编码────────────────────────────────────
    // 经验（用户发言）= **人的认知**，可见域 `shared`——智能体读、本人写
    // （§3 的 grants 行语义， ①：内容侧产地）。单归属主部署下
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
        .with_vis_scope(symbio_core::VisScope::Shared),
    ) {
        remember("编码", why);
    }
    // ── 判断编码（七类认知内容的「判断」写方）────────
    // 本轮**成功收束**的正文（`response`）→ 一条 tag="判断" 的记忆，与用户发言
    // 那条同文去重 / 同截断 / 同溯源。兜底收束不记（`response = None`）——固化
    // 失败套路等于把没走通的路写成记忆（与技能编译同一条纪律）。
    // 判断 = 智能体自己的收束记录，可见域 `thread_private`（C10 缺省）——
    // 它是会话私有的工作记忆，不是可跨主体借用的认知（①）。
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
            remember("判断编码", why);
        }
    }
    // ── 检索入格：本轮召回视图 → 一条 `memory.recalled` ─────────────
    if let Some(view) = recalled {
        if let Err(why) =
            super::memory_events::record_recalled(store, view, user_seq, principal, now)
        {
            remember("检索入格", why);
        }
    }
    // ── 巩固（用**刷新后**的快照）──────────────────────────────────
    // 巩固是**累计口径**：它要把
    // doc-link-allow D-004: 「曾经记过什么」是记忆族的语义问句，非版本变更史
    // 「曾经记过什么」收成一族记忆。`range(0)` 给的是完整史，被快照取代而移入归档段
    // 的那些记忆也在里面 ⇒ 压缩前的记忆不会被**静默**丢掉。
    if let Err(why) = super::memory_events::consolidate(store, &store.range(Seq::new(0)), turn, now)
    {
        remember("巩固", why);
    }
}

#[cfg(test)]
#[path = "derived_facts.test.rs"]
mod tests;
