//! 阶段④ 落格 + 定稿答话轮的独立入口（ · B2，DE-021 §5 动作 B2）。
//!
//! [`persist_turn`] 是 `execute_turn` 的末段：派生事实（承诺 / 任务表 / 熔断 /
//! 淘汰 / 记忆）落格、兜底结局上抛、折进的补充并入落库镜像。
//! [`execute_final_reply_turn`] 是同族入口（它自己就是一段最短的「装配→运行→落格」），
//! 本批随判据 4（单文件 ≤800 行）逐字搬入，路径经 `turn_exec.rs` 的再导出保持不变。
//!
//! **纯结构拆分契约**：两段体内自 `turn_exec.rs` 逐字搬移；跨模块引用仅做
//! `super::` → `super::super::` 两跳改写与文档链接深度修正。

use super::*;

/// 一轮的执行（阶段④）：落格——消费 [`Settled`]，产本轮的最终结果。
pub(crate) fn persist_turn(s: Settled<'_>) -> Result<ExecTurnResult, TurnExecExit> {
    let Settled {
        session,
        store,
        outcome,
        produced,
        derived,
        collected,
        turn_no,
        user_seq,
        principal,
        to,
        user_text,
        recalled,
        skill_obs,
        child_id,
        reasoning_child_id,
        reasoning_text,
    } = s;
    let response = (!outcome.fell_back).then_some(outcome.text.as_str());
    // 本轮开口格（`u-{turn}`）的 seq —— 记忆 / 任务 / 承诺的溯源锚。
    //
    // 读**完整史**（`range(0)`，S2.7-b2）：这三个写方（`record_derived` 的
    // 承诺与任务表 / `record_learning` 的记忆三段）都是**累计口径**——它们的幂等
    // doc-link-allow D-004: 「曾经落过什么」问的是事实网格里有无该条记录，属数据语义
    // 判据与状态推导要看**曾经**落过什么。被快照取代而移入归档段的那些事实同样在
    // `range(0)` 里 ⇒ 压缩前开过的任务不会在就绪集里消失，幂等判据也不会失效
    // （看不见 ⇒ 重复落格）。
    //
    // 用户格 seq **不再从这里反查**：它在判决之前就落了，由编排层沿
    // `ExecTurn::user_seq` 直给——本函数只把完整史带给累计口径的写方。
    let all = store.range(Seq::new(0));
    // 承诺 / 任务表 / 熔断：写方是 `record_derived`。
    // 每族的失败在函数内各自记日志、互不打断——轮次已收束，派生事实失败不该
    // 把成功的一轮说成失败（与记忆三段同一条口径）。本调用点因此不判返回值。
    //
    // `to` = 来源主体（D-067）：本主体向「这条消息的出处」立约。缺省落
    // 本机用户——本机路径与接线前一致（取值点见 `request_source_principal`，
    // 在 `ctx` 被移进分发口之前取好）。
    super::super::derived_facts::record_derived(
        session.session_id(),
        store.as_ref(),
        &all,
        turn_no,
        user_seq,
        &principal,
        &to,
        &format!("t{turn_no}"),
        &derived.delegations,
        &derived.tasks,
        &derived.breaks,
    );
    // 淘汰回路（S13 §3，）：**必须排在承诺落格之后**——它读的声誉投影
    // 要看得见本轮刚写下的那条违约（委托失败 ⇒ 声誉 −1 ⇒ 收回 assign.work）。
    // 顺序反了会让判据滞后一个轮次：那一轮里主体已经又派出去一轮。
    super::super::derived_facts::record_elimination(store.as_ref(), turn_no, user_seq);
    // 记忆与学习：同一函数（`record_learning`）。
    super::super::derived_facts::record_learning(
        session.session_id(),
        // `store` 是 `Arc<dyn Store>`（轮内注入闭包要 `'static`），而本函数
        // 收 `&dyn Store` ⇒ `as_ref()` 一次取到 trait object。
        store.as_ref(),
        &all,
        turn_no,
        user_seq,
        user_text,
        &principal,
        &format!("t{turn_no}"),
        response,
        recalled,
        skill_obs,
        session.skill_compile_enabled(),
        crate::symbio_core::clock_now_ms(),
    );

    // 兜底已入格（I3），失败呈现走 Failed 出口 + 重试。
    if outcome.fell_back {
        return Err(TurnExecExit::Failed(PluginError::InternalError(
            outcome.text,
        )));
    }
    // 等待用户（`outcome.awaits_user`）：本轮尚未了结（运行器只落了进展格
    // `status: "pending"`，收束格留给恢复幕），但**按正常收束返回**——待用户动作由
    // 分发方广播的 `user_prompt` 节点承载（`WaitingUserAction`），呈现是卡片 +
    // 恢复入口；恢复时经 [`TurnResume`] 续写同一轮（不重开用户格）。
    // 此处的 `text` 只是模型在工具轮说过的话，不是终答。

    // 折进的补充也要进落库镜像（缺口 3）：排在 `produced` **之前** ——
    // 用户先插话、工具结果后到，这是时间序（`persist_messages` 只写尾部增量，
    // 顺序错了落库就乱了）。
    // 同上：锁拿不到就用空的（落库镜像少这批补充，其余不受影响）。
    let mut all_messages: Vec<ChatNode> = collected
        .lock()
        .map(|mut v| std::mem::take(&mut *v))
        .unwrap_or_default();
    all_messages.extend(produced);

    Ok(ExecTurnResult {
        output: TurnOutput {
            text: outcome.text,
            reasoning: reasoning_text,
            response_text_child_id: child_id,
            reasoning_child_id,
            // 实测用量直通：校准反馈的唯一数据源（缺口 6），与上面被拒出口同一条。
            usage: outcome.usage,
            // 末轮原因直通：与上面被拒出口同一条纪律。`..Default::default`
            // 会把这一格静默填成 Stop，长度截断的自动续写因此不可达。
            finish: outcome.finish,
            ..Default::default()
        },
        messages: all_messages,
    })
}

/// **定稿答话轮**（缺口 5）：`classify` 判 `Answered` ⇒ 本轮**一个模型请求都不发**，
/// 但收束格仍须由 v2 运行器落（用户格已由编排层在判决之前落，）。
///
/// ## 它解决的是什么
///
/// `Answered` 判决在 `chat_loop` 步骤 2 就短路收尾，**根本不进 [`super::execute_turn`]**；
/// 若就此返回，那一轮在事实网格里只有用户格、没有 `chat.assistant.final`。于是下一轮
/// prompt 里这次问答**整个消失**，`session/stats` 的轮次计数也缺收束——而
/// `messages.json` 里有（`context.messages` 落库）⇒ **两条真源**。完全静默：不报错、
/// 不告警、既有断言全绿。判据 = e2e `t22`。
///
/// ## 为什么走运行器，而不是本函数自己写收束格
///
/// 「一个 turn = 用户格 opened + 至多一条收束 closed」这条不变量由运行器守着。插件自己
/// 补写就成了**第二个轮次事实写方**——同轮两份记账的假象随之回来。所以答话交给运行器
/// （`TurnInput::final_reply`），收束格仍由它落。
///
/// ## 与 [`super::execute_turn`] 的两处刻意差异
///
/// 1. **不写派生事实**：`record_learning` / `record_derived` 一律不调——`Answered` 轮没有
///    工具交换、没有模型参与，`finish_turn` 直接收尾，没有派生副作用可记。
/// 2. **不上线 UI 帧**：答话节点已由 `apply_verdict` 推进 `context.messages`
///    （`dialog_node`，`exclude_from_context = false`），由 `finish_turn` 落库 ⇒ 这里用
///    [`SilentDeltas`]；再发一次就是同一条答话重复上屏。
///
/// ## `llm` / `tok` 是 `run_with_tools` 的端口，不是本路的依赖
///
/// 短路发生在**用到它们之前**（`TurnRunner::run_with_tools` 步骤 1b）——这一轮一次模型
/// 往返都不发。仍然照签名传，是因为「工具轮入口」只有那一个：另开一个入口就得把
/// 「用户格 + 收束格」的落格逻辑写第二遍，那才是真正的重复。
///
/// 形参宽（`consolidation` 是 S2.7-b2 加的，`stages` 是 加的，三个 seq 是
/// 加的）：与 [`super::execute_turn`] 同一条口径——「一轮的全部输入」本就是
/// 一个宽签名，收成结构体只会把同一次调用拆成两处。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn execute_final_reply_turn(
    session: &LoopSession,
    ctx: &Arc<dyn crate::symbio_core::PluginInvokeRequest>,
    provider: Arc<dyn ModelProvider>,
    abort: ExecAbortSignal,
    // 轮号与溯源锚由编排层在判决之前定下：答话轮同样先落用户格，
    // 这里只收三个 seq（`drive_seq` 供派发格幂等键）。
    turn_no: u64,
    user_seq: u64,
    drive_seq: u64,
    reply: &str,
    // 答话**对话面节点**的身份与标记（`id` / `reason` / `exclude_from_context`）：
    // 收束格据它复刻那条根级节点（`Answered` 答话 `exclude = false`）；必须随
    // 事实入格（WAL 为唯一事实源，见 `TurnInput::dialog`）。
    dialog: super::super::turn_runner::DialogFacts,
    // 生成这次答话的**思考事实**（`(reasoning_id, reasoning)`）：措辞产线的模型
    // 往回是这条轮次里唯一一次真实推理，不入格则 `Reason` 节点对普通答话轮从未
    // 存在（AMD MiMo 实测 2026-10-13）。模板产线 / 生成失败退模板 ⇒ `None`。
    reasoning: Option<(String, String)>,
    // 本轮之前发生过压缩 ⇒ 压缩后的整份消息列表（S2.7-b2，与 [`super::execute_turn`] 同源）。
    // 答话轮在步骤 2 就短路收尾，**不进** [`super::execute_turn`]，所以它也得自己落这一格
    // ——否则「上一轮末尾压过一次、这一轮是答话轮」时那次压缩永远不入格。
    consolidation: Option<&[ChatNode]>,
    // 本轮已执行的一次性阶段（判决事实 + 「已做」，D-107 判据 2）。答话轮同样
    // 不进 [`super::execute_turn`]，判决草稿由本入口随 `TurnInput` 递进运行器。
    stages: Vec<super::super::stage_events::StageFact>,
) -> Result<(), PluginError> {
    // 事实源：与 [`super::execute_turn`] 同一条路径——交付包构造的唯一句柄（D-084 §2）。
    let store = session.facts().ok_or_else(|| {
        PluginError::InternalError("v2 执行需要持久会话（临时会话无事实源）".into())
    })?;
    // **补落寄存的变更**：答话轮同样是会话侧的落格窗口，经端口动作发起。
    // 与 `execute_turn` 同序（在运行器落格之前——本轮的用户格已由编排层先落）。
    if let Err(e) = session.run_op_unit(run_ops::DRAIN_PARKED, ()).await {
        crate::plugin_warn!("session", "[Turn] 补落寄存变更失败（本轮继续）: {e}");
    }
    // 身份与档位取值点同 [`super::execute_turn`]：主体 = 会话选定的那个 agent；档位 = 深度
    // （`Answered` 轮没有可跳过的模型调用，不属于反射档）。规格从授权表行取
    // （`request_actor_spec`，能力集 / 模式不再是平凡值）。
    // 作用域与身份同批取（相应批次）：子会话挂父主体，顶层会话恒 `Root`。
    let scope = session.scope();
    let actor = super::super::chat_loop::request_actor_spec(ctx.as_ref(), LatencyTier::Deep, scope);
    // 压缩入格 + 归档：与 [`super::execute_turn`] 同序（在 `turn_no` 之后、运行器之前）。
    if let Some(msgs) = consolidation {
        if let Err(e) = session
            .run_op_unit(
                run_ops::CONSOLIDATE,
                ConsolidateRequest {
                    messages: msgs.to_vec(),
                },
            )
            .await
        {
            crate::plugin_warn!("session", "[Turn] 压缩事实落格失败（压缩已生效）: {e}");
        }
    }
    let llm = ProviderLlmAdapter::for_turn(provider, "", abort.clone());
    let tok = TokenIssuer::issue_deep();
    let input = TurnInput {
        turn: turn_no,
        user_seq,
        drive_seq,
        tier: LatencyTier::Deep,
        // 不生成 ⇒ 没有 prompt，也就没有窗口可裁。
        window_turns: None,
        actor,
        prefix: None,
        stages,
        inject: None,
        final_reply: Some(reply.to_string()),
        // 答话**对话面节点**的身份与标记：收束格据它复刻那条根级节点。答话轮没有
        // 流式过程 ⇒ 无半截可记：载荷 `turn_id` 记空串（重建侧按 `dialog.id` 与
        // 事件 id 复刻），取数口按有无思考事实决定挂不挂（见下方 `nodes`）。
        dialog: Some(dialog),
        // 答话轮**不收束前的开场白**：`Answered` 在轮首就判定了（`Spoke` 是
        // `Escalate` 的效应），本入口只处理已定稿的答话。
        opening_text: None,
        opening_dialog: None,
        turn_root_id: String::new(),
        // 取数口：**恒定快照**——答话轮没有流式过程（措辞插件的模型往返是静默的，
        // 此刻已结束），思考事实是调用方搬运来的定值。挂上它，1b 收束格的
        // `produced` 载荷才带 `reasoning` / `reasoning_id`（重建据此复刻 Reason 节点）。
        nodes: reasoning.map(|(reasoning_id, reasoning)| {
            let facts = super::super::turn_runner::StreamNodeFacts {
                text_id: None,
                text: None,
                reasoning_id: Some(reasoning_id),
                reasoning: Some(reasoning),
            };
            std::sync::Arc::new(move || facts.clone()) as super::super::turn_runner::NodeFactsFn
        }),
        // 定稿答话轮不经过工具循环的闸门空隙——它一次请求都不发（`final_reply` 走
        // 1b 分支直接收束）。但信号照传：若用户在答话落格前按下停止，1c 的中止检查
        // 会在落完用户格后接管，与其它轮同一条路径（终止是状态，不是特例）。
        abort: Some(abort),
    };
    drive_steps(
        store.as_ref(),
        &llm,
        &tok,
        input,
        Arc::new(SilentDeltas),
        &[],
        None,
        super::super::loop_rows::ACTORS,
    )
    .await
    .map_err(|e| PluginError::InternalError(format!("v2 运行器落格失败：{e:?}")))?;
    Ok(())
}
