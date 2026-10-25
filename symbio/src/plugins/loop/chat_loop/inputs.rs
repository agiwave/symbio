//! 一次 LLM 请求的**输入准备**与**压缩响应点**（收口 ②③）。
//!
//! - [`resolve_system_prompt`]：系统提示词唯一真源
//! - [`prepare_turn_inputs`]：提示词 + 工具 + 压缩 + 请求视图的唯一收集点
//! - [`apply_compaction`]：上下文水位的唯一响应点（自动语义压缩）

use crate::symbio_core::CapabilityMeta;

use super::super::port::Compress;
use super::super::stage_events::{StageFact, StageName};
use super::*;

/// 本轮用户发言 = 轮首入列的最后一条 user 消息的正文。
///
/// **取值点与 `turn_exec` 的 `user_text` 同源**（该处晚于 `prepare_turn_inputs`，
/// 但用户消息在步骤 3 之前已入列，两次取到的是同一条）。混合打分
/// 的语义维由它驱动；取不到 ⇒ `None` = 「本轮没有问题」，投影退化为纯 ts 降序
/// （平凡值，J2）。用类型区分「没有问题」与「问题恰好是空串」：前者是事实，
/// 后者得写出来才成立。
fn last_user_text(context: &SessionContext) -> Option<String> {
    context
        .messages
        .iter()
        .rev()
        .find(|m| m.node_role() == Some(NodeRole::User))
        .and_then(|m| match &m.content {
            Some(MessageContent::Text(t)) => Some(t.clone()),
            _ => None,
        })
}

/// 系统提示词的唯一真源。
///
/// 结果 = **请求显式段（若有）** + **全部注册段**，按「先显式、后注册顺序」拼接。
///
/// ## 只有一个集合，没有竞争
///
/// 插件经 `put(CapabilityRow::prompt(…))` 注册的每一段都**一定会送达
/// 模型**——不存在「按优先级链取一个」的竞争，因此也没有「谁把谁挤掉」这类事故。
/// 人格的选择发生在**注册期**（model 插件按 `PROVIDER_ID` 解析出唯一生效 provider
/// 后才注册），消费侧不需要也不应该再选一次。
///
/// ## 显式段**不吞掉**注册段
///
/// `req_system_prompt` 只决定「开头是什么」，不决定「后面还有没有」。若它一出现就
/// 让注册段消失，换个调用方（CLI / 子智能体 / 测试）就会静默丢掉记忆与人格——
/// 而丢的方式是「模型不再知道它存在」，没有任何报错。
///
/// ## 空段不占位
///
/// 注册了空串/纯空白的段会被跳过，因此不会产出「连续空行」这种噪声。
///
/// 单点约束：压缩开销估算与实际请求**必须**共用同一份解析结果——
/// 若各自取值，插件经 `traverse` 注册的系统提示词会被实际请求绕过，
/// 从未真正送达模型。
pub(crate) async fn resolve_system_prompt(
    req_system_prompt: Option<&str>,
    ctx: &Arc<dyn PluginInvokeRequest>,
) -> String {
    let mut out = String::new();

    if let Some(explicit) = req_system_prompt.filter(|s| !s.trim().is_empty()) {
        out.push_str(explicit);
    }

    if let Some(visitor) = ctx.get(crate::symbio_core::CAPABILITY_VISITOR) {
        for (_, text) in crate::symbio_core::capability_prompts(&*visitor).await {
            if text.trim().is_empty() {
                continue;
            }
            if !out.is_empty() {
                out.push_str("\n\n");
            }
            out.push_str(&text);
        }
    }

    // 全空（既无显式段也无注册段）→ 硬编码兜底，绝不把空提示词发给模型
    if out.trim().is_empty() {
        return FALLBACK_SYSTEM_PROMPT.to_string();
    }
    out
}

/// 兜底系统提示词 —— 只在「显式段与注册段都为空」时使用。
///
/// 正常情况下 model 插件会注册一段人格，本常量因此几乎不会出现；它的存在是为了
/// 让「没装 model 插件的裸环境」也有一句可说，而不是把空串发给模型。
pub(crate) const FALLBACK_SYSTEM_PROMPT: &str = "You are a helpful MODEL assistant.";

/// 一次 LLM 请求所需的全部输入（收口 ② 的产物）。
///
/// **唯一收集点**：系统提示词与工具在同一函数（[`prepare_turn_inputs`]）内依次取得
/// ——同一个 `Registry`、同一个时刻；下游（推理 / 收尾）只读本结构，
/// 不再各自去 visitor 取值。
pub(crate) struct TurnInputs {
    /// Turn 根节点 id（本步内创建，供流式占位与推理产物挂载）
    pub(crate) root_id: String,
    /// 系统提示词（`resolve_system_prompt` 为唯一真源）
    pub(crate) system_prompt: String,
    /// 本轮到模型的完整工具清单
    pub(crate) tools: Vec<CapabilityMeta>,
    /// 请求级注入段（`build_request_sections` 唯一入口的产物，**不落库**）
    pub(crate) request_sections: Vec<ChatNode>,
}

/// 收口 ②：本轮 LLM 请求输入的**唯一准备点**。
///
/// ## 硬约束：系统提示词与工具必须同机制、同时机、始终一起收集
///
/// 两者都由插件经 `Registry` 注册、经同一次 `traverse` 汇集到同一个
/// visitor 上——来源与生命周期完全一致。拆成两处、两个时机收集，会让"这一轮模型
/// 看到的人格"与"这一轮模型能调的工具"出现不一致窗口。故本函数是它们的唯一取值
/// 点，且两步紧邻、中间不插入任何其它动作。
///
/// ## 时机：每轮 Turn 开头（循环内），**不**提升到前步骤
///
/// `capability_prompts` / 工具表的 `list` 都是纯内存读聚合表、不触发
/// I/O，相对一次 LLM 推理可忽略；换来的是"每轮可动态调整人格与工具集"（例如中途
/// 新连上的 MCP 工具，下一轮即可见）。若将来要提升到循环外（例如单轮工具轮次极多
/// 使查询成为可观开销，或需要"一次请求内人格与工具集冻结"），**必须两者一起提升**，
/// 不允许只提升其一。
///
/// ## 本步内的顺序（1–5 与收口前逐条对齐，语义不变；6 为 D-047 新增）
///
/// 1. 系统提示词 + 工具（同函数、同一时刻）
/// 2. 派生请求级开销 `overhead_tokens`（口径与压缩阈值共用，见 [`apply_compaction`]）
/// 3. 压缩（收口 ③：[`apply_compaction`]，自动语义压缩的唯一响应点）
///    - 3′. 紧随压缩：取本轮上下文水位（压缩之后的那一体积，随 4 的骨架进 `meta.context`）
/// 4. Turn 根节点流式占位（**必须在压缩之后**：压缩失败时不留半截 Turn 节点）
/// 5. 长期记忆召回（S5 步 12：跨会话读视图 → 置顶注入请求视图；只在本轮**第一个**
///    工具轮取一次，后续工具轮复用同一份视图）
/// 6. 委派者真源段（Q1 判定 + Q2 能力目录）：**在①②把 `tools` 收集成形之后**才动笔
///    ——这是 D-047 的注入点约束本身，见 [`delegate`] 模块头
/// 7. 请求视图重建（`build_request_view` 唯一入口）
pub(crate) async fn prepare_turn_inputs(
    orchestrator: &ChatOrchestrator,
    ctx: &Arc<dyn PluginInvokeRequest>,
    context: &mut SessionContext,
    sink: &ExecEventSink,
    turn: &mut TurnState,
    req: &TurnRequest,
) -> Result<TurnInputs, TurnExit> {
    // ── ① 系统提示词 + 工具：同一函数、同一时刻、同一 Registry ──────
    let system_prompt = resolve_system_prompt(req.system_prompt.as_deref(), ctx).await;

    let tools = crate::symbio_core::tool_metas(ctx).await;

    // ── ② 请求级固定开销：一轮只算一次，供下方压缩共用
    //        （收口前两处各算一次，且各自要重新取一遍工具清单）────────────────
    let overhead_tokens = context::estimate_overhead_with_tools(&system_prompt, &tools);

    // ── ③ 压缩（收口 ③：自动语义压缩，唯一响应点）───────────────────────────
    apply_compaction(orchestrator, ctx, context, turn, overhead_tokens).await?;

    // ── ③′ 取本轮的上下文水位（压缩**之后**：量的是这一轮真正发出去的那一体）──
    // 只在这里算一次，随 ④ 的骨架挂进 `meta.context`；判据是「一个请求级事实挂在
    // 它所属的那一轮上」，不是「前端要轮询」。取不到也不算异常：没有水位
    // ⇒ 前端不渲染水位条，而不是补一个 0。
    let watermark = context::context_watermark(
        &context.messages,
        overhead_tokens,
        orchestrator.context_limit as usize,
    );

    // ── ④ Turn 根节点流式占位 ────────────────────────────────────────────
    // 续写幕复用原 Turn 根（d）：那个骨架在本轮首幕已经发过，再发一次
    // 只会让同一个 id 重生一条「流式中」的根，把已完成的卡打回运行态。
    // 续写幕也就不再补水位——首幕那条 `meta.context` 是这一轮的唯一水位读数。
    let root_id: String = match turn.resume_root.take() {
        Some(root) => root,
        None => {
            let root = llm_short_id();
            emit_streaming_start(
                sink,
                &root,
                Some(turn.request_round),
                Some(watermark),
                &orchestrator.session.principal(),
            )
            .await;
            root
        }
    };

    // ── ⑤ 长期记忆召回（S5 步 12）：跨会话读视图，本轮内只取一次 ─────────────
    // 判据在表里（`policy.recall`）：「本轮第一次 ∧ 有落盘目录 ∧ 未做过」。
    // 三件事（记忆 / 技能 / 任务段）共用一个阶段——它们共前提、共时机。
    // `done` = 本次执行里这一步已经做过（`turn.stage` 的 `recall` 格，按幕锚折叠）。
    // 续写幕从空开始 ⇒ 照常重跑（恢复后事实源变了，本轮视图应当重取）。
    if super::super::loop_rows::owes("policy.recall", &turn.facts, turn.turn_no, turn.now_ms) {
        run_recall(ctx, context, turn).await?;
    }
    let recall_section = turn
        .recall_view
        .as_ref()
        .and_then(super::super::memory_events::prompt_section);

    // ── ⑥ 请求级注入段（唯一入口 build_request_sections）──────────────────
    // 三段**追加**的读视图：长期记忆 / 就绪任务 / 委派者真源。它们不落库、不碰历史——
    // 对话体是事实网格的纯投影（D-048a），请求视图级的淡化 / 骨架化已按
    // [D-078](../../session/README.md#d-078-请求视图三层淡化退役请求体是事实网格的纯投影注入段只追加不改写 "symbio/src/plugins/session/README") 退役。
    //
    // ── ⑥′ 委派者真源段（D-047）：Q1 判定 + Q2 能力目录 ─────────────────────
    //
    // **位置即约束**：能力目录必须在 ①② 把 `tools` 收集成形**之后**才动笔——收集是
    // 广播，遍历期取集合必然漏掉还没被访问到的兄弟插件（静默、且漏哪几项次次不同）。
    // 这里取的是本轮**最终**发给模型的同一份 `tools`（即上面 ① 收下的那一份，
    // 中间没有任何 push）⇒ 同源，不构成第二份真相。
    //
    // Q1 的输入取 `turn.input_utterance`（循环**前**锚定的 `(消息 id, 正文)`）⇒
    // 同一请求内不随工具轮变化，判定不漂移；`None` = resume / 心跳这类没有用户新
    // 发言的请求，判决无从谈起（与判决侧「None 根本不进判决」同一条口径）。
    // Q3：worker 进展现读磁盘（无子会话 ⇒ 零额外 IO）。整条读链住在
    // `session::worker_progress`（D-081 裁决一第 4 条），驱动器只调这一个入口。
    // 读失败向上传播——静默降级就是「没有观测 = 静默失效」：看不出是没 worker 还是没读到。
    let workers = context
        .session
        .run_op::<Vec<crate::symbio_core::WorkerProgress>>(
            crate::symbio_core::run_ops::WORKER_PROGRESS,
            (),
        )
        .await
        .map_err(TurnExit::Failed)?;
    // Q1 的三条判据按**本会话当前配置**取（`worker_*` 三字段），不再读判定模块的
    // 常量——常量只是出厂值的单一真源，运行时值属于配置面。
    let settings = context.session.settings();
    let delegate_section = delegate::delegate_section_with(
        turn.input_utterance.as_ref().map(|(_, text)| text.as_str()),
        &tools,
        &workers,
        delegate::WorkerPolicy {
            force_prefix: &settings.worker_force_prefix,
            keywords: &settings.worker_keywords,
            min_chars: settings.worker_min_chars,
        },
    );
    // 段的**次序**在这里声明（= 模型读到的上下文次序），段的**名字**由 `SectionKind`
    // 唯一给出。加一段 = 这个数组加一行 + 枚举加一个变体，两处都由编译器逼着补全。
    let request_sections = context::build_request_sections(&[
        // 长期记忆：**第一项** ⇒ 落在 index 0（背景事实置顶，见函数文档）。
        (context::SectionKind::Recall, recall_section.as_deref()),
        // 调度段（S7 步 16–17，批⑨）：就绪任务集。
        (
            context::SectionKind::ReadySet,
            turn.ready_section.as_deref(),
        ),
        // 委派者真源段（D-047）：Q1 判定 + Q2 能力目录。
        (context::SectionKind::Delegate, delegate_section.as_deref()),
    ]);

    Ok(TurnInputs {
        root_id,
        system_prompt,
        tools,
        request_sections,
    })
}

/// 轮首准备格的执行体（`policy.recall`）：长期记忆召回 + 技能路由 + 调度段。
///
/// 调用点只问「这一格欠不欠」；执行体留在本层——它的入参是 `TurnState` /
/// `SessionContext` 这类私有上下文，不是行本身（D-107）。
async fn run_recall(
    ctx: &Arc<dyn PluginInvokeRequest>,
    context: &SessionContext,
    turn: &mut TurnState,
) -> Result<(), TurnExit> {
    // `query` = 本轮用户发言（混合打分的语义维）——与 `turn_exec`
    // 取 `user_text` 同一取值点（轮首入列的最后一条 user 消息；该处晚于本步，
    // 但用户消息在步骤 3 之前已入列，两次取到的是同一条）。
    let recall_query = last_user_text(context);
    // 读方 = **本轮主体**（`request_principal`，与写事件的 actor 同一取值点）：
    // 召回的可见切片按它收放——判断记忆只给自己会话看，共享记忆只
    // 给矩阵内的主体看。取值点不另立：判的是 A、看的是 B 就没有隔离可言。
    let recall_viewer = crate::symbio_core::request_principal(ctx.as_ref());
    // 事实是**值**（[D-084](../../../../../docs/003-design/001-核心架构.md#d-084-能力模块公用基础协议上下文只递值事实只经引擎abcde-五类需要的唯一取得方式 "DE-001-核心架构")）：
    // 本会话切片取一次，召回 / 调度段 / 技能路由三个读方共用——同一轮内
    // 它们看到的是同一刻的事实，不各开各的文件。
    let facts = context.session.facts_snapshot();
    turn.recall_view = super::super::memory_events::recall_view(
        context
            .session
            .run_op::<Vec<crate::symbio_core::FleetSnapshot>>(
                crate::symbio_core::run_ops::RECALL_SNAPSHOTS,
                (),
            )
            .await
            .map_err(TurnExit::Failed)?,
        recall_query.as_deref(),
        &recall_viewer,
    );
    // 调度段（S7 步 16–17，批⑨）：就绪任务集只读一次、本轮各工具轮复用——与
    // 召回同一条口径（同一轮内集合不漂移）。
    turn.ready_section =
        super::super::task_events::prompt_section(&facts, context.session.task_cap());
    // 技能路由（S9 步 22，[04 §3.1 批⑪](../../../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证")）：
    // 置信度闸判 + 把低置信的技能**摘出本轮召回视图**（反自动化回退，S11 §5）。
    // 与召回 / 调度段同一条 `request_round == 0` 口径（同一轮内判定不漂移）；
    // 临时会话同「不读不写」——没有事实源就没有技能，无从判。
    //
    // **一次判定、两个出口**：观测存进 `TurnState::skill_route`（轮末收束才入格，
    // 溯源锚那时才在事实源里），可用集存进 `TurnState::skill_hits`（供 `turn_exec`
    // 装配反射档，S11 §2「命中技能后走 `budget_ms` 更小的 ActorSpec」）。
    // 分两处各判一遍必然漂移成「摘出视图的那条」与「拿去执行的那条」不是同一条。
    let routing = turn
        .recall_view
        .as_mut()
        .map(|view| super::super::skill_events::route(&facts, view))
        .unwrap_or_default();
    // 快路开关（S11 §4 参数表的 `actor.pattern`；**不是** core 的 `ActorSpec.pattern`，
    // 两者同名不同物，见 D-079）：关着时可用集恒空——判定照常发生、
    // 观测照常落格（"只编译不加速"的观察形态），只是没人拿它去装配——这就是它
    // 作为一个独立开关的全部含义（见 `SessionConfig` 字段文档）。
    let fast_armed = context.session.skill_fast_path();
    turn.skill_route = routing.obs;
    turn.skill_hits = if fast_armed { routing.hits } else { Vec::new() };
    // 「已做」与它的读视图同一时刻产生：下一问问表读它是为了不再做，
    // 落格交给运行器（写者令牌在 `turn_exec`）。`result` 记技能命中与否——
    // 义务环的 `has_match` 只从日志读（输入闭包）。
    let hit = last_user_text(context).as_deref().is_some_and(|text| {
        super::super::skill_events::take_match(&turn.skill_hits, text).is_some()
    });
    turn.mark_stage(
        StageFact::ran(StageName::Recall, turn.request_round).with_result(if hit {
            "hit"
        } else {
            "miss"
        }),
    );
    Ok(())
}

/// 收口 ③：上下文水位的**唯一响应点**——自动语义压缩只在这里判定与执行。
///
/// ## 顺序不可交换（收口前由主循环体的行序隐含保证，现在由本函数内部保证）
///
/// 按 70% 阈值执行 L1 自动语义压缩（就地改写 `context.messages`），压缩结果直接
/// 决定后续请求视图的水位。55% 水位提醒（nudge）已随 `context_compact` 一并退役
/// （[D-072](../../session/README.md#d-072-context_compact-工具与-55-水位提醒一并退役压缩只剩一个入口 "symbio/src/plugins/session/README")）：它存在的唯一理由是引导模型主动调用那个工具，工具没了，提醒就是
/// 一条指向空处的指令。
///
/// 将来若要恢复「模型主动压」：注册成能力行走 v2 分发，再评估是否恢复提醒——
/// 不再回 v1 的 `close_turn` 拦截形态。
pub(crate) async fn apply_compaction(
    orchestrator: &ChatOrchestrator,
    ctx: &Arc<dyn PluginInvokeRequest>,
    context: &mut SessionContext,
    turn: &mut TurnState,
    overhead_tokens: usize,
) -> Result<(), TurnExit> {
    // 判据在表里（`policy.compress`）：本处只问"这一格要不要进压缩产线"。
    // 压不压得动（水位 / 熔断 / 失败回滚）是压缩内核自己的事，不在本行。
    // `done` 的口径是**本请求轮**：内核每轮重新取水位（未过水位返回 `Ok(None)`），
    // 下一轮照常再试；但同一幕内不重复进产线（D-107 判据 2）。
    if super::super::loop_rows::owes("policy.compress", &turn.facts, turn.turn_no, turn.now_ms) {
        run_compress(orchestrator, ctx, context, turn, overhead_tokens).await;
    }
    Ok(())
}

/// 压缩格的执行体（`policy.compress`）：进一次压缩产线并落「已做」。
///
/// 失败**不失败整轮**——压缩是优化项，理由见函数体内的注释。
async fn run_compress(
    orchestrator: &ChatOrchestrator,
    ctx: &Arc<dyn PluginInvokeRequest>,
    context: &mut SessionContext,
    turn: &mut TurnState,
    overhead_tokens: usize,
) {
    // 入账面：摘要那次模型往返走端口 `model`（D-122 决策 2）——三格由端口层落，
    // 这一次往返在账上看得见。没有事实源就没有账可记（临时会话），压缩照旧跑，
    // 只是那一次不在账上。
    let compress = context.session.facts().map(|store| {
        Compress::new(
            store,
            turn.turn_no,
            turn.user_seq,
            turn.drive_seq,
            turn.now_ms,
            turn.request_round,
        )
    });
    match auto_compress_process(
        orchestrator,
        context,
        ctx,
        &turn.abort,
        overhead_tokens,
        false,
        compress.as_ref(),
    )
    .await
    {
        Ok(Some(history_count)) => {
            plugin_info!(
                "session",
                "[Compress] 上下文已压缩：{} 条消息 → 1 条摘要",
                history_count
            );
            turn.last_saved = context.messages.len();
        }
        Ok(None) => {}
        // 压缩失败**不失败整轮**：压缩是优化项，失败已就地回滚、历史完整，
        // 本轮仍应正常回复用户。失败原因已写入压缩节点（meta.failure_kind +
        // error），连续失败会触发熔断（跳过后续自动压缩）。
        Err(e) => {
            plugin_warn!("session", "[Compress] 自动压缩失败: {e}");
        }
    }
    // 「已做」= 本请求轮进过产线，不是「压成了」：水位未过 / 内核失败时
    // 下一轮仍会再试（判据按 `request_round` 计），同一轮内不重复调用。
    //
    // `model_calls` 是这一压打了几次模型（摘要 + 纠正重试）：它是那次往返在事实层
    // 唯一的数，判据侧拿它对账「观测 ⇔ 三格」。同一性带上本幕的锚——续写幕会重跑
    // 压缩（恢复后事实源变了），不带上就撞首幕的幂等键，这一格连同它的数一起不落。
    turn.mark_stage(StageFact::compress(
        turn.request_round,
        compress.as_ref().map(|c| c.calls()).unwrap_or(0),
        turn.drive_seq,
    ));
}

#[cfg(test)]
#[path = "inputs.test.rs"]
mod tests;
