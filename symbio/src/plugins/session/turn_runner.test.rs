//! `turn_runner` 单测 —— 随运行器一起从 `symbio_core::actors` 下沉（2026-10-09）。
//!
//! 三段：`turn_runner_tests`（单轮闭环 / WAL 持久 / 兜底 / 授权闸）、
//! `tool_round_tests`（工具轮：注入、前缀、窗口、产物、待办、恢复）、
//! `reflex_turn_tests`（反射档落格）。
//!
//! 另附 `run` / `run_streaming` 两个**测试专用**入口：生产一律走
//! `TurnRunner::run_with_tools`（`v2_exec` 自己构造 `ActorSpec`），无工具轮的简单路径
//! 只有本文件的用例用得到 ⇒ 按「测试消费的代码住测试模块」就地定义（`impl` 复用调用点的
//! `.run(…)` 写法），**不占生产文件的表面**（生产面只留 `run_with_tools` / `run_reflex`）。

use super::{TurnInput, TurnOutcome, TurnRunner};
use crate::symbio_core::{
    ActorSpec, AppendError, DeltaSink, Event, FullModel, LatencyTier, LlmAdapter, SilentDeltas,
    Store,
};

impl TurnRunner {
    /// 跑一轮（测试侧）：用户消息（声明档位）→ 生成 → 收束事件落格。
    ///
    /// `turn` 是本轮的 turn 号（调用方保证单调递增——N3 的轮次锚）；`tier` 是本轮装配进
    /// 哪一档（随用户消息入格，ADR-044）。令牌 `tok` 是闸门：只有持 `FullModel` 的调用方
    /// 进得来（反射 / 快速档在类型上就到不了这里）。
    pub async fn run<S>(
        &self,
        store: &S,
        llm: &dyn LlmAdapter,
        tok: &FullModel,
        turn: u64,
        text: &str,
        tier: LatencyTier,
    ) -> Result<TurnOutcome, AppendError>
    where
        S: Store<Event = Event>,
    {
        self.run_streaming(
            store,
            llm,
            tok,
            TurnInput {
                turn,
                text: text.to_string(),
                tier,
                window_turns: None,
                // 单请求入口（无工具轮）没有轮内循环 ⇒ 没有注入时机。
                inject: None,
                resume: None,
                // 平凡值身份（S08 §4：单主体 = 今天的状态）。
                actor: ActorSpec::trivial("agent:main"),
                // 没有请求视图层可拼（三段由调用方的 `build_request_view` 给出）。
                prefix: None,
                // 恒走生成：定稿答话轮由 `v2_exec` 自己把答话交进来。
                final_reply: None,
            },
            std::sync::Arc::new(SilentDeltas),
        )
        .await
    }

    /// 流式版（测试侧）：生成增量逐片经 `sink` 送出；落格语义与 [`Self::run`] 同一条路径。
    ///
    /// 无工具轮 = [`Self::run_with_tools`] 的**入参为空的同一路径**（工具清单空、无分发方）
    /// ——「有工具」与「无工具」不是两条执行路径。
    pub async fn run_streaming<S>(
        &self,
        store: &S,
        llm: &dyn LlmAdapter,
        tok: &FullModel,
        input: TurnInput,
        sink: std::sync::Arc<dyn DeltaSink>,
    ) -> Result<TurnOutcome, AppendError>
    where
        S: Store<Event = Event>,
    {
        self.run_with_tools(store, llm, tok, input, sink, &[], None)
            .await
    }
}

// ── v2 会话运行时：TurnRunner 验收（不变量靠构造成立 + WAL 持久 + 兜底联动）──

mod turn_runner_tests {
    use super::super::{TurnInput, TurnRunner};
    use crate::symbio_core::{
        check_all, cost_ledger, fallback_rate, transcript, Budget, Event, EventStore, LatencyTier,
        Seq, Store, StubLlmAdapter, TokenIssuer, WalStore, EVENT_USER_MESSAGE,
    };

    /// 成功轮：final 落格、溯源指向本轮用户消息、实测 cost_ms > 0、
    /// 不变量绿、收束投影 settled；成本台账与兜底率同账（ADR-044）。
    #[tokio::test]
    async fn turn_runner_happy_path_lands_final_with_measured_cost() {
        let store = crate::symbio_core::EventStore::new();
        let tok = TokenIssuer::issue_deep();
        let llm = StubLlmAdapter::succeed("stub-model");

        let out = TurnRunner
            .run(&store, &llm, &tok, 0, "你好", LatencyTier::Deep)
            .await
            .expect("桩必答");
        assert_eq!(out.turn, 0);
        assert!(!out.fell_back);
        assert!(out.text.contains("你好"), "桩回显 prompt：{}", out.text);
        // 桩零延迟 ⇒ cost_ms 可为 0（诚实值）；真实链路的正数断言在真实端点彩排里。

        let snapshot = store.range(Seq::new(0));
        assert!(
            check_all(&snapshot).is_empty(),
            "{:?}",
            check_all(&snapshot)
        );

        // 成本台账：实测值入账（agent:main 一笔）。
        let ledger = cost_ledger().apply(&snapshot, i64::MAX, Budget::generous());
        assert_eq!(ledger.value.of("agent:main").spent_ms, out.cost_ms);

        // 兜底率：deep 档 1 turn，0 兜底。
        let fr = fallback_rate().apply(&snapshot, i64::MAX, Budget::generous());
        let deep = fr.value.of("deep");
        assert_eq!((deep.turns, deep.fallbacks), (1, 0));
        assert_eq!(deep.rate(), 0.0);

        // turnstate：收束且 final 可见。
        let view = crate::symbio_core::turnstate().apply(&snapshot, 0, Budget::generous());
        assert!(view.value.settled() && view.value.final_text.is_some());
    }

    /// 失败轮（I3）：兜底事件落格（`turn × closed` + `why`），`fell_back = true`，
    /// 不变量照常绿——「失败也是一句话」，不是静默。
    #[tokio::test]
    async fn turn_runner_failure_lands_fallback_not_silence() {
        let store = crate::symbio_core::EventStore::new();
        let tok = TokenIssuer::issue_deep();
        let llm = StubLlmAdapter::always_fail("模型挂了");

        let out = TurnRunner
            .run(&store, &llm, &tok, 0, "你好", LatencyTier::Deep)
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
            check_all(&snapshot).is_empty(),
            "{:?}",
            check_all(&snapshot)
        );

        // 兜底率：deep 档 1 turn 1 兜底 = 100%（被看见，不是被藏）。
        let fr = fallback_rate().apply(&snapshot, i64::MAX, Budget::generous());
        let deep = fr.value.of("deep");
        assert_eq!((deep.turns, deep.fallbacks), (1, 1));
        assert_eq!(deep.rate(), 1.0);
    }

    /// WAL 持久：多轮写入 → 重开 → 逐字节一致（N2 家族：恢复 = 重放，
    /// 投影不需要状态迁移）；turn 号跨重启由调用方续排（N3 仍成立）。
    #[tokio::test]
    async fn turn_runner_over_wal_survives_reopen() {
        let dir = std::env::temp_dir().join(format!("symbio-turn-runner-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("临时目录");
        let wal = dir.join("session.wal");

        let tok = TokenIssuer::issue_deep();
        let llm = StubLlmAdapter::succeed("stub-model");
        {
            let store = WalStore::<Event>::open(&wal).expect("open");
            for turn in 0..3u64 {
                TurnRunner
                    .run(&store, &llm, &tok, turn, "问", LatencyTier::Deep)
                    .await
                    .expect("桩必答");
            }
        } // drop ⇒ 文件在盘上

        let reopened = WalStore::<Event>::open(&wal).expect("reopen");
        assert_eq!(reopened.head().value(), 6, "3 轮 × 2 事件");

        // 恢复后直接续排（turn 号接续）——N3 由 turn 号单调保证。
        let out = TurnRunner
            .run(&reopened, &llm, &tok, 3, "再问", LatencyTier::Deep)
            .await
            .expect("桩必答");
        assert_eq!(out.turn, 3);

        let snapshot = reopened.range(Seq::new(0));
        assert!(
            check_all(&snapshot).is_empty(),
            "{:?}",
            check_all(&snapshot)
        );
        assert_eq!(snapshot.len(), 8);

        // 与一遍直写（不经过重启）的台账逐字节一致（N1/N2 家族）。
        let fresh = crate::symbio_core::EventStore::new();
        for turn in 0..4u64 {
            TurnRunner
                .run(
                    &fresh,
                    &llm,
                    &tok,
                    turn,
                    if turn == 3 { "再问" } else { "问" },
                    LatencyTier::Deep,
                )
                .await
                .unwrap();
        }
        let a = cost_ledger().apply(&snapshot, i64::MAX, Budget::generous());
        let b = cost_ledger().apply(&fresh.range(Seq::new(0)), i64::MAX, Budget::generous());
        assert_eq!(a.value.total_ms, b.value.total_ms, "恢复后的台账与直写一致");
        assert_eq!(a.value.by_principal, b.value.by_principal);

        std::fs::remove_dir_all(&dir).ok();
    }

    /// 多轮 turn 号单调：N3（每 turn ≤ 1 条 final）由构造成立——
    /// 同一 turn 号重复使用被幂等键挡住（事件 id 撞车 ⇒ Duplicate）。
    #[tokio::test]
    async fn turn_runner_reused_turn_number_is_rejected() {
        let store = crate::symbio_core::EventStore::new();
        let tok = TokenIssuer::issue_deep();
        let llm = StubLlmAdapter::succeed("stub-model");

        TurnRunner
            .run(&store, &llm, &tok, 0, "第一轮", LatencyTier::Deep)
            .await
            .unwrap();
        let err = TurnRunner
            .run(&store, &llm, &tok, 0, "重复 turn 号", LatencyTier::Deep)
            .await;
        assert!(err.is_err(), "事件 id 撞车必须被 Store 幂等键拒绝");
        assert!(matches!(
            err,
            Err(crate::symbio_core::AppendError::Duplicate)
        ));
    }

    /// 多轮对话带历史：③ transcript 投影读同一事实源，第二轮请求含第一轮。
    ///
    /// 判据按**角色**查（不只问「历史在不在」，还问「上一轮的提问是 `role: user`、
    /// 答复是 `role: assistant`」）——历史就是消息序列，没有分隔符。
    #[tokio::test]
    async fn multi_turn_transcript_carries_history_from_fact_source() {
        let store = EventStore::new();
        let tok = TokenIssuer::issue_deep();
        let llm = StubLlmAdapter::succeed("stub-model");

        // 第一轮：单轮 = 只有当前那条 user 消息。
        let r1 = TurnRunner
            .run(&store, &llm, &tok, 0, "第一轮问题", LatencyTier::Deep)
            .await
            .unwrap();
        assert!(r1.text.contains("第一轮问题"));

        // 第二轮：历史在场，且**角色正确**——模型看得见自己上一轮的答复，
        // 并且知道那句答复是自己说的（不是用户说的）。
        let r2 = TurnRunner
            .run(&store, &llm, &tok, 1, "第二轮问题", LatencyTier::Deep)
            .await
            .unwrap();
        let last = crate::symbio_core::Reasoner::render_messages(
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
            check_all(&snapshot).is_empty(),
            "{:?}",
            check_all(&snapshot)
        );
        let view = transcript().apply(&snapshot, i64::MAX, Budget::generous());
        assert_eq!(view.value.entries.len(), 4, "两轮 × (问+答)");
        assert_eq!(view.value.entries[0].role, "user");
        assert_eq!(view.value.entries[1].role, "assistant");
        let again = transcript().apply(&snapshot, i64::MAX, Budget::generous());
        assert_eq!(view.value, again.value);
    }

    /// fallback 行进转写（assistant / why）——兜底话术是用户实际看到的回复。
    #[test]
    fn transcript_includes_fallback_as_assistant_line() {
        use crate::symbio_core::{Entity, Verb, EVENT_ASSISTANT_FALLBACK, EVENT_USER_MESSAGE};
        let store = EventStore::new();
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
                    EVENT_ASSISTANT_FALLBACK,
                    Entity::Turn,
                    Verb::Closed,
                    0,
                    "agent:main",
                )
                .with_produced_by(0)
                .with_payload(serde_json::json!({ "why": "上游 402" })),
            )
            .unwrap();
        let snapshot = store.range(Seq::new(0));
        let view = transcript().apply(&snapshot, i64::MAX, Budget::generous());
        assert_eq!(view.value.entries.len(), 2);
        assert_eq!(view.value.entries[1].role, "assistant");
        assert_eq!(view.value.entries[1].text, "上游 402");
    }

    /// 工具结果进转写（`tool` / 工具名 + text）——跨轮 prompt 因此能重建**含工具**的
    /// 对话（[plan/10 批 3](../../../../docs/plan/10-工具轮v2化实施方案.md)）；且工具行
    /// 投影成 `role: "tool"` 的**结构化消息**（带工具名与合成 `tool_call_id`），与轮内
    /// 交换（`tool_exchange_messages`）同形。
    #[test]
    fn transcript_includes_artifact_as_tool_line() {
        use crate::symbio_core::{
            Entity, Verb, EVENT_ARTIFACT_ADDED, EVENT_ASSISTANT_FINAL, EVENT_USER_MESSAGE,
        };
        let store = EventStore::new();
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
                    "agent:main",
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
                    EVENT_ASSISTANT_FINAL,
                    Entity::Turn,
                    Verb::Closed,
                    0,
                    "agent:main",
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
        let view = transcript().apply(&snapshot, i64::MAX, Budget::generous());
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

    impl crate::symbio_core::DeltaSink for CollectingDeltas {
        fn on_delta(&self, text: &str) {
            self.0.lock().unwrap().push(text.to_string());
        }
        // 本组用例的桩不产推理增量；推理通道由
        // `plugins/model/bound_provider.test.rs` 的桥接用例覆盖（那里有 SSE 级 fixture）。
        fn on_reasoning(&self, _text: &str) {}
    }

    /// 流式运行：分片按序进 sink；落格语义与 run() 同一条路径（final 照常、
    /// 不变量照绿）；无分片的桩走默认降级（一次性全文一帧）。
    #[tokio::test]
    async fn streaming_run_forwards_deltas_and_writes_final() {
        use crate::symbio_core::DeltaSink;
        use std::sync::{Arc, Mutex};

        let store = EventStore::new();
        let tok = TokenIssuer::issue_deep();
        let llm = StubLlmAdapter::succeed_streaming("stub-model", &["你", "好", "！"]);
        let got = Arc::new(CollectingDeltas(Mutex::new(Vec::new())));

        let out = TurnRunner
            .run_streaming(
                &store,
                &llm,
                &tok,
                TurnInput {
                    turn: 0,
                    text: "问".into(),
                    tier: LatencyTier::Deep,
                    window_turns: None,
                    resume: None,
                    inject: None,
                    actor: crate::symbio_core::ActorSpec::trivial("agent:main"),
                    // 请求级前缀：测试不走请求视图层（三段皆空）。
                    prefix: None,
                    final_reply: None,
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
            check_all(&snapshot).is_empty(),
            "{:?}",
            check_all(&snapshot)
        );
        let view = transcript().apply(&snapshot, i64::MAX, Budget::generous());
        assert_eq!(view.value.entries[1].text, "你好！", "final 落的是全文");

        // 默认降级：无分片桩经 run_streaming = 一帧全文（诚实降级，不是静默）。
        let store2 = EventStore::new();
        let llm2 = StubLlmAdapter::succeed("stub-model");
        let got2 = Arc::new(CollectingDeltas(Mutex::new(Vec::new())));
        TurnRunner
            .run_streaming(
                &store2,
                &llm2,
                &tok,
                TurnInput {
                    turn: 0,
                    text: "问".into(),
                    tier: LatencyTier::Deep,
                    inject: None,
                    window_turns: None,
                    resume: None,
                    actor: crate::symbio_core::ActorSpec::trivial("agent:main"),
                    // 请求级前缀：测试不走请求视图层（三段皆空）。
                    prefix: None,
                    final_reply: None,
                },
                got2.clone() as Arc<dyn DeltaSink>,
            )
            .await
            .unwrap();
        let frames = got2.0.lock().unwrap();
        assert_eq!(frames.len(), 1, "非流式适配器 = 一帧全文");
        assert!(frames[0].contains("问"));
    }

    /// 中止（AdapterError::Aborted）：**不落收束格**——网格只剩已入格的用户
    /// 消息（少一格是诚实的缺口，ADR-044 同源），`aborted` 为真且 `text` 为空；
    /// 不变量照绿（C4 会看见这个缺口，那是设计而非缺陷：中止不是静默中断）。
    #[tokio::test]
    async fn aborted_run_leaves_only_user_event() {
        use crate::symbio_core::unresolved_turns;

        let store = EventStore::new();
        let tok = TokenIssuer::issue_deep();
        let llm = StubLlmAdapter::aborting("stub-model");

        let out = TurnRunner
            .run(&store, &llm, &tok, 0, "问", LatencyTier::Deep)
            .await
            .unwrap();

        assert!(out.aborted, "中止必须在结果里可见");
        assert!(!out.fell_back, "中止不是兜底（两者互斥）");
        assert!(out.text.is_empty(), "中止没有答复文本");

        let snapshot = store.range(Seq::new(0));
        assert_eq!(snapshot.len(), 1, "只剩用户格：{snapshot:?}");
        assert_eq!(snapshot[0].kind, EVENT_USER_MESSAGE);
        assert!(
            check_all(&snapshot).is_empty(),
            "{:?}",
            check_all(&snapshot)
        );
        assert_eq!(
            unresolved_turns(&snapshot, false).len(),
            1,
            "缺口要**可被判出**（不是假装收束）——C4 看得见它"
        );
    }

    /// 写侧闸（[04 §3.1 批⑥](../../../../docs/plan/04-工程落地.md) 的 **full 档半边**）：
    /// 主体不持 `reply.first` ⇒ 收束格**不落**——该轮留在未收束态（C4 报得出），
    /// 但用户的答案仍回给调用方。
    ///
    /// ## 这个用例挡的是什么
    ///
    /// 收束格的写方**跟着执行路径走**：`bridge` 档由 `v2_facts::record_to_wal` 落格
    /// （那里有 `authorize_close`），`full` 档由**运行器**原生落格。闸原先只在桥档，
    /// 运行器这一侧整条漏判——而**没有任何东西会变红**（两条路径各写各的收束格，
    /// 谁也不看谁）。与 S12 那批查出的「文档断言了、生产数据里却相反」是同一类缺口。
    ///
    /// 判据分两半，各自钉一件事：
    /// - **拒绝**：`PRINCIPAL_AUTONOMOUS`（表里只持 `define.work`）驱动一轮 ⇒ 网格里
    ///   **没有** `chat.assistant.final`，`unresolved_turns` 看得见这一格缺口；
    /// - **放行对照**：`agent:main` 驱动同样一轮 ⇒ final 照落（否则「拒绝」可能只是
    ///   「闸把所有人都拒了」的平凡真）。
    #[tokio::test]
    async fn closure_is_withheld_when_the_principal_lacks_the_grant() {
        use crate::symbio_core::authz::PRINCIPAL_AUTONOMOUS;
        use crate::symbio_core::unresolved_turns;
        use crate::symbio_core::ActorSpec;
        use crate::symbio_core::SilentDeltas;

        // ── 拒绝：自主发起者不持 `reply.first` ⇒ 收束格不落 ────────────────
        let store = EventStore::new();
        let tok = TokenIssuer::issue_deep();
        let llm = StubLlmAdapter::succeed("stub-model");
        let out = TurnRunner
            .run_with_tools(
                &store,
                &llm,
                &tok,
                TurnInput {
                    turn: 0,
                    text: "问".into(),
                    tier: LatencyTier::Deep,
                    inject: None,
                    window_turns: None,
                    resume: None,
                    actor: ActorSpec::trivial(PRINCIPAL_AUTONOMOUS),
                    // 请求级前缀：测试不走请求视图层（三段皆空）。
                    prefix: None,
                    final_reply: None,
                },
                std::sync::Arc::new(SilentDeltas),
                &[],
                None,
            )
            .await
            .expect("落格不报错——拒绝的语义是**不入格**，不是失败");
        assert!(!out.fell_back, "拒绝不是兜底");

        let snapshot = store.range(Seq::new(0));
        assert!(
            !snapshot
                .iter()
                .any(|e| e.kind == crate::symbio_core::EVENT_ASSISTANT_FINAL),
            "缺 reply.first ⇒ 收束格不得入格：{:?}",
            snapshot.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>()
        );
        assert_eq!(
            unresolved_turns(&snapshot, false).len(),
            1,
            "拒绝要**可被判出**（留在未收束态）——C4 看得见它"
        );

        // ── 放行对照：`agent:main` 持 `reply.first` ⇒ final 照落 ───────────
        let store2 = EventStore::new();
        TurnRunner
            .run_with_tools(
                &store2,
                &llm,
                &tok,
                TurnInput {
                    turn: 0,
                    text: "问".into(),
                    tier: LatencyTier::Deep,
                    inject: None,
                    window_turns: None,
                    resume: None,
                    actor: ActorSpec::trivial("agent:main"),
                    // 请求级前缀：测试不走请求视图层（三段皆空）。
                    prefix: None,
                    final_reply: None,
                },
                std::sync::Arc::new(SilentDeltas),
                &[],
                None,
            )
            .await
            .expect("桩必答");
        let snapshot2 = store2.range(Seq::new(0));
        assert!(
            snapshot2
                .iter()
                .any(|e| e.kind == crate::symbio_core::EVENT_ASSISTANT_FINAL),
            "持有 reply.first ⇒ final 照落（否则拒绝可能只是「闸全拒」的平凡真）"
        );
    }
}

// ── 工具轮（`run_with_tools`）：产物落格 + 结果回灌 + 等待用户停下 ──────────

mod tool_round_tests {
    use std::sync::{Arc, Mutex};

    use super::super::{RoundInjector, TurnInput, TurnResume, TurnRunner};
    use crate::symbio_core::{
        check_all, unresolved_turns, AdapterError, CapabilityMeta, DeltaSink, DispatchOutcome,
        DispatchPort, Entity, EventStore, FullModel, LatencyTier, LlmAdapter, LlmTurn,
        PromptMessage, Seq, SilentDeltas, Store, TokenIssuer, TurnToolCallInfo, Verb,
        EVENT_ARTIFACT_ADDED, EVENT_ASSISTANT_FINAL, EVENT_USER_MESSAGE,
    };

    /// 注入的消息必须**进本轮的下一次请求**。
    ///
    /// ## 为什么 core 层只验这一半
    ///
    /// 另一半——「注入的内容成为事实、下一轮仍可见」——**不在 core**：
    /// 「什么是补充」「它怎么落成 `turn × asserted`」是插件侧的概念（收件箱、
    /// `merge_supplements`、`EventWalStore` 都在 `plugins/session`），core 只提供
    /// 「循环里什么时候问一次」这个挂点（见 [`TurnInput::inject`]）。
    ///
    /// 把落格也做进 core 会让 core 知道「补充」是什么——而它不必知道。
    /// 那一半的判据在 `v2_exec.test.rs`（`supplements_become_facts_visible_next_turn`）。
    ///
    /// ## 反向自检
    ///
    /// 把 2f 那两行（`if let Some(inject) = &inject { exchange.extend(inject()); }`）
    /// 删掉，本用例必红。
    #[tokio::test]
    async fn injected_messages_reach_the_next_request_of_the_same_turn() {
        let store = EventStore::new();
        let tok = TokenIssuer::issue_deep();
        let llm = ToolCallingLlm::new();
        let prompts = llm.prompts.clone();
        let dispatch = FakeDispatch {
            pending: false,
            rounds: Arc::new(Mutex::new(0)),
        };

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

        let mut i = input();
        i.inject = Some(inject);
        TurnRunner
            .run_with_tools(
                &store,
                &llm,
                &tok,
                i,
                Arc::new(SilentDeltas),
                &tools(),
                Some(&dispatch),
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
                tool_calls: Vec::new(),
                cost_ms: 5,
                usage: Some(crate::symbio_core::ModelUsage {
                    input: Some(11),
                    output: Some(22),
                }),
            })
        }
    }

    /// 假适配器：把**收到的最后一条消息**当答复（`run` 系列会把答复反射进事实源，
    /// 便于断言「历史里有哪几轮」）；记录每次收到的**消息数组**。
    ///
    /// ## 为什么记录消息而不是 prompt 字符串（ADR-048a）
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
                tool_calls: Vec::new(),
                cost_ms: 1,
                usage: None,
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
                    tool_calls: vec![TurnToolCallInfo {
                        id: Some("tc-1".into()),
                        wire_id: Some("w-1".into()),
                        name: Some("vdfs_read".into()),
                        arguments: serde_json::json!({ "path": "a.md" }),
                        parse_error: None,
                    }],
                    cost_ms: 3,
                    usage: None,
                })
            } else {
                Ok(LlmTurn {
                    text: "读到了。".into(),
                    tool_calls: Vec::new(),
                    cost_ms: 4,
                    usage: None,
                })
            }
        }
    }

    /// 假分发方：把工具调用映射成一条结果事实；`pending` 决定是否收束于等用户。
    struct FakeDispatch {
        pending: bool,
        rounds: Arc<Mutex<usize>>,
    }

    #[async_trait::async_trait]
    impl DispatchPort for FakeDispatch {
        async fn dispatch(&self, turn: &LlmTurn) -> Vec<DispatchOutcome> {
            *self.rounds.lock().unwrap() += 1;
            turn.tool_calls
                .iter()
                .map(|tc| DispatchOutcome {
                    call_id: tc.id.clone().unwrap_or_default(),
                    name: tc.name.clone().unwrap_or_default(),
                    text: "文件内容：hello".into(),
                    ok: true,
                    needs_user_action: self.pending,
                })
                .collect()
        }
    }

    fn tools() -> Vec<CapabilityMeta> {
        vec![CapabilityMeta {
            name: "vdfs_read".into(),
            ..Default::default()
        }]
    }

    fn input() -> TurnInput {
        TurnInput {
            turn: 0,
            text: "读 a.md".into(),
            tier: LatencyTier::Deep,
            inject: None,
            window_turns: None,
            resume: None,
            actor: crate::symbio_core::ActorSpec::trivial("agent:main"),
            // 请求级前缀：测试不走请求视图层（三段皆空）。
            prefix: None,
            final_reply: None,
        }
    }

    /// 定稿答话轮（缺口 5）：`final_reply` 给定时**不调模型**，但两格照落。
    ///
    /// ## 为什么要钉「两格都在」
    ///
    /// `Answered` 那一轮在 v1 路径上**根本不进运行器** ⇒ 事实网格里一格都没有。下一轮
    /// prompt 从网格投影 ⇒ 这次问答整个消失，而 `messages.json` 里有 ⇒ **两条真源**。
    /// 判据钉的是「用户格 + 收束格都在、且收束格溯源指向用户格」，不是「答话文本对不对」。
    ///
    /// ## 为什么要钉「不调模型」
    ///
    /// 这一轮一次模型往返都不该发——那是 `Answered` 的全部意义。用记录型桩的 `prompts`
    /// 长度来钉：它长了就说明请求发了出去。
    ///
    /// **反向自检**：把 `run_with_tools` 步骤 1b 那段短路删掉，本用例必红（桩会被调到）。
    #[tokio::test]
    async fn final_reply_lands_both_cells_without_calling_the_model() {
        let store = EventStore::new();
        let tok = TokenIssuer::issue_deep();
        let llm = ToolCallingLlm::new();
        let prompts = llm.prompts.clone();
        let mut i = input();
        i.final_reply = Some("北京今天晴。".into());
        let out = TurnRunner
            .run_with_tools(&store, &llm, &tok, i, Arc::new(SilentDeltas), &[], None)
            .await
            .unwrap();
        assert_eq!(out.text, "北京今天晴。", "答话就是这一轮的输出");
        assert!(!out.fell_back, "定稿答话不是兜底");
        assert!(
            prompts.lock().unwrap().is_empty(),
            "定稿答话轮一次模型请求都不该发"
        );
        let ev = store.range(Seq::new(0));
        assert_eq!(ev.len(), 2, "用户格 + 收束格，一格不多一格不少");
        assert_eq!(ev[0].kind, EVENT_USER_MESSAGE);
        assert_eq!(ev[1].kind, EVENT_ASSISTANT_FINAL);
        assert_eq!(
            ev[1].produced_by,
            ev[0].seq.map(|s| s.value()),
            "I2：收束格的溯源必须指向本轮用户格"
        );
    }

    /// `TurnInput::prefix` 排在基线**之前**，且**不进**「就地累积」那条路。
    ///
    /// ## 为什么要钉「在之前」而不是只钉「在里面」
    ///
    /// 三段（记忆召回 / 就绪任务集 / 委派者真源）在 v1 里是**置顶**消息，语义上在
    /// 对话之前。若前缀被追加到基线末尾（本轮发言之后），模型就是先答后看指令——
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
        let store = EventStore::new();
        let tok = TokenIssuer::issue_deep();
        let llm = ToolCallingLlm::new();
        let prompts = llm.prompts.clone();
        let dispatch = FakeDispatch {
            pending: false,
            rounds: Arc::new(Mutex::new(0)),
        };
        let mut i = input();
        i.prefix = Some("【长期记忆】\n- 偏好：简洁".to_string());

        TurnRunner
            .run_with_tools(
                &store,
                &llm,
                &tok,
                i,
                Arc::new(SilentDeltas),
                &tools(),
                Some(&dispatch),
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
            let store = EventStore::new();
            let llm = ToolCallingLlm::new();
            let prompts = llm.prompts.clone();
            let mut i = input();
            i.prefix = empty;
            TurnRunner
                .run_with_tools(&store, &llm, &tok, i, Arc::new(SilentDeltas), &[], None)
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
    /// 原先 `window_by_turn` 的 `keep == 0` 分支返回**空切片**，于是同一个配置数在
    /// 两条执行路径上含义相反：v1 读它作「不按轮次截断」、v2 读它作「历史全丢」。
    /// 出厂档位翻到 `full` 之后，任何把 `context_messages` 配成 `0` 的实例，
    /// **模型眼前的历史是空的**——静默、无告警、事实照样入格、`session/stats` 照样有数。
    ///
    /// **反向自检**：把 `keep == 0` 分支改回 `&events[events.len()..]`，本用例必须红。
    #[tokio::test]
    async fn zero_window_means_no_truncation_not_empty_history() {
        let store = EventStore::new();
        let tok = TokenIssuer::issue_deep();
        let llm = EchoLlm::new();
        let prompts = llm.prompts.clone();

        for turn in 0..3u64 {
            let mut i = input();
            i.turn = turn;
            i.text = format!("第 {turn} 轮问题");
            i.window_turns = Some(0);
            TurnRunner
                .run_with_tools(&store, &llm, &tok, i, Arc::new(SilentDeltas), &[], None)
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
        let store = EventStore::new();
        let llm = EchoLlm::new();
        let prompts = llm.prompts.clone();
        for turn in 0..3u64 {
            let mut i = input();
            i.turn = turn;
            i.text = format!("第 {turn} 轮问题");
            i.window_turns = Some(1);
            TurnRunner
                .run_with_tools(&store, &llm, &tok, i, Arc::new(SilentDeltas), &[], None)
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

    /// 工具轮：产物落 `artifact × asserted` 且溯源指向本轮用户格；final 只落一次；
    /// **工具结果进了下一次请求**（回读 prompt——不变量与网格都证明不了这条）。
    #[tokio::test]
    async fn tool_round_lands_artifact_and_feeds_next_prompt() {
        let store = EventStore::new();
        let tok = TokenIssuer::issue_deep();
        let llm = ToolCallingLlm::new();
        let prompts = llm.prompts.clone();
        let dispatch = FakeDispatch {
            pending: false,
            rounds: Arc::new(Mutex::new(0)),
        };

        let out = TurnRunner
            .run_with_tools(
                &store,
                &llm,
                &tok,
                input(),
                Arc::new(SilentDeltas),
                &tools(),
                Some(&dispatch),
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

        // ── ADR-048a 的核心断言：结果带**角色**与 **tool_call_id** ──
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
            Some("call_vdfs_read_0"),
            "role=tool 必须关联到那次调用（否则模型无法配对多次并发调用）"
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

        // 网格：用户格 + 产物格 + final 格（final 只一条——N3）。
        let snapshot = store.range(Seq::new(0));
        assert_eq!(snapshot.len(), 3, "{snapshot:?}");
        assert!(
            check_all(&snapshot).is_empty(),
            "{:?}",
            check_all(&snapshot)
        );
        assert_eq!(snapshot[0].kind, EVENT_USER_MESSAGE);
        assert_eq!(snapshot[1].kind, EVENT_ARTIFACT_ADDED);
        assert_eq!(snapshot[1].entity, Entity::Artifact);
        assert_eq!(snapshot[1].verb, Verb::Asserted);
        assert_eq!(snapshot[1].payload["tool"], "vdfs_read");
        assert_eq!(snapshot[1].payload["text"], "文件内容：hello");
        assert_eq!(
            snapshot[1].produced_by,
            Some(0),
            "产物格溯源指向本轮用户格（S02 §3 的 caused_by 断言）"
        );
        assert_eq!(snapshot[2].kind, EVENT_ASSISTANT_FINAL);
        assert_eq!(snapshot[2].payload["text"], "读到了。");
    }

    /// 收束于等待用户：运行器**停下且不落收束格**——本轮还没了结（网格少一格是
    /// 诚实缺口，与中止同纪律），且这个缺口必须**可被判出**（C4 看得见它）。
    #[tokio::test]
    async fn pending_tool_stops_loop_without_closing_cell() {
        let store = EventStore::new();
        let tok = TokenIssuer::issue_deep();
        let llm = ToolCallingLlm::new();
        let dispatch = FakeDispatch {
            pending: true,
            rounds: Arc::new(Mutex::new(0)),
        };

        let out = TurnRunner
            .run_with_tools(
                &store,
                &llm,
                &tok,
                input(),
                Arc::new(SilentDeltas),
                &tools(),
                Some(&dispatch),
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
            snapshot.len(),
            2,
            "用户格 + 产物格，**没有**收束格：{snapshot:?}"
        );
        assert_eq!(snapshot[1].kind, EVENT_ARTIFACT_ADDED);
        assert!(
            check_all(&snapshot).is_empty(),
            "{:?}",
            check_all(&snapshot)
        );
        assert_eq!(
            unresolved_turns(&snapshot, false).len(),
            1,
            "缺口要可被判出（不是假装收束）"
        );
    }

    /// 无分发方却收到工具调用 = 配置缺口：按失败诚实回报（兜底格），
    /// 不假装成功、也不静默丢工具调用。
    #[tokio::test]
    async fn tool_call_without_dispatch_falls_back() {
        let store = EventStore::new();
        let tok = TokenIssuer::issue_deep();
        let llm = ToolCallingLlm::new();

        let out = TurnRunner
            .run_with_tools(
                &store,
                &llm,
                &tok,
                input(),
                Arc::new(SilentDeltas),
                &tools(),
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
        assert_eq!(snapshot.len(), 2, "用户格 + 兜底格：{snapshot:?}");
        assert_eq!(
            snapshot[1].kind,
            crate::symbio_core::EVENT_ASSISTANT_FALLBACK
        );
    }

    /// **缺口 6 的锚**：provider 实测用量必须沿 `LlmTurn → TurnOutcome` 带出来。
    ///
    /// 唯一消费方是 session 的 token 估算校准（`chat_loop/turn.rs::feedback_estimate`）：
    /// 这一格断掉，校准比就冻结在初值 1.0，未校准启发式的系统偏差（实测对 CJK
    /// 高估约 31%）永久无人修正，压缩预检把本可成功的摘要请求误判成「注定超限」
    /// ——e2e `t8` / `t11` / `t18` 红的正因。链路后半段（`TurnOutcome.usage` →
    /// `TurnOutput.usage`）在 `v2_exec` 是两处同名字段直通，不另设旁路。
    #[tokio::test]
    async fn provider_usage_survives_the_run_to_the_outcome() {
        let store = EventStore::new();
        let tok = TokenIssuer::issue_deep();
        let llm = AnsweringLlm::new();
        let out = TurnRunner
            .run_with_tools(
                &store,
                &llm,
                &tok,
                input(),
                Arc::new(SilentDeltas),
                &[],
                None,
            )
            .await
            .expect("桩必答");
        assert_eq!(
            out.usage,
            Some(crate::symbio_core::ModelUsage {
                input: Some(11),
                output: Some(22)
            }),
            "用量断了 ⇒ 校准永远停在 1.0（缺口 6）"
        );
    }

    /// 续写同一轮（审批 / 问答恢复）：**不重开用户格**、收束记在**原轮**上。
    ///
    /// 这是 [plan/11 批 2](../../../../docs/plan/11-多执行器与多主体加固实施方案.md)
    /// 的核心判据，也是 C4 的**反向用例**：等待轮留一个缺口（`unresolved_turns`
    /// 看得见），续写把它**真正填上**。若续写另开新轮（`u-1` + `f-1`），原轮永远等不到
    /// 收束事件，C4 会把它当**永久假缺口**——不变量随即失去判据价值。
    #[tokio::test]
    async fn resume_continues_same_turn_without_reopening_user_cell() {
        let store = EventStore::new();
        let tok = TokenIssuer::issue_deep();

        // ① 等待轮：工具报 pending ⇒ 停下、不落收束格（网格留 1 个可判出的缺口）。
        let pending_llm = ToolCallingLlm::new();
        let pending_dispatch = FakeDispatch {
            pending: true,
            rounds: Arc::new(Mutex::new(0)),
        };
        let first = TurnRunner
            .run_with_tools(
                &store,
                &pending_llm,
                &tok,
                input(),
                Arc::new(SilentDeltas),
                &tools(),
                Some(&pending_dispatch),
            )
            .await
            .expect("等待用户不是失败");
        assert!(first.awaits_user, "等待轮必须在结果里可见");
        assert_eq!(
            unresolved_turns(&store.range(Seq::new(0)), false).len(),
            1,
            "等待轮留一个可判出的缺口"
        );

        // ② 恢复：续写**同一轮**（turn 0），用户格 seq = 0（网格第一格）。
        let answer_llm = AnsweringLlm::new();
        let prompts = answer_llm.prompts.clone();
        let out = TurnRunner
            .run_with_tools(
                &store,
                &answer_llm,
                &tok,
                TurnInput {
                    turn: 0,
                    text: String::new(), // 续写不新开用户格，此字段不参与入格
                    tier: LatencyTier::Deep,
                    inject: None,
                    window_turns: None,
                    actor: crate::symbio_core::ActorSpec::trivial("agent:main"),
                    resume: Some(TurnResume {
                        user_seq: 0,
                        call: TurnToolCallInfo {
                            id: Some("tc-1".into()),
                            wire_id: Some("w-1".into()),
                            name: Some("vdfs_read".into()),
                            arguments: serde_json::json!({ "path": "a.md", "approved": true }),
                            parse_error: None,
                        },
                        text: "文件内容：hello（已批准）".into(),
                    }),
                    // 请求级前缀：测试不走请求视图层（三段皆空）。
                    prefix: None,
                    final_reply: None,
                },
                Arc::new(SilentDeltas),
                &tools(),
                // 续写轮若真的再请求工具才有分发可言；本用例的收尾轮不请求工具。
                Some(&FakeDispatch {
                    pending: false,
                    rounds: Arc::new(Mutex::new(0)),
                }),
            )
            .await
            .expect("续写轮必答");

        assert_eq!(out.turn, 0, "续写记在原轮上");
        assert_eq!(out.text, "批准后已完成。");
        assert!(!out.fell_back && !out.aborted && !out.awaits_user);

        let snapshot = store.range(Seq::new(0));
        let kinds: Vec<&str> = snapshot.iter().map(|e| e.kind.as_str()).collect();
        assert_eq!(
            kinds,
            vec![
                EVENT_USER_MESSAGE,
                EVENT_ARTIFACT_ADDED,
                EVENT_ARTIFACT_ADDED,
                EVENT_ASSISTANT_FINAL
            ],
            "续写**不重开用户格**（否则 5 格），只补产物格与收束格：{snapshot:?}"
        );
        assert_eq!(
            snapshot
                .iter()
                .filter(|e| e.kind == EVENT_USER_MESSAGE)
                .count(),
            1,
            "同一句话在网格里只能出现一次"
        );
        assert_eq!(
            snapshot[2].payload["tool"], "vdfs_read",
            "恢复的产物格记被恢复的工具"
        );
        assert_eq!(snapshot[2].payload["text"], "文件内容：hello（已批准）");
        assert_eq!(
            snapshot[2].produced_by,
            Some(0),
            "溯源仍指向**原轮**用户格（I2）"
        );
        assert_eq!(snapshot[3].turn, 0, "收束格记在原轮号上");
        assert_eq!(snapshot[3].payload["text"], "批准后已完成。");
        assert!(
            check_all(&snapshot).is_empty(),
            "{:?}",
            check_all(&snapshot)
        );
        assert!(
            unresolved_turns(&snapshot, false).is_empty(),
            "续写填上了原轮的缺口（否则 C4 会把它当永久假缺口）"
        );

        // prompt：恢复的交换进了模型看得见的地方（形状与在途工具轮同一套消息）。
        let seen = prompts.lock().unwrap().clone();
        assert_eq!(seen.len(), 1, "收尾轮一次请求：{seen:?}");
        let s = Seen::from(seen[0].clone());
        assert!(
            s.texts_of("assistant")
                .iter()
                .any(|x| x.contains("调用工具 vdfs_read")),
            "恢复的工具调用要进 prompt（作为 assistant 消息）：{s:?}"
        );
        // 结果**带角色**：这是续写轮与在途工具轮同形的判据——恢复的结果不是
        // 「拼在末尾的一段文字」，而是一条 `role: tool` 消息，带调用 id。
        //
        // **两条** tool 消息都是事实，不是重复：等待轮落了一条未批准的结果
        // （`文件内容：hello`），续写轮又落了一条已批准的（`…（已批准）`）。
        // 网格是 append-only 的事实源，两次调用两次结果都在；把它们合成一条
        // 才是信息丢失。
        assert_eq!(
            s.tools(),
            vec![
                (
                    "文件内容：hello",
                    Some("vdfs_read"),
                    Some("call_vdfs_read_0")
                ),
                (
                    "文件内容：hello（已批准）",
                    Some("vdfs_read"),
                    Some("call_vdfs_read_0")
                ),
            ],
            "两次调用的两次结果都在，且都带角色与调用 id：{s:?}"
        );
    }
}

// ── 反射档（`run_reflex`，S11 快路的执行半边）────────────────────────────────
//
// 判据分列（一个规则一个判定方）：
// - **落格**：`u-{turn}` / `f-{turn}` 与 [`TurnRunner::run_with_tools`] 同一份纪律
//   （收束溯源指向开轮格、`cost_ms` 随事件入账）——本模块只读事实源；
// - **档位**：开轮格载荷的 `tier` 由**令牌类型**给出（`RuleOnly` ⇒ `reflex`），
//   不是调用方填的字符串；
// - **产物上线**：正文经 `sink` 到出口（不上线，收束节点在前端建不起来）。

mod reflex_turn_tests {
    use std::sync::{Arc, Mutex};

    use super::super::TurnRunner;
    use crate::symbio_core::{
        check_all, unresolved_turns, ActorSpec, DeltaSink, Entity, EventStore, LatencyTier, Seq,
        Store, TokenIssuer, Verb, EVENT_ASSISTANT_FINAL, EVENT_USER_MESSAGE,
    };

    /// 收集口：按序记增量（反射产物上线的判据）。
    struct CollectingDeltas(Mutex<Vec<String>>);

    impl DeltaSink for CollectingDeltas {
        fn on_delta(&self, text: &str) {
            self.0.lock().unwrap().push(text.to_string());
        }
        // 反射档不调模型 ⇒ 没有推理增量；通道由 `bound_provider.test.rs` 覆盖。
        fn on_reasoning(&self, _text: &str) {}
    }

    /// 正向：反射档一轮**只落两格**（开轮 + 收束），收束溯源指向开轮格、
    /// `cost_ms` 在反射档预算内、开轮格声明 `reflex`、产物经 `sink` 上线。
    #[tokio::test]
    async fn reflex_turn_lands_open_and_close_with_reflex_tier() {
        let store = EventStore::new();
        let tok = TokenIssuer::issue_reflex();
        let actor = ActorSpec::trivial("agent:main");
        let got = Arc::new(CollectingDeltas(Mutex::new(Vec::new())));

        let out = TurnRunner
            .run_reflex(
                &store,
                &tok,
                0,
                &actor,
                "把周报整理成摘要",
                "先列要点；再合并同类项。",
                got.clone() as Arc<dyn DeltaSink>,
            )
            .await
            .expect("反射档必答（不生成也要有产物）");

        assert_eq!(out.text, "先列要点；再合并同类项。");
        assert!(!out.fell_back && !out.aborted && !out.awaits_user);
        assert!(
            out.cost_ms <= LatencyTier::Reflex.budget_ms(),
            "命中后耗时必须在反射档预算内（S11 §6.2：命中显著低于未命中）：{}",
            out.cost_ms
        );

        let snapshot = store.range(Seq::new(0));
        assert_eq!(
            snapshot.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>(),
            vec![EVENT_USER_MESSAGE, EVENT_ASSISTANT_FINAL],
            "反射档只落开轮与收束两格（没有产物格——它没跑工具）：{snapshot:?}"
        );
        let open = &snapshot[0];
        assert_eq!(open.entity, Entity::Turn);
        assert_eq!(open.verb, Verb::Opened);
        assert_eq!(
            open.payload["text"], "把周报整理成摘要",
            "开轮格记的是**用户说的**，不是反射产物"
        );
        assert_eq!(
            open.payload["tier"], "reflex",
            "档位由令牌类型给出（RuleOnly ⇒ reflex），不是调用方填的字符串"
        );
        let close = &snapshot[1];
        assert_eq!(close.verb, Verb::Closed);
        assert_eq!(close.turn, 0);
        assert_eq!(close.payload["text"], "先列要点；再合并同类项。");
        assert_eq!(
            close.payload["model"], "reflex",
            "反射档没有模型：载荷的 `model` 记产者，空着会让命中轮与普通轮无从区分"
        );
        assert_eq!(close.cost_ms, out.cost_ms, "实测耗时随事件入账");
        assert_eq!(
            close.produced_by,
            Some(0),
            "收束溯源指向开轮格（I2；开轮格是网格第一格，seq = 0）"
        );
        assert_eq!(
            close.actor, "agent:main",
            "收束格的 actor 取本轮主体（与 run_with_tools 同一处取值）"
        );

        assert_eq!(
            *got.0.lock().unwrap(),
            vec!["先列要点；再合并同类项。".to_string()],
            "产物必须经 sink 上线——不上线，收束节点在前端建不起来"
        );
        assert!(
            check_all(&snapshot).is_empty(),
            "{:?}",
            check_all(&snapshot)
        );
        assert!(
            unresolved_turns(&snapshot, false).is_empty(),
            "开过的轮必须收束（I3 到点必答）"
        );
    }
}
