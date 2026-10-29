//! 阶段④ 落格 + 定稿答话轮的独立入口（· B2，DE-021 §5 动作 B2）。
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
        &derived.effects,
        &derived.breaks,
    );
    // 调度（S04 §3，`D-139` 裁决 4）：**必须排在任务表落格之后**，且**重新取史**
    // ——调度的输入是本轮刚开的格；拿 `all`（轮初那份）算，这一轮新开的任务要等到
    // 下一轮才进候选集，第一轮因此永远派不出东西。
    let scheduled = store.range(Seq::new(0));
    super::super::derived_facts::record_schedule(
        session.session_id(),
        store.as_ref(),
        &scheduled,
        session.task_cap(),
        &principal,
        turn_no,
        user_seq,
        &format!("t{turn_no}-s"),
        symbio_core::clock_now_ms(),
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
        &user_text,
        &principal,
        &format!("t{turn_no}"),
        response,
        recalled,
        skill_obs,
        session.skill_compile_enabled(),
        symbio_core::clock_now_ms(),
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
/// `Answered` 判决在 `chat_loop` 步骤 2 就短路收尾，**根本不进 [`super::drive`]**；
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
/// ## 与生成路径的差异
///
/// 1. **承诺 / 任务表 / 熔断不写**：那三族的事实来源是工具交换（派活、就绪集、
///    失败），`Answered` 轮一次工具都不调，没有可记的东西。
///    但**记忆不在此列**：编码的是「这一轮用户说了什么、智能体答了什么」，与
///    有没有工具交换无关。跳过它就是答话轮记忆静默丢失（实测：默认配置下
///    「记住我的幸运数字」由 `classify` 判成答话 ⇒ WAL 里没有 `memory.encoded`，
///    下一个会话当然想不起来）。因此这里与 [`persist_turn`] 一样调
///    `record_learning`——见 DE-036「记忆编码是轮级派生，不依附驱动循环」。
/// 2. **不上线 UI 帧**：答话节点已由 `apply_verdict` 推进 `context.messages`
///    （`dialog_node`，`exclude_from_context = false`），由 `finish_turn` 落库 ⇒ 这里用
///    [`SilentDeltas`]；再发一次就是同一条答话重复上屏。
///
/// ## `llm` / `tok` 是 `drive` 的端口，不是本路的依赖
///
/// 短路发生在**用到它们之前**（`turn_exec::drive` 的答话守卫）——这一轮一次模型
/// 往返都不发。仍然照签名传，是因为「工具轮入口」只有那一个：另开一个入口就得把
/// 「用户格 + 收束格」的落格逻辑写第二遍，那才是真正的重复。
///
/// 形参宽（`consolidation` 与三个 seq 都是这一轮的**事实端口**）：与
/// [`super::open_turn`] 同一条口径——「一轮的全部输入」本就是
/// 一个宽签名，收成结构体只会把同一次调用拆成两处。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn execute_final_reply_turn(
    session: &LoopSession,
    ctx: &Arc<dyn symbio_core::PluginInvokeRequest>,
    _provider: Arc<dyn ModelProvider>,
    abort: ExecAbortSignal,
    // 轮号与溯源锚由编排层在判决之前定下：答话轮同样先落用户格，
    // 这里只收三个 seq（`drive_seq` 供派发格幂等键）。
    turn_no: u64,
    user_seq: u64,
    drive_seq: u64,
    // 本轮用户发言：记忆编码的**内容**（`record_learning` 步 11）。答话轮同样
    // 是「用户说了什么」的一轮，不传它就只能记下判断、丢掉经验。
    user_text: &str,
    reply: &str,
    // 生成 `reply` 的那次措辞往返的实测耗时（`WordingReply.cost_ms`）。它是本
    // 轮唯一的模型往返代价，落进收束格的 `cost_ms`；模板 / 无往返产线给 0。
    reply_cost_ms: u64,
    // 答话**对话面节点**的身份与标记（`id` / `reason` / `exclude_from_context`）：
    // 收束格据它复刻那条根级节点（`Answered` 答话 `exclude = false`）；必须随
    // 事实入格（WAL 为唯一事实源，见 `TurnInput::dialog`）。
    dialog: super::super::turn_runner::DialogFacts,
    // 生成这次答话的**思考事实**（`(reasoning_id, reasoning)`）：措辞产线的模型
    // 往回是这条轮次里唯一一次真实推理，不入格则 `Reason` 节点对普通答话轮从未
    // 存在（AMD MiMo 实测 2026-10-13）。模板产线 / 生成失败退模板 ⇒ `None`。
    reasoning: Option<(String, String)>,
    // 本轮之前发生过压缩 ⇒ 压缩后的整份消息列表（S2.7-b2，与工具轮同源：
    // `run_seam` 从 `context.consolidated` 取走的那一份）。
    // 答话轮在步骤 2 就短路收尾，**不进** [`super::drive`]，所以它也得自己落这一格
    // ——否则「上一轮末尾压过一次、这一轮是答话轮」时那次压缩永远不入格。
    consolidation: Option<&[ChatNode]>,
) -> Result<(), PluginError> {
    // 事实源：与 [`super::assemble_turn`] 同一条路径——交付包构造的唯一句柄（D-084 §2）。
    let store = session.facts().ok_or_else(|| {
        PluginError::InternalError("v2 执行需要持久会话（临时会话无事实源）".into())
    })?;
    // **补落寄存的变更**：答话轮同样是会话侧的落格窗口，经端口动作发起。
    // 与 `execute_turn` 同序（在运行器落格之前——本轮的用户格已由编排层先落）。
    if let Err(e) = session.run_op_unit(run_ops::DRAIN_PARKED, ()).await {
        symbio_core::plugin_warn!("session", "[Turn] 补落寄存变更失败（本轮继续）: {e}");
    }
    // 身份与档位取值点同 [`super::assemble_turn`]：主体 = 会话选定的那个 agent；档位 = 深度
    // （`Answered` 轮没有可跳过的模型调用，不属于反射档）。规格从授权表行取
    // （`request_actor_spec`，能力集 / 模式不再是平凡值）。
    //
    // 主体**不吃档位**（`D-223` 订正 2）：规格里那一格是主体的声明档，本幕的档与预算
    // 归 `turn.dispatch`（下面那次 `land_dispatch` 直接折 `tier.budget_ms()`）。
    // 作用域与身份同批取（[]）：子会话挂父主体，顶层会话恒 `Root`。
    let scope = session.scope();
    let actor = super::super::chat_loop::request_actor_spec(ctx.as_ref(), scope);
    // 压缩入格 + 归档：与接缝同序（`D-223` 订正 1：压缩必须早于派发格）。归档之后才
    // 落派发格，本幕的 `turn.dispatch` 才不会随「本轮之前的一切」一起被移进归档段。
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
            symbio_core::plugin_warn!("session", "[Turn] 压缩事实落格失败（压缩已生效）: {e}");
        }
    }
    // 派发档（`D-223` 订正 1）：答话轮不走快路（`Answered` 是判决的结论，不是技能的命中），
    // 档位恒为深度档。它与工具轮那一格同一条纪律——落在这里而不是草稿的构造里。
    super::super::chat_loop::land_dispatch(
        store.as_ref(),
        turn_no,
        user_seq,
        drive_seq,
        LatencyTier::Deep,
        &actor,
    )
    .map_err(|e| PluginError::InternalError(format!("派发档落格失败：{e:?}")))?;
    // 本入口不开环 ⇒ 不需要模型端口与令牌（那两者只由 [`super::drive`] 的动作读）。
    // `provider` 仍照签名收下（`_provider`）：答话轮的调用点与工具轮同形，删参数只会
    // 让两侧漂。
    let input = TurnInput {
        turn: turn_no,
        user_seq,
        drive_seq,
        // 不生成 ⇒ 没有 prompt，也就没有窗口可裁。
        window_turns: None,
        actor,
        inject: None,
        final_reply: Some(reply.to_string()),
        final_cost_ms: reply_cost_ms,
        // 答话**对话面节点**的身份与标记：收束格据它复刻那条根级节点。答话轮没有
        // 流式过程 ⇒ 无半截可记：载荷 `turn_id` 记空串（重建侧按 `dialog.id` 与
        // 事件 id 复刻），取数口按有无思考事实决定挂不挂（见下方 `nodes`）。
        dialog: Some(dialog),
        turn_root_id: String::new(),
        // 取数口：**恒定快照**——答话轮没有流式过程（措辞插件的模型往返是静默的，
        // 此刻已结束），思考事实是调用方搬运来的定值。挂上它，定稿答话收束格的
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
        // 定稿答话轮不经过工具循环的闸门空隙——它一次请求都不发（`final_reply` 直接
        // 收束）。但信号照传：若用户在答话落格前按下停止，首请求前中止检查
        // 会在落完用户格后接管，与其它轮同一条路径（终止是状态，不是特例）。
        abort: Some(abort),
    };
    // `Run` 这一支在本入口**按构造不可达**（`final_reply` 非空 ⇒ `prepare_turn` 的
    // 定稿答话分支就地收束、返回 `Done`）。本入口**不开环**（`D-222` 裁决 5：环只住在
    // [`super::drive`]，而它要的那一片编排层状态这一轮根本没有）——真到了这里说明
    // 收束路径与交付包对不上，出声。
    let outcome = match TurnRunner::begin(store.as_ref(), input)
        .await
        .map_err(|e| PluginError::InternalError(format!("v2 运行器落格失败：{e:?}")))?
    {
        TurnSessionBegin::Done(outcome) => outcome,
        TurnSessionBegin::Run(_) => {
            return Err(PluginError::InternalError(
                "定稿答话轮开出了驱动环（按构造不可达）".into(),
            ))
        }
    };
    // ── 记忆与学习（与 [`persist_turn`] 同一条口径）──────────────────────
    //
    // 三个跳过条件与 `classify_turn` 逐条对齐：中止 / 收束被授权拒绝 ⇒ 本轮
    // 没有定稿产出，派生事实全不写；兜底 ⇒ 只写用户经验（判断与技能编译都不写，
    // 失败的路不该被固化成记忆）。**没有工具交换不构成跳过条件**——记忆是轮级
    // 派生，不是工具副产物。
    if !outcome.aborted && !outcome.closure_denied {
        let principal = symbio_core::request_principal(ctx.as_ref());
        let response = (!outcome.fell_back).then_some(reply);
        super::super::derived_facts::record_learning(
            session.session_id(),
            store.as_ref(),
            &store.range(Seq::new(0)),
            turn_no,
            user_seq,
            user_text,
            &principal,
            &format!("t{turn_no}"),
            response,
            // 答话轮不建召回视图：分类器判 `direct` 的前提就是「现有对话里答得出」，
            // 这一轮没有检索往返，也就没有 `memory.recalled` 可记。
            None,
            &[],
            session.skill_compile_enabled(),
            symbio_core::clock_now_ms(),
        );
    }
    Ok(())
}
