//! `turn_runner` 单测 —— 随运行器一起从 `symbio_core::actors` 下沉（2026-10-09）。
//!
//! 三段：`turn_runner_tests`（单轮闭环 / WAL 持久 / 兜底 / 授权闸）、
//! `tool_round_tests`（工具轮：注入、前缀、窗口、产物、待办、恢复）、
//! `reflex_turn_tests`（反射档落格）。
//!
//! 另附 `run` / `run_streaming` / `drive_with_tools` 三个**测试专用**入口：生产入口是
//! `TurnRunner::begin` 与派发器 `turn_exec::drive`（`turn_exec` 自己构造 `ActorSpec`），
//! 无工具轮的简单路径与「固定绑定表整轮驱动」只有本文件的用例用得到 ⇒ 按「测试消费的
//! 代码住测试模块」就地定义，**不占生产文件的表面**。

use super::super::turn_exec::ActorError;
use super::{TurnInput, TurnOutcome, TurnRunner};
use symbio_core::{DeltaSink, Entity, Event, FullModel, LlmAdapter, SilentDeltas, Store, Verb};
use symbio_plugin_api::events::EVENT_USER_MESSAGE;
use symbio_testkit::trivial_actor_spec;

/// 测试侧先落用户格：生产里由 `chat_loop::drive` 在任何判决之前落，
/// 运行器只收 seq。返回用户格 seq（单幕场景下 `user_seq == drive_seq`）。
fn seed_user<S>(store: &S, turn: u64, text: &str) -> u64
where
    S: Store<Event = Event> + ?Sized,
{
    store
        .append(
            Event::pending(
                format!("u-{turn}"),
                EVENT_USER_MESSAGE,
                Entity::Turn,
                Verb::Opened,
                turn,
                "user",
            )
            .with_payload(serde_json::json!({ "text": text })),
        )
        .expect("用户格入格")
        .value()
}

/// 本轮落下的**事实名序列**（协议事实在内）。
///
/// 把它单点成一个函数：用例断言的是**整条序列**而不是几个下标，于是新增一条
/// 协议事实时，漏改的用例会在这里报出差异，而不是散在各处静默漂移。
fn kinds(snapshot: &[Event]) -> Vec<&str> {
    snapshot.iter().map(|e| e.kind.as_str()).collect()
}

impl TurnRunner {
    /// 跑一轮（测试侧）：用户消息（声明档位）→ 生成 → 收束事件落格。
    ///
    /// `turn` 是本轮的 turn 号（调用方保证单调递增——N3 的轮次锚）；`tier` 是本轮装配进
    /// 哪一档（随用户消息入格，D-044）。令牌 `tok` 是闸门：只有持 `FullModel` 的调用方
    /// 进得来（反射 / 快速档在类型上就到不了这里）。
    pub async fn run(
        &self,
        store: &std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>>,
        llm: std::sync::Arc<dyn LlmAdapter>,
        tok: FullModel,
        turn: u64,
        text: &str,
    ) -> Result<TurnOutcome, ActorError> {
        let user_seq = seed_user(store.as_ref(), turn, text);
        self.run_streaming(
            store,
            llm,
            tok,
            TurnInput {
                turn,
                user_seq,
                drive_seq: user_seq,
                window_turns: None,
                // 单请求入口（无工具轮）没有轮内循环 ⇒ 没有注入时机。
                inject: None,
                // 平凡值身份（DE-038 §4：单主体 = 今天的状态）。
                actor: trivial_actor_spec("session/main"),
                // 没有请求视图层可拼（三段由调用方的 `build_request_view` 给出）。
                // 恒走生成：定稿答话轮由 `turn_exec` 自己把答话交进来。
                final_reply: None,
                final_cost_ms: 0,
                turn_root_id: String::new(),
                dialog: None,
                nodes: None,
                // 无中止语义的路径：`None` ⇒ 首请求前中止检查是空操作。
                abort: None,
            },
            std::sync::Arc::new(SilentDeltas),
        )
        .await
    }

    /// 流式版（测试侧）：生成增量逐片经 `sink` 送出；落格语义与 [`Self::run`] 同一条路径。
    ///
    /// 无工具轮 = [`Self::drive_with_tools`] 的**入参为空的同一路径**（工具清单空、无分发方）
    /// ——「有工具」与「无工具」不是两条执行路径。
    pub async fn run_streaming(
        &self,
        store: &std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>>,
        llm: std::sync::Arc<dyn LlmAdapter>,
        tok: FullModel,
        input: TurnInput,
        sink: std::sync::Arc<dyn DeltaSink>,
    ) -> Result<TurnOutcome, ActorError> {
        self.drive_with_tools(
            store,
            llm,
            tok,
            input,
            sink,
            std::sync::Arc::from(Vec::new()),
            None,
        )
        .await
    }

    /// 测试侧的步驱动：**直接调生产派发器**（`turn_exec::drive`），不复制循环。
    ///
    /// 早先这里另写了一份等价循环；那会让「无进展收束」「步数上界」的用例只测到副本。
    /// 现在测试驱动与生产驱动是同一条路径（后果 2 的验收要求）。
    #[allow(clippy::too_many_arguments)]
    pub async fn drive_with_tools(
        &self,
        store: &std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>>,
        llm: std::sync::Arc<dyn LlmAdapter>,
        tok: FullModel,
        input: TurnInput,
        sink: std::sync::Arc<dyn DeltaSink>,
        tools: std::sync::Arc<[symbio_core::CapabilityMeta]>,
        dispatch: Option<std::sync::Arc<dyn super::super::tool_dispatch::ToolDispatch>>,
    ) -> Result<TurnOutcome, ActorError> {
        self.drive_with_actors(
            store,
            llm,
            tok,
            input,
            sink,
            tools,
            dispatch,
            super::super::loop_rows::ACTORS,
            // 夹具不跑轮首那几行 ⇒ 备输入的两样都是空（`D-222` 订正 3）。
            None,
            None,
        )
        .await
    }

    /// **换绑定表**的同一条派发路径（D-123 决策 4 的接缝）：在真实行表还在欠
    /// `announce-model` 时，让那个 `by` 跑一个别的动作（空动作 / 越界动作）。
    ///
    /// 判据 14「动作写了零格 ⇒ `exhausted{no_progress}`」与判据 2「写了名单外的格 ⇒
    /// `exhausted{undeclared-write}`」两条用例没有这条接缝就永远只能挂在「按构造不可达」上。
    ///
    /// `prefix` / `reflex` 是**备输入的产物**（`D-222` 订正 3）：生产里由
    /// `prepare_request_sections` 的 ⑥ 与轮首的 `policy.reflex` 给，夹具自己造。它们是这条
    /// 路径上的第二个接缝——「前缀排在基线之前」与「快路零往返」两条用例要它才造得出来。
    ///
    /// 本函数是**生产编排的测试门面**：`begin` → 填 → 驱动，三步与
    /// `turn_exec::call_model` 逐字同序。`D-222` 订正 3 把「建草稿」从派发器里搬出来
    /// 之后，夹具也必须自己做这一步——否则它测的就不是生产路径了。
    #[allow(clippy::too_many_arguments)]
    pub async fn drive_with_actors(
        &self,
        store: &std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>>,
        llm: std::sync::Arc<dyn LlmAdapter>,
        tok: FullModel,
        input: TurnInput,
        sink: std::sync::Arc<dyn DeltaSink>,
        tools: std::sync::Arc<[symbio_core::CapabilityMeta]>,
        dispatch: Option<std::sync::Arc<dyn super::super::tool_dispatch::ToolDispatch>>,
        actors: &'static [super::super::loop_rows::ActorEntry],
        prefix: Option<String>,
        reflex: Option<super::super::turn_runner::ReflexAnswer>,
    ) -> Result<TurnOutcome, ActorError> {
        // 接缝那一行的两格出口由夹具自己落（`D-222` 裁决 5：轮首四行、接缝、汇报与
        // 轮内各行现在同住一张绑定表，本夹具不跑前三者 ⇒ 它们的账要提前关好）。
        //
        // 派发格不只是审计——`turn.dispatch` 的幂等键（`d-{turn}-{drive_seq}`）还兼着
        // 「同一幕不许跑两次」的那道闸；`inputs` 格是 `turn_head.prepared` 的来源，
        // 没它的话环的第一笔债是 `prepare-turn`（`prio` 52），而那个动作要的是编排层
        // 那一片状态，本夹具没有。
        let facts = store.range(symbio_core::Seq::new(0));
        let now = super::super::loop_rows::wake_now_ms(&facts, input.drive_seq);
        let tier = super::super::loop_rows::turn_tier(&facts, input.turn, now);
        super::super::stage_events::land(
            store.as_ref(),
            input.turn,
            input.user_seq,
            &input.actor.principal,
            &[super::super::stage_events::StageFact::inputs(
                0,
                input.drive_seq,
                "ready",
            )],
        )
        .map_err(ActorError::Append)?;
        super::super::chat_loop::land_dispatch(
            store.as_ref(),
            input.turn,
            input.user_seq,
            input.drive_seq,
            tier,
            &input.actor,
        )
        .map_err(ActorError::Append)?;
        let mut session = match super::super::turn_runner::TurnRunner::begin(store.as_ref(), input)
            .await
            .map_err(ActorError::Append)?
        {
            super::super::turn_runner::TurnSessionBegin::Done(outcome) => return Ok(outcome),
            super::super::turn_runner::TurnSessionBegin::Run(session) => session,
        };
        // 三样「填」与生产逐字同序（`turn_exec::call_model`）：基线 prompt 先渲染
        // （压缩已经由上面那次派发格之前的步骤落好），再插前缀，最后寄存快路正文。
        session.render_base(store.as_ref());
        session.set_prefix(prefix);
        session.set_reflex(reflex);
        // `head` 恒 `None`：本夹具没有编排层那一片状态，而它选到的行也不读它。
        // 幕按裸形交进去（没有装配束 ⇒ 没有宿主面）：收幕把结局按值交回
        // （`DriveEnd::Bare`），归类与落库本夹具不做。
        let stage = super::super::turn_exec::StageRuntime::bare(
            session,
            store.clone(),
            llm,
            tok,
            sink,
            tools,
            dispatch,
        );
        let end = super::super::turn_exec::drive(
            super::super::turn_exec::DriveStart::Ready(Box::new(
                super::super::turn_exec::ReadyStage { stage, head: None },
            )),
            actors,
        )
        .await
        .map_err(|exit| match exit {
            super::super::turn_exec::TurnExecExit::Failed(e) => ActorError::Read(e),
            super::super::turn_exec::TurnExecExit::Aborted(_) => {
                ActorError::Read(symbio_core::PluginError::InternalError(
                    "环交出了中止出口（本夹具的绑定表里没有中止那一行）".into(),
                ))
            }
        })?;
        match end {
            super::super::turn_exec::DriveEnd::Bare(outcome) => Ok(outcome),
            super::super::turn_exec::DriveEnd::Answered => {
                Err(ActorError::Read(symbio_core::PluginError::InternalError(
                    "夹具没有编排层那一片状态，判不成答话".into(),
                )))
            }
            _ => Err(ActorError::Read(symbio_core::PluginError::InternalError(
                "裸幕只交出结局（环交回了别的出口）".into(),
            ))),
        }
    }
}

// ── v2 会话运行时：TurnRunner 验收（不变量靠构造成立 + WAL 持久 + 兜底联动）──

mod turn_runner_tests {
    use super::super::{DialogFacts, NodeFactsFn, StreamNodeFacts, TurnInput, TurnRunner};
    use crate::control_decl::{guard_vocab, EVENT_TURN_STAGE};
    use crate::control_decl::{CHAT_VOCAB, MESSAGE_VOCAB};
    use symbio_core::{
        check_all, cost_ledger, transcript, AppendError, Budget, Event, EventStore, Seq,
        SilentDeltas, Store, TokenIssuer,
    };
    use symbio_plugin_api::events::{
        EVENT_CHAT_ASSISTANT_FALLBACK, EVENT_CHAT_ASSISTANT_FINAL, EVENT_USER_MESSAGE,
    };
    use symbio_testkit::{trivial_actor_spec, StubLlmAdapter};

    /// 成功轮：final 落格、溯源指向本轮用户消息、实测 cost_ms > 0、
    /// 不变量绿、收束投影 settled；成本台账与兜底率同账（D-044）。
    #[tokio::test]
    async fn turn_runner_happy_path_lands_final_with_measured_cost() {
        let store: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>> =
            std::sync::Arc::new(symbio_core::EventStore::new());
        let tok = TokenIssuer::issue_deep();
        let llm = std::sync::Arc::new(StubLlmAdapter::succeed("stub-model"));

        let out = TurnRunner
            .run(&store, llm.clone(), tok, 0, "你好")
            .await
            .expect("桩必答");
        assert_eq!(out.turn, 0);
        assert!(!out.fell_back);
        assert!(out.text.contains("你好"), "桩回显 prompt：{}", out.text);
        // 桩零延迟 ⇒ cost_ms 可为 0（诚实值）；真实链路的正数断言在真实端点彩排里。

        let snapshot = store.range(Seq::new(0));
        assert!(
            check_all(guard_vocab(), &snapshot).is_empty(),
            "{:?}",
            check_all(guard_vocab(), &snapshot)
        );

        // 成本台账：实测值入账（agent:main 一笔）。
        let ledger = cost_ledger().apply(&snapshot, i64::MAX, Budget::generous());
        assert_eq!(ledger.value.of("session/main").spent_ms, out.cost_ms);

        // 兜底格：成功轮不落兜底（兜底率是 session 的读数，本处只钉驱动器产出）。
        assert_eq!(
            snapshot
                .iter()
                .filter(|e| e.kind == EVENT_CHAT_ASSISTANT_FALLBACK)
                .count(),
            0,
            "成功轮不落兜底格"
        );

        // turnstate：收束且 final 可见。
        let view = symbio_core::turnstate(CHAT_VOCAB).apply(&snapshot, 0, Budget::generous());
        assert!(view.value.settled() && view.value.final_text.is_some());
    }

    /// 失败轮（I3）：兜底事件落格（`turn × closed` + `reason`），`fell_back = true`，
    /// 不变量照常绿——「失败也是一句话」，不是静默。
    #[tokio::test]
    async fn turn_runner_failure_lands_fallback_not_silence() {
        let store: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>> =
            std::sync::Arc::new(symbio_core::EventStore::new());
        let tok = TokenIssuer::issue_deep();
        let llm = std::sync::Arc::new(StubLlmAdapter::always_fail("模型挂了"));

        let out = TurnRunner
            .run(&store, llm.clone(), tok, 0, "你好")
            .await
            .expect("兜底路径仍返回 Ok（轮次本身收束了）");
        assert!(out.fell_back, "失败轮必须走兜底");
        assert!(
            out.text.contains("模型挂了"),
            "兜底话术 = 失败原因：{}",
            out.text
        );

        let snapshot = store.range(Seq::new(0));
        assert!(
            check_all(guard_vocab(), &snapshot).is_empty(),
            "{:?}",
            check_all(guard_vocab(), &snapshot)
        );

        // 兜底格：失败轮恰好一条兜底（被看见，不是被藏）。
        assert_eq!(
            snapshot
                .iter()
                .filter(|e| e.kind == EVENT_CHAT_ASSISTANT_FALLBACK)
                .count(),
            1,
            "失败轮必须落一条兜底格"
        );
    }

    /// 事实轨可重放：多轮写入 → 重放事件 → 逐字节一致（N2 家族：恢复 = 重放，
    /// 投影不需要状态迁移）；turn 号跨重启由调用方续排（N3 仍成立）。
    ///
    /// 持久化介质（WAL 文件的重开）由 session 的存储层用例覆盖；驱动器只依赖
    /// [`Store`] 契约，故这里用平凡 `EventStore` 钉住「写进去的事件可完整重放」。
    #[tokio::test]
    async fn turn_runner_events_are_replayable() {
        let tok = TokenIssuer::issue_deep();
        let llm = std::sync::Arc::new(StubLlmAdapter::succeed("stub-model"));
        let store: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>> =
            std::sync::Arc::new(EventStore::new());
        for turn in 0..3u64 {
            TurnRunner
                .run(&store, llm.clone(), tok, turn, "问")
                .await
                .expect("桩必答");
        }

        assert_eq!(
            store.head().value(),
            21,
            "3 轮 × 7 事件（用户 / 接缝出口 / 派发 / 模型端口三格 / 收束）"
        );

        // 恢复后直接续排（turn 号接续）——N3 由 turn 号单调保证。
        let out = TurnRunner
            .run(&store, llm.clone(), tok, 3, "再问")
            .await
            .expect("桩必答");
        assert_eq!(out.turn, 3);

        let snapshot = store.range(Seq::new(0));
        assert!(
            check_all(guard_vocab(), &snapshot).is_empty(),
            "{:?}",
            check_all(guard_vocab(), &snapshot)
        );
        assert_eq!(snapshot.len(), 28);

        // 与一遍直写（不经过重放）的台账一致（N1/N2 家族）。
        let fresh: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>> =
            std::sync::Arc::new(symbio_core::EventStore::new());
        for turn in 0..4u64 {
            TurnRunner
                .run(
                    &fresh,
                    llm.clone(),
                    tok,
                    turn,
                    if turn == 3 { "再问" } else { "问" },
                )
                .await
                .unwrap();
        }
        let a = cost_ledger().apply(&snapshot, i64::MAX, Budget::generous());
        let b = cost_ledger().apply(&fresh.range(Seq::new(0)), i64::MAX, Budget::generous());
        assert_eq!(a.value.total_ms, b.value.total_ms, "重放后的台账与直写一致");
        assert_eq!(a.value.by_principal, b.value.by_principal);
    }

    /// 多轮 turn 号单调：N3（每 turn ≤ 1 条 final）由构造成立——
    /// 同一 turn 号重复使用被幂等键挡住（事件 id 撞车 ⇒ Duplicate）。
    #[tokio::test]
    async fn turn_runner_reused_turn_number_is_rejected() {
        let store: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>> =
            std::sync::Arc::new(symbio_core::EventStore::new());
        let tok = TokenIssuer::issue_deep();
        let llm = std::sync::Arc::new(StubLlmAdapter::succeed("stub-model"));

        TurnRunner
            .run(&store, llm.clone(), tok, 0, "第一轮")
            .await
            .unwrap();
        // 第二轮沿用同一轮号：用户格已落（`seed_user` 会先撞），故直接用同一份
        // 输入再跑一遍——派发格 / 收束格的幂等键同样必须挡住重号。
        let err = TurnRunner
            .run_streaming(
                &store,
                llm.clone(),
                tok,
                TurnInput {
                    turn: 0,
                    user_seq: 0,
                    drive_seq: 0,
                    inject: None,
                    window_turns: None,
                    actor: trivial_actor_spec("session/main"),
                    final_reply: None,
                    final_cost_ms: 0,
                    turn_root_id: String::new(),
                    dialog: None,
                    nodes: None,
                    abort: None,
                },
                std::sync::Arc::new(SilentDeltas),
            )
            .await;
        assert!(err.is_err(), "事件 id 撞车必须被 Store 幂等键拒绝");
        assert!(matches!(
            err,
            Err(super::ActorError::Append(AppendError::Duplicate))
        ));
    }

    /// 多轮对话带历史：③ transcript 投影读同一事实源，第二轮请求含第一轮。
    ///
    /// 判据按**角色**查（不只问「历史在不在」，还问「上一轮的提问是 `role: user`、
    /// 答复是 `role: assistant`」）——历史就是消息序列，没有分隔符。
    #[tokio::test]
    async fn multi_turn_transcript_carries_history_from_fact_source() {
        let store: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>> =
            std::sync::Arc::new(EventStore::new());
        let tok = TokenIssuer::issue_deep();
        let llm = std::sync::Arc::new(StubLlmAdapter::succeed("stub-model"));

        // 第一轮：单轮 = 只有当前那条 user 消息。
        let r1 = TurnRunner
            .run(&store, llm.clone(), tok, 0, "第一轮问题")
            .await
            .unwrap();
        assert!(r1.text.contains("第一轮问题"));

        // 第二轮：历史在场，且**角色正确**——模型看得见自己上一轮的答复，
        // 并且知道那句答复是自己说的（不是用户说的）。
        let r2 = TurnRunner
            .run(&store, llm.clone(), tok, 1, "第二轮问题")
            .await
            .unwrap();
        let last = super::super::prepare::render_messages(
            &store
                .range(Seq::new(0))
                .into_iter()
                .filter(|e| e.turn <= 1)
                .collect::<Vec<_>>(),
        );
        assert!(
            last.iter()
                .any(|m| m.role == "user" && m.text.contains("第一轮问题")),
            "历史里有上一轮提问，且它是 user 说的：{last:?}"
        );
        assert!(
            last.iter()
                .any(|m| m.role == "assistant" && m.text.contains("stub-model")),
            "历史里有上一轮答复，且它是 assistant 说的：{last:?}"
        );
        assert!(
            last.iter().any(|m| m.text.contains("第二轮问题")),
            "当前消息在历史之外：{last:?}"
        );
        // 顺序：历史在前，当前轮在后（时间序 = append-only）
        let at = |needle: &str| last.iter().position(|m| m.text.contains(needle));
        assert!(
            at("第一轮问题") < at("第二轮问题"),
            "时间序不得倒置：{last:?}"
        );
        assert!(r2.text.contains("第二轮问题"), "当前消息在历史之外");

        // 投影联动与 N1。
        let snapshot = store.range(Seq::new(0));
        assert!(
            check_all(guard_vocab(), &snapshot).is_empty(),
            "{:?}",
            check_all(guard_vocab(), &snapshot)
        );
        let view =
            transcript(CHAT_VOCAB, MESSAGE_VOCAB).apply(&snapshot, i64::MAX, Budget::generous());
        assert_eq!(view.value.entries.len(), 4, "两轮 × (问+答)");
        assert_eq!(view.value.entries[0].role, "user");
        assert_eq!(view.value.entries[1].role, "assistant");
        let again =
            transcript(CHAT_VOCAB, MESSAGE_VOCAB).apply(&snapshot, i64::MAX, Budget::generous());
        assert_eq!(view.value, again.value);
    }

    /// fallback 行进转写（assistant / detail）——兜底话术是用户实际看到的回复。
    #[test]
    fn transcript_includes_fallback_as_assistant_line() {
        use symbio_core::{Entity, Verb};
        use symbio_plugin_api::events::{EVENT_CHAT_ASSISTANT_FALLBACK, EVENT_USER_MESSAGE};
        let store: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>> =
            std::sync::Arc::new(EventStore::new());
        store
            .append(
                Event::pending(
                    "u-0",
                    EVENT_USER_MESSAGE,
                    Entity::Turn,
                    Verb::Opened,
                    0,
                    "user",
                )
                .with_payload(serde_json::json!({ "text": "问", "tier": "deep" })),
            )
            .unwrap();
        store
            .append(
                Event::pending(
                    "fb-0",
                    EVENT_CHAT_ASSISTANT_FALLBACK,
                    Entity::Turn,
                    Verb::Closed,
                    0,
                    "session/main",
                )
                .with_produced_by(0)
                .with_payload(serde_json::json!({
                    "reason": "fallback",
                    "detail": "上游 402",
                })),
            )
            .unwrap();
        let snapshot = store.range(Seq::new(0));
        let view =
            transcript(CHAT_VOCAB, MESSAGE_VOCAB).apply(&snapshot, i64::MAX, Budget::generous());
        assert_eq!(view.value.entries.len(), 2);
        assert_eq!(view.value.entries[1].role, "assistant");
        assert_eq!(view.value.entries[1].text, "上游 402");
    }

    /// 工具结果进转写（`tool` / 工具名 + text）——跨轮 prompt 因此能重建**含工具**的
    /// 对话；且工具行
    /// 投影成 `role: "tool"` 的**结构化消息**（带工具名与合成 `tool_call_id`），与轮内
    /// 交换（`tool_exchange_messages`）同形。
    #[test]
    fn transcript_includes_artifact_as_tool_line() {
        use symbio_core::{Entity, Verb};
        use symbio_plugin_api::events::{
            EVENT_ARTIFACT_ADDED, EVENT_CHAT_ASSISTANT_FINAL, EVENT_USER_MESSAGE,
        };
        let store: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>> =
            std::sync::Arc::new(EventStore::new());
        // 轮 0：用户 → 工具结果 → 收束
        store
            .append(
                Event::pending(
                    "u-0",
                    EVENT_USER_MESSAGE,
                    Entity::Turn,
                    Verb::Opened,
                    0,
                    "user",
                )
                .with_payload(serde_json::json!({ "text": "帮我回显", "tier": "deep" })),
            )
            .unwrap();
        store
            .append(
                Event::pending(
                    "a-0-0",
                    EVENT_ARTIFACT_ADDED,
                    Entity::Artifact,
                    Verb::Asserted,
                    0,
                    "session/main",
                )
                .with_produced_by(0)
                .with_payload(
                    serde_json::json!({ "tool": "mcp__mockserv__echo", "text": "回显内容" }),
                ),
            )
            .unwrap();
        store
            .append(
                Event::pending(
                    "f-0",
                    EVENT_CHAT_ASSISTANT_FINAL,
                    Entity::Turn,
                    Verb::Closed,
                    0,
                    "session/main",
                )
                .with_produced_by(0)
                .with_payload(serde_json::json!({ "text": "回显完成" })),
            )
            .unwrap();
        // 轮 1：用户（本轮）——历史里应看得见轮 0 的工具结果。
        store
            .append(
                Event::pending(
                    "u-1",
                    EVENT_USER_MESSAGE,
                    Entity::Turn,
                    Verb::Opened,
                    1,
                    "user",
                )
                .with_payload(serde_json::json!({ "text": "再问一句", "tier": "deep" })),
            )
            .unwrap();

        let snapshot = store.range(Seq::new(0));
        let view =
            transcript(CHAT_VOCAB, MESSAGE_VOCAB).apply(&snapshot, i64::MAX, Budget::generous());
        let entries = &view.value.entries;
        assert_eq!(entries.len(), 4, "四格 → 四行：{entries:?}");
        assert_eq!(entries[0].role, "user");
        assert_eq!(entries[1].role, "tool");
        assert_eq!(entries[1].tool.as_deref(), Some("mcp__mockserv__echo"));
        assert_eq!(entries[1].text, "回显内容");
        assert_eq!(entries[0].tool, None, "非工具行不带 tool");
        assert_eq!(entries[2].role, "assistant");

        // prompt：轮 1 的历史里含工具结果行，投影成结构化消息（与轮内交换同形）。
        let messages = view.value.to_messages();
        assert_eq!(messages.len(), 4, "四格 → 四条消息：{messages:?}");
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[1].role, "tool");
        assert_eq!(messages[1].tool.as_deref(), Some("mcp__mockserv__echo"));
        assert_eq!(
            messages[1].tool_call_id.as_deref(),
            Some("call_mcp__mockserv__echo_0"),
            "工具消息带合成 tool_call_id"
        );
        assert_eq!(messages[1].text, "回显内容");
        assert_eq!(messages[2].role, "assistant");
        assert_eq!(messages[3].role, "user");

        // 线格式：非工具行保持 `{role, text}`（新增角色不改旧角色的形状）；工具行多一个 `tool`。
        let wire = serde_json::to_value(&view.value).unwrap();
        assert_eq!(
            wire["entries"][0],
            serde_json::json!({ "role": "user", "text": "帮我回显" })
        );
        assert_eq!(wire["entries"][1]["tool"], "mcp__mockserv__echo");
    }

    /// 收集口：按序记增量（流式验收用）。
    struct CollectingDeltas(std::sync::Mutex<Vec<String>>);

    impl symbio_core::DeltaSink for CollectingDeltas {
        fn on_delta(&self, text: &str) {
            self.0.lock().unwrap().push(text.to_string());
        }
        // 本组用例的桩不产推理增量；推理通道由
        // `crates/symbio-plugin-model/src/bound_provider.test.rs` 的桥接用例覆盖（那里有 SSE 级 fixture）。
        fn on_reasoning(&self, _text: &str) {}
        fn on_tool_frame(&self, _frame: &symbio_core::chat_node::ChatNode) {}
    }

    /// 流式运行：分片按序进 sink；落格语义与 run() 同一条路径（final 照常、
    /// 不变量照绿）；无分片的桩走默认降级（一次性全文一帧）。
    #[tokio::test]
    async fn streaming_run_forwards_deltas_and_writes_final() {
        use std::sync::{Arc, Mutex};
        use symbio_core::DeltaSink;

        let store: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>> =
            std::sync::Arc::new(EventStore::new());
        let tok = TokenIssuer::issue_deep();
        let llm = std::sync::Arc::new(StubLlmAdapter::succeed_streaming(
            "stub-model",
            &["你", "好", "！"],
        ));
        let got = Arc::new(CollectingDeltas(Mutex::new(Vec::new())));
        let user_seq = super::seed_user(store.as_ref(), 0, "问");

        let out = TurnRunner
            .run_streaming(
                &store,
                llm.clone(),
                tok,
                TurnInput {
                    turn: 0,
                    user_seq,
                    drive_seq: user_seq,
                    window_turns: None,
                    inject: None,
                    actor: trivial_actor_spec("session/main"),
                    // 请求级前缀：测试不走请求视图层（三段皆空）。
                    final_reply: None,
                    final_cost_ms: 0,
                    turn_root_id: String::new(),
                    dialog: None,
                    nodes: None,
                    abort: None,
                },
                got.clone() as Arc<dyn DeltaSink>,
            )
            .await
            .unwrap();

        assert_eq!(
            *got.0.lock().unwrap(),
            vec!["你", "好", "！"],
            "分片按序送达"
        );
        assert_eq!(out.text, "你好！", "全文 = 分片拼接");
        assert!(!out.fell_back);
        let snapshot = store.range(Seq::new(0));
        assert!(
            check_all(guard_vocab(), &snapshot).is_empty(),
            "{:?}",
            check_all(guard_vocab(), &snapshot)
        );
        let view =
            transcript(CHAT_VOCAB, MESSAGE_VOCAB).apply(&snapshot, i64::MAX, Budget::generous());
        assert_eq!(view.value.entries[1].text, "你好！", "final 落的是全文");

        // 默认降级：无分片桩经 run_streaming = 一帧全文（诚实降级，不是静默）。
        let store2: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>> =
            std::sync::Arc::new(EventStore::new());
        let llm2 = std::sync::Arc::new(StubLlmAdapter::succeed("stub-model"));
        let got2 = Arc::new(CollectingDeltas(Mutex::new(Vec::new())));
        let user_seq = super::seed_user(store2.as_ref(), 0, "问");
        TurnRunner
            .run_streaming(
                &store2,
                llm2.clone(),
                tok,
                TurnInput {
                    turn: 0,
                    user_seq,
                    drive_seq: user_seq,
                    inject: None,
                    window_turns: None,
                    actor: trivial_actor_spec("session/main"),
                    // 请求级前缀：测试不走请求视图层（三段皆空）。
                    final_reply: None,
                    final_cost_ms: 0,
                    turn_root_id: String::new(),
                    dialog: None,
                    nodes: None,
                    abort: None,
                },
                got2.clone() as Arc<dyn DeltaSink>,
            )
            .await
            .unwrap();
        let frames = got2.0.lock().unwrap();
        assert_eq!(frames.len(), 1, "非流式适配器 = 一帧全文");
        assert!(frames[0].contains("问"));
    }

    /// 判据 5：流式帧直通，但**不产生事实**——同一次请求的两种跑法事件条数相同。
    ///
    /// 帧走 `ExecEventSink` 直通（DE-014 §3），本来就不进存储；本用例把「帧数 =
    /// 分片数、日志不增」钉住。若将来有人把帧顺手写进日志，条数会多出来，这里先红。
    #[tokio::test]
    async fn streaming_frames_do_not_add_facts() {
        use std::sync::{Arc, Mutex};
        use symbio_core::DeltaSink;

        // 非流式：一次全文，一条 final。
        let plain_store: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>> =
            std::sync::Arc::new(EventStore::new());
        let tok = TokenIssuer::issue_deep();
        TurnRunner
            .run(
                &plain_store,
                std::sync::Arc::new(StubLlmAdapter::succeed("stub-model")),
                tok,
                0,
                "问",
            )
            .await
            .unwrap();

        // 流式：三个分片，同样一次模型往返。
        let stream_store: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>> =
            std::sync::Arc::new(EventStore::new());
        let got = Arc::new(CollectingDeltas(Mutex::new(Vec::new())));
        let user_seq = super::seed_user(stream_store.as_ref(), 0, "问");
        TurnRunner
            .run_streaming(
                &stream_store,
                std::sync::Arc::new(StubLlmAdapter::succeed_streaming(
                    "stub-model",
                    &["你", "好", "！"],
                )),
                tok,
                TurnInput {
                    turn: 0,
                    user_seq,
                    drive_seq: user_seq,
                    window_turns: None,
                    inject: None,
                    actor: trivial_actor_spec("session/main"),
                    final_reply: None,
                    final_cost_ms: 0,
                    turn_root_id: String::new(),
                    dialog: None,
                    nodes: None,
                    abort: None,
                },
                got.clone() as Arc<dyn DeltaSink>,
            )
            .await
            .unwrap();

        assert_eq!(
            *got.0.lock().unwrap(),
            vec!["你", "好", "！"],
            "分片按序直通"
        );
        assert_eq!(
            stream_store.range(Seq::new(0)).len(),
            plain_store.range(Seq::new(0)).len(),
            "流式帧不落事实：两种跑法事件条数相同"
        );
    }

    /// 开场白**落格的同一时刻**上线到实时面（一半），且节点 id 就是落格载荷里
    /// 那个 `dialog.id`。
    ///
    /// ## 为什么要钉「它就是第一帧」
    ///
    /// 开场白的作用就是「本轮要跑一阵子，先说一声」。它若只进事实源、等轮末的落库
    /// 镜像才到界面 ⇒ 一次八分钟的工具轮里终端一个字都没有，用户读到的是「卡死」。
    /// 顺序就是这条缺陷的判据，所以钉到达次序而不是只钉「到达过」。
    ///
    /// ## 为什么要钉 id 与落格一致
    ///
    /// 轮末同一格还会经落库镜像再来一次；消费端按 id 合并成同一格，id 对不上就是两张卡。
    ///
    /// ## 为什么钉的是接缝上的那个函数，不是运行器
    ///
    /// 开场白是轮首判决行的产物，而草稿在幕首就建（`D-222` 订正 3）——运行器开跑时这句
    /// 话还没说出来。落点因此是接缝（`D-223` 订正 1）：`assemble_turn` 手里同时有事实源
    /// 与流式桥的那一刻。本用例直接调它，钉的正是那条时序。
    ///
    /// **反向自检**：删掉 `chat_loop::land_opening` 末尾那次 `sink.on_tool_frame` 本用例
    /// 必红（开场白只剩事实、实时面上一个字都没有）。
    #[tokio::test]
    async fn opening_node_reaches_the_live_surface_at_production_time() {
        use std::sync::{Arc, Mutex};
        use symbio_core::{chat_node::MessageContent, DeltaSink};

        /// 按序记「实时面上到过的帧」：`(节点 id, 正文)`，增量记为空 id。
        #[derive(Default)]
        struct Frames(Mutex<Vec<(String, String)>>);

        impl DeltaSink for Frames {
            fn on_delta(&self, text: &str) {
                self.0
                    .lock()
                    .unwrap()
                    .push((String::new(), text.to_string()));
            }
            fn on_reasoning(&self, _text: &str) {}
            fn on_tool_frame(&self, frame: &symbio_core::chat_node::ChatNode) {
                self.0.lock().unwrap().push((
                    frame.id.clone(),
                    frame
                        .content
                        .as_ref()
                        .map(MessageContent::to_text)
                        .unwrap_or_default(),
                ));
            }
        }

        let store: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>> =
            std::sync::Arc::new(EventStore::new());
        let got = Arc::new(Frames::default());
        let user_seq = super::seed_user(store.as_ref(), 0, "问");

        super::super::super::chat_loop::land_opening(
            store.as_ref(),
            got.as_ref() as &dyn DeltaSink,
            0,
            user_seq,
            "session/main",
            Some("我先看一下。"),
            Some(&DialogFacts {
                id: "op-node-1".into(),
                reason: "escalate".into(),
                exclude_from_context: true,
            }),
        )
        .expect("开场白必须落得下");

        // 事实：一格 `chat.assistant.opening`，载荷带节点身份。
        let opening = store
            .range(Seq::new(0))
            .into_iter()
            .find(|e| e.kind == "chat.assistant.opening")
            .expect("开场白要入格");
        assert_eq!(opening.payload["id"], "op-node-1");
        assert_eq!(opening.payload["text"], "我先看一下。");

        // 实时面：同一时刻、同一个 id。
        let got = got.0.lock().unwrap();
        assert_eq!(
            got.first(),
            Some(&("op-node-1".to_string(), "我先看一下。".to_string())),
            "开场白要在落格的同一时刻、带落格那个节点 id 上线（否则它只是轮末的旧闻）：{got:?}"
        );
        drop(got);

        // 空正文不落格也不上线（与汇报 / 收束同一条纪律）。
        let empty = Arc::new(Frames::default());
        super::super::super::chat_loop::land_opening(
            store.as_ref(),
            empty.as_ref() as &dyn DeltaSink,
            0,
            user_seq,
            "session/main",
            Some("   "),
            None,
        )
        .expect("空正文不算失败");
        assert!(empty.0.lock().unwrap().is_empty(), "空正文不上线");
    }

    /// 中止轮的入参包：半截产出事实经 `nodes` 闭包随收束入格（生产里这个闭包读
    /// UI 桥上攒的半截正文 / 思考；测试直接给值）。
    fn abort_input(store: &dyn Store<Event = Event>, nodes: NodeFactsFn) -> TurnInput {
        let user_seq = super::seed_user(store, 0, "问");
        TurnInput {
            turn: 0,
            user_seq,
            drive_seq: user_seq,
            window_turns: None,
            inject: None,
            actor: trivial_actor_spec("session/main"),
            // 请求级前缀：测试不走请求视图层（三段皆空）。
            final_reply: None,
            final_cost_ms: 0,
            turn_root_id: "r-1".into(),
            dialog: None,
            nodes: Some(nodes),
            // 无中止语义：本夹具走 `AdapterError::Aborted`（模型中途返回），
            // 不测首请求前中止的闸门空隙。
            abort: None,
        }
    }

    /// 中止（AdapterError::Aborted）：**照落收束格**（`chat.assistant.final`，
    /// 载荷 `status: "aborted"` + `terminated: { at }` + 顶层 `reason: interrupted`）——思考 / 正文是
    /// 事实，完成 / 终止是**状态**：「用户按了停止」是本轮的结局，不是另一种
    /// 事实种类，kind 不为它新增变体，也绝不复用兜底格（`chat.assistant.fallback`
    /// 是**失败**的形状）。本轮无半截产出时 `text` 为空，收束格照落：轮次必须
    /// 收束，未收束的缺口才真的合得上（C4）。
    #[tokio::test]
    async fn aborted_run_closes_the_cell_with_aborted_status() {
        use symbio_core::unresolved_turns;

        let store: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>> =
            std::sync::Arc::new(EventStore::new());
        let tok = TokenIssuer::issue_deep();
        let llm = std::sync::Arc::new(StubLlmAdapter::aborting("stub-model"));

        let out = TurnRunner
            .run(&store, llm.clone(), tok, 0, "问")
            .await
            .unwrap();

        assert!(out.aborted, "中止必须在结果里可见");
        assert!(!out.fell_back, "中止不是兜底（两者互斥）");
        assert!(out.text.is_empty(), "本轮没有半截产出");

        let snapshot = store.range(Seq::new(0));
        // 模型端口的协议事实落在收束格**之前**：中止也是一次模型
        // 往返，它同样有 `request` / `sent` / `failed` 三格——中止不是「没发生」。
        assert_eq!(
            super::kinds(&snapshot),
            vec![
                "user.message",
                // 接缝的出口格：轮首那几行跑完、任何请求之前的装配。
                "turn.stage",
                "turn.dispatch",
                "request",
                "sent",
                "failed",
                "chat.assistant.final",
            ],
            "用户格 + 派发格 + 模型端口三格 + 收束格：{snapshot:?}"
        );
        assert_eq!(snapshot[0].kind, EVENT_USER_MESSAGE);
        assert_eq!(snapshot[1].kind, EVENT_TURN_STAGE, "接缝的出口格");
        assert_eq!(
            snapshot[2].kind,
            symbio_plugin_api::events::EVENT_TURN_DISPATCH,
            "派发档另立一格"
        );
        let cell = snapshot
            .iter()
            .find(|e| e.kind == EVENT_CHAT_ASSISTANT_FINAL)
            .expect("中止照落收束格");
        assert_eq!(cell.payload["status"], "aborted", "终止是状态字段");
        assert_eq!(
            cell.payload["reason"], "interrupted",
            "停机原因入格（顶层 `reason`，闭集取值）"
        );
        assert!(
            cell.payload["terminated"]["at"].as_i64().is_some(),
            "半截产出停在哪里由 `terminated.at` 说（原因不再在 terminated 里写第二份）"
        );
        assert!(
            cell.payload["terminated"]["at"].as_u64().is_some(),
            "终止时刻是时间戳"
        );
        assert!(
            check_all(guard_vocab(), &snapshot).is_empty(),
            "{:?}",
            check_all(guard_vocab(), &snapshot)
        );
        assert!(
            unresolved_turns(&snapshot, false).is_empty(),
            "收束格了结本轮——中止不是未收束缺口"
        );
    }

    /// 事实完整性：**用户中断后重建的历史里那半句话还在**——模型说到一半被打断
    /// （"让我想想这个名词的含义……"），那半句照原样入格（`text` + 节点身份
    /// `text_id` + 本轮 `turn_id`），重建投影里仍是 assistant 的一行。中止不落
    /// 这半句 = 用户追问名词时前置上下文消失，那才是真正的丢事实。
    #[tokio::test]
    async fn abort_keeps_the_half_sentence_in_the_grid() {
        let store: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>> =
            std::sync::Arc::new(EventStore::new());
        let tok = TokenIssuer::issue_deep();
        let llm = std::sync::Arc::new(StubLlmAdapter::aborting("stub-model"));

        let out = TurnRunner
            .drive_with_tools(
                &store,
                llm.clone(),
                tok,
                abort_input(
                    store.as_ref(),
                    std::sync::Arc::new(|| StreamNodeFacts {
                        text_id: Some("n-text".into()),
                        text: Some("让我想想这个名词的含义……".into()),
                        reasoning_id: None,
                        reasoning: None,
                    }),
                ),
                std::sync::Arc::new(SilentDeltas),
                std::sync::Arc::from(Vec::new()),
                None,
            )
            .await
            .unwrap();

        assert!(out.aborted);
        assert_eq!(out.text, "让我想想这个名词的含义……", "半截正文随结果带走");

        let snapshot = store.range(Seq::new(0));
        assert_eq!(
            super::kinds(&snapshot),
            vec![
                "user.message",
                // 接缝的出口格：轮首那几行跑完、任何请求之前的装配。
                "turn.stage",
                "turn.dispatch",
                "request",
                "sent",
                "failed",
                "chat.assistant.final",
            ],
            "用户格 + 派发格 + 模型端口三格 + 收束格：{snapshot:?}"
        );
        let cell = snapshot
            .iter()
            .find(|e| e.kind == EVENT_CHAT_ASSISTANT_FINAL)
            .expect("收束格在");
        let output = snapshot
            .iter()
            .find(|e| e.kind == symbio_core::EVENT_FAILED)
            .expect("失败格在");
        // 半截正文属于**那一次模型往返**（`failed`），收束格只引用它的节点身份。
        assert_eq!(
            output.payload["text"], "让我想想这个名词的含义……",
            "半截正文照原样入格（不因没说完就不记）"
        );
        assert_eq!(
            output.payload["text_id"], "n-text",
            "节点身份入格（重建复刻同一节点）"
        );
        assert_eq!(output.payload["turn_id"], "r-1", "本轮 Turn 根 id 入格");
        assert_eq!(output.payload["status"], "aborted", "停法随失败入格");
        assert_eq!(
            cell.payload["answer_id"], "n-text",
            "收束格引用同一次往返的正文身份"
        );
        assert_eq!(cell.payload["text"], "", "收束格不复制正文");
        assert_eq!(cell.payload["status"], "aborted");

        // 重建侧：网格投影出的历史里那半句仍在（判据原文说的「重建的历史」）。
        let view =
            transcript(CHAT_VOCAB, MESSAGE_VOCAB).apply(&snapshot, i64::MAX, Budget::generous());
        assert!(
            view.value
                .entries
                .iter()
                .any(|e| e.role == "assistant" && e.text.contains("让我想想这个名词的含义")),
            "重建的历史里那半句话还在：{:?}",
            view.value.entries
        );
        assert!(
            check_all(guard_vocab(), &snapshot).is_empty(),
            "{:?}",
            check_all(guard_vocab(), &snapshot)
        );
    }

    /// 事实完整性：**reasoning 在网格里查得到**——半截思考与正文同等对待（它也是
    /// 事实），`reasoning` / `reasoning_id` 入格，重建可查。
    #[tokio::test]
    async fn reasoning_is_queryable_in_the_grid() {
        let store: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>> =
            std::sync::Arc::new(EventStore::new());
        let tok = TokenIssuer::issue_deep();
        let llm = std::sync::Arc::new(StubLlmAdapter::aborting("stub-model"));

        TurnRunner
            .drive_with_tools(
                &store,
                llm.clone(),
                tok,
                abort_input(
                    store.as_ref(),
                    std::sync::Arc::new(|| StreamNodeFacts {
                        text_id: None,
                        text: None,
                        reasoning_id: Some("n-rea".into()),
                        reasoning: Some("这个名词需要先定义。".into()),
                    }),
                ),
                std::sync::Arc::new(SilentDeltas),
                std::sync::Arc::from(Vec::new()),
                None,
            )
            .await
            .unwrap();

        let snapshot = store.range(Seq::new(0));
        let output = snapshot
            .iter()
            .find(|e| e.kind == symbio_core::EVENT_FAILED)
            .expect("失败格在");
        assert_eq!(
            output.payload["reasoning"], "这个名词需要先定义。",
            "半截思考照原样入格"
        );
        assert_eq!(output.payload["reasoning_id"], "n-rea", "推理节点身份入格");
        assert!(
            check_all(guard_vocab(), &snapshot).is_empty(),
            "{:?}",
            check_all(guard_vocab(), &snapshot)
        );
    }

    /// 中止不污染兜底语义：收束 kind 仍是 `chat.assistant.final`（网格里**没有**
    /// `chat.assistant.fallback` 格），`fell_back = false`，兜底率的分子不认它——
    /// 用户按的停止不是「模型答不出」，两者在格与指标上都要分得开。
    #[tokio::test]
    async fn abort_does_not_pollute_fallback_semantics() {
        let store: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>> =
            std::sync::Arc::new(EventStore::new());
        let tok = TokenIssuer::issue_deep();
        let llm = std::sync::Arc::new(StubLlmAdapter::aborting("stub-model"));

        let out = TurnRunner
            .drive_with_tools(
                &store,
                llm.clone(),
                tok,
                abort_input(
                    store.as_ref(),
                    std::sync::Arc::new(|| StreamNodeFacts {
                        text_id: Some("n-text".into()),
                        text: Some("让我想想这个名词的含义……".into()),
                        reasoning_id: None,
                        reasoning: None,
                    }),
                ),
                std::sync::Arc::new(SilentDeltas),
                std::sync::Arc::from(Vec::new()),
                None,
            )
            .await
            .unwrap();

        assert!(out.aborted && !out.fell_back, "中止与兜底互斥");
        let snapshot = store.range(Seq::new(0));
        assert!(
            snapshot
                .iter()
                .all(|e| e.kind != EVENT_CHAT_ASSISTANT_FALLBACK),
            "中止绝不落兜底格：{snapshot:?}"
        );
        assert_eq!(
            snapshot
                .iter()
                .find(|e| e.kind == EVENT_CHAT_ASSISTANT_FINAL)
                .map(|e| e.kind.as_str()),
            Some("chat.assistant.final"),
            "收束 kind 不换"
        );
    }

    /// 写侧闸：
    /// 主体不持 `reply.first` ⇒ 收束格**不落**——该轮留在未收束态（C4 报得出），
    /// 但用户的答案仍回给调用方。
    ///
    /// ## 这个用例挡的是什么
    ///
    /// 收束格的写方是**运行器**（闸在落格前判，见 `closure_gate`）。闸漏判不会
    /// 让任何东西变红——收束格照落、既有断言全绿，只有未收束态的 C4 与本用例
    /// 看得见它。
    ///
    /// 判据分两半，各自钉一件事：
    /// - **拒绝**：`PRINCIPAL_AUTONOMOUS`（表里只持 `define.work`）驱动一轮 ⇒ 网格里
    ///   **没有** `chat.assistant.final`，`unresolved_turns` 看得见这一格缺口；
    /// - **放行对照**：`agent:main` 驱动同样一轮 ⇒ final 照落（否则「拒绝」可能只是
    ///   「闸把所有人都拒了」的平凡真）。
    #[tokio::test]
    async fn closure_is_withheld_when_the_principal_lacks_the_grant() {
        use symbio_core::unresolved_turns;
        use symbio_core::PRINCIPAL_AUTONOMOUS;

        // ── 拒绝：自主发起者不持 `reply.first` ⇒ 收束格不落 ────────────────
        let store: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>> =
            std::sync::Arc::new(EventStore::new());
        let tok = TokenIssuer::issue_deep();
        let llm = std::sync::Arc::new(StubLlmAdapter::succeed("stub-model"));
        let user_seq = super::seed_user(store.as_ref(), 0, "问");
        let out = TurnRunner
            .drive_with_tools(
                &store,
                llm.clone(),
                tok,
                TurnInput {
                    turn: 0,
                    user_seq,
                    drive_seq: user_seq,
                    inject: None,
                    window_turns: None,
                    actor: trivial_actor_spec(PRINCIPAL_AUTONOMOUS),
                    // 请求级前缀：测试不走请求视图层（三段皆空）。
                    final_reply: None,
                    final_cost_ms: 0,
                    turn_root_id: String::new(),
                    dialog: None,
                    nodes: None,
                    abort: None,
                },
                std::sync::Arc::new(SilentDeltas),
                std::sync::Arc::from(Vec::new()),
                None,
            )
            .await
            .expect("落格不报错——拒绝的语义是**不入格**，不是失败");
        assert!(!out.fell_back, "拒绝不是兜底");

        let snapshot = store.range(Seq::new(0));
        assert!(
            !snapshot
                .iter()
                .any(|e| e.kind == symbio_plugin_api::events::EVENT_CHAT_ASSISTANT_FINAL),
            "缺 reply.first ⇒ 收束格不得入格：{:?}",
            snapshot.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>()
        );
        assert_eq!(
            unresolved_turns(&snapshot, false).len(),
            1,
            "拒绝要**可被判出**（留在未收束态）——C4 看得见它"
        );

        // ── 放行对照：`agent:main` 持 `reply.first` ⇒ final 照落 ───────────
        let store2: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>> =
            std::sync::Arc::new(EventStore::new());
        let user_seq = super::seed_user(store2.as_ref(), 0, "问");
        TurnRunner
            .drive_with_tools(
                &store2,
                llm.clone(),
                tok,
                TurnInput {
                    turn: 0,
                    user_seq,
                    drive_seq: user_seq,
                    inject: None,
                    window_turns: None,
                    actor: trivial_actor_spec("session/main"),
                    // 请求级前缀：测试不走请求视图层（三段皆空）。
                    final_reply: None,
                    final_cost_ms: 0,
                    turn_root_id: String::new(),
                    dialog: None,
                    nodes: None,
                    abort: None,
                },
                std::sync::Arc::new(SilentDeltas),
                std::sync::Arc::from(Vec::new()),
                None,
            )
            .await
            .expect("桩必答");
        let snapshot2 = store2.range(Seq::new(0));
        assert!(
            snapshot2
                .iter()
                .any(|e| e.kind == symbio_plugin_api::events::EVENT_CHAT_ASSISTANT_FINAL),
            "持有 reply.first ⇒ final 照落（否则拒绝可能只是「闸全拒」的平凡真）"
        );
    }
}

// ── 工具轮（`drive_with_tools`）：产物落格 + 结果回灌 + 等待用户停下 ──────────

mod tool_round_tests {
    use crate::control_decl::guard_vocab;
    use std::sync::{Arc, Mutex};
    use symbio_testkit::trivial_actor_spec;

    use super::super::super::tool_dispatch::{
        DispatchOutcome, ToolBegin, ToolDispatch, ToolPort, ToolPortCall, WakeBatch, WakeSpec,
    };
    use super::super::{NodeFactsFn, RoundInjector, StreamNodeFacts, TurnInput, TurnRunner};
    use crate::control_decl::{EVENT_AUTHZ_GRANTED, EVENT_TURN_STAGE};
    use symbio_core::{
        check_all, unresolved_turns, AdapterError, CapabilityMeta, DeltaSink, Entity, Event,
        EventStore, FullModel, LlmAdapter, LlmTurn, ModelFinishReason, PromptMessage, Seq,
        SilentDeltas, Store, TokenIssuer, TurnToolCallInfo, Verb,
    };
    use symbio_plugin_api::events::{
        EVENT_ARTIFACT_ADDED, EVENT_CHAT_ASSISTANT_FINAL, EVENT_CHAT_ASSISTANT_PROGRESS,
        EVENT_SESSION_CONSOLIDATED, EVENT_USER_MESSAGE,
    };

    /// 判据：**每一次**模型往返都留下完整的协议事实——
    /// `request` → `sent` → `response` / `failed`，且三者共享同一个逻辑 id。
    ///
    /// ## 它钉的是什么
    ///
    /// 「模型调用是可重放的最小单位」这句话的可执行形状：崩溃之后能回答
    /// 「上次发到第几次模型请求」「那次有没有回来」，靠的就是这三格。
    /// 少一格（引擎忘了落 `sent`、或失败不落 `failed`）就是一笔**静默挂起**的账——
    /// 没有任何东西会变红，只有这条用例能叫出来。
    ///
    /// ## 为什么 id 必须互不相同
    ///
    /// 一次往返一个逻辑 id。撞了 id，第二次的 `sent` 会被幂等键吃掉，于是「两次
    /// 往返」在日志里长成「一次」——重放时只能发一次，而模型确实收过两次请求。
    #[tokio::test]
    async fn every_model_call_leaves_a_sent_and_an_answer() {
        let store: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>> =
            std::sync::Arc::new(EventStore::new());
        let tok = TokenIssuer::issue_deep();
        let llm = std::sync::Arc::new(ToolCallingLlm::new());
        let dispatch = std::sync::Arc::new(FakeDispatch {
            pending: false,
            rounds: Arc::new(Mutex::new(0)),
        });

        TurnRunner
            .drive_with_tools(
                &store,
                llm.clone(),
                tok,
                input(store.as_ref()),
                Arc::new(SilentDeltas),
                std::sync::Arc::from(tools()),
                Some(dispatch.clone()),
            )
            .await
            .expect("工具轮必答");

        let snapshot = store.range(Seq::new(0));
        // 模型端口的协议事实（其余端口的债不算）。
        let model: Vec<_> = snapshot
            .iter()
            .filter(|e| e.payload.get("port").and_then(|v| v.as_str()) == Some("model"))
            .collect();

        let id_of = |e: &Event| e.payload["id"].as_str().unwrap_or_default().to_string();
        let requests: Vec<String> = model
            .iter()
            .filter(|e| e.kind == symbio_core::EVENT_REQUEST)
            .map(|e| id_of(e))
            .collect();
        let sents: Vec<String> = model
            .iter()
            .filter(|e| e.kind == symbio_core::EVENT_SENT)
            .map(|e| id_of(e))
            .collect();
        let answers: Vec<String> = model
            .iter()
            .filter(|e| {
                e.kind == symbio_core::EVENT_RESPONSE || e.kind == symbio_core::EVENT_FAILED
            })
            .map(|e| id_of(e))
            .collect();

        // 工具轮 = 两次模型往返（第一次请求工具、第二次收尾）。
        assert_eq!(requests.len(), 2, "两次模型往返 ⇒ 两条 request：{model:?}");
        assert_eq!(
            sents.len(),
            2,
            "每次派发都落 `sent`（引擎写，跑之前）：{model:?}"
        );
        assert_eq!(
            answers.len(),
            2,
            "每次派发都有应答（`response` / `failed`）：{model:?}"
        );
        assert_eq!(requests, sents, "`sent` 与 `request` 同 id（同一笔账）");
        assert_eq!(requests, answers, "应答与 `request` 同 id（同一笔账）");
        assert_ne!(requests[0], requests[1], "两次往返两个 id（撞了就是一次）");
        assert!(
            model.iter().all(|e| e.turn == 0),
            "模型端口的事实记在**本轮**上（不是全局）：{model:?}"
        );
        assert!(
            model.iter().all(|e| e.produced_by == Some(0)),
            "协议事实带溯源（本轮用户格）：{model:?}"
        );
    }

    /// 注入的消息必须**进本轮的下一次请求**。
    ///
    /// ## 为什么 core 层只验这一半
    ///
    /// 另一半——「注入的内容成为事实、下一轮仍可见」——**不在 core**：
    /// 「什么是补充」「它怎么落成 `turn × asserted`」是插件侧的概念（收件箱、
    /// `merge_supplements`、`EventWalStore` 都在 `crates/symbio-plugin-session`），core 只提供
    /// 「循环里什么时候问一次」这个挂点（见 [`TurnInput::inject`]）。
    ///
    /// 把落格也做进 core 会让 core 知道「补充」是什么——而它不必知道。
    /// 那一半的判据在 `turn_exec.test.rs`（`supplements_become_facts_visible_next_turn`）。
    ///
    /// ## 反向自检
    ///
    /// 把 `land_batch` 里那两行注入（`if let Some(inject) = &inject { exchange.extend(inject()); }`）
    /// 删掉，本用例必红。
    #[tokio::test]
    async fn injected_messages_reach_the_next_request_of_the_same_turn() {
        let store: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>> =
            std::sync::Arc::new(EventStore::new());
        let tok = TokenIssuer::issue_deep();
        let llm = std::sync::Arc::new(ToolCallingLlm::new());
        let prompts = llm.prompts.clone();
        let dispatch = std::sync::Arc::new(FakeDispatch {
            pending: false,
            rounds: Arc::new(Mutex::new(0)),
        });

        // 注入口：只返回一次（模拟「用户在工具跑的时候插了一句」）。
        let fired = Arc::new(Mutex::new(0usize));
        let inject = {
            let fired = fired.clone();
            Arc::new(
                move |_anchor: Option<u64>| -> std::pin::Pin<
                    Box<dyn std::future::Future<Output = Vec<PromptMessage>> + Send>,
                > {
                    let fired = fired.clone();
                    Box::pin(async move {
                        let mut n = fired.lock().unwrap();
                        if *n > 0 {
                            return Vec::new();
                        }
                        *n += 1;
                        vec![PromptMessage {
                            role: "user".into(),
                            text: "顺便也看看 README".into(),
                            tool_call_id: None,
                            tool: None,
                            tool_calls: None,
                        }]
                    })
                },
            ) as RoundInjector
        };

        let mut i = input(store.as_ref());
        i.inject = Some(inject);
        TurnRunner
            .drive_with_tools(
                &store,
                llm.clone(),
                tok,
                i,
                Arc::new(SilentDeltas),
                std::sync::Arc::from(tools()),
                Some(dispatch.clone()),
            )
            .await
            .expect("工具轮必答");

        let seen: Vec<Seen> = prompts
            .lock()
            .unwrap()
            .clone()
            .into_iter()
            .map(Seen::from)
            .collect();
        assert_eq!(seen.len(), 2, "工具轮 + 收尾轮：{seen:?}");
        // 第一次请求里还没有（那时注入口还没被问）。
        assert!(
            !seen[0]
                .texts_of("user")
                .iter()
                .any(|t| t.contains("README")),
            "第一次请求还没有注入内容：{seen:?}"
        );
        // 第二次请求里有了，且**作为独立的 user 消息**——不与别的 user 消息合并
        // （合并会让模型分不清哪句是本轮提问、哪句是插话）。
        assert_eq!(
            seen[1]
                .texts_of("user")
                .into_iter()
                .filter(|t| t.contains("README"))
                .count(),
            1,
            "注入的消息必须在**本轮的下一次请求**里，且是独立一条 user 消息：{seen:?}"
        );
    }

    /// 假适配器：**直接作答**（无工具调用）——续写轮的收尾轮；记录收到的**消息数组**。
    struct AnsweringLlm {
        prompts: Arc<Mutex<Vec<Vec<PromptMessage>>>>,
    }

    impl AnsweringLlm {
        fn new() -> Self {
            Self {
                prompts: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    #[async_trait::async_trait]
    impl LlmAdapter for AnsweringLlm {
        fn model_id(&self) -> &str {
            "answer-mock"
        }
        async fn generate(
            &self,
            _tok: &FullModel,
            _messages: &[PromptMessage],
        ) -> Result<String, AdapterError> {
            Err(AdapterError::GenerationFailed("unused".into()))
        }
        async fn generate_turn(
            &self,
            _tok: &FullModel,
            messages: &[PromptMessage],
            _tools: &[CapabilityMeta],
            _sink: Arc<dyn DeltaSink>,
        ) -> Result<LlmTurn, AdapterError> {
            self.prompts.lock().unwrap().push(messages.to_vec());
            Ok(LlmTurn {
                text: "批准后已完成。".into(),
                reasoning: String::new(),
                tool_calls: Vec::new(),
                finish: ModelFinishReason::Stop,
                cost_ms: 5,
                usage: Some(symbio_core::ModelUsage {
                    input: Some(11),
                    output: Some(22),
                }),
                spend: None,
            })
        }
    }

    /// 假适配器：把**收到的最后一条消息**当答复（`run` 系列会把答复反射进事实源，
    /// 便于断言「历史里有哪几轮」）；记录每次收到的**消息数组**。
    ///
    /// ## 为什么记录消息而不是 prompt 字符串（D-048a）
    ///
    /// 断言要看**角色**有没有丢——把消息拍回文本再解析，等于把拍平塞回测试里，而
    /// 测试是唯一能照出「role 有没有丢」的地方。所以这里记 `Vec<PromptMessage>`，
    /// 断言直接看角色。
    struct EchoLlm {
        prompts: Arc<Mutex<Vec<Vec<PromptMessage>>>>,
    }

    impl EchoLlm {
        fn new() -> Self {
            Self {
                prompts: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    #[async_trait::async_trait]
    impl LlmAdapter for EchoLlm {
        fn model_id(&self) -> &str {
            "echo-mock"
        }
        async fn generate(
            &self,
            _tok: &FullModel,
            _messages: &[PromptMessage],
        ) -> Result<String, AdapterError> {
            Err(AdapterError::GenerationFailed("unused".into()))
        }
        async fn generate_turn(
            &self,
            _tok: &FullModel,
            messages: &[PromptMessage],
            _tools: &[CapabilityMeta],
            _sink: Arc<dyn DeltaSink>,
        ) -> Result<LlmTurn, AdapterError> {
            self.prompts.lock().unwrap().push(messages.to_vec());
            // 末条即当前轮发言（结构化之后没有「行」，只有消息序列）
            let echo = messages
                .last()
                .map(|m| m.text.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "答".to_string());
            Ok(LlmTurn {
                text: echo,
                reasoning: String::new(),
                tool_calls: Vec::new(),
                finish: ModelFinishReason::Stop,
                cost_ms: 1,
                usage: None,
                spend: None,
            })
        }
    }

    /// 一次请求的**断言视图**：保留结构，同时给断言一个「拼起来看」的入口。
    ///
    /// ## 为什么不让断言直接读 `Vec<PromptMessage>`
    ///
    /// 因为那样每个断言都要写 `m.iter().filter(|m| m.role == "user").map(...)`，
    /// 而**真正要断言的往往不是角色**（例如「历史里有第 0 轮的问题」）。所以给
    /// 两条路：按角色精确查（`texts_of`）、按全文查（`joined`）。
    ///
    /// `joined` 是**给人看的**，不是给生产路径用的——它存在的唯一理由是让「某句话
    /// 在不在」这类断言保持简短。它**不参与**任何 role 断言，所以「role 有没有丢」
    /// 仍然只能靠 `texts_of` / `roles` 照出来。
    ///
    /// ⚠️ 如果哪天发现自己在用 `joined` 断言**角色相关**的东西，那是判据退化了——
    /// 拍平会重新溜回测试里。正确做法是加一条 `roles` 断言。
    #[derive(Debug, Clone, Default)]
    struct Seen {
        msgs: Vec<PromptMessage>,
    }

    impl Seen {
        /// 角色序列（断言 role 有没有丢 / 顺序对不对用它）。
        fn roles(&self) -> Vec<&str> {
            self.msgs.iter().map(|m| m.role.as_str()).collect()
        }

        /// 角色为 `r` 的那些正文。
        fn texts_of(&self, r: &str) -> Vec<&str> {
            self.msgs
                .iter()
                .filter(|m| m.role == r)
                .map(|m| m.text.as_str())
                .collect()
        }

        /// 角色为 `tool` 的消息：正文 + 工具名 + `tool_call_id`。
        fn tools(&self) -> Vec<(&str, Option<&str>, Option<&str>)> {
            self.msgs
                .iter()
                .filter(|m| m.role == "tool")
                .map(|m| {
                    (
                        m.text.as_str(),
                        m.tool.as_deref(),
                        m.tool_call_id.as_deref(),
                    )
                })
                .collect()
        }

        /// 全部正文按顺序拼起来（**仅**给「某句话在不在」这类断言用）。
        fn joined(&self) -> String {
            self.msgs
                .iter()
                .map(|m| m.text.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        }
    }

    impl From<Vec<PromptMessage>> for Seen {
        fn from(msgs: Vec<PromptMessage>) -> Self {
            Seen { msgs }
        }
    }

    /// 断言消息里**含**某段话（不钉角色）——给「历史里有第 N 轮的问题」这类。
    macro_rules! says {
        ($seen:expr, $needle:expr) => {
            $seen.joined().contains($needle)
        };
    }

    /// 假适配器：首次请求回一个工具调用，之后回正文；记录每次收到的**消息数组**。
    struct ToolCallingLlm {
        prompts: Arc<Mutex<Vec<Vec<PromptMessage>>>>,
    }

    impl ToolCallingLlm {
        fn new() -> Self {
            Self {
                prompts: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    #[async_trait::async_trait]
    impl LlmAdapter for ToolCallingLlm {
        fn model_id(&self) -> &str {
            "tool-mock"
        }
        async fn generate(
            &self,
            _tok: &FullModel,
            _messages: &[PromptMessage],
        ) -> Result<String, AdapterError> {
            // 本用例只走工具通道（`generate_turn`）——留一个诚实的不支持。
            Err(AdapterError::GenerationFailed("unused".into()))
        }
        async fn generate_turn(
            &self,
            _tok: &FullModel,
            messages: &[PromptMessage],
            _tools: &[CapabilityMeta],
            _sink: Arc<dyn DeltaSink>,
        ) -> Result<LlmTurn, AdapterError> {
            let round = {
                let mut p = self.prompts.lock().unwrap();
                p.push(messages.to_vec());
                p.len()
            };
            if round == 1 {
                Ok(LlmTurn {
                    text: "我先查一下。".into(),
                    reasoning: String::new(),
                    tool_calls: vec![TurnToolCallInfo {
                        id: Some("tc-1".into()),
                        wire_id: Some("w-1".into()),
                        name: Some("vdfs_read".into()),
                        arguments: serde_json::json!({ "path": "a.md" }),
                        parse_error: None,
                        overflow_notice: None,
                    }],
                    finish: ModelFinishReason::Stop,
                    cost_ms: 3,
                    usage: None,
                    spend: None,
                })
            } else {
                Ok(LlmTurn {
                    text: "读到了。".into(),
                    reasoning: String::new(),
                    tool_calls: Vec::new(),
                    finish: ModelFinishReason::Stop,
                    cost_ms: 4,
                    usage: None,
                    spend: None,
                })
            }
        }
    }

    /// 本批的**事实面口径**：最新一条 `response(port=model)` 的轮号、溯源与
    /// `calls` 里的端口 id。
    ///
    /// 替身必须按这条应答写 `request`：`open-tools` 行的展开读的就是 `calls`，
    /// 重算一个别的 id 会让创建债永远还不掉（派发器在同一笔债上空转到步数上界）。
    fn model_batch(store: &dyn Store<Event = Event>) -> Option<(u64, u64, Vec<String>)> {
        let facts = store.range(Seq::new(0));
        let e = facts.iter().rev().find(|e| {
            e.kind == symbio_core::EVENT_RESPONSE
                && e.payload.get("port").and_then(|v| v.as_str()) == Some("model")
        })?;
        let ids = e
            .payload
            .get("calls")?
            .as_array()?
            .iter()
            .map(|c| {
                c.get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string()
            })
            .collect();
        Some((e.turn, e.produced_by.unwrap_or(0), ids))
    }

    /// 替身落协议事实：幂等键命中按「已经落过」处理，其余错误让用例红。
    fn append_fact(store: &dyn Store<Event = Event>, event: Event) {
        match store.append(event) {
            Ok(_) | Err(symbio_core::AppendError::Duplicate) => {}
            Err(e) => panic!("替身协议事实落格失败：{e:?}"),
        }
    }

    /// 替身按生产口径开账：逐条落 `request(port=tool)`，id 取模型应答 `calls` 里那一笔。
    ///
    /// 行表按事实判欠不欠：替身不落账，`open-tools` 就永远挨不到派发（b）。
    fn open_batch(store: &dyn Store<Event = Event>, turn: &LlmTurn) -> Vec<ToolPortCall> {
        let Some((turn_no, produced_by, ids)) = model_batch(store) else {
            return Vec::new();
        };
        turn.tool_calls
            .iter()
            .enumerate()
            .map(|(idx, tc)| {
                let id = ids
                    .get(idx)
                    .cloned()
                    .unwrap_or_else(|| format!("tool-{turn_no}-c{idx}"));
                let call = super::super::super::port::Call::new("tool", id, 1);
                append_fact(
                    store,
                    Event::pending(
                        format!("request-tool-{}-{}", call.id, call.attempt),
                        "request",
                        Entity::Turn,
                        Verb::Asserted,
                        turn_no,
                        "system",
                    )
                    .with_produced_by(produced_by)
                    .with_payload(serde_json::json!({
                        "port": "tool",
                        "id": call.id,
                        "attempt": call.attempt,
                    })),
                );
                ToolPortCall {
                    model_id: tc.id.clone().unwrap_or_default(),
                    call,
                    name: tc.name.clone().unwrap_or_default(),
                    canonical: None,
                    risk: None,
                    approved: false,
                    invalid: false,
                    granted: false,
                    deny_why: None,
                    sent: false,
                    wake: None,
                }
            })
            .collect()
    }

    /// 替身把一批调用全部放行，并落 `authz.granted`。
    fn authorize_batch(store: &dyn Store<Event = Event>, calls: &mut [ToolPortCall]) {
        let Some((turn_no, produced_by, _)) = model_batch(store) else {
            return;
        };
        for call in calls.iter_mut() {
            let id = call.call.id.clone();
            append_fact(
                store,
                Event::pending(
                    format!("grant-{turn_no}-{id}"),
                    EVENT_AUTHZ_GRANTED,
                    Entity::Turn,
                    Verb::Asserted,
                    turn_no,
                    "authorizer",
                )
                .with_produced_by(produced_by)
                .with_payload(serde_json::json!({
                    "key": id,
                    "port": "tool",
                    "id": id,
                    // 与生产同键（D-138）：授权关同一**次尝试**。
                    "attempt": call.call.attempt,
                    "tool": call.name,
                    "risk": "unknown",
                    "deferred": true,
                })),
            );
            call.granted = true;
        }
    }

    /// 替身逐条落 `sent`（`port::begin` 派发前自己读 `dispatch-tools` 这一行）。
    fn begin_batch(
        store: &dyn Store<Event = Event>,
        calls: &mut [ToolPortCall],
    ) -> Option<(u64, u64)> {
        let (turn_no, produced_by, _) = model_batch(store)?;
        for call in calls.iter_mut().filter(|c| c.granted) {
            if super::super::super::port::begin(
                store,
                super::super::super::loop_rows::PORTS,
                super::super::super::port::Dispatch {
                    call: &call.call,
                    turn: turn_no,
                    produced_by,
                    row: "dispatch-tools",
                    now: 0,
                    awaited: call.wake.is_some(),
                },
            )
            .is_ok()
            {
                call.sent = true;
            }
        }
        Some((turn_no, produced_by))
    }

    /// 替身逐条落 `response`（`port::succeed`）。
    ///
    /// 轮号与溯源从**日志**取（`model_batch`）：收尾半与派出半分处两个动作，拿不到
    /// 派出那一刻的局部值——而这两个值本来就在 `response(port=model)` 上，读它比
    /// 从派出半偷偷传下来更省一段交接件（替身不该凭空多出生产的形状）。
    fn succeed_batch(store: &dyn Store<Event = Event>, calls: &[ToolPortCall]) {
        let Some((turn_no, produced_by, _)) = model_batch(store) else {
            return;
        };
        for call in calls.iter().filter(|c| c.granted && c.sent) {
            if let Err(e) =
                super::super::super::port::succeed(store, &call.call, turn_no, produced_by)
            {
                panic!("替身应答落格失败：{e:?}");
            }
        }
    }

    /// 假分发方：把工具调用映射成一条结果事实；`pending` 决定是否收束于等用户。
    ///
    /// 它按生产口径落协议事实（`request` → `authz.granted` → `sent` → `response`）：
    /// 行表按事实判欠不欠，替身不落账，创建行就永远挨不到派发（b）。
    struct FakeDispatch {
        pending: bool,
        rounds: Arc<Mutex<usize>>,
    }

    #[async_trait::async_trait]
    impl ToolDispatch for FakeDispatch {
        async fn open(
            &self,
            store: &dyn Store<Event = Event>,
            turn: &LlmTurn,
        ) -> Vec<ToolPortCall> {
            open_batch(store, turn)
        }

        async fn authorize(&self, store: &dyn Store<Event = Event>, calls: &mut [ToolPortCall]) {
            authorize_batch(store, calls);
        }

        async fn begin(
            self: Arc<Self>,
            store: Arc<dyn Store<Event = Event>>,
            turn: &LlmTurn,
            calls: &mut [ToolPortCall],
        ) -> ToolBegin {
            *self.rounds.lock().unwrap() += 1;
            if begin_batch(store.as_ref(), calls).is_none() {
                return ToolBegin::None;
            }
            let outcomes: Vec<DispatchOutcome> = turn
                .tool_calls
                .iter()
                .map(|tc| DispatchOutcome {
                    call_id: tc.id.clone().unwrap_or_default(),
                    name: tc.name.clone().unwrap_or_default(),
                    text: "文件内容：hello".into(),
                    ok: true,
                    needs_user_action: self.pending,
                    // 等待用户时的卡片载荷与原因码：假分发方建的是 `user_prompt` 节点，
                    // 这里给一份最小可辨形状（真值由 `tool_dispatch` 从节点 `meta` 取）。
                    prompt: if self.pending {
                        Some(serde_json::json!({ "question": "选哪个" }))
                    } else {
                        None
                    },
                    failure_kind: if self.pending {
                        "needs_interaction".into()
                    } else {
                        String::new()
                    },
                    // 节点身份与参数随结果入格（载荷补全）；假分发方不建结果节点
                    // （`result_id` 空串 = 无结果子节点）。
                    wire_id: tc.wire_id.clone().unwrap_or_default(),
                    args: symbio_core::node_build::tool_call_args_text(tc),
                    result_id: String::new(),
                    annotations: Vec::new(),
                })
                .collect();
            ToolBegin::InFlight(Box::pin(async move { outcomes }))
        }

        fn finish(
            &self,
            store: &dyn Store<Event = Event>,
            calls: &[ToolPortCall],
            _outcomes: &[DispatchOutcome],
        ) {
            succeed_batch(store, calls);
        }
    }

    /// 唤醒幕的假分发方：认领**开着**的唤醒请求（`request(port=tool)`），按行表的
    /// 两行（授权 → 派发）把它走完。
    ///
    /// 读回那一笔用的是**生产**的读侧（[`ToolPort::wake_batch`]）：唤醒载荷的形状
    /// 不在替身里复述一遍（复述即第二处真相——改了一处，另一处还认旧形状）。
    struct WakeDispatch {
        store: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>>,
        turn: u64,
        user_seq: u64,
        rounds: Arc<Mutex<usize>>,
    }

    impl WakeDispatch {
        fn port(&self) -> ToolPort {
            let facts: Arc<dyn Store<Event = Event>> = self.store.clone();
            ToolPort::new(facts, self.turn, self.user_seq, 0)
        }
    }

    #[async_trait::async_trait]
    impl ToolDispatch for WakeDispatch {
        async fn open(
            &self,
            _store: &dyn Store<Event = Event>,
            _turn: &LlmTurn,
        ) -> Vec<ToolPortCall> {
            // 唤醒幕没有模型应答可寄存。
            Vec::new()
        }

        async fn authorize(&self, _store: &dyn Store<Event = Event>, calls: &mut [ToolPortCall]) {
            if !calls.is_empty() {
                return;
            }
            // 草稿里没有模型批次 ⇒ 认领开幕前列队的那一笔唤醒请求。
            let Some(mut batch) = self.port().wake_batch() else {
                return;
            };
            // 用户已经做过判断 ⇒ 放行（授权事实按 `(port, id)` 绑定，与生产同键）。
            for call in batch.calls.iter_mut() {
                let id = call.call.id.clone();
                append_fact(
                    self.store.as_ref(),
                    Event::pending(
                        format!("grant-{}-{}", self.turn, id),
                        EVENT_AUTHZ_GRANTED,
                        Entity::Turn,
                        Verb::Asserted,
                        self.turn,
                        "authorizer",
                    )
                    .with_produced_by(self.user_seq)
                    .with_payload(serde_json::json!({
                        "key": id,
                        "port": "tool",
                        "id": id,
                        // 与生产同键（D-138）：授权关同一**次尝试**。
                        "attempt": call.call.attempt,
                        "tool": call.name,
                        "risk": "unknown",
                        "deferred": true,
                    })),
                );
                call.granted = true;
            }
        }

        async fn begin(
            self: Arc<Self>,
            _store: Arc<dyn Store<Event = Event>>,
            turn: &LlmTurn,
            calls: &mut [ToolPortCall],
        ) -> ToolBegin {
            *self.rounds.lock().unwrap() += 1;
            // 派发前落 `sent`（端口账第三笔：派出）。
            for call in calls.iter_mut().filter(|c| c.granted) {
                if super::super::super::port::begin(
                    self.store.as_ref(),
                    super::super::super::loop_rows::PORTS,
                    super::super::super::port::Dispatch {
                        call: &call.call,
                        turn: self.turn,
                        produced_by: self.user_seq,
                        row: "dispatch-tools",
                        now: 0,
                        awaited: call.wake.is_some(),
                    },
                )
                .is_ok()
                {
                    call.sent = true;
                }
            }
            let outcomes: Vec<DispatchOutcome> = turn
                .tool_calls
                .iter()
                .map(|tc| DispatchOutcome {
                    call_id: tc.id.clone().unwrap_or_default(),
                    name: tc.name.clone().unwrap_or_default(),
                    text: "文件内容：hello（已批准）".into(),
                    ok: true,
                    needs_user_action: false,
                    prompt: None,
                    failure_kind: String::new(),
                    wire_id: tc.wire_id.clone().unwrap_or_default(),
                    args: symbio_core::node_build::tool_call_args_text(tc),
                    result_id: String::new(),
                    annotations: Vec::new(),
                })
                .collect();
            ToolBegin::InFlight(Box::pin(async move { outcomes }))
        }

        fn finish(
            &self,
            _store: &dyn Store<Event = Event>,
            calls: &[ToolPortCall],
            _outcomes: &[DispatchOutcome],
        ) {
            // 关账：工具回了话 ⇒ `response`。
            for call in calls.iter().filter(|c| c.granted && c.sent) {
                super::super::super::port::succeed(
                    self.store.as_ref(),
                    &call.call,
                    self.turn,
                    self.user_seq,
                )
                .expect("唤醒应答落格");
            }
        }

        async fn wake(&self, _store: &dyn Store<Event = Event>) -> Option<WakeBatch> {
            self.port().wake_batch()
        }
    }

    /// 全批拒绝的假分发方：证明 `deny-tools` 那一行真的被选中并跑到执行层，
    /// 且被拒的 `request` 以 `failed{why}` 关账（b）。
    struct DenyDispatch {
        rounds: Arc<Mutex<usize>>,
    }

    #[async_trait::async_trait]
    impl ToolDispatch for DenyDispatch {
        async fn open(
            &self,
            store: &dyn Store<Event = Event>,
            turn: &LlmTurn,
        ) -> Vec<ToolPortCall> {
            open_batch(store, turn)
        }

        async fn authorize(&self, store: &dyn Store<Event = Event>, calls: &mut [ToolPortCall]) {
            let Some((turn_no, produced_by, _)) = model_batch(store) else {
                return;
            };
            for call in calls.iter_mut() {
                let id = call.call.id.clone();
                append_fact(
                    store,
                    Event::pending(
                        format!("deny-{turn_no}-{id}"),
                        symbio_plugin_api::events::EVENT_AUTHZ_DENIED,
                        Entity::Turn,
                        Verb::Asserted,
                        turn_no,
                        "authorizer",
                    )
                    .with_produced_by(produced_by)
                    .with_payload(serde_json::json!({
                        "key": id,
                        "port": "tool",
                        "id": id,
                        // 与生产同键（D-138）：拒绝也关同一**次尝试**。
                        "attempt": call.call.attempt,
                        "tool": call.name,
                        "kind": "tool",
                        "reason": "denied",
                        "detail": "needs_approval",
                    })),
                );
                call.deny_why = Some("needs_approval");
            }
        }

        async fn begin(
            self: Arc<Self>,
            store: Arc<dyn Store<Event = Event>>,
            turn: &LlmTurn,
            calls: &mut [ToolPortCall],
        ) -> ToolBegin {
            *self.rounds.lock().unwrap() += 1;
            let Some((turn_no, produced_by, _)) = model_batch(store.as_ref()) else {
                return ToolBegin::None;
            };
            for call in calls.iter().filter(|c| !c.granted) {
                let why = call.deny_why.unwrap_or("denied");
                if let Err(e) = super::super::super::port::failed(
                    store.as_ref(),
                    &call.call,
                    turn_no,
                    produced_by,
                    why,
                ) {
                    panic!("替身拒绝落格失败：{e:?}");
                }
            }
            let outcomes: Vec<DispatchOutcome> = turn
                .tool_calls
                .iter()
                .map(|tc| DispatchOutcome {
                    call_id: tc.id.clone().unwrap_or_default(),
                    name: tc.name.clone().unwrap_or_default(),
                    text: "工具被拒绝：needs_approval".into(),
                    ok: false,
                    needs_user_action: false,
                    prompt: None,
                    failure_kind: "denied".into(),
                    wire_id: tc.wire_id.clone().unwrap_or_default(),
                    args: symbio_core::node_build::tool_call_args_text(tc),
                    result_id: String::new(),
                    annotations: Vec::new(),
                })
                .collect();
            ToolBegin::InFlight(Box::pin(async move { outcomes }))
        }

        fn finish(
            &self,
            _store: &dyn Store<Event = Event>,
            _calls: &[ToolPortCall],
            _outcomes: &[DispatchOutcome],
        ) {
            // 拒绝的 `failed` 在派出半就落了（那一批没有 `sent` 要等，`deny-tools`
            // 的出口也就是它）；收尾半没有第二件事。
        }
    }

    fn tools() -> Vec<CapabilityMeta> {
        vec![CapabilityMeta {
            name: "vdfs_read".into(),
            ..Default::default()
        }]
    }

    fn input(store: &dyn Store<Event = Event>) -> TurnInput {
        input_at(store, 0, "读 a.md")
    }

    /// 第 `turn` 轮的入参（测试侧先落该轮用户格）。
    fn input_at(store: &dyn Store<Event = Event>, turn: u64, text: &str) -> TurnInput {
        let user_seq = super::seed_user(store, turn, text);
        TurnInput {
            turn,
            user_seq,
            drive_seq: user_seq,
            ..input_base()
        }
    }

    fn input_base() -> TurnInput {
        TurnInput {
            turn: 0,
            user_seq: 0,
            drive_seq: 0,
            inject: None,
            window_turns: None,
            actor: trivial_actor_spec("session/main"),
            // 请求级前缀：测试不走请求视图层（三段皆空）。
            final_reply: None,
            final_cost_ms: 0,
            turn_root_id: String::new(),
            dialog: None,
            nodes: None,
            abort: None,
        }
    }

    /// 定稿答话轮：`final_reply` 给定时**不调模型**，但两格照落。
    ///
    /// ## 为什么要钉「两格都在」
    ///
    /// `Answered` 那一轮若不进运行器，事实网格里就一格都没有。下一轮
    /// prompt 从网格投影 ⇒ 这次问答整个消失，而 `messages.json` 里有 ⇒ **两条真源**。
    /// 判据钉的是「用户格 + 收束格都在、且收束格溯源指向用户格」，不是「答话文本对不对」。
    ///
    /// ## 为什么要钉「不调模型」
    ///
    /// 这一轮一次模型往返都不该发——那是 `Answered` 的全部意义。用记录型桩的 `prompts`
    /// 长度来钉：它长了就说明请求发了出去。
    ///
    /// **反向自检**：把 `prepare_turn` 里 `final_reply` 那段短路删掉，本用例必红（桩会被调到）。
    #[tokio::test]
    async fn final_reply_lands_both_cells_without_calling_the_model() {
        let store: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>> =
            std::sync::Arc::new(EventStore::new());
        let tok = TokenIssuer::issue_deep();
        let llm = std::sync::Arc::new(ToolCallingLlm::new());
        let prompts = llm.prompts.clone();
        let mut i = input(store.as_ref());
        i.final_reply = Some("北京今天晴。".into());
        i.final_cost_ms = 1234;
        let out = TurnRunner
            .drive_with_tools(
                &store,
                llm.clone(),
                tok,
                i,
                Arc::new(SilentDeltas),
                std::sync::Arc::from(Vec::new()),
                None,
            )
            .await
            .unwrap();
        assert_eq!(out.text, "北京今天晴。", "答话就是这一轮的输出");
        assert!(!out.fell_back, "定稿答话不是兜底");
        assert_eq!(
            out.cost_ms, 1234,
            "措辞往返的实测耗时随结局返回（台账 / 读数的输入）"
        );
        assert!(
            prompts.lock().unwrap().is_empty(),
            "定稿答话轮一次模型请求都不该发"
        );
        let ev = store.range(Seq::new(0));
        assert_eq!(
            super::kinds(&ev),
            vec![
                "user.message",
                "turn.stage",
                "turn.dispatch",
                "chat.assistant.final"
            ],
            "用户格 + 接缝出口 + 派发格 + 收束格，一格不多一格不少"
        );
        assert_eq!(
            ev[3].produced_by,
            ev[0].seq.map(|s| s.value()),
            "I2：收束格的溯源必须指向本轮用户格"
        );
        assert_eq!(
            ev[3].cost_ms, 1234,
            "收束格带这次措辞往返的实测 cost_ms（不是恒 0）"
        );
    }

    /// 请求级前缀排在基线**之前**，且**不进**「就地累积」那条路。
    ///
    /// ## 为什么要钉「在之前」而不是只钉「在里面」
    ///
    /// 三段（记忆召回 / 就绪任务集 / 委派者真源）语义上在对话**之前**（置顶）。
    /// 若前缀被追加到基线末尾（本轮发言之后），模型就是先答后看指令——
    /// 而「模型有没有照着记忆段作答」这件事，网格与不变量都证明不了，只有回读
    /// prompt 位置能证。
    ///
    /// ## 为什么要钉「不进就地累积」
    ///
    /// 就地累积（`exchange`）是**本轮**的工具交换追加位。前缀若走那里，每轮重算的
    /// 上一轮前缀会残留在下一轮的上下文里——同一段记忆段被反复注入，且越滚越长。
    ///
    /// **反向自检**：把 `base_prompt` 的前置改成后置（拼到 `exchange` 之后），本用例必须红。
    #[tokio::test]
    async fn request_prefix_precedes_the_baseline_and_is_not_accumulated() {
        let store: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>> =
            std::sync::Arc::new(EventStore::new());
        let tok = TokenIssuer::issue_deep();
        let llm = std::sync::Arc::new(ToolCallingLlm::new());
        let prompts = llm.prompts.clone();
        let dispatch = std::sync::Arc::new(FakeDispatch {
            pending: false,
            rounds: Arc::new(Mutex::new(0)),
        });
        let i = input(store.as_ref());

        TurnRunner
            .drive_with_actors(
                &store,
                llm.clone(),
                tok,
                i,
                Arc::new(SilentDeltas),
                std::sync::Arc::from(tools()),
                Some(dispatch.clone()),
                super::super::super::loop_rows::ACTORS,
                // 前缀是**备输入的产物**（`D-222` 订正 3）：生产里由
                // `prepare_request_sections` 的 ⑥ 收集，草稿建好之后填。
                Some("【长期记忆】\n- 偏好：简洁".to_string()),
                None,
            )
            .await
            .expect("必答");

        let raw = prompts.lock().unwrap().clone();
        assert_eq!(raw.len(), 2, "工具轮 + 收尾轮：{raw:?}");
        for (i, msgs) in raw.iter().enumerate() {
            let s = Seen::from(msgs.clone());
            // 结构化之后「位置」是**下标**，不是字符偏移——这比 `find` 强：
            // 前缀混进发言正文那种退化，在下标上直接看得见（不再是同一条消息）。
            let prefix_at = msgs
                .iter()
                .position(|m| m.text.contains("【长期记忆】"))
                .expect("前缀两轮都在");
            let spoken_at = msgs
                .iter()
                .position(|m| m.text.contains("读 a.md"))
                .expect("本轮发言两轮都在");
            assert_eq!(
                prefix_at,
                0,
                "第 {} 轮：前缀须是**最前一条独立消息**（{s:?}）",
                i + 1
            );
            assert!(
                prefix_at < spoken_at,
                "第 {} 轮：前缀须在发言之前（{s:?}）",
                i + 1
            );
            // 工具交换在基线**之后**：前缀不得被它挤到中间
            if let Some(call_at) = msgs.iter().position(|m| m.text.contains("调用工具")) {
                assert!(
                    prefix_at < call_at,
                    "第 {} 轮：前缀须在工具交换之前（{s:?}）",
                    i + 1
                );
            }
        }
        // 不累积：两轮各**一份**前缀，不是两份
        assert_eq!(
            raw.iter()
                .map(|msgs| msgs
                    .iter()
                    .filter(|m| m.text.contains("【长期记忆】"))
                    .count())
                .sum::<usize>(),
            2,
            "前缀每轮一份，不得跨轮累积：{raw:?}"
        );
        // `None` / 空串 = 不加（平凡值必须真的什么都不加）
        for empty in [None, Some(String::new()), Some("   \n ".to_string())] {
            let store: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>> =
                std::sync::Arc::new(EventStore::new());
            let llm = std::sync::Arc::new(ToolCallingLlm::new());
            let prompts = llm.prompts.clone();
            let i = input(store.as_ref());
            TurnRunner
                .drive_with_actors(
                    &store,
                    llm.clone(),
                    tok,
                    i,
                    Arc::new(SilentDeltas),
                    std::sync::Arc::from(Vec::new()),
                    None,
                    super::super::super::loop_rows::ACTORS,
                    empty,
                    None,
                )
                .await
                .expect("必答");
            let p = prompts.lock().unwrap().clone();
            assert!(
                !p[0][0].text.starts_with('\n') && says!(Seen::from(p[0].clone()), "读 a.md"),
                "空前缀不得留下空壳或多余换行：{:?}",
                p[0]
            );
        }
    }

    /// `window_turns = Some(0)` = **不截断**，与 `SessionConfig::context_messages`
    /// 的 `0` 同口径。
    ///
    /// ## 回归的是什么
    ///
    /// `window_by_turn` 的 `keep == 0` 语义是**不按轮次截断**（返回全量），不是
    /// **历史全丢**（返回空切片）——同一个配置数只有这一种含义。返回空切片时，
    /// 任何把 `context_messages` 配成 `0` 的实例，
    /// **模型眼前的历史是空的**——静默、无告警、事实照样入格、`session/stats` 照样有数。
    ///
    /// **反向自检**：把 `keep == 0` 分支改成 `&events[events.len()..]`，本用例必须红。
    #[tokio::test]
    async fn zero_window_means_no_truncation_not_empty_history() {
        let store: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>> =
            std::sync::Arc::new(EventStore::new());
        let tok = TokenIssuer::issue_deep();
        let llm = std::sync::Arc::new(EchoLlm::new());
        let prompts = llm.prompts.clone();

        for turn in 0..3u64 {
            let mut i = input_at(store.as_ref(), turn, &format!("第 {turn} 轮问题"));
            i.window_turns = Some(0);
            TurnRunner
                .drive_with_tools(
                    &store,
                    llm.clone(),
                    tok,
                    i,
                    Arc::new(SilentDeltas),
                    std::sync::Arc::from(Vec::new()),
                    None,
                )
                .await
                .expect("必答");
        }

        let seen: Vec<Seen> = prompts
            .lock()
            .unwrap()
            .clone()
            .into_iter()
            .map(Seen::from)
            .collect();
        assert_eq!(seen.len(), 3, "三轮各一次请求：{seen:?}");
        let last = &seen[2];
        assert!(
            says!(last, "第 0 轮问题") && says!(last, "第 1 轮问题"),
            "`window_turns = 0` 不得清空历史：{last:?}"
        );
        assert!(says!(last, "第 2 轮问题"), "当前轮在历史之外：{last:?}");
        // 历史里的每一轮都必须是**独立**的 user 消息（不是一整段散文）
        assert_eq!(
            last.texts_of("user").len(),
            3,
            "三轮各一条 user 消息：{last:?}"
        );

        // 对照：`Some(1)` 只留当前轮（窗口**确实**在裁）
        let store: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>> =
            std::sync::Arc::new(EventStore::new());
        let llm = std::sync::Arc::new(EchoLlm::new());
        let prompts = llm.prompts.clone();
        for turn in 0..3u64 {
            let mut i = input_at(store.as_ref(), turn, &format!("第 {turn} 轮问题"));
            i.window_turns = Some(1);
            TurnRunner
                .drive_with_tools(
                    &store,
                    llm.clone(),
                    tok,
                    i,
                    Arc::new(SilentDeltas),
                    std::sync::Arc::from(Vec::new()),
                    None,
                )
                .await
                .expect("必答");
        }
        let last = Seen::from(prompts.lock().unwrap().clone().pop().unwrap());
        assert!(
            !says!(last, "第 0 轮问题")
                && !says!(last, "第 1 轮问题")
                && says!(last, "第 2 轮问题"),
            "`Some(1)` 只留当前轮，窗口本身仍在裁：{last:?}"
        );
    }

    /// **压缩快照不在窗口内也要带上**：它是当前历史的**基线**。
    ///
    /// ## 回归的是什么
    ///
    /// 归档之后没被取代的那段只剩「快照事件 + 压缩后的新事件」，而快照事件的 `turn`
    /// （按约定 `message.*` 一律 `0`）**不在窗口内** ⇒ 纯按轮切会把它切掉。后果是
    /// 模型**既看不见摘要、也看不见刚聊过什么**——被保留的近轮只在快照载荷里，
    /// 它们的原始事件已移进归档段。两者都静默（prompt 短了没有任何东西会红）。
    ///
    /// **反向自检**：把 `window_by_turn` 里 `latest_snapshot_index` 那段拿掉，本用例必红。
    #[tokio::test]
    async fn the_compaction_snapshot_is_always_in_the_window() {
        let store: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>> =
            std::sync::Arc::new(EventStore::new());
        let tok = TokenIssuer::issue_deep();
        let llm = std::sync::Arc::new(EchoLlm::new());
        let prompts = llm.prompts.clone();

        // ① 先落一格压缩快照：载荷 = 压缩后的整份列表（摘要 + 被保留的近轮）。
        store
            .append(
                Event::pending(
                    "snap-0",
                    EVENT_SESSION_CONSOLIDATED,
                    Entity::Message,
                    Verb::Progressed,
                    0,
                    "session/main",
                )
                .with_payload(serde_json::json!({
                    "messages": [
                        { "id": "snap-root", "role": "user", "type": "body",
                          "content": "[CONTEXT SNAPSHOT] 被压掉的历史摘要" },
                        { "id": "u-keep", "role": "user", "type": "body",
                          "content": "保留的近轮问题" },
                    ]
                })),
            )
            .expect("快照入格");

        // ② 其后的两轮真实事实。
        for turn in 0..2u64 {
            let mut i = input_at(store.as_ref(), turn, &format!("第 {turn} 轮问题"));
            i.window_turns = Some(0);
            TurnRunner
                .drive_with_tools(
                    &store,
                    llm.clone(),
                    tok,
                    i,
                    Arc::new(SilentDeltas),
                    std::sync::Arc::from(Vec::new()),
                    None,
                )
                .await
                .expect("必答");
        }

        // ③ 第 3 轮：窗口只留最近 1 轮（快照的 turn 不在窗口内）。
        let mut i = input_at(store.as_ref(), 2, "压缩之后的问题");
        i.window_turns = Some(1);
        TurnRunner
            .drive_with_tools(
                &store,
                llm.clone(),
                tok,
                i,
                Arc::new(SilentDeltas),
                std::sync::Arc::from(Vec::new()),
                None,
            )
            .await
            .expect("必答");

        let last = Seen::from(prompts.lock().unwrap().clone().pop().unwrap());
        assert!(
            says!(last, "[CONTEXT SNAPSHOT] 被压掉的历史摘要"),
            "快照必须在窗口内（它是当前历史的基线）：{last:?}"
        );
        assert!(
            says!(last, "保留的近轮问题"),
            "快照载荷里的近轮也在——它们的原始事件已移进归档段：{last:?}"
        );
        assert!(
            !says!(last, "第 0 轮问题"),
            "窗口仍裁更早的轮次（带快照不等于不裁）：{last:?}"
        );
    }

    /// 工具轮：产物落 `artifact × asserted` 且溯源指向本轮用户格；final 只落一次；
    /// **工具结果进了下一次请求**（回读 prompt——不变量与网格都证明不了这条）。
    #[tokio::test]
    async fn tool_round_lands_artifact_and_feeds_next_prompt() {
        let store: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>> =
            std::sync::Arc::new(EventStore::new());
        let tok = TokenIssuer::issue_deep();
        let llm = std::sync::Arc::new(ToolCallingLlm::new());
        let prompts = llm.prompts.clone();
        let dispatch = std::sync::Arc::new(FakeDispatch {
            pending: false,
            rounds: Arc::new(Mutex::new(0)),
        });

        let out = TurnRunner
            .drive_with_tools(
                &store,
                llm.clone(),
                tok,
                input(store.as_ref()),
                Arc::new(SilentDeltas),
                std::sync::Arc::from(tools()),
                Some(dispatch.clone()),
            )
            .await
            .expect("工具轮必答");

        assert_eq!(out.text, "读到了。");
        assert!(!out.fell_back && !out.aborted && !out.awaits_user);
        assert_eq!(out.cost_ms, 7, "跨轮累加实测耗时（3 + 4）");
        assert_eq!(*dispatch.rounds.lock().unwrap(), 1, "分发恰好一次");

        // 两次请求；第二次带上了模型请求过的工具与它的结果。
        let seen: Vec<Seen> = prompts
            .lock()
            .unwrap()
            .clone()
            .into_iter()
            .map(Seen::from)
            .collect();
        assert_eq!(seen.len(), 2, "工具轮 + 收尾轮：{seen:?}");
        assert!(
            !says!(seen[0], "文件内容"),
            "第一次请求还没有结果：{:?}",
            seen[0]
        );
        assert!(
            says!(seen[1], "vdfs_read") && says!(seen[1], "文件内容：hello"),
            "第二次请求要带上工具调用与结果：{:?}",
            seen[1]
        );

        // ── D-048a 的核心断言：结果带**角色**与 **tool_call_id** ──
        //
        // 拍成散文时这两样都没有——所以这条断言直接钉住「结果是不是真消息」。
        let tools_in_2nd = seen[1].tools();
        assert_eq!(
            tools_in_2nd.len(),
            1,
            "第二次请求里恰好一条 role=tool 消息：{:?}",
            seen[1]
        );
        let (text, tool, call_id) = tools_in_2nd[0];
        assert_eq!(text, "文件内容：hello", "工具结果正文");
        assert_eq!(tool, Some("vdfs_read"), "工具名是事实，必须单独成字段");
        assert_eq!(
            call_id,
            Some("w-1"),
            "role=tool 必须关联到那次调用（否则模型无法配对多次并发调用）。\
             id 取调用**自带的身份**（wire id 优先，与 v1 的请求构建同一口径），\
             不是按工具名合成的——合成名跨轮会撞（见 `prompt_call_id`）。"
        );
        // 且调用请求本身是一条 assistant 消息——不是散文里「助手请求工具:」那种
        // 人类不会说的话。
        assert!(
            seen[1]
                .texts_of("assistant")
                .iter()
                .any(|x| x.contains("调用工具 vdfs_read")),
            "工具调用请求应是 assistant 消息：{:?}",
            seen[1]
        );

        // 角色**序列**：工具结果必须夹在 assistant 的调用之后——这是一条独立判据
        // （不依赖正文里有没有某个词）。
        let roles = seen[1].roles();
        let tool_at = roles
            .iter()
            .position(|r| *r == "tool")
            .expect("有 role=tool");
        assert!(
            roles[..tool_at].contains(&"assistant"),
            "工具结果之前应有 assistant 的调用请求（角色序列）：{roles:?}"
        );
        assert!(
            roles.contains(&"tool"),
            "角色序列里要有 tool（角色序列）：{roles:?}"
        );

        // 网格：用户格 + 派发格 + **每次模型往返三格** + 工具批三笔债的协议格
        // + 产物格 + final 格（final 只一条——N3）。
        let snapshot = store.range(Seq::new(0));
        assert_eq!(
            super::kinds(&snapshot),
            vec![
                "user.message",
                // 接缝的出口格：轮首那几行跑完、任何请求之前的装配。
                "turn.stage",
                "turn.dispatch",
                "request",
                "sent",
                "response",
                // 工具批：创建（request）/ 授权（granted）/ 派发（sent + response）。
                "request",
                "authz.granted",
                "sent",
                "response",
                "artifact.added",
                "request",
                "sent",
                "response",
                "chat.assistant.final",
            ],
            "{snapshot:?}"
        );
        assert!(
            check_all(guard_vocab(), &snapshot).is_empty(),
            "{:?}",
            check_all(guard_vocab(), &snapshot)
        );
        assert_eq!(snapshot[0].kind, EVENT_USER_MESSAGE);
        assert_eq!(snapshot[1].kind, EVENT_TURN_STAGE, "接缝的出口格");
        assert_eq!(
            snapshot[2].kind,
            symbio_plugin_api::events::EVENT_TURN_DISPATCH,
            "派发档另立一格"
        );
        let artifact = snapshot
            .iter()
            .find(|e| e.kind == EVENT_ARTIFACT_ADDED)
            .expect("产物格在");
        assert_eq!(artifact.entity, Entity::Artifact);
        assert_eq!(artifact.verb, Verb::Asserted);
        assert_eq!(artifact.payload["tool"], "vdfs_read");
        assert_eq!(artifact.payload["text"], "文件内容：hello");
        assert_eq!(
            artifact.produced_by,
            Some(0),
            "产物格溯源指向本轮用户格（DE-032 §3 的 caused_by 断言）"
        );
        let cell = snapshot
            .iter()
            .find(|e| e.kind == EVENT_CHAT_ASSISTANT_FINAL)
            .expect("收束格在");
        let output = snapshot
            .iter()
            .rev()
            .find(|e| e.kind == symbio_core::EVENT_RESPONSE && e.payload["port"] == "model")
            .expect("末轮应答在");
        assert_eq!(
            output.payload["answer"], "读到了。",
            "正文归那一次模型往返，收束格只存结局"
        );
        assert_eq!(cell.payload["answer_id"], output.payload["answer_id"]);
        assert_eq!(cell.payload["text"], "", "收束格不复制正文");
    }

    /// 一批**全被拒**的调用：`dispatch-tools` 不欠（没有 `granted`），收尾归
    /// `deny-tools`——它把同一动作选出来，落 `failed{why}` 关掉每一笔 `request`。
    ///
    /// 反向自检：删掉 `deny-tools` 行，派发器在授权之后直接 `no_progress`，
    /// 下面的 `failed` 与第二轮请求都消失。
    #[tokio::test]
    async fn a_fully_denied_batch_is_closed_by_deny_tools() {
        let store: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>> =
            std::sync::Arc::new(EventStore::new());
        let tok = TokenIssuer::issue_deep();
        let llm = std::sync::Arc::new(ToolCallingLlm::new());
        let prompts = llm.prompts.clone();
        let dispatch = std::sync::Arc::new(DenyDispatch {
            rounds: Arc::new(Mutex::new(0)),
        });

        let out = TurnRunner
            .drive_with_tools(
                &store,
                llm.clone(),
                tok,
                input(store.as_ref()),
                Arc::new(SilentDeltas),
                std::sync::Arc::from(tools()),
                Some(dispatch.clone()),
            )
            .await
            .expect("拒绝不中止本轮");

        assert_eq!(out.text, "读到了。", "拒绝之后模型接着答完");
        assert_eq!(*dispatch.rounds.lock().unwrap(), 1, "执行层跑过一次");

        let snapshot = store.range(Seq::new(0));
        assert_eq!(
            super::kinds(&snapshot),
            vec![
                "user.message",
                // 接缝的出口格：轮首那几行跑完、任何请求之前的装配。
                "turn.stage",
                "turn.dispatch",
                "request",
                "sent",
                "response",
                "request",
                "authz.denied",
                "failed",
                "artifact.added",
                "request",
                "sent",
                "response",
                "chat.assistant.final",
            ],
            "{snapshot:?}"
        );
        let failed = snapshot
            .iter()
            .find(|e| e.kind == symbio_core::EVENT_FAILED)
            .expect("被拒的一笔以 failed 关账");
        assert_eq!(failed.payload["port"], "tool");
        assert_eq!(failed.payload["why"], "needs_approval");
        assert!(
            !snapshot
                .iter()
                .any(|e| e.kind == "sent" && e.payload["port"] == "tool"),
            "拒绝的调用不得落 `sent`（那等于说它被派出去了）"
        );

        // 拒绝的事实回灌进下一次请求：模型看得到「这一笔没执行」。
        let seen: Vec<Seen> = prompts
            .lock()
            .unwrap()
            .clone()
            .into_iter()
            .map(Seen::from)
            .collect();
        assert_eq!(
            seen[1].tools(),
            vec![("工具被拒绝：needs_approval", Some("vdfs_read"), Some("w-1"))],
            "拒绝结果带角色与调用 id 进下一轮：{:?}",
            seen[1]
        );
    }

    /// 收束于等待用户：运行器停下并落**进展格**（`chat.assistant.progress`，
    /// `turn × progressed`，载荷 `status: "pending"`）——模型说过的话照样是事实
    /// （"我先查一下。"），等待是**状态**不是另一种事实种类；本轮尚未了结，
    /// 不落收束格（进展格不占「一轮至多一条收束格」），恢复续写后以收束格了结。
    /// 未收束的缺口必须**可被判出**（C4 看得见它）。
    #[tokio::test]
    async fn pending_tool_lands_progress_cell_without_closing() {
        let store: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>> =
            std::sync::Arc::new(EventStore::new());
        let tok = TokenIssuer::issue_deep();
        let llm = std::sync::Arc::new(ToolCallingLlm::new());
        let dispatch = std::sync::Arc::new(FakeDispatch {
            pending: true,
            rounds: Arc::new(Mutex::new(0)),
        });

        let out = TurnRunner
            .drive_with_tools(
                &store,
                llm.clone(),
                tok,
                input(store.as_ref()),
                Arc::new(SilentDeltas),
                std::sync::Arc::from(tools()),
                Some(dispatch.clone()),
            )
            .await
            .expect("等待用户不是失败");

        assert!(out.awaits_user, "等待用户必须在结果里可见");
        assert!(
            !out.fell_back && !out.aborted,
            "等待用户既不是失败也不是中止"
        );
        assert_eq!(*dispatch.rounds.lock().unwrap(), 1, "停下：不再发起第二轮");

        let snapshot = store.range(Seq::new(0));
        assert_eq!(
            super::kinds(&snapshot),
            vec![
                "user.message",
                // 接缝的出口格：轮首那几行跑完、任何请求之前的装配。
                "turn.stage",
                "turn.dispatch",
                "request",
                "sent",
                "response",
                // 工具批的创建 / 授权 / 派发各有协议事实；等待用户是**状态**，
                // 所以结果格与进展格照落，收束格没有。
                "request",
                "authz.granted",
                "sent",
                "response",
                "artifact.added",
                "chat.assistant.progress",
            ],
            "用户格 + 派发格 + 模型端口三格 + 工具批协议格 + 产物格 + 进展格，**没有**收束格：{snapshot:?}"
        );
        assert_eq!(snapshot[1].kind, EVENT_TURN_STAGE, "接缝的出口格");
        assert_eq!(
            snapshot[2].kind,
            symbio_plugin_api::events::EVENT_TURN_DISPATCH
        );
        assert!(
            snapshot.iter().any(|e| e.kind == EVENT_ARTIFACT_ADDED),
            "产物格在"
        );
        let progress = snapshot
            .iter()
            .find(|e| e.kind == EVENT_CHAT_ASSISTANT_PROGRESS)
            .expect("进展格在");
        assert_eq!(progress.entity, Entity::Turn);
        assert_eq!(progress.verb, Verb::Progressed);
        assert_eq!(progress.payload["status"], "pending", "等待是状态字段");
        assert_eq!(
            progress.payload["text"], "",
            "进展格不复制正文（正文归那一次模型往返）"
        );
        let output = snapshot
            .iter()
            .rev()
            .find(|e| e.kind == symbio_core::EVENT_RESPONSE && e.payload["port"] == "model")
            .expect("本轮应答在");
        assert_eq!(
            output.payload["answer"], "我先查一下。",
            "等待前说过的话照样是事实"
        );
        assert_eq!(progress.payload["answer_id"], output.payload["answer_id"]);
        assert_eq!(
            progress.produced_by,
            Some(0),
            "进展格溯源指向本轮用户格（I2）"
        );
        assert!(
            check_all(guard_vocab(), &snapshot).is_empty(),
            "{:?}",
            check_all(guard_vocab(), &snapshot)
        );
        // C4 口径（D-067）：落过产物格的轮**不再**被判静默中断——工具结果是说给
        // 用户的话，它就是收束。等待轮的**收束格**仍刻意留给恢复幕（进展格不是
        // 收束格），那是 `last_open_turn` 的口径，不是 C4 的。
        assert!(
            unresolved_turns(&snapshot, false).is_empty(),
            "产物收束 ⇒ 不算静默中断：{:?}",
            unresolved_turns(&snapshot, false)
        );
    }

    /// 无分发方却收到工具调用 = 配置缺口：按失败诚实回报（兜底格），
    /// 不假装成功、也不静默丢工具调用。
    #[tokio::test]
    async fn tool_call_without_dispatch_falls_back() {
        let store: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>> =
            std::sync::Arc::new(EventStore::new());
        let tok = TokenIssuer::issue_deep();
        let llm = std::sync::Arc::new(ToolCallingLlm::new());

        let out = TurnRunner
            .drive_with_tools(
                &store,
                llm.clone(),
                tok,
                input(store.as_ref()),
                Arc::new(SilentDeltas),
                std::sync::Arc::from(tools()),
                None,
            )
            .await
            .expect("失败轮仍返回 Ok（轮次收束了）");

        assert!(out.fell_back, "没有分发通道 ⇒ 兜底");
        assert!(
            out.text.contains("工具分发通道"),
            "兜底话术说明原因：{}",
            out.text
        );
        let snapshot = store.range(Seq::new(0));
        assert_eq!(
            super::kinds(&snapshot),
            vec![
                "user.message",
                // 接缝的出口格：轮首那几行跑完、任何请求之前的装配。
                "turn.stage",
                "turn.dispatch",
                "request",
                "sent",
                "response",
                "chat.assistant.fallback",
            ],
            "用户格 + 派发格 + 模型端口三格 + 兜底格：{snapshot:?}"
        );
        assert!(
            snapshot
                .iter()
                .any(|e| e.kind == symbio_plugin_api::events::EVENT_CHAT_ASSISTANT_FALLBACK),
            "兜底格在"
        );
    }

    /// **实测用量直通的锚**：provider 实测用量必须沿 `LlmTurn → TurnOutcome` 带出来。
    ///
    /// 唯一消费方是 session 的 token 估算校准（`chat_loop/turn.rs::feedback_estimate`）：
    /// 这一格断掉，校准比就冻结在初值 1.0，未校准启发式的系统偏差（实测对 CJK
    /// 高估约 31%）永久无人修正，压缩预检把本可成功的摘要请求误判成「注定超限」
    /// ——e2e `t8` / `t11` / `t18` 红的正因。链路后半段（`TurnOutcome.usage` →
    /// `TurnOutput.usage`）在 `turn_exec` 是两处同名字段直通，不另设旁路。
    #[tokio::test]
    async fn provider_usage_survives_the_run_to_the_outcome() {
        let store: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>> =
            std::sync::Arc::new(EventStore::new());
        let tok = TokenIssuer::issue_deep();
        let llm = std::sync::Arc::new(AnsweringLlm::new());
        let out = TurnRunner
            .drive_with_tools(
                &store,
                llm.clone(),
                tok,
                input(store.as_ref()),
                Arc::new(SilentDeltas),
                std::sync::Arc::from(Vec::new()),
                None,
            )
            .await
            .expect("桩必答");
        assert_eq!(
            out.usage,
            Some(symbio_core::ModelUsage {
                input: Some(11),
                output: Some(22)
            }),
            "用量断了 ⇒ 校准永远停在 1.0"
        );
    }

    /// **截断原因的锚**：`LlmTurn.finish` 必须沿 `TurnOutcome → TurnOutput` 一路带出。
    ///
    /// 唯一消费方是 `chat_loop/turn.rs`：`settle_reasoning` 用 `finish.is_length()` 给
    /// 本轮正文节点打 `meta.finish_reason = "length"`（前端「继续」按钮 + 回溯），
    /// `persist_turn` 与幕间判据（`loop_rows::next_stage_round`）在同判据上触发自动续写
    /// （上界 `MAX_STAGE_ROUNDS`）。此前 `turn_exec.rs`
    /// 用 `TurnOutput { ..Default::default() }` 重建时把 `finish` 静默填成 Stop，
    /// 两条判据在 v2 执行路径都读不到 Length ⇒ 被 `max_tokens` 掐断的半截正文既无标记
    /// 也不续写，界面上表现为「对话突然结束」。
    ///
    /// **反向自检**：把 `TurnOutcome.finish` 或 `turn_exec` 的重建里那行 `finish: outcome.finish`
    /// 拿掉，本用例必红。
    #[tokio::test]
    async fn truncated_answer_carries_length_reason_to_the_outcome() {
        /// 末轮桩：正文非空 + `finish=Length`（被 `max_tokens` 掐断的半截正文）。
        struct TruncatedLlm;
        #[async_trait::async_trait]
        impl LlmAdapter for TruncatedLlm {
            fn model_id(&self) -> &str {
                "truncated-mock"
            }
            async fn generate(
                &self,
                _tok: &FullModel,
                _messages: &[PromptMessage],
            ) -> Result<String, AdapterError> {
                Err(AdapterError::GenerationFailed("unused".into()))
            }
            async fn generate_turn(
                &self,
                _tok: &FullModel,
                _messages: &[PromptMessage],
                _tools: &[CapabilityMeta],
                _sink: Arc<dyn DeltaSink>,
            ) -> Result<LlmTurn, AdapterError> {
                Ok(LlmTurn {
                    text: "被 max_tokens 掐断的半截正文".into(),
                    reasoning: String::new(),
                    tool_calls: Vec::new(),
                    finish: ModelFinishReason::Length,
                    cost_ms: 3,
                    usage: None,
                    spend: None,
                })
            }
        }

        let store: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>> =
            std::sync::Arc::new(EventStore::new());
        let tok = TokenIssuer::issue_deep();
        let out = TurnRunner
            .drive_with_tools(
                &store,
                std::sync::Arc::new(TruncatedLlm),
                tok,
                input(store.as_ref()),
                Arc::new(SilentDeltas),
                std::sync::Arc::from(Vec::new()),
                None,
            )
            .await
            .expect("截断轮不是失败");
        assert_eq!(
            out.finish,
            ModelFinishReason::Length,
            "长度截断的收尾原因必须在 TurnOutcome 上可见"
        );
        // 收束格照常落（`fell_back=false`，正文照交付）：本判据只钉「带得出真值」，
        // 「带出之后 chat_loop 干什么」不在运行器职责内。
        let cells = store.range(Seq::new(0));
        assert!(
            cells.iter().any(|e| e.kind == EVENT_CHAT_ASSISTANT_FINAL),
            "非空正文应落 final 收束格：{cells:?}"
        );
    }

    /// **唤醒幕续写同一轮**（审批 / 问答恢复）：**不重开用户格**、收束记在**原轮**上。
    ///
    /// 这是唤醒幕的核心判据：等待轮的**收束格**刻意留给恢复幕（进展格不是收束格），续写把它
    /// **真正填上**（见收尾的 kinds 断言）。若续写另开新轮（`u-1` + `f-1`），原轮
    /// 永远等不到收束事件——C4 的产物口径（D-067）会掩盖它，但 `last_open_turn`
    /// 仍按 final / fallback 找轮，恢复链路会先断。
    ///
    /// ## 唤醒身份在**事实**上，不在入参里（d）
    ///
    /// 用户动作先落成 `request(port=tool)`（载荷带 `resume` 标记），行表的
    /// `authorize-tool`（0）与 `dispatch-tools`（68）两行自己认领它——恢复路径不再
    /// 自己写协议事实、也不再自己跑执行层。派发方读回那一笔用的是**生产**的读侧
    /// （[`ToolPort::wake_batch`]）：载荷形状不在替身里复述一遍。
    #[tokio::test]
    async fn a_wake_continues_the_same_turn_without_reopening_the_user_cell() {
        let store: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>> =
            std::sync::Arc::new(EventStore::new());
        let tok = TokenIssuer::issue_deep();

        // ① 等待轮：工具报 pending ⇒ 停下、落进展格、不落收束格。
        // C4 口径（D-067）：产物格已落 ⇒ 不算静默中断；但收束格还没落——
        //    收尾断言钉住「续写把它补在**原轮**上」。
        let pending_llm = ToolCallingLlm::new();
        let pending_dispatch = std::sync::Arc::new(FakeDispatch {
            pending: true,
            rounds: Arc::new(Mutex::new(0)),
        });
        let first = TurnRunner
            .drive_with_tools(
                &store,
                std::sync::Arc::new(pending_llm),
                tok,
                input(store.as_ref()),
                Arc::new(SilentDeltas),
                std::sync::Arc::from(tools()),
                Some(pending_dispatch.clone()),
            )
            .await
            .expect("等待用户不是失败");
        assert!(first.awaits_user, "等待轮必须在结果里可见");
        let mid = store.range(Seq::new(0));
        assert!(
            !mid.iter().any(|e| e.kind == EVENT_CHAT_ASSISTANT_FINAL),
            "等待轮不落收束格（收束格留给恢复幕）：{mid:?}"
        );
        let user_seq = mid
            .iter()
            .find(|e| e.kind == EVENT_USER_MESSAGE)
            .and_then(|e| e.produced_by)
            .or_else(|| {
                mid.iter()
                    .find(|e| e.kind == EVENT_USER_MESSAGE)
                    .map(|e| e.seq.map(|s| s.value()).unwrap_or(0))
            })
            .expect("本轮有用户格");

        // ② 用户点了「批准」⇒ 唤醒请求落格（幕锚**之前**）：它只是一笔
        //    `request(port=tool)`，授权与派发由行表的两行承担。
        let facts: Arc<dyn Store<Event = Event>> = store.clone();
        let port = ToolPort::new(facts, 0, user_seq, 0);
        port.land_wake(&WakeSpec {
            kind: "approve",
            target: "tc-1",
            tool: "vdfs_read",
            args: &serde_json::json!({ "path": "a.md", "approved": true }),
            wire_id: "w-1",
            turn_root: "root-1",
            intent: &super::super::super::tool_dispatch::WakeIntent::Execute,
        })
        .expect("唤醒请求落格");
        assert!(
            super::super::super::loop_rows::owes("authorize-tool", &store.range(Seq::new(0)), 0, 0),
            "唤醒请求一落，授权行就欠着（它自己会认领这一笔）"
        );

        // ③ 唤醒幕：续写**同一轮**（turn 0），用户格 seq 不变，本幕锚是新落的那条。
        let answer_llm = AnsweringLlm::new();
        let prompts = answer_llm.prompts.clone();
        let wake_dispatch = std::sync::Arc::new(WakeDispatch {
            store: store.clone(),
            turn: 0,
            user_seq,
            rounds: Arc::new(Mutex::new(0)),
        });
        let out = TurnRunner
            .drive_with_tools(
                &store,
                std::sync::Arc::new(answer_llm),
                tok,
                TurnInput {
                    turn: 0,
                    // 续写不新开用户格：溯源锚 = 首幕用户格 seq。
                    user_seq,
                    // 本幕锚与首幕（0）不同 ⇒ 派发格幂等键不撞。
                    drive_seq: 1,
                    inject: None,
                    window_turns: None,
                    actor: trivial_actor_spec("session/main"),
                    // 请求级前缀：测试不走请求视图层（三段皆空）。
                    final_reply: None,
                    final_cost_ms: 0,
                    turn_root_id: String::new(),
                    dialog: None,
                    nodes: None,
                    abort: None,
                },
                Arc::new(SilentDeltas),
                std::sync::Arc::from(tools()),
                Some(wake_dispatch.clone()),
            )
            .await
            .expect("唤醒幕必答");

        assert_eq!(out.turn, 0, "续写记在原轮上");
        assert_eq!(out.text, "批准后已完成。");
        assert!(!out.fell_back && !out.aborted && !out.awaits_user);

        let snapshot = store.range(Seq::new(0));
        assert_eq!(
            snapshot
                .iter()
                .filter(|e| e.kind == EVENT_USER_MESSAGE)
                .count(),
            1,
            "同一句话在网格里只能出现一次（唤醒幕不重开用户格）"
        );
        // 收束格记在**原轮**上，溯源仍是原轮用户格（I2）——它填上了 C4 的缺口。
        let cell = snapshot
            .iter()
            .find(|e| e.kind == EVENT_CHAT_ASSISTANT_FINAL)
            .expect("收束格在");
        assert_eq!(cell.turn, 0, "收束格记在原轮号上");
        let output = snapshot
            .iter()
            .rev()
            .find(|e| e.kind == symbio_core::EVENT_RESPONSE && e.payload["port"] == "model")
            .expect("唤醒幕的应答在");
        assert_eq!(
            output.payload["answer"], "批准后已完成。",
            "正文归唤醒幕这一次模型往返"
        );
        assert_eq!(cell.payload["text"], "", "收束格只存结局");
        assert_eq!(
            cell.produced_by,
            Some(user_seq),
            "溯源仍指向原轮用户格（I2）"
        );
        // 唤醒那一笔的结果落成产物格（同在原轮上）。
        let resumed_artifact = snapshot
            .iter()
            .rfind(|e| e.kind == symbio_plugin_api::events::EVENT_ARTIFACT_ADDED)
            .expect("唤醒的产物格在");
        assert_eq!(resumed_artifact.payload["tool"], "vdfs_read");
        assert_eq!(resumed_artifact.produced_by, Some(user_seq));
        assert!(
            check_all(guard_vocab(), &snapshot).is_empty(),
            "{:?}",
            check_all(guard_vocab(), &snapshot)
        );
        assert!(
            unresolved_turns(&snapshot, false).is_empty(),
            "续写填上了原轮的缺口（否则 C4 会把它当永久假缺口）"
        );

        // 端口面：唤醒请求已关账（`sent` → `response`），不留悬着的债。
        assert!(
            snapshot.iter().any(|e| {
                e.kind == symbio_core::EVENT_RESPONSE
                    && e.payload.get("port").and_then(|v| v.as_str()) == Some("tool")
            }),
            "唤醒那一笔必须有应答（端口债关清）：{snapshot:?}"
        );

        // prompt：唤醒的结果进了模型看得见的地方——**带角色**的一条 `role: tool`
        // 消息（与在途工具轮同形），不是「拼在末尾的一段文字」。
        //
        // 首幕那一笔的结果也还在（跨轮累加）：唤醒幕不是新会话，它接的是同一轮
        // 的上下文；模型看得见「等审批时那次读到的」与「批准后这次读到的」。
        let seen = prompts.lock().unwrap().clone();
        assert_eq!(seen.len(), 1, "收尾轮一次请求：{seen:?}");
        let s = Seen::from(seen[0].clone());
        let tool_msgs = s.tools();
        assert_eq!(
            tool_msgs.last(),
            Some(&("文件内容：hello（已批准）", Some("vdfs_read"), Some("w-1"))),
            "唤醒的结果带自己的调用 id 进 prompt：{s:?}"
        );
        assert_eq!(tool_msgs.len(), 2, "首幕那次的结果仍在（跨轮累加）：{s:?}");
    }

    // ── 守卫：交换**跨轮累加**（失忆的实测形状）───────────────────────────────
    //
    // 存在的理由是一条实测事故：工具轮之间清掉交换（只留最后一轮）时，模型每轮
    // 都重新开局——同一份文件读四遍、待办在 5 项与 6 项之间反复重置，最后吐一轮
    // 空正文收进兜底格。单侧的旧断言（`tool_round_lands_artifact_and_feeds_next_prompt`）
    // 只有**一个**工具轮，那种形状下清空与累加长得一模一样，照不出来。
    //
    // 本组判据钉的是「**第三次**请求里还在不在**第一次**的结果」——只有 2+ 个工具轮
    // 才存在这条边。

    /// 两个工具轮的假适配器：每次请求换一个新调用（id 各不相同），第三次收束。
    ///
    /// id 必须互不相同，因为累加后**同一次请求里**会有两次调用的声明与两条结果；
    /// 撞 id 会被 adapter 当成同一次调用的两条结果而吃掉前一条（见 `prompt_call_id`）。
    struct TwoRoundToolLlm {
        prompts: Arc<Mutex<Vec<Vec<PromptMessage>>>>,
    }

    #[async_trait::async_trait]
    impl LlmAdapter for TwoRoundToolLlm {
        fn model_id(&self) -> &str {
            "two-round-mock"
        }
        async fn generate(
            &self,
            _tok: &FullModel,
            _messages: &[PromptMessage],
        ) -> Result<String, AdapterError> {
            Err(AdapterError::GenerationFailed("unused".into()))
        }
        async fn generate_turn(
            &self,
            _tok: &FullModel,
            messages: &[PromptMessage],
            _tools: &[CapabilityMeta],
            _sink: Arc<dyn DeltaSink>,
        ) -> Result<LlmTurn, AdapterError> {
            let round = {
                let mut p = self.prompts.lock().unwrap();
                p.push(messages.to_vec());
                p.len()
            };
            let call = |id: &str, path: &str| TurnToolCallInfo {
                id: Some(id.into()),
                wire_id: Some(format!("w-{}", id)),
                name: Some("vdfs_read".into()),
                arguments: serde_json::json!({ "path": path }),
                parse_error: None,
                overflow_notice: None,
            };
            Ok(match round {
                1 => LlmTurn {
                    text: "先读 a。".into(),
                    reasoning: String::new(),
                    tool_calls: vec![call("tc-1", "a.md")],
                    finish: ModelFinishReason::Stop,
                    cost_ms: 1,
                    usage: None,
                    spend: None,
                },
                2 => LlmTurn {
                    text: "再读 b。".into(),
                    reasoning: String::new(),
                    tool_calls: vec![call("tc-2", "b.md")],
                    finish: ModelFinishReason::Stop,
                    cost_ms: 1,
                    usage: None,
                    spend: None,
                },
                _ => LlmTurn {
                    text: "两个都读到了。".into(),
                    reasoning: String::new(),
                    tool_calls: Vec::new(),
                    finish: ModelFinishReason::Stop,
                    cost_ms: 1,
                    usage: None,
                    spend: None,
                },
            })
        }
    }

    /// 按调用参数回显的假分发方：两次调用给出**两条可区分**的结果。
    ///
    /// 常量正文的假分发方（[`FakeDispatch`]）在这里不够用——「第一次的结果还在不在」
    /// 必须能从正文里认出来。
    struct PathEchoDispatch;

    #[async_trait::async_trait]
    impl ToolDispatch for PathEchoDispatch {
        async fn open(
            &self,
            store: &dyn Store<Event = Event>,
            turn: &LlmTurn,
        ) -> Vec<ToolPortCall> {
            open_batch(store, turn)
        }

        async fn authorize(&self, store: &dyn Store<Event = Event>, calls: &mut [ToolPortCall]) {
            authorize_batch(store, calls);
        }

        async fn begin(
            self: Arc<Self>,
            store: Arc<dyn Store<Event = Event>>,
            turn: &LlmTurn,
            calls: &mut [ToolPortCall],
        ) -> ToolBegin {
            if begin_batch(store.as_ref(), calls).is_none() {
                return ToolBegin::None;
            }
            let outcomes: Vec<DispatchOutcome> = turn
                .tool_calls
                .iter()
                .map(|tc| DispatchOutcome {
                    call_id: tc.id.clone().unwrap_or_default(),
                    name: tc.name.clone().unwrap_or_default(),
                    text: format!(
                        "结果({})",
                        tc.arguments
                            .get("path")
                            .and_then(|v| v.as_str())
                            .unwrap_or("?")
                    ),
                    ok: true,
                    needs_user_action: false,
                    prompt: None,
                    failure_kind: String::new(),
                    wire_id: tc.wire_id.clone().unwrap_or_default(),
                    args: symbio_core::node_build::tool_call_args_text(tc),
                    result_id: String::new(),
                    annotations: Vec::new(),
                })
                .collect();
            ToolBegin::InFlight(Box::pin(async move { outcomes }))
        }

        fn finish(
            &self,
            store: &dyn Store<Event = Event>,
            calls: &[ToolPortCall],
            _outcomes: &[DispatchOutcome],
        ) {
            succeed_batch(store, calls);
        }
    }

    #[tokio::test]
    async fn two_tool_rounds_accumulate_every_earlier_result_in_the_next_request() {
        let store: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>> =
            std::sync::Arc::new(EventStore::new());
        let tok = TokenIssuer::issue_deep();
        let llm = std::sync::Arc::new(TwoRoundToolLlm {
            prompts: Arc::new(Mutex::new(Vec::new())),
        });
        let prompts = llm.prompts.clone();

        // 节点事实取数口按**取数次数**给不同的 id：真实桥上每轮的两个节点是新建的
        // （分发时定格并归零），这里要的是「第 i 轮的思考挂在第 i 轮的产物格上」这条
        // 配对关系，静态同值照不出错位。
        let round_no = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = round_no.clone();
        let nodes: NodeFactsFn = Arc::new(move || {
            let i = counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            StreamNodeFacts {
                text_id: Some(format!("T-r{i}")),
                text: Some(format!("第 {i} 轮说")),
                reasoning_id: Some(format!("R-r{i}")),
                reasoning: Some(format!("第 {i} 轮在想")),
            }
        });

        let out = TurnRunner
            .drive_with_tools(
                &store,
                llm.clone(),
                tok,
                TurnInput {
                    nodes: Some(nodes),
                    ..input(store.as_ref())
                },
                Arc::new(SilentDeltas),
                std::sync::Arc::from(tools()),
                Some(std::sync::Arc::new(PathEchoDispatch)),
            )
            .await
            .expect("两个工具轮后收束");

        assert!(!out.fell_back, "两轮工具交换后正常收束，不该落兜底格");
        assert_eq!(out.text, "两个都读到了。");

        let seen: Vec<Seen> = prompts
            .lock()
            .unwrap()
            .clone()
            .into_iter()
            .map(Seen::from)
            .collect();
        assert_eq!(seen.len(), 3, "两次工具轮 + 收尾轮：{seen:?}");

        // 逐轮**只增不减**：请求 n 带得上前 n-1 轮的全部结果。
        assert!(
            seen[0].tools().is_empty(),
            "第一次请求还没有任何结果：{:?}",
            seen[0]
        );
        assert_eq!(
            seen[1].tools(),
            vec![("结果(a.md)", Some("vdfs_read"), Some("w-tc-1"))],
            "第二次请求带第一次的调用与结果：{:?}",
            seen[1]
        );
        assert_eq!(
            seen[2].tools(),
            vec![
                ("结果(a.md)", Some("vdfs_read"), Some("w-tc-1")),
                ("结果(b.md)", Some("vdfs_read"), Some("w-tc-2")),
            ],
            "第三次请求必须**仍然**带着第一次的结果——丢掉它等于让模型重新开局，\
             它会再读一遍同一个文件：{:?}",
            seen[2]
        );
        // 前一轮模型说过的话也在（不只是工具结果）：中途正文是回合内事实的一部分。
        assert!(
            seen[2].texts_of("assistant").contains(&"先读 a。"),
            "第一轮的中途正文要留在第三次请求里：{:?}",
            seen[2].texts_of("assistant")
        );
        // 每次调用**自己的** id（不是按工具名合成的同名 id）——两次调用两个 id，
        // 结果各归各的调用。
        let ids: Vec<String> = seen[2]
            .msgs
            .iter()
            .filter_map(|m| m.tool_calls.clone())
            .flatten()
            .map(|c| c.id)
            .collect();
        assert_eq!(
            ids,
            vec!["w-tc-1".to_string(), "w-tc-2".to_string()],
            "两次调用的声明按各自 id 出现，且不撞名：{ids:?}"
        );

        // 事实面：三个产物格 + 一个收束格，无兜底。
        let snapshot = store.range(Seq::new(0));
        let kinds: Vec<&str> = snapshot.iter().map(|e| e.kind.as_str()).collect();
        assert_eq!(
            kinds,
            vec![
                EVENT_USER_MESSAGE,
                // 接缝的出口格：轮首那几行跑完、任何请求之前的装配。
                EVENT_TURN_STAGE,
                symbio_plugin_api::events::EVENT_TURN_DISPATCH,
                // 三次模型往返 = 三组协议事实（request / sent / response）；
                // 两个工具批各带创建 / 授权 / 派发三笔债的协议格。
                "request",
                "sent",
                "response",
                "request",
                "authz.granted",
                "sent",
                "response",
                EVENT_ARTIFACT_ADDED,
                "request",
                "sent",
                "response",
                "request",
                "authz.granted",
                "sent",
                "response",
                EVENT_ARTIFACT_ADDED,
                "request",
                "sent",
                "response",
                EVENT_CHAT_ASSISTANT_FINAL
            ],
            "两轮产物 + 一轮收束（外加接缝出口格、派发格、三次模型往返与两个工具批的协议事实）：{snapshot:?}"
        );
        // 每轮的中途产出挂在**该轮自己的模型应答事实**上（顺序锚）。
        //
        // 重建按格的发射顺序发号，中途产出必须落在**应答落格的位置**：只挂收束格会
        // 让十几个思考块排到全部工具卡片之后（实测症状见 `rebuild.test.rs` 的
        // `round_output_rides_…`）；复制一份到收束格则是两个 owner。
        let outputs: Vec<_> = snapshot
            .iter()
            .filter(|e| e.kind == symbio_core::EVENT_RESPONSE && e.payload["port"] == "model")
            .collect();
        assert_eq!(outputs.len(), 3, "三次模型往返各一条应答事实");
        assert_eq!(
            outputs[0].payload["reasoning_id"],
            serde_json::json!("R-r0"),
            "第一轮的思考挂在第一轮：{:#?}",
            outputs[0].payload
        );
        assert_eq!(
            outputs[1].payload["reasoning_id"],
            serde_json::json!("R-r1"),
            "第二轮的思考挂在第二轮（同一份载荷挪到别的轮即错位）"
        );
        assert_eq!(
            outputs[0].payload["text_id"],
            serde_json::json!("T-r0"),
            "中途正文与思考同格同轮，id 一起带上——重建照 id 复刻，不发新身份"
        );
        assert!(
            snapshot
                .iter()
                .find(|e| e.kind == EVENT_CHAT_ASSISTANT_FINAL)
                .expect("收束格在")
                .payload
                .get("rounds")
                .is_none(),
            "收束格不复制中途产出（正文归各次往返，一个事实一个 owner）"
        );
        assert!(
            check_all(guard_vocab(), &snapshot).is_empty(),
            "{:?}",
            check_all(guard_vocab(), &snapshot)
        );
    }

    // ── 守卫：末轮**只有思考**不是失败 ──────────────────────────────────────
    //
    /// 第一个工具轮说话并请求工具；末轮**只有 reasoning、正文为空**——reasoning
    /// 模型收束时的真实形状（它想清楚了，但没有把结论复述成 content）。
    struct ReasoningOnlyFinalLlm {
        prompts: Arc<Mutex<Vec<Vec<PromptMessage>>>>,
    }

    #[async_trait::async_trait]
    impl LlmAdapter for ReasoningOnlyFinalLlm {
        fn model_id(&self) -> &str {
            "reasoning-only-mock"
        }
        async fn generate(
            &self,
            _tok: &FullModel,
            _messages: &[PromptMessage],
        ) -> Result<String, AdapterError> {
            Err(AdapterError::GenerationFailed("unused".into()))
        }
        async fn generate_turn(
            &self,
            _tok: &FullModel,
            messages: &[PromptMessage],
            _tools: &[CapabilityMeta],
            _sink: Arc<dyn DeltaSink>,
        ) -> Result<LlmTurn, AdapterError> {
            let round = {
                let mut p = self.prompts.lock().unwrap();
                p.push(messages.to_vec());
                p.len()
            };
            Ok(if round == 1 {
                LlmTurn {
                    text: "先读一下。".into(),
                    reasoning: String::new(),
                    tool_calls: vec![TurnToolCallInfo {
                        id: Some("tc-1".into()),
                        wire_id: Some("w-tc-1".into()),
                        name: Some("vdfs_read".into()),
                        arguments: serde_json::json!({ "path": "a.md" }),
                        parse_error: None,
                        overflow_notice: None,
                    }],
                    finish: ModelFinishReason::Stop,
                    cost_ms: 1,
                    usage: None,
                    spend: None,
                }
            } else {
                LlmTurn {
                    text: String::new(),
                    reasoning: "想清楚了：答案是 42。".into(),
                    tool_calls: Vec::new(),
                    finish: ModelFinishReason::Stop,
                    cost_ms: 1,
                    usage: None,
                    spend: None,
                }
            })
        }
    }

    #[tokio::test]
    async fn reasoning_only_final_round_closes_as_answer_not_fallback() {
        let store: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>> =
            std::sync::Arc::new(EventStore::new());
        let tok = TokenIssuer::issue_deep();
        let llm = std::sync::Arc::new(ReasoningOnlyFinalLlm {
            prompts: Arc::new(Mutex::new(Vec::new())),
        });

        let out = TurnRunner
            .drive_with_tools(
                &store,
                llm.clone(),
                tok,
                input(store.as_ref()),
                Arc::new(SilentDeltas),
                std::sync::Arc::from(tools()),
                Some(std::sync::Arc::new(PathEchoDispatch)),
            )
            .await
            .expect("末轮必答");

        // 那句思考**就是回答**（v1 的 `effective_text` 同一语义），不是「没答」。
        assert!(!out.fell_back, "只有思考的末轮是回答，不该判成失败");
        assert_eq!(out.text, "想清楚了：答案是 42。");

        let snapshot = store.range(Seq::new(0));
        let last = snapshot.last().expect("有收束格");
        assert_eq!(
            last.kind, EVENT_CHAT_ASSISTANT_FINAL,
            "收束格而不是兜底格——落兜底格等于把「答了（以思考的形式）」记成「没答」"
        );
        assert_eq!(last.payload["text"], "", "收束格只存结局");
        assert_eq!(last.payload["status"], "completed");
        let output = snapshot
            .iter()
            .rev()
            .find(|e| e.kind == symbio_core::EVENT_RESPONSE && e.payload["port"] == "model")
            .expect("末轮应答在");
        assert_eq!(
            output.payload["answer"], "想清楚了：答案是 42。",
            "那段思考就是回答（`answer` 由 `final_answer` 折出）"
        );
    }

    #[tokio::test]
    async fn final_round_with_neither_text_nor_reasoning_still_falls_back() {
        /// 末轮两样都空的假适配器：那是**真失败**（模型确实什么也没产出）。
        struct SilentFinalLlm {
            prompts: Arc<Mutex<Vec<Vec<PromptMessage>>>>,
        }

        #[async_trait::async_trait]
        impl LlmAdapter for SilentFinalLlm {
            fn model_id(&self) -> &str {
                "silent-mock"
            }
            async fn generate(
                &self,
                _tok: &FullModel,
                _messages: &[PromptMessage],
            ) -> Result<String, AdapterError> {
                Err(AdapterError::GenerationFailed("unused".into()))
            }
            async fn generate_turn(
                &self,
                _tok: &FullModel,
                messages: &[PromptMessage],
                _tools: &[CapabilityMeta],
                _sink: Arc<dyn DeltaSink>,
            ) -> Result<LlmTurn, AdapterError> {
                let round = {
                    let mut p = self.prompts.lock().unwrap();
                    p.push(messages.to_vec());
                    p.len()
                };
                Ok(if round == 1 {
                    LlmTurn {
                        text: "先读一下。".into(),
                        reasoning: String::new(),
                        tool_calls: vec![TurnToolCallInfo {
                            id: Some("tc-1".into()),
                            wire_id: Some("w-tc-1".into()),
                            name: Some("vdfs_read".into()),
                            arguments: serde_json::json!({ "path": "a.md" }),
                            parse_error: None,
                            overflow_notice: None,
                        }],
                        finish: ModelFinishReason::Stop,
                        cost_ms: 1,
                        usage: None,
                        spend: None,
                    }
                } else {
                    LlmTurn {
                        text: String::new(),
                        reasoning: String::new(),
                        tool_calls: Vec::new(),
                        finish: ModelFinishReason::Stop,
                        cost_ms: 1,
                        usage: None,
                        spend: None,
                    }
                })
            }
        }

        let store: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>> =
            std::sync::Arc::new(EventStore::new());
        let tok = TokenIssuer::issue_deep();
        let llm = std::sync::Arc::new(SilentFinalLlm {
            prompts: Arc::new(Mutex::new(Vec::new())),
        });

        let out = TurnRunner
            .drive_with_tools(
                &store,
                llm.clone(),
                tok,
                input(store.as_ref()),
                Arc::new(SilentDeltas),
                std::sync::Arc::from(tools()),
                Some(std::sync::Arc::new(PathEchoDispatch)),
            )
            .await
            .expect("失败也要有产出");

        assert!(out.fell_back, "正文与思考**都**空才是没答");
        let snapshot = store.range(Seq::new(0));
        let last = snapshot.last().expect("有兜底格");
        assert_eq!(
            last.kind,
            symbio_plugin_api::events::EVENT_CHAT_ASSISTANT_FALLBACK
        );
        assert_eq!(last.payload["detail"], "model returned empty text");
        assert_eq!(
            last.payload["reason"], "fallback",
            "兜底是闭集里的一个取值，不是自由文本"
        );
        assert_eq!(last.payload["status"], "failed");
    }

    // ── 守卫：截断的**半段思考**不是答话 ────────────────────────────────────
    //
    /// 与 [`ReasoningOnlyFinalLlm`] **只差一个字段**：末轮同样是「正文空 + 只有思考」，
    /// 但 provider 说这次响应是被 `max_tokens` 掐断的（`finish = length`）。
    /// 两个用例并排的意义就在这对差异上——把 `finish` 改成 `Stop` 本用例必红，
    /// 把 `Stop` 改成 `Length` 上面那条必红（判据只认「思考写完没有」）。
    struct TruncatedReasoningFinalLlm {
        prompts: Arc<Mutex<Vec<Vec<PromptMessage>>>>,
    }

    #[async_trait::async_trait]
    impl LlmAdapter for TruncatedReasoningFinalLlm {
        fn model_id(&self) -> &str {
            "truncated-reasoning-mock"
        }
        async fn generate(
            &self,
            _tok: &FullModel,
            _messages: &[PromptMessage],
        ) -> Result<String, AdapterError> {
            Err(AdapterError::GenerationFailed("unused".into()))
        }
        async fn generate_turn(
            &self,
            _tok: &FullModel,
            messages: &[PromptMessage],
            _tools: &[CapabilityMeta],
            _sink: Arc<dyn DeltaSink>,
        ) -> Result<LlmTurn, AdapterError> {
            let round = {
                let mut p = self.prompts.lock().unwrap();
                p.push(messages.to_vec());
                p.len()
            };
            Ok(if round == 1 {
                LlmTurn {
                    text: "先读一下。".into(),
                    reasoning: String::new(),
                    tool_calls: vec![TurnToolCallInfo {
                        id: Some("tc-1".into()),
                        wire_id: Some("w-tc-1".into()),
                        name: Some("vdfs_read".into()),
                        arguments: serde_json::json!({ "path": "a.md" }),
                        parse_error: None,
                        overflow_notice: None,
                    }],
                    finish: ModelFinishReason::Stop,
                    cost_ms: 1,
                    usage: None,
                    spend: None,
                }
            } else {
                LlmTurn {
                    text: String::new(),
                    // 实测形状：正文 0 字符、思考 13045 字的**草稿**（「让我先检查字数…」）。
                    reasoning: "打算这样写报告：先读三处，再逐段列数字，然后检查字数。".into(),
                    tool_calls: Vec::new(),
                    finish: ModelFinishReason::Length,
                    cost_ms: 1,
                    usage: None,
                    spend: None,
                }
            })
        }
    }

    #[tokio::test]
    async fn length_truncated_reasoning_is_not_delivered_as_answer() {
        let store: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>> =
            std::sync::Arc::new(EventStore::new());
        let tok = TokenIssuer::issue_deep();
        let llm = std::sync::Arc::new(TruncatedReasoningFinalLlm {
            prompts: Arc::new(Mutex::new(Vec::new())),
        });

        let out = TurnRunner
            .drive_with_tools(
                &store,
                llm.clone(),
                tok,
                input(store.as_ref()),
                Arc::new(SilentDeltas),
                std::sync::Arc::from(tools()),
                Some(std::sync::Arc::new(PathEchoDispatch)),
            )
            .await
            .expect("失败也要有产出");

        let draft = "打算这样写报告：先读三处，再逐段列数字，然后检查字数。";
        assert!(
            out.fell_back,
            "被长度截断的思考不是答话——交付它就是把草稿当答案"
        );
        assert_ne!(out.text, draft, "回给调用方的不是草稿");

        let snapshot = store.range(Seq::new(0));
        let last = snapshot.last().expect("有收束格");
        assert_eq!(
            last.kind,
            symbio_plugin_api::events::EVENT_CHAT_ASSISTANT_FALLBACK,
            "落兜底格（失败形状），不是 final——落 final 会把「没写完」记成「答了」"
        );
        assert_eq!(last.payload["status"], "failed");
        assert_eq!(
            last.payload["text"],
            serde_json::Value::String(String::new()),
            "答话载荷是空的：草稿不得冒充正文"
        );
        assert!(
            last.payload["detail"]
                .as_str()
                .unwrap_or_default()
                .contains("长度上限"),
            "兜底原因要说得出是长度截断，不是一句「空文本」（实得 {:?}）",
            last.payload["detail"]
        );
        assert!(
            check_all(guard_vocab(), &snapshot).is_empty(),
            "{:?}",
            check_all(guard_vocab(), &snapshot)
        );
    }

    /// 假适配器：**每一轮都要工具**——模拟适配器 / 分流在打转，永不自己收束。
    ///
    /// 与 [`ToolCallingLlm`] 的唯一差别是「没有最后一轮」：它存在的目的就是
    /// 让步循环永远不会因为模型不再请求工具而停下来。
    struct RunawayLlm {
        calls: Arc<Mutex<usize>>,
    }

    #[async_trait::async_trait]
    impl LlmAdapter for RunawayLlm {
        fn model_id(&self) -> &str {
            "runaway-mock"
        }
        async fn generate(
            &self,
            _tok: &FullModel,
            _messages: &[PromptMessage],
        ) -> Result<String, AdapterError> {
            Err(AdapterError::GenerationFailed("unused".into()))
        }
        async fn generate_turn(
            &self,
            _tok: &FullModel,
            _messages: &[PromptMessage],
            _tools: &[CapabilityMeta],
            _sink: Arc<dyn DeltaSink>,
        ) -> Result<LlmTurn, AdapterError> {
            *self.calls.lock().unwrap() += 1;
            Ok(LlmTurn {
                text: "我再查一下。".into(),
                reasoning: String::new(),
                tool_calls: vec![TurnToolCallInfo {
                    id: Some("tc-loop".into()),
                    wire_id: Some("w-loop".into()),
                    name: Some("vdfs_read".into()),
                    arguments: serde_json::json!({ "path": "a.md" }),
                    parse_error: None,
                    overflow_notice: None,
                }],
                finish: ModelFinishReason::Stop,
                cost_ms: 1,
                usage: None,
                spend: None,
            })
        }
    }

    /// 步数上界（[D-115](../../../docs/003-design/015-义务环.md#d-115-义务环的输入闭包与执行契约owed-只读事实视图事件时间执行体异步可失败端口分两级 "DE-015-义务环") 决策 3）：
    /// 一个**永远**要工具的适配器不会把这一轮拖死，而是在上界处收束为
    /// `exhausted{step_limit}`。
    ///
    /// ## 为什么这条判据必须有
    ///
    /// 步循环之前，模型往返回「未收束」只有一个意思——「再欠一步」，而**没有
    /// 上界**。一个总回工具调用的适配器因此是一次永不返回的轮，每一圈都真花一次
    /// 模型往返。上限不是性能阀，是**停机判据**：它把「不收敛」变成一条可观测的
    /// 事实，而不是一个卡住的会话。
    ///
    /// ## 为什么落兜底格而不是另开一格
    ///
    /// 「没说完就停」与失败同形，差别只在 `reason` 这个闭集值；另开一格会让读侧
    /// 为了同一个问题（这一轮为什么停）去问两个名字。
    ///
    /// ## 反向自检
    ///
    /// 把上界检查删掉，本用例**不会变红，而是挂住**——那正是它防的病：一个不收敛
    /// 的轮在测试里表现为永不返回，在生产里表现为一个永远在花钱的会话。
    #[tokio::test]
    async fn a_runaway_adapter_is_stopped_at_the_step_bound() {
        let store: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>> =
            std::sync::Arc::new(EventStore::new());
        let tok = TokenIssuer::issue_deep();
        let llm = std::sync::Arc::new(RunawayLlm {
            calls: Arc::new(Mutex::new(0)),
        });
        let dispatch = std::sync::Arc::new(FakeDispatch {
            pending: false,
            rounds: Arc::new(Mutex::new(0)),
        });

        let out = TurnRunner
            .drive_with_tools(
                &store,
                llm.clone(),
                tok,
                input(store.as_ref()),
                Arc::new(SilentDeltas),
                std::sync::Arc::from(tools()),
                Some(dispatch.clone()),
            )
            .await
            .expect("到点必答：上界耗尽也是一句话");

        assert_eq!(
            *llm.calls.lock().unwrap(),
            super::super::MAX_STEPS_PER_DRIVE,
            "打转的轮必须在上界处停下，且每步恰好一次模型往返"
        );
        assert!(out.fell_back, "没说完就停 ⇒ 失败形状（I3 到点必答）");

        let snapshot = store.range(Seq::new(0));
        let last = snapshot.last().expect("有收束格");
        assert_eq!(
            last.kind,
            symbio_plugin_api::events::EVENT_CHAT_ASSISTANT_FALLBACK,
            "落兜底格（失败形状）——落 final 会把「没说完」记成「答了」"
        );
        assert_eq!(last.payload["status"], "failed");
        assert_eq!(
            last.payload["reason"], "exhausted",
            "上界耗尽用既有闭集值 exhausted，不新造词"
        );
        assert_eq!(last.payload["detail"], "step_limit");
        assert!(
            check_all(guard_vocab(), &snapshot).is_empty(),
            "{:?}",
            check_all(guard_vocab(), &snapshot)
        );
    }

    /// 一次往返**还没好**的那一圈：环回到选债口，而不是判 `no_progress`。
    ///
    /// ## 这条用例钉的是什么
    ///
    /// `await-model` 的动作只 poll 在飞的 future **一次**（`D-222` 订正 2）。往返在飞，
    /// 这一圈因此零写入——派发器的空账本守卫若只问「写了没有」，一个**正常**的模型
    /// 往返会被自己的守卫打死（`exhausted{no_progress}`）。所以那条守卫多问一句
    /// 「有没有一笔已派出、未收清」（`loop_rows::waiting`）。
    ///
    /// ## 反向自检
    ///
    /// 去掉空账本守卫里的 `&& !waiting(..)` ⇒ 本用例红在 `no_progress` 上（收束格从
    /// `final` 变成 `fallback`）。把 `poll_flight` 的 `Poll::Pending` 臂换回「等到底」
    /// ⇒ 本用例**仍然绿**：它钉的是「没写不等于没进展」，不是「必须 poll 一次」。
    /// 后者的后果在别处显形（等到底把窗口又合上了，`turn_head.model_open` 与
    /// `waiting` 那一圈读不出来）。
    #[tokio::test]
    async fn a_round_still_in_flight_parks_the_ring_instead_of_killing_it() {
        let store: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>> =
            std::sync::Arc::new(EventStore::new());
        let tok = TokenIssuer::issue_deep();
        let parked = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (open, opened) = tokio::sync::oneshot::channel::<()>();
        let llm = std::sync::Arc::new(ParkingLlm {
            parked: parked.clone(),
            gate: Mutex::new(Some(opened)),
        });

        // 放行方：等适配器确实停在「已进入、还没等到」的那一刻才发信号。
        // 适配器在同一个 poll 里先置位、再去 poll 通道（中间不让出执行权），
        // 所以看到置位就等于**第一次 poll 已经返回过 `Pending`**。
        let flag = parked.clone();
        let releaser = tokio::spawn(async move {
            while !flag.load(std::sync::atomic::Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
            let _ = open.send(());
        });

        let out = TurnRunner
            .run(&store, llm, tok, 0, "路上慢一点没关系")
            .await
            .expect("停在路上不等于失败：放行之后这一轮照常答完");

        releaser.await.expect("放行方跑完");
        assert!(
            parked.load(std::sync::atomic::Ordering::SeqCst),
            "适配器没停过 ⇒ 这条用例什么都没验（夹具退化成同步适配器）"
        );
        assert!(
            !out.fell_back,
            "在等的那一圈被判成没进展 ⇒ 正常的模型往返被自己的守卫打死"
        );

        let snapshot = store.range(Seq::new(0));
        let kinds_seen: Vec<&str> = snapshot.iter().map(|e| e.kind.as_str()).collect();
        assert!(
            kinds_seen.contains(&symbio_plugin_api::events::EVENT_CHAT_ASSISTANT_FINAL),
            "答完了要落 final（实得 {kinds_seen:?}）"
        );
        assert!(
            !snapshot
                .iter()
                .any(|e| e.payload["detail"] == "no_progress"),
            "不许出现 no_progress：在等不是没事可做"
        );
        assert!(
            check_all(guard_vocab(), &snapshot).is_empty(),
            "{:?}",
            check_all(guard_vocab(), &snapshot)
        );
    }

    /// 「已派出、未收清」在**正常跑**里读得到——本任务的交货判据。
    ///
    /// 装饰事实源：每次 `append` 之后按**那一刻**的日志问一次 `loop_rows::waiting`。
    /// 一轮正常的模型往返必须在某一刻为**真**。若 `sent` 与 `response` 又落回同一个
    /// 动作窗口（`port::dispatch_full` 的老形状），两次选债之间就永远没有那一格，
    /// 于是 `stale_sent`（租约看护）与 `exhausted{waiting}`（停机理由）都只剩
    /// 「读上一次生命遗留的日志」这一个读者。
    ///
    /// ## 反向自检
    ///
    /// 把 `dispatch-model` 的出口改回 `response|failed` 并把 `await-model` 的 `by`
    /// 改回 `None` ⇒ 每一次 `append` 之后都不存在「有 `sent`、无终局」⇒ 本用例红。
    #[tokio::test]
    async fn an_in_flight_round_is_observable_between_selection_rounds() {
        let inner: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>> =
            std::sync::Arc::new(EventStore::new());
        let seen = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let store: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>> =
            std::sync::Arc::new(WatchingStore {
                inner: inner.clone(),
                seen: seen.clone(),
                turn: 0,
            });
        let llm = std::sync::Arc::new(symbio_testkit::StubLlmAdapter::succeed("stub-model"));

        let out = TurnRunner
            .run(
                &store,
                llm,
                TokenIssuer::issue_deep(),
                0,
                "走到一半我好看一眼",
            )
            .await
            .expect("一轮正常的模型往返");

        assert!(!out.fell_back, "这一轮该答完（在途可见不该改变结果）");
        assert!(
            seen.load(std::sync::atomic::Ordering::SeqCst),
            "整轮没有任何一刻「已派出、未收清」⇒ 派出与应答还在同一个动作窗口里"
        );
    }

    /// 事实源装饰器：每次 `append` 之后记一次「此刻有没有在途」。
    ///
    /// 观察点必须在 **append 之后、下一个动作之前**：那正是两次选债之间的那一圈。
    /// 放在动作前后各读一次都不行——动作是原子的一段，中间那一态只在写入落地的
    /// 瞬间成立。
    struct WatchingStore {
        inner: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>>,
        seen: Arc<std::sync::atomic::AtomicBool>,
        turn: u64,
    }

    impl Store for WatchingStore {
        type Event = Event;

        fn append(&self, event: Event) -> Result<symbio_core::Seq, symbio_core::AppendError> {
            let seq = self.inner.append(event)?;
            if super::super::super::loop_rows::waiting(
                &self.inner.range(symbio_core::Seq::new(0)),
                self.turn,
                0,
            ) {
                self.seen.store(true, std::sync::atomic::Ordering::SeqCst);
            }
            Ok(seq)
        }

        fn range(&self, from: symbio_core::Seq) -> Vec<Event> {
            self.inner.range(from)
        }

        fn head(&self) -> symbio_core::Seq {
            self.inner.head()
        }
    }

    /// 工具批次的「已派出、未收清」在**正常跑**里读得到——工具端口与模型端口同形。
    ///
    /// ## 它钉的是什么
    ///
    /// `dispatch-tools` 落 `sent`、`await-tools` 落 `response`，两个动作分处两个环步。
    /// 那么在**两次选债之间**那一圈，事实源上「工具这一笔开着、别的都关着」必须读得到：
    /// `open_calls` 里有一笔 `port=tool`，且 [`super::super::super::loop_rows::waiting`]
    /// 为**真**。`waiting` 视图、`stale_sent`（租约看护）与 `reap` 读的都是它——
    /// 它们在工具端口上不再是「只读上一次生命遗留的日志」。
    ///
    /// ## 观察点为什么是**动作边界**而不是 `append`
    ///
    /// `sent` 与 `response` 是两次独立 append：装饰 `append` 也能在两次 append 之间
    /// 读到那一瞬，但那在**老形状**（派发与收应答合回同一个动作窗口）下同样成立 ⇒
    /// 判不红。动作边界才判得红：老形状下两格落在同一圈里，边界上读不到「已派出、
    /// 未收清」。派发器每跑一个动作前读一次 `head()`（`turn_exec.rs`），那一读正落在
    /// 「上一圈的动作已跑完、这一圈的动作还没跑」之间。
    ///
    /// ## 反向自检
    ///
    /// 把 `ToolDispatch` 的 `begin` / `finish` 合回同一个动作窗口（老形状：
    /// 一个动作里 `begin_calls` → 跑执行层 → `close_batch`）⇒ 边界上工具这一笔永远
    /// 已经关账 ⇒ `has_tool` 全程为假 ⇒ 本用例红。
    #[tokio::test]
    async fn an_in_flight_tool_batch_is_observable_between_selection_rounds() {
        let inner: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>> =
            std::sync::Arc::new(EventStore::new());
        let windows = Arc::new(Mutex::new(Vec::new()));
        let store: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>> =
            std::sync::Arc::new(BatchWindowStore {
                inner: inner.clone(),
                windows: windows.clone(),
                turn: 0,
            });
        let llm = std::sync::Arc::new(ToolCallingLlm::new());
        let dispatch = std::sync::Arc::new(FakeDispatch {
            pending: false,
            rounds: Arc::new(Mutex::new(0)),
        });

        let out = TurnRunner
            .drive_with_tools(
                &store,
                llm.clone(),
                TokenIssuer::issue_deep(),
                input(store.as_ref()),
                Arc::new(SilentDeltas),
                std::sync::Arc::from(tools()),
                Some(dispatch.clone()),
            )
            .await
            .expect("工具轮必答");

        assert_eq!(out.text, "读到了。", "在途可观测不该改变结果");

        let windows = windows.lock().unwrap().clone();
        assert!(
            !windows.is_empty(),
            "一个动作边界都没读到 ⇒ 这条用例什么都没验（观察点没接上）"
        );
        // 找**那一圈**：工具这一笔开着、别的都关着、且 `waiting` 为真。
        let window = windows
            .iter()
            .find(|(_, has_tool, waiting)| *has_tool && *waiting)
            .expect("工具批次「已派出、未收清」在正常跑里读不到");
        assert_eq!(
            window.0, 1,
            "那一刻开着的应当只有工具这一笔（模型那一笔早已关账）：{windows:?}"
        );
    }

    /// 事实源装饰器：**动作边界**上的一次快照。
    ///
    /// 观察点是 `Store::head()`——派发器每跑一个动作前读它一次（`turn_exec.rs`），
    /// 那一读正落在「上一圈的动作已跑完、这一圈的动作还没跑」之间。
    struct BatchWindowStore {
        inner: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>>,
        /// 每个边界上的快照：(开着几笔、其中有没有工具端口、`waiting` 是否为真)。
        windows: Arc<Mutex<Vec<(usize, bool, bool)>>>,
        turn: u64,
    }

    impl Store for BatchWindowStore {
        type Event = Event;

        fn append(&self, event: Event) -> Result<symbio_core::Seq, symbio_core::AppendError> {
            self.inner.append(event)
        }

        fn range(&self, from: symbio_core::Seq) -> Vec<Event> {
            self.inner.range(from)
        }

        fn head(&self) -> symbio_core::Seq {
            let head = self.inner.head();
            let facts = self.inner.range(symbio_core::Seq::new(0));
            let open = super::super::super::control_decl::open_calls(&facts);
            let has_tool = open
                .iter()
                .any(|e| e.payload.get("port").and_then(|v| v.as_str()) == Some("tool"));
            let waiting = super::super::super::loop_rows::waiting(&facts, self.turn, 0);
            self.windows
                .lock()
                .unwrap()
                .push((open.len(), has_tool, waiting));
            head
        }
    }

    /// 停一次的假适配器：第一次被 poll 时把「已进入」置位，然后等外部放行。
    ///
    /// 放行之前它的 future 返回 `Pending`——环因此必须在「已派出、未收清」的状态上
    /// 转一圈。`gate` 用 `Mutex<Option<…>>` 装是因为 `generate_turn` 收 `&self`：
    /// 通道只能取一次（第二次进来已放行过，不该再等）。
    struct ParkingLlm {
        parked: Arc<std::sync::atomic::AtomicBool>,
        gate: Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
    }

    #[async_trait::async_trait]
    impl LlmAdapter for ParkingLlm {
        fn model_id(&self) -> &str {
            "parking-mock"
        }
        async fn generate(
            &self,
            _tok: &FullModel,
            _messages: &[PromptMessage],
        ) -> Result<String, AdapterError> {
            Err(AdapterError::GenerationFailed("unused".into()))
        }
        async fn generate_turn(
            &self,
            _tok: &FullModel,
            _messages: &[PromptMessage],
            _tools: &[CapabilityMeta],
            _sink: Arc<dyn DeltaSink>,
        ) -> Result<LlmTurn, AdapterError> {
            self.parked.store(true, std::sync::atomic::Ordering::SeqCst);
            // 先取出来再 await：`MutexGuard` 不 `Send`，跨 await 持有会让整个
            // future 不是 `Send`（适配器的返回值要求 `Send`）。
            let gate = self.gate.lock().unwrap().take();
            if let Some(rx) = gate {
                let _ = rx.await;
            }
            Ok(LlmTurn {
                text: "路上耽搁了一下，但答上了。".into(),
                reasoning: String::new(),
                tool_calls: Vec::new(),
                finish: ModelFinishReason::Stop,
                cost_ms: 1,
                usage: None,
                spend: None,
            })
        }
    }
}
//
// **选中**那一半在轮首 host（`policy.reflex` 的出口格由 `loop_rows` / `turn_exec` 的
// 用例覆盖）；这里只钉**收束**：正文直落 `final`、`model` 记 `reflex`、经 sink 上线、
// `cost_ms` 入账。三条判据分列：
// - **落格**：载荷与模型应答同一形状，`produced_by` 指向开轮格；
// - **零往返**：`announce-model` 的前件排除已作答的快路轮，桩模型一次都不该被调到；
// - **产物上线**：正文经 `sink` 到出口（不上线，收束节点在前端建不起来）。

mod reflex_turn_tests {
    use crate::control_decl::guard_vocab;
    use std::sync::{Arc, Mutex};
    use symbio_testkit::{trivial_actor_spec, StubLlmAdapter};

    use super::super::super::stage_events::{land, StageFact};
    use super::super::{TurnInput, TurnRunner};
    use symbio_core::{
        check_all, unresolved_turns, DeltaSink, Entity, EventStore, LatencyTier, Seq, TokenIssuer,
        Verb,
    };
    use symbio_plugin_api::events::{
        EVENT_CHAT_ASSISTANT_FINAL, EVENT_TURN_DISPATCH, EVENT_USER_MESSAGE,
    };

    use crate::control_decl::EVENT_TURN_STAGE;

    /// 收集口：按序记增量（快路产物上线的判据）。
    struct CollectingDeltas(Mutex<Vec<String>>);

    impl DeltaSink for CollectingDeltas {
        fn on_delta(&self, text: &str) {
            self.0.lock().unwrap().push(text.to_string());
        }
        // 快路不调模型 ⇒ 没有推理增量；通道由 `bound_provider.test.rs` 覆盖。
        fn on_reasoning(&self, _text: &str) {}
        fn on_tool_frame(&self, _frame: &symbio_core::chat_node::ChatNode) {}
    }

    /// 正向：快路已作答的轮**只落开轮 / 阶段 / 派发 / 收束**，收束溯源指向开轮格，
    /// 收束档位 `reflex`、耗时入账、正文经 sink 上线；桩模型一次都没被调到。
    #[tokio::test]
    async fn reflex_answer_closes_with_reflex_payload_and_zero_model_round() {
        let store: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>> =
            std::sync::Arc::new(EventStore::new());
        let tok = TokenIssuer::issue_deep();
        let actor = trivial_actor_spec("session/main");
        let got = Arc::new(CollectingDeltas(Mutex::new(Vec::new())));
        let user_seq = super::seed_user(store.as_ref(), 0, "把周报整理成摘要");

        // 轮首 host 的出口格：`policy.reflex` 已选中（收束行的前件就是它）。
        land(
            store.as_ref(),
            0,
            user_seq,
            "session/main",
            &[StageFact::reflex(0, user_seq, "weekly-report")],
        )
        .expect("阶段格必须落得下");

        // 桩只在**不该发生**的模型往返里失败：它若失败，说明收束行没抢到第一笔债。
        let llm = std::sync::Arc::new(StubLlmAdapter::always_fail("快路轮不该发模型请求"));
        let out = TurnRunner
            .drive_with_actors(
                &store,
                llm.clone(),
                tok,
                TurnInput {
                    turn: 0,
                    user_seq,
                    drive_seq: user_seq,
                    window_turns: None,
                    inject: None,
                    actor: actor.clone(),
                    final_reply: None,
                    final_cost_ms: 0,
                    turn_root_id: String::new(),
                    dialog: None,
                    nodes: None,
                    abort: None,
                },
                got.clone() as Arc<dyn DeltaSink>,
                std::sync::Arc::from(Vec::new()),
                None,
                super::super::super::loop_rows::ACTORS,
                // 快路正文是**备输入的产物**（`D-222` 订正 3）：生产里由轮首那台 host
                // 的 `policy.reflex` 寄存，草稿建好之后填。
                None,
                Some(super::super::super::turn_runner::ReflexAnswer {
                    text: "先列要点；再合并同类项。".to_string(),
                }),
            )
            .await
            .expect("快路必答（不生成也要有产物）");

        assert_eq!(out.text, "先列要点；再合并同类项。");
        assert!(!out.fell_back && !out.aborted && !out.awaits_user);
        assert!(
            out.cost_ms <= LatencyTier::Reflex.budget_ms(),
            "命中后耗时必须在反射档预算内（DE-041 §6.2：命中显著低于未命中）：{}",
            out.cost_ms
        );

        let snapshot = store.range(Seq::new(0));
        assert_eq!(
            snapshot.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>(),
            vec![
                EVENT_USER_MESSAGE,
                // 快路自己的阶段格 + 接缝的出口格。
                EVENT_TURN_STAGE,
                EVENT_TURN_STAGE,
                EVENT_TURN_DISPATCH,
                EVENT_CHAT_ASSISTANT_FINAL
            ],
            "快路轮落开轮 / 两格阶段 / 派发 / 收束（没有产物格，也没有模型往返）：{snapshot:?}"
        );
        let open = &snapshot[0];
        assert_eq!(open.entity, Entity::Turn);
        assert_eq!(open.verb, Verb::Opened);
        assert_eq!(
            open.payload["text"], "把周报整理成摘要",
            "开轮格记的是**用户说的**，不是快路产物"
        );
        let dispatch = &snapshot[3];
        assert_eq!(
            dispatch.payload["tier"], "reflex",
            "档位由**选中的那条行**给出（`loop_rows::tier`），不是调用方填的字符串"
        );
        assert_eq!(
            dispatch.payload["budget_ms"],
            LatencyTier::Reflex.budget_ms(),
            "派发格带**本幕档位折出的预算数值**（`D-223` 订正 2）：`I3` 读它，所以快路轮 \
             拿到的必须是反射档那 80ms——不是主体规格里那一格（那是声明档，建主体时 \
             档位还没折出来）"
        );
        let close = &snapshot[4];
        assert_eq!(close.verb, Verb::Closed);
        assert_eq!(close.turn, 0);
        assert_eq!(close.payload["text"], "先列要点；再合并同类项。");
        assert_eq!(
            close.payload["model"], "reflex",
            "快路没有模型：载荷的 `model` 记产者，空着会让命中轮与普通轮无从区分"
        );
        assert_eq!(close.cost_ms, out.cost_ms, "实测耗时随事件入账");
        assert_eq!(
            close.produced_by,
            Some(user_seq),
            "收束溯源指向开轮格（I2）"
        );
        assert_eq!(
            close.actor, "session/main",
            "收束格的 actor 取本轮主体（与模型应答同一处取值）"
        );

        assert_eq!(
            *got.0.lock().unwrap(),
            vec!["先列要点；再合并同类项。".to_string()],
            "产物必须经 sink 上线——不上线，收束节点在前端建不起来"
        );
        assert!(
            check_all(guard_vocab(), &snapshot).is_empty(),
            "{:?}",
            check_all(guard_vocab(), &snapshot)
        );
        assert!(
            unresolved_turns(&snapshot, false).is_empty(),
            "开过的轮必须收束（I3 到点必答）"
        );
    }
}

/// 作用域上限：**委派不放大权威**。
///
/// `closure_gate` 的两道——主体自己那一行、以及 `scope = child <parent>` 时的父主体那道。
/// 本模块钉的是**第二道**：同一份能力集，换 `scope` 必须换结果（反向用例），
/// 且父主体持同一能力时不受影响（正向对照）。
///
mod scope_ceiling_tests {
    use super::super::{closure_gate, ClosureKind};
    use symbio_core::Scope;
    use symbio_testkit::trivial_actor_spec;

    /// 反向用例：父主体**不持**该能力 ⇒ 收束被拒；把 `scope` 改回 `Root`（接线前的形状）
    /// ⇒ 同一份能力集必须重新放行。改了不变，说明作用域上限没在算。
    #[test]
    fn a_child_cannot_close_what_its_parent_cannot_close() {
        // `user` 在授权表里没有行（能力按「智能体的权威动作种类」划分）⇒ 什么都不持。
        let mut spec = trivial_actor_spec("session/main");
        assert!(
            symbio_core::actor_can_land_speech(&spec),
            "前提：主智能体自己那一行持 `reply.first`"
        );

        spec.scope = Scope::Child("user".to_string());
        assert!(
            closure_gate(&spec, ClosureKind::Speech).is_err(),
            "父主体不持 `reply.first` ⇒ 子主体的发言收束必须被拒"
        );

        spec.scope = Scope::Root;
        assert!(
            closure_gate(&spec, ClosureKind::Speech).is_ok(),
            "改回 `Root`（无上限的独立主体）⇒ 同一份能力集必须放行"
        );
    }

    /// 真实的越权形态：**窄父主体派生出宽子主体**。
    ///
    /// 对等主体只持 `produce.artifact`（`authz::ROWS` 第四行）。它派生出的子会话
    /// 主体是 `session/main`（主智能体那一行，持 `reply.*`）——子会话因此比父主体宽。
    /// 上限那道闸把它收回来：发言收束被拒，产物收束照常（父主体也持）。
    #[test]
    fn a_narrow_parent_cannot_spawn_a_wider_child() {
        let mut spec = trivial_actor_spec("session/main");
        spec.scope = Scope::Child(symbio_core::PRINCIPAL_PEER.to_string());

        assert!(
            symbio_core::actor_can_land_speech(&spec),
            "前提：子主体自己那一行放行（这正是「放大」的来源）"
        );
        assert!(
            closure_gate(&spec, ClosureKind::Speech).is_err(),
            "父主体（对等体）不持 `reply.*` ⇒ 子主体不得发言"
        );
        assert!(
            closure_gate(&spec, ClosureKind::Artifact).is_ok(),
            "父主体持 `produce.artifact` ⇒ 产物收束不受上限影响"
        );
    }

    /// 正向对照：父主体持同一能力 ⇒ 与 `Root` 逐字同判（平凡值下两臂恒放行）。
    #[test]
    fn a_child_of_an_equally_strong_parent_is_unaffected() {
        let mut spec = trivial_actor_spec("session/main");
        spec.scope = Scope::Child("session/main".to_string());
        assert!(closure_gate(&spec, ClosureKind::Speech).is_ok());
        assert!(closure_gate(&spec, ClosureKind::Artifact).is_ok());
    }
}

// ── 拒绝落事实（DE-015 §4.2）────────────────────────────────────
//
// 收束闸拒绝时此前只有一条 `warn`：日志里留下的是「该轮没收束」，与「根本没跑完」
// **同形**。`deny(…)` 把否定落成 `authz.denied{key, kind, reason, detail}`——判定与矩阵一个字
// 不动，变的只是否定的表达。
//
// 四条判据，最后一条是**对拍**：拒绝原因里写的能力名必须真的是表里那一个。
mod denial_fact_tests {
    use super::super::{closure_gate, deny, ClosureKind};
    use crate::control_decl::guard_vocab;
    use symbio_core::{check_all, unresolved_turns, ActorSpec, EventStore, Scope};
    use symbio_plugin_api::events::{EVENT_AUTHZ_DENIED, EVENT_USER_MESSAGE};
    use symbio_testkit::trivial_actor_spec;

    /// 造一个「主体自己那一行放行、父主体不放行」的规格——上限那道闸是生产里
    /// 唯一可达的拒绝路径（主体自己的行由部署事实决定，测试里不必伪造矩阵）。
    fn child_of_a_principal_without_reply() -> ActorSpec {
        let mut spec = trivial_actor_spec("session/main");
        assert!(
            symbio_core::actor_can_land_speech(&spec),
            "前提：主智能体自己那一行持 reply——拒绝必须来自上限那道闸"
        );
        spec.scope = Scope::Child("user".to_string());
        spec
    }

    /// 种一条本轮用户格（收束格的溯源锚），返回它的 seq。
    fn seed_user_turn(
        store: &dyn symbio_core::Store<Event = symbio_core::Event>,
        turn: u64,
    ) -> u64 {
        store
            .append(symbio_core::Event::pending(
                format!("u-{turn}"),
                EVENT_USER_MESSAGE,
                symbio_core::Entity::Turn,
                symbio_core::Verb::Opened,
                turn,
                "user",
            ))
            .expect("用户格入格")
            .value()
    }

    /// 判据 1：拒绝 ⇒ 日志里有 `authz.denied`，且载荷答得出「拒了什么、为什么」。
    #[test]
    fn a_denied_closure_lands_a_denial_fact() {
        let store: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>> =
            std::sync::Arc::new(EventStore::new());
        let user_seq = seed_user_turn(store.as_ref(), 1);
        let spec = child_of_a_principal_without_reply();
        let why = closure_gate(&spec, ClosureKind::Speech).expect_err("上限那道闸必须拒");
        deny(
            store.as_ref(),
            &spec,
            1,
            user_seq,
            ClosureKind::Speech,
            symbio_plugin_api::events::EVENT_CHAT_ASSISTANT_FINAL,
            &why,
        );

        let snapshot = store.range(symbio_core::Seq::new(0));
        let denied: Vec<_> = snapshot
            .iter()
            .filter(|e| e.kind == EVENT_AUTHZ_DENIED)
            .collect();
        assert_eq!(denied.len(), 1, "拒绝必须落一条事实：{denied:?}");
        let d = denied[0];
        assert_eq!(d.entity, symbio_core::Entity::Turn);
        assert_eq!(
            d.verb,
            symbio_core::Verb::Asserted,
            "落在 turn × asserted，**不是** closed"
        );
        assert_eq!(d.turn, 1);
        assert_eq!(d.actor, spec.principal, "谁");
        assert_eq!(
            d.produced_by,
            Some(user_seq),
            "何时/因何而起：溯源指向本轮用户格"
        );
        assert_eq!(
            d.payload.get("key").and_then(|v| v.as_str()),
            Some(symbio_plugin_api::events::EVENT_CHAT_ASSISTANT_FINAL),
            "拒的是哪一格"
        );
        assert_eq!(
            d.payload.get("kind").and_then(|v| v.as_str()),
            Some("speech")
        );
        let reason = d
            .payload
            .get("detail")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert!(
            reason.contains("user") && reason.contains("reply"),
            "原因要说清是谁缺哪一项：{reason}"
        );
        assert!(
            check_all(guard_vocab(), &snapshot).is_empty(),
            "落一条否定事实不得让不变量变红：{:?}",
            check_all(guard_vocab(), &snapshot)
        );
    }

    /// 判据 2：拒绝轮**仍然**未收束。
    ///
    /// 这是本任务最容易做反的一处：为了让日志好看而把 `denied` 算成收束，等于把
    /// 「静默中断」这条最该被发现的失效一起掩盖掉。反向锚定——删掉这条断言，
    /// 一个把 `denied` 当收束格的实现照样全绿。
    #[test]
    fn a_denied_turn_stays_unresolved() {
        let store: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>> =
            std::sync::Arc::new(EventStore::new());
        let user_seq = seed_user_turn(store.as_ref(), 1);
        let spec = child_of_a_principal_without_reply();
        deny(
            store.as_ref(),
            &spec,
            1,
            user_seq,
            ClosureKind::Speech,
            symbio_plugin_api::events::EVENT_CHAT_ASSISTANT_FINAL,
            "拒",
        );

        let snapshot = store.range(symbio_core::Seq::new(0));
        let open = unresolved_turns(&snapshot, false);
        assert_eq!(
            open.len(),
            1,
            "拒绝轮必须仍然报未收束（denied 不是收束格）：{open:?}"
        );
        assert!(
            !symbio_core::closes_turn(
                snapshot
                    .iter()
                    .find(|e| e.kind == EVENT_AUTHZ_DENIED)
                    .unwrap()
            ),
            "`authz.denied` 绝不能被认成收束格"
        );
    }

    /// 幂等：同一轮同一格被拒多次只落一条（产物臂在工具结果循环里，会重复走到闸前）。
    #[test]
    fn a_denial_is_recorded_once_per_turn_and_kind() {
        let store: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>> =
            std::sync::Arc::new(EventStore::new());
        let user_seq = seed_user_turn(store.as_ref(), 1);
        let spec = child_of_a_principal_without_reply();
        for _ in 0..3 {
            deny(
                store.as_ref(),
                &spec,
                1,
                user_seq,
                ClosureKind::Artifact,
                symbio_plugin_api::events::EVENT_ARTIFACT_ADDED,
                "拒",
            );
        }
        let snapshot = store.range(symbio_core::Seq::new(0));
        let denied = snapshot
            .iter()
            .filter(|e| e.kind == EVENT_AUTHZ_DENIED)
            .count();
        assert_eq!(denied, 1, "同一轮同一格只记一次：{denied}");
    }

    /// 两种拒绝落在**两个键**上：一轮里产物被拒、发言也被拒，是两条事实。
    #[test]
    fn speech_and_artifact_denials_do_not_collide() {
        let store: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>> =
            std::sync::Arc::new(EventStore::new());
        let user_seq = seed_user_turn(store.as_ref(), 1);
        let spec = child_of_a_principal_without_reply();
        for (kind, key) in [
            (
                ClosureKind::Speech,
                symbio_plugin_api::events::EVENT_CHAT_ASSISTANT_FINAL,
            ),
            (
                ClosureKind::Artifact,
                symbio_plugin_api::events::EVENT_ARTIFACT_ADDED,
            ),
        ] {
            deny(store.as_ref(), &spec, 1, user_seq, kind, key, "拒");
        }
        let snapshot = store.range(symbio_core::Seq::new(0));
        let kinds: Vec<&str> = snapshot
            .iter()
            .filter(|e| e.kind == EVENT_AUTHZ_DENIED)
            .filter_map(|e| e.payload.get("kind").and_then(|v| v.as_str()))
            .collect();
        assert_eq!(kinds, vec!["speech", "artifact"], "两臂各自成键：{kinds:?}");
    }

    /// **对拍**：拒绝原因里写的能力名，必须真的是授权表里那一项。
    ///
    /// `capability_name()` 是手写的一份拼写（本模块看不见 core 的 `governance`，
    /// 那是私有子模块）。手写一份没有对拍，就只是又一处「表授予 A、闸门判 B」——
    /// 两边都是合法能力名，事后没有任何东西能指出是哪半边写错了。
    /// 这里用**生产表派生的能力集**对拍：拼错一个字符，两条断言当场红。
    #[test]
    fn denial_names_the_capability_the_gate_judges() {
        let main = trivial_actor_spec("session/main");
        let peer = trivial_actor_spec(symbio_core::PRINCIPAL_PEER);

        let speech_cap = ClosureKind::Speech.capability_name();
        let artifact_cap = ClosureKind::Artifact.capability_name();

        assert!(
            main.capabilities.iter().any(|c| c == speech_cap),
            "主智能体的登记行必须持 {speech_cap}——拒绝原因说的就是这一项"
        );
        assert!(
            peer.capabilities.iter().any(|c| c == artifact_cap),
            "对等主体的登记行必须持 {artifact_cap}"
        );
        // 反向：对等主体不持发言那一项（D-067 的窄行），所以上一条不是「随便都对」。
        assert!(
            !peer.capabilities.iter().any(|c| c == speech_cap),
            "对等主体不持 {speech_cap}——否则上一条对拍是平凡真"
        );
    }
}

// ── 纯决策与答话取值（DE-014 步骤 1）─────────────────────────────────────
//
// 这两个函数是环体里**唯一可脱离 store / sink 单测**的部分：决策只看
// 「有没有工具调用 + 有没有分发通道」，答话取值只看「正文 / 截断 / 两处思考」。
// 钉住它们，执行段搬家就只剩位置变化。

mod round_decision_tests {
    use super::super::{classify_round, final_answer, RoundDecision, StreamNodeFacts};
    use symbio_core::{LlmTurn, ModelFinishReason, TurnToolCallInfo};

    fn one_tool_call() -> TurnToolCallInfo {
        TurnToolCallInfo {
            id: Some("c1".to_string()),
            wire_id: Some("w1".to_string()),
            name: Some("vdfs_read".to_string()),
            arguments: serde_json::json!({}),
            parse_error: None,
            overflow_notice: None,
        }
    }

    /// 四种组合逐一钉住——环的**唯一**决策点，错一格就是「该收束时去派发」。
    #[test]
    fn classify_round_covers_all_four_combinations() {
        let mut rt = LlmTurn::default();
        assert_eq!(
            classify_round(&rt, true),
            RoundDecision::Halt,
            "无工具调用 ⇒ 收束"
        );
        assert_eq!(
            classify_round(&rt, false),
            RoundDecision::Halt,
            "无工具调用时通道有无都不影响收束"
        );
        rt.tool_calls.push(one_tool_call());
        assert_eq!(
            classify_round(&rt, true),
            RoundDecision::Act,
            "有调用且有通道 ⇒ 派发"
        );
        assert_eq!(
            classify_round(&rt, false),
            RoundDecision::ConfigGap,
            "有调用无通道 ⇒ 配置缺口（不是正常收束）"
        );
    }

    /// 答话取值的四条顺序：正文 → 截断 → 端口思考 → 桥上思考。
    #[test]
    fn final_answer_prefers_text_then_reasoning_and_refuses_truncated_draft() {
        let empty = StreamNodeFacts::default();

        let with_text = LlmTurn {
            text: "答案".to_string(),
            reasoning: "思考".to_string(),
            ..Default::default()
        };
        assert_eq!(final_answer(&with_text, &empty), "答案", "正文优先于思考");

        let reasoning_only = LlmTurn {
            text: "   ".to_string(),
            reasoning: "思考".to_string(),
            ..Default::default()
        };
        assert_eq!(
            final_answer(&reasoning_only, &empty),
            "思考",
            "正文空 ⇒ 思考即正文"
        );

        let truncated = LlmTurn {
            text: String::new(),
            reasoning: "被掐断的草稿".to_string(),
            finish: ModelFinishReason::Length,
            ..Default::default()
        };
        assert_eq!(
            final_answer(&truncated, &empty),
            "",
            "finish=length 时草稿不得提升为答案"
        );

        let bridged = StreamNodeFacts {
            reasoning: Some("桥上的思考".to_string()),
            ..Default::default()
        };
        assert_eq!(
            final_answer(&LlmTurn::default(), &bridged),
            "桥上的思考",
            "端口没带思考 ⇒ 用流式桥那一份"
        );
    }

    /// 截断但正文非空：半截正文照常交付（另一条缺口，不在本判据内）。
    #[test]
    fn truncated_text_is_still_delivered() {
        let rt = LlmTurn {
            text: "半截正文".to_string(),
            finish: ModelFinishReason::Length,
            ..Default::default()
        };
        assert_eq!(final_answer(&rt, &StreamNodeFacts::default()), "半截正文");
    }
}

// ── 派发器读账本（判据 4 / 判据 2）：换绑定表的两条夹具 ──────────────────
//
// 判据要的是「**不接线就红**」，不是「按构造不可达」：
// - 判据 4（空账本 ⇒ `exhausted{no_progress}`）：删掉派发器的空账本守卫，本模块第一条
//   用例落在 `step_limit` 而不是 `no_progress` ⇒ 红；
// - 判据 2（名单外的格 ⇒ `exhausted{undeclared-write}`）：删掉对账，第一条越界写入
//   静默入格 ⇒ 红。
//
// 两条都走生产派发器（`turn_exec::drive`），差别只在**绑定表**：真实行表照旧欠
// `announce-model`，那个 `by` 上换的是别的手法（写零格 / 写名单外）。生产动作既不会写零格
// 也不会写名单外的格 ⇒ 没有这条接缝，两条判据只能挂在口头。
mod actor_ledger_tests {
    use crate::control_decl::guard_vocab;
    use std::sync::Arc;
    use symbio_testkit::{trivial_actor_spec, StubLlmAdapter};

    use super::super::super::loop_rows::{ActorEntry, ACTOR_EXECUTE};
    use super::super::super::turn_exec::{ActorError, Ctx, StepFuture};
    use super::super::{TurnInput, TurnRunner};
    use symbio_core::{
        check_all, Entity, Event, EventStore, Seq, SilentDeltas, Store, TokenIssuer, Verb,
    };
    use symbio_plugin_api::events::{EVENT_CHAT_ASSISTANT_FALLBACK, EVENT_MESSAGE_ADDED};

    /// 写**零格**的手法：只记一次模型往返（步数口径归执行体），不落任何事实。
    ///
    /// 那一步 `record_model_step()` 是为反向自检留的：守卫一旦被删，派发器会继续选同一笔债，
    /// 记步数让它收在 `step_limit`（用例红），不记就永不返回（用例挂）。
    fn writes_nothing<'a>(step: &'a mut Ctx<'a>) -> StepFuture<'a> {
        Box::pin(async move {
            step.session.record_model_step();
            Ok(())
        })
    }

    /// 写**名单外**格的手法：`message.added` 既不在 `execute` 的 `writes` 里，也不是协议格。
    fn writes_off_the_list<'a>(step: &'a mut Ctx<'a>) -> StepFuture<'a> {
        Box::pin(async move {
            let turn = step.session.turn();
            step.store
                .append(Event::pending(
                    "fixture-off-list".to_string(),
                    EVENT_MESSAGE_ADDED,
                    Entity::Message,
                    Verb::Asserted,
                    turn,
                    "session/main",
                ))
                .map_err(ActorError::Append)?;
            Ok(())
        })
    }

    /// 夹具绑定表：名单照抄生产声明（[`ACTOR_EXECUTE`]），只换手法。
    static ACTORS_WRITE_NOTHING: &[ActorEntry] = &[ActorEntry {
        decl: ACTOR_EXECUTE,
        run: writes_nothing,
    }];
    static ACTORS_WRITE_OFF_LIST: &[ActorEntry] = &[ActorEntry {
        decl: ACTOR_EXECUTE,
        run: writes_off_the_list,
    }];

    /// 本轮入参：先落用户格（与 [`super::seed_user`] 同一条纪律），执行体一律不给工具。
    fn input(store: &dyn Store<Event = Event>) -> TurnInput {
        let user_seq = super::seed_user(store, 0, "账本夹具");
        TurnInput {
            turn: 0,
            user_seq,
            drive_seq: user_seq,
            inject: None,
            window_turns: None,
            actor: trivial_actor_spec("session/main"),
            final_reply: None,
            final_cost_ms: 0,
            turn_root_id: String::new(),
            dialog: None,
            nodes: None,
            abort: None,
        }
    }

    /// 空账本：动作什么都没写 ⇒ 环不会前进 ⇒ 收在 `exhausted{no_progress}`。
    ///
    /// 这条用例钉的是「执行体不自报进展」这件事：账本是日志尾段，不是返回值。
    #[tokio::test]
    async fn an_actor_that_writes_nothing_stops_as_no_progress() {
        let store: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>> =
            std::sync::Arc::new(EventStore::new());
        let tok = TokenIssuer::issue_deep();
        let llm = std::sync::Arc::new(StubLlmAdapter::succeed("stub-model"));

        let out = TurnRunner
            .drive_with_actors(
                &store,
                llm.clone(),
                tok,
                input(store.as_ref()),
                Arc::new(SilentDeltas),
                std::sync::Arc::from(Vec::new()),
                None,
                ACTORS_WRITE_NOTHING,
                None,
                None,
            )
            .await
            .expect("到点必答：停机也是一句话");

        assert!(out.fell_back, "什么都没写的收束是失败形状，不是「答完了」");
        let snapshot = store.range(Seq::new(0));
        let last = snapshot.last().expect("有收束格");
        assert_eq!(last.kind, EVENT_CHAT_ASSISTANT_FALLBACK);
        assert_eq!(last.payload["reason"], "exhausted");
        assert_eq!(
            last.payload["detail"], "no_progress",
            "空账本收在 no_progress；删掉派发器的守卫会收在 step_limit——那条分支就是这条用例的红"
        );
        assert!(
            check_all(guard_vocab(), &snapshot).is_empty(),
            "{:?}",
            check_all(guard_vocab(), &snapshot)
        );
    }

    /// 名单外的写入：动作落了一格 `execute` 没声明的领域格 ⇒ 收在
    /// `exhausted{undeclared-write}`，且那一格**确实在账本里**（对账读的是实际写入）。
    #[tokio::test]
    async fn an_actor_that_writes_off_its_list_stops_as_undeclared_write() {
        let store: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>> =
            std::sync::Arc::new(EventStore::new());
        let tok = TokenIssuer::issue_deep();
        let llm = std::sync::Arc::new(StubLlmAdapter::succeed("stub-model"));

        let out = TurnRunner
            .drive_with_actors(
                &store,
                llm.clone(),
                tok,
                input(store.as_ref()),
                Arc::new(SilentDeltas),
                std::sync::Arc::from(Vec::new()),
                None,
                ACTORS_WRITE_OFF_LIST,
                None,
                None,
            )
            .await
            .expect("越界也是一条事实，不是 panic");

        assert!(out.fell_back);
        let snapshot = store.range(Seq::new(0));
        let kinds: Vec<&str> = snapshot.iter().map(|e| e.kind.as_str()).collect();
        assert!(
            kinds.contains(&EVENT_MESSAGE_ADDED),
            "越界那一格要先真入格，对账才有东西可对：{kinds:?}"
        );
        let last = snapshot.last().expect("有收束格");
        assert_eq!(last.kind, EVENT_CHAT_ASSISTANT_FALLBACK);
        assert_eq!(
            last.payload["detail"], "undeclared-write",
            "名单外即停机；删掉对账这一步，这条用例红在 `no_progress`/静默收束上"
        );
    }
}

// ── 宣布与派出之间被杀的那一笔（D-166）─────────────────────────────────
//
// 两个环步之间没有 `sent`，所以租约看护看不见它（`stale_sent` 只按派出之后卡住的
// 那些开账）。没人管它就会一直开着：新的一幕照旧把它选出来，而等它的 future 早就
// 没了。这条用例钉的是「判死」这一半——不判死，那一笔会永远占着 `dispatch-model`。
mod dangling_round_tests {
    use symbio_core::{Entity, Event, Seq, Verb};

    /// 一笔**已宣布、没派出**的请求：`kill_dangling_round` 给它落 `failed`。
    #[tokio::test]
    async fn an_undispatched_request_is_killed_with_its_own_identity() {
        let store: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>> =
            std::sync::Arc::new(symbio_core::EventStore::new());
        store
            .append(
                Event::pending(
                    "request-model-m-1-1",
                    symbio_core::EVENT_REQUEST,
                    Entity::Turn,
                    Verb::Asserted,
                    3,
                    "system",
                )
                .with_produced_by(7)
                .with_payload(serde_json::json!({
                    "port": "model",
                    "id": "model-3-d5-s0",
                    "attempt": 1,
                })),
            )
            .expect("夹具落请求");

        let killed = super::super::stream::kill_dangling_round(store.as_ref()).expect("判死不失败");
        assert!(killed, "有一笔开着的请求 ⇒ 必须判死");

        let facts = store.range(Seq::new(0));
        let failed = facts
            .iter()
            .find(|e| e.kind == symbio_core::EVENT_FAILED)
            .expect("判死要落 failed");
        assert_eq!(failed.payload["id"], "model-3-d5-s0", "身份随原请求走");
        assert_eq!(failed.payload["attempt"], 1);
        assert_eq!(failed.payload["why"], "no_pending_round");
        assert_eq!(failed.turn, 3, "判死落在**那一笔**所属的轮上，不是当前轮");
        assert_eq!(failed.produced_by, Some(7), "溯源随原请求走");

        // 第二遍没有可判的：不能重复落 `failed`（幂等键会撞，但更该什么都不写）。
        assert!(
            !super::super::stream::kill_dangling_round(store.as_ref()).expect("再判一次"),
            "已经关掉的那笔不再算开着"
        );
    }

    /// 派出过的请求**不**归它管：`sent` 之后的失败由租约看护 / 适配器落。
    #[tokio::test]
    async fn a_dispatched_request_is_left_to_the_lease_keeper() {
        let store: std::sync::Arc<dyn symbio_core::Store<Event = symbio_core::Event>> =
            std::sync::Arc::new(symbio_core::EventStore::new());
        for (kind, payload) in [
            (
                symbio_core::EVENT_REQUEST,
                serde_json::json!({"port": "model", "id": "model-1-d0-s0", "attempt": 1}),
            ),
            (
                symbio_core::EVENT_SENT,
                serde_json::json!({"port": "model", "id": "model-1-d0-s0", "attempt": 1}),
            ),
        ] {
            store
                .append(
                    Event::pending(
                        format!("{kind}-fixture"),
                        kind,
                        Entity::Turn,
                        Verb::Asserted,
                        1,
                        "system",
                    )
                    .with_payload(payload),
                )
                .expect("夹具落格");
        }
        assert!(
            !super::super::stream::kill_dangling_round(store.as_ref()).expect("不该失败"),
            "派出过的请求有等待方，判死是租约看护的活"
        );
    }
}

// ── 应答载荷的实测用量（写侧）────────────────────────────────────

/// 判据：**「测到了」的唯一记号是 `usage` 这个键在不在**。
///
/// 三件事一起才叫判据（读侧 `token_ledger` 的 `measured` 就靠这个记号分辨
/// 「没测量」与「测到零」）：
///
/// 1. 有实测 ⇒ 载荷带 `usage`，两侧各一；
/// 2. provider 没给（`None`）⇒ 键**不出现**，不是 `usage: null`——后者在收数侧
///    与「测到 0」同形，而两件事的处置完全不同；
/// 3. 只报一侧（有的协议只报 `output_tokens`）⇒ 只写那一侧，缺的那侧不补 0；
/// 4. 失败格（没有 `rt`）不带 usage——失败的那次往返没有实测可报。
#[test]
fn the_response_payload_carries_the_measured_usage() {
    let facts = super::facts::node_facts(&None);
    let turn = |usage: Option<symbio_core::ModelUsage>| symbio_core::LlmTurn {
        text: "答".into(),
        reasoning: String::new(),
        tool_calls: Vec::new(),
        finish: symbio_core::ModelFinishReason::Stop,
        cost_ms: 1,
        usage,
        spend: None,
    };
    let fields = |payload: &symbio_core::LlmTurn| {
        super::facts::output_fields(
            "model-1-d0-r0",
            "turn-1",
            "session/main",
            &facts,
            Some(payload),
        )
    };

    let both = fields(&turn(Some(symbio_core::ModelUsage {
        input: Some(1200),
        output: Some(340),
    })));
    assert_eq!(both["usage"]["input"], serde_json::json!(1200));
    assert_eq!(both["usage"]["output"], serde_json::json!(340));

    // 没给：键不出现（`null` 会被读成「测到 0」）。
    let none = fields(&turn(None));
    assert!(
        none.get("usage").is_none(),
        "provider 没给就不能写 `usage`：{none}"
    );

    // 只报一侧。
    let only = fields(&turn(Some(symbio_core::ModelUsage {
        input: None,
        output: Some(7),
    })));
    assert_eq!(only["usage"]["output"], serde_json::json!(7));
    assert!(
        only["usage"].get("input").is_none(),
        "缺的那侧不补 0：{only}"
    );

    // 失败格。
    let failed =
        super::facts::output_fields("model-1-d0-r0", "turn-1", "session/main", &facts, None);
    assert!(failed.get("usage").is_none(), "{failed}");
}
