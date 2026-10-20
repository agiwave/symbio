//! 反射档（S11 快路）——技能命中即收束，一次模型调用都不发生（T-179 · B3 搬移）。
//!
//! 从 `turn_runner.rs` 逐字搬入；本文件只承载 `TurnRunner::run_reflex`。

use super::*;

impl TurnRunner {
    /// **反射档的一轮**（[roadmap/S11 §2–§3](../../../../../docs/009-task/131-技能编译与自我改进.md "T-131-技能编译与自我改进")）：
    /// 技能命中 ⇒ 以技能正文收束，**一次模型调用都不发生**。
    ///
    /// ## 为什么是独立入口，而不是给 [`Self::run_with_tools`] 塞一个"不调模型"的适配器
    ///
    /// 时延闸门的全部表达在**签名**上（[`crate::symbio_core::adapters`] 模块文档）：
    /// `run_with_tools` 要 `&FullModel`，而反射档只能签出 `RuleOnly`。若为了复用那条
    /// 路径给反射档发一张 `FullModel`，「反射档不得调用模型」就从**构造期约束**退回成
    /// 一句声明——正是 `docs/004-plan/verify/latency_gate.rs` 要取代的那种形态（那里
    /// `assemble(Reflex)` 的产物**结构上就没有 LLM 字段**）。本函数是它的落地：
    /// **没有 `llm` 形参**，于是"反射档调模型"不是被检测到，而是写不出来。
    ///
    /// ## 为什么入参里没有 [`TurnInput`]
    ///
    /// `TurnInput` 的 `tier` / `window_turns` / `resume` 三项都服务**生成**：prompt 从
    /// 哪接、历史看多远、按哪一档装配。反射档不生成、不读 prompt，三项一个都用不上；
    /// 带上它们只会让「反射档**续写**一轮」这种语义上说不通的调用在类型上变得可以写
    /// （续写轮要跑的是刚被批准的那次工具，拿技能正文把它顶掉等于把用户批准的动作丢掉）。
    /// 故本函数只收它真正需要的：轮号、主体、用户发言、产物、出口。
    ///
    /// ## 落格与 [`Self::run_with_tools`] 同一份纪律
    ///
    /// `u-{turn}`（开轮）→ `f-{turn}`（收束，`produced_by` 指向开轮格、`cost_ms` 实测）。
    /// 开轮载荷的 `tier` 恒为 `reflex`——档位不是调用方填的字符串，而是**令牌类型**所
    /// 证明的那一档（`RuleOnly` 与 [`LatencyTier::Reflex`] 是同一个事实的两种写法，
    /// 另写一处必然漂移）。收束载荷的 `model` 也记 `reflex`：反射档没有模型，但"这段话
    /// 是谁产的"仍要可观测，空着会让命中轮与普通轮在事件面上无从区分；**命中哪一条技能**
    /// 则由触发串与技能事件唯一确定，不在这里复述一遍（能算出来的不占字段）。
    ///
    /// 产物经 `sink` 上线——与真实路径**同一个出口**。不上线，收束节点在前端就永远建不
    /// 起来（`chat_loop` 用 `response_text_child_id` 定格本轮正文），表现为"答了但看不见"。
    ///
    /// ## 为什么参数多到要 `allow`
    ///
    /// 八个参数**各自是一个不同的端口**（事实源 / 令牌 / 轮号 / 主体 / 用户发言 / 产物 /
    /// 出口 / 阶段事实），与 [`Self::run_with_tools`] 同款理由：打包成一个结构体只是把
    /// "八个端口"改名叫"一个结构体 + 八个字段"，调用方仍要逐个填。
    #[allow(clippy::too_many_arguments)]
    pub async fn run_reflex<S>(
        &self,
        store: &S,
        _tok: &crate::symbio_core::RuleOnly,
        turn: u64,
        actor: &ActorSpec,
        utterance: &str,
        text: &str,
        sink: std::sync::Arc<dyn crate::symbio_core::DeltaSink>,
        stages: &[super::super::stage_events::StageFact],
    ) -> Result<TurnOutcome, crate::symbio_core::AppendError>
    where
        S: Store<Event = Event> + ?Sized,
    {
        let started = std::time::Instant::now();
        let user_seq = store
            .append(
                Event::pending(
                    format!("u-{turn}"),
                    EVENT_USER_MESSAGE,
                    Entity::Turn,
                    Verb::Opened,
                    turn,
                    "user",
                )
                .with_payload(serde_json::json!({
                    "text": utterance,
                    "tier": LatencyTier::Reflex.name(),
                    // 规格派生的预算数值（T-082 批①），与深度档开轮格同一形状：
                    // 读方 `invariants::declared_budget_ms` 优先读它。
                    "budget_ms": actor.budget_ms,
                })),
            )
            .map(|seq| seq.value())?;
        // 一次性阶段（判决 / 召回 / 压缩）：反射档不经过 `prepare_turn`，但同样在
        // 用户格之后落（I2 的溯源锚）；此处的 `done` 同样让本轮后续不再问这些格。
        super::super::stage_events::land(store, turn, user_seq, &actor.principal, stages)?;
        sink.on_delta(text);
        let cost_ms = started.elapsed().as_millis() as u64;
        if !closure_granted(actor, ClosureKind::Speech) {
            // 与深度档同一条写侧闸（见 [`closure_granted`]）：拒绝 ⇒ 不落收束格。
            crate::plugin_warn!(
                "actors",
                "[v2] 反射档收束被授权拒绝（{} 缺 reply.first），本轮不入格（turn={turn}）",
                actor.principal
            );
            return Ok(TurnOutcome {
                turn,
                text: text.to_string(),
                cost_ms,
                usage: None,
                fell_back: false,
                aborted: false,
                awaits_user: false,
                closure_denied: true,
                finish: crate::symbio_core::ModelFinishReason::Stop,
            });
        }
        store.append(
            Event::pending(
                format!("f-{turn}"),
                EVENT_ASSISTANT_FINAL,
                Entity::Turn,
                Verb::Closed,
                turn,
                &actor.principal,
            )
            .with_produced_by(user_seq)
            .with_cost_ms(cost_ms)
            // 与深度档同一个产出形状（`produced_payload`）+ 状态字段：反射档无模型、
            // 无流式节点，`turn_id` 记空串（重建侧按事件 id 稳定合成）、取数口不挂。
            // 载荷形状不另写一份——重建只按一个形状读。
            .with_payload(with_status(
                produced_payload(
                    text,
                    LatencyTier::Reflex.name(),
                    "",
                    &StreamNodeFacts::default(),
                    &[],
                    &[],
                ),
                "completed",
            )),
        )?;
        Ok(TurnOutcome {
            turn,
            text: text.to_string(),
            cost_ms,
            usage: None,
            fell_back: false,
            aborted: false,
            awaits_user: false,
            closure_denied: false,
            // 反射档没有模型轮，长度戳判据对它无意义（技能正文一次写完）。
            finish: crate::symbio_core::ModelFinishReason::Stop,
        })
    }
}
