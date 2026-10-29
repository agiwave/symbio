//! 一次 LLM 请求的**输入准备**：环前的纯读、轮首那几行，与它们的执行体。
//!
//! - [`resolve_system_prompt`]：系统提示词唯一真源
//! - [`collect_reads`]：提示词 + 工具 + 开销——环前的纯读，不含任何选债口
//! - [`prepare_request_sections`]：水位、根骨架上线路与请求视图——接缝动作在
//!   轮首行之后调它，任何请求之前
//! - [`run_recall`]：`policy.recall` 行的执行体（记忆 / 技能 / 调度段三段读视图）
//! - [`run_compress`]：`policy.compress` 行的执行体（上下文水位的响应，产线住在
//!   这里，**被选中才跑**）

use symbio_core::CapabilityMeta;
use symbio_plugin_api::context::{Cadence, DelegateVerdict, LiveInputs};

use super::super::port::Compress;
use super::super::stage_events::StageFact;
use super::*;

/// 本轮用户发言 = 轮首入列的最后一条 user 消息的正文。
///
/// **取值点与 `turn_exec` 的 `user_text` 同源**（该处晚于 `prepare_request_sections`，
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

/// 本轮读侧值包：轮首与请求级两次收集共用同一构造口径。
///
/// 两种节奏是**同一请求的两半**（轮首段 / 请求级段），必须看到同一刻的值——各拼
/// 一份就会漂移成「召回读的是 A、委派读的是 B」。`live` 只在请求级收集时为 `Some`。
#[allow(clippy::too_many_arguments)]
fn context_request<'a>(
    turn: &TurnState,
    principal: &'a str,
    query: Option<&'a str>,
    facts: &'a [symbio_core::Event],
    recall: Option<&'a symbio_plugin_api::RecallFacts<'a>>,
    task_cap: usize,
    live: Option<&'a LiveInputs<'a>>,
) -> symbio_plugin_api::ContextRequest<'a> {
    symbio_plugin_api::ContextRequest {
        turn: turn.turn_no,
        request_round: u32::try_from(turn.request_round).unwrap_or(u32::MAX),
        principal,
        query,
        facts,
        recall,
        task_cap,
        live,
    }
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

    if let Some(visitor) = ctx.get(symbio_core::CAPABILITY_VISITOR) {
        for (_, text) in symbio_core::capability_prompts(&*visitor).await {
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

/// 环前的**纯读**（①②）：系统提示词 + 工具 + 请求级开销。
///
/// 不含任何选债口、不落任何格——它发生在幕首（`assemble_turn` / `begin`）之前，
/// 而环上的每一行都会改事实面。`tools` 同时是 ⑥ 的输入（请求级段要在
/// 工具收集成形**之后**才收集，D-047 的注入点约束），故随本结构交给
/// [`prepare_request_sections`]。
pub(crate) struct TurnReads {
    /// 系统提示词（`resolve_system_prompt` 为唯一真源）。
    pub(crate) system_prompt: String,
    /// 本轮到模型的完整工具清单。
    pub(crate) tools: Vec<CapabilityMeta>,
}

/// 收口 ②：本轮 LLM 请求读侧的**唯一收集点**。
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
pub(crate) async fn collect_reads(
    ctx: &Arc<dyn PluginInvokeRequest>,
    req: &TurnRequest,
    turn: &mut TurnState,
) -> TurnReads {
    // ── ① 系统提示词 + 工具：同一函数、同一时刻、同一 Registry ──────
    let system_prompt = resolve_system_prompt(req.system_prompt.as_deref(), ctx).await;

    let tools = symbio_core::tool_metas(ctx).await;

    // ── ② 请求级固定开销：一轮只算一次，寄存进本轮状态供压缩行读水位
    //        （收口前两处各算一次，且各自要重新取一遍工具清单）────────────────
    let overhead_tokens = context::estimate_overhead_with_tools(&system_prompt, &tools);
    turn.request_overhead = overhead_tokens;

    TurnReads {
        system_prompt,
        tools,
    }
}

/// 轮首那几行跑完之后才知道的一切：在**草稿建好之后**、任何请求之前。
///
/// ## 为什么它晚于 `begin`
///
/// 中止守卫在环首（`D-222` 订正 3）：被中止的幕不该再跑判决 / 压缩 / 召回。草稿在
/// 幕首就建，本函数因此只能排在那之后——轮首的每一行都是一笔会改事实面的动作。
///
/// ## 它仍然早于第一次请求
///
/// 判决 / 压缩 / 召回 / 快路的出口格正是本轮请求与收束的前件；本函数末尾的请求
/// 视图也依赖它们的寄存物（召回寄存的段、快路寄存的正文）。
///
/// ## 本步内的顺序
///
/// 1. **上下文水位**（③″）：压缩**之后**量——量的是这一轮真正发出去的那一体；
/// 2. **Turn 根节点流式占位**（④，**必须在压缩之后**：压缩失败时不留半截 Turn
///    节点）。根 id 由幕首（`turn_exec::drive` 的 `open_stage`）分配——「定 id」与「把它发给
///    流式桥」是两件事，前者是 `begin` 的输入。`fresh_root` 说这个 id 是不是本幕
///    新分配的：续写幕复用原根，重发骨架会把已完成的卡打回运行态。
/// 3. **请求级上下文段**（`Cadence::Request`，D-220）：**在 `tools` 收集成形之后**
///    才收集——这是 D-047 的注入点约束本身，见 [`delegate`] 模块头；
/// 4. **请求视图重建**（`build_request_view` 唯一入口）。
///
/// 失败交回 [`ActorError::Read`]：轮首的读失败发生在幕锚之后、任何请求格之前，
/// 端口面上没有一笔账可关，由环上抛给编排层的兜底路径。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn prepare_request_sections(
    orchestrator: &ChatOrchestrator,
    ctx: &Arc<dyn PluginInvokeRequest>,
    context: &mut SessionContext,
    sink: &ExecEventSink,
    turn: &mut TurnState,
    root_id: &str,
    fresh_root: bool,
    tools: &[CapabilityMeta],
) -> Result<Vec<ChatNode>, super::super::turn_exec::ActorError> {
    // ── ③″ 取本轮的上下文水位（压缩**之后**：量的是这一轮真正发出去的那一体）──
    // 只在这里算一次，随 ④ 的骨架挂进 `meta.context`；判据是「一个请求级事实挂在
    // 它所属的那一轮上」，不是「前端要轮询」。取不到也不算异常：没有水位
    // ⇒ 前端不渲染水位条，而不是补一个 0。
    let watermark = context::context_watermark(
        &context.messages,
        turn.request_overhead,
        orchestrator.context_limit as usize,
    );

    // ── ④ Turn 根节点流式占位 ────────────────────────────────────────────
    // 续写幕复用原 Turn 根（d）：那个骨架在本轮首幕已经发过，再发一次
    // 只会让同一个 id 重生一条「流式中」的根，把已完成的卡打回运行态。
    // 续写幕也就不再补水位——首幕那条 `meta.context` 是这一轮的唯一水位读数。
    //
    // 根 id 由**调用方**分配（一幕一个根；`begin` 要用它，故它在幕首就定了）。
    // 本步只做「把它发给流式桥」——新建的根要上线，续写的根不重发。
    if fresh_root {
        emit_streaming_start(
            sink,
            root_id,
            Some(turn.request_round),
            Some(watermark),
            &orchestrator.session.principal(),
        )
        .await;
    }

    // ── ⑤ 上下文段（provider 收集的产物）──────────────────────────────────────
    // 收集在环上的 `policy.recall` 那一行（D-159）一次做完：那里把「判完的值」交给
    // 端口，provider 各自渲染成段。这里只读本轮寄存的那一份——本函数因此**不知道
    // 有哪几段**（DE-025 D-215）。

    // ── ⑥ 请求级注入段（唯一入口 build_request_sections）──────────────────
    // 追加的读视图：provider 贡献的各段。它们不落库、不碰历史——
    // 对话体是事实网格的纯投影（D-048a），请求视图级的淡化 / 骨架化已按
    // [D-078] 退役。
    //
    // ── ⑥′ 请求级上下文段（`Cadence::Request`，DE-025 D-220）────────────────
    //
    // **位置即约束**：委派段的能力目录要的是 ①② 把 `tools` 收集成形**之后**的产物
    // ——收集是广播，遍历期取集合必然漏掉尚未访问到的兄弟插件（静默、且漏哪几项
    // 次次不同）。这里取的是本轮**最终**发给模型的同一份 `tools`（中间没有任何
    // push）⇒ 同源，不构成第二份真相。
    //
    // Q1 判定仍在本函数（控制面）：`turn.input_utterance` 是循环**前**锚定的
    // `(消息 id, 正文)`，同一请求内不随工具轮变化；判定读的是**本会话当前配置**
    // （`worker_*` 三字段，出厂值真源在 core）。provider 只接到判完的
    // `DelegateVerdict`——判断闭集（D-208）。
    //
    // Q3：worker 进展现读磁盘（无子会话 ⇒ 零额外 IO）。整条读链住在
    // `session::worker_progress`（D-081 裁决一第 4 条），驱动器只调这一个入口。
    // 读失败向上传播——静默降级就是「没有观测 = 静默失效」：看不出是没 worker 还是没读到。
    let workers = context
        .session
        .run_op::<Vec<symbio_core::WorkerProgress>>(symbio_core::run_ops::WORKER_PROGRESS, ())
        .await
        .map_err(super::super::turn_exec::ActorError::Read)?;
    let settings = context.session.settings();
    let delegate_verdict = match turn.input_utterance.as_ref().map(|(_, text)| text.as_str()) {
        Some(text) => {
            // 关键词存**逗号串**（面板无数组控件），切开即用；空串由
            // `worker_start_reason` 的「空关键词跳过」处理 ⇒ 空串 = 空集 = 本层关闭。
            let keywords: Vec<&str> = settings.worker_keywords.split(',').collect();
            match delegate::worker_start_reason(
                text,
                &settings.worker_force_prefix,
                &keywords,
                settings.worker_min_chars,
            ) {
                Some(reason) => DelegateVerdict::Start { reason },
                None => DelegateVerdict::Idle,
            }
        }
        None => DelegateVerdict::NotJudged,
    };

    // 段的**次序**在这里声明（= 模型读到的上下文次序）：轮首段在前、请求级段在后。
    // 加一段 provider 只需提交一条实现，本数组不动。
    let facts = context.session.facts_snapshot();
    let live_lines: Vec<symbio_plugin_api::RecallLine<'_>> = turn
        .recall_view
        .as_ref()
        .map(|view| {
            view.entries
                .iter()
                .map(|e| symbio_plugin_api::RecallLine {
                    content: &e.content,
                    tag: &e.tag,
                })
                .collect()
        })
        .unwrap_or_default();
    let live_recall = turn
        .recall_view
        .as_ref()
        .map(|view| symbio_plugin_api::RecallFacts {
            degraded: view.degraded,
            lines: &live_lines,
        });
    let query = last_user_text(context);
    let principal = symbio_core::request_principal(ctx.as_ref());
    let live = LiveInputs {
        tools,
        delegate: delegate_verdict,
        workers: &workers,
    };
    let request = context_request(
        turn,
        &principal,
        query.as_deref(),
        &facts,
        live_recall.as_ref(),
        context.session.task_cap(),
        Some(&live),
    );
    let mut sections: Vec<(&'static str, String)> = turn
        .context_sections
        .iter()
        .map(|s| (s.slot, s.text.clone()))
        .collect();
    sections.extend(
        symbio_plugin_api::context_sections(&request, Cadence::Request)
            .into_iter()
            .map(|s| (s.slot, s.text)),
    );
    Ok(context::build_request_sections(&sections))
}

/// `recall` 执行体（`policy.recall`）：长期记忆召回 + 技能路由 + 调度段。
///
/// 它是环上 `policy.recall` 那一行的动作（[`super::super::turn_exec::run_recall`]
/// 转过来），入参由 `Ctx` 的轮首借面给出：请求句柄 + 事实源 + 会话上下文 + 本轮状态。
/// 账在表上、动作也在表上，调用点不再按名问这一行欠不欠（DE-015 的 D-159 + `D-222`
/// 裁决 5）。
///
/// ## 出口那一格在这里落
///
/// 派发器在两次动作之间重读日志，而 `turn_stages` 视图只折叠**已落**的 `turn.stage`
/// 格。动作做完却不入日志，本幕的第二次选债照样读到「没做过」⇒ 环走不动。
/// 落格能力此刻就在本层（与 `begin_drive` 落用户格 / 配置 / 轮次格同一条路、同一个句柄）。
///
/// `result` 记技能命中与否——义务环的 `has_match` 与反射行的 `skill_hit` 都只从
/// 日志读（输入闭包）。同一性缀本幕锚（`drive_seq`）：续写幕照常重跑召回，撞首幕
/// 的幂等键会让那一格的载荷不落、结论在事实面消失（规则见 `stage_events` 模块文档）。
pub(crate) async fn run_recall(
    ctx: &Arc<dyn PluginInvokeRequest>,
    store: &dyn symbio_core::Store<Event = symbio_core::Event>,
    context: &SessionContext,
    turn: &mut TurnState,
) -> Result<(), super::super::turn_exec::ActorError> {
    // `query` = 本轮用户发言（混合打分的语义维）——与 `turn_exec`
    // 取 `user_text` 同一取值点（轮首入列的最后一条 user 消息；该处晚于本步，
    // 但用户消息在步骤 3 之前已入列，两次取到的是同一条）。
    let recall_query = last_user_text(context);
    // 读方 = **本轮主体**（`request_principal`，与写事件的 actor 同一取值点）：
    // 召回的可见切片按它收放——判断记忆只给自己会话看，共享记忆只
    // 给矩阵内的主体看。取值点不另立：判的是 A、看的是 B 就没有隔离可言。
    let recall_viewer = symbio_core::request_principal(ctx.as_ref());
    // 事实是**值**（[D-084](../../../../docs/003-design/001-核心架构.md#d-084-能力模块公用基础协议上下文只递值事实只经引擎abcde-五类需要的唯一取得方式 "DE-001-核心架构")）：
    // 本会话切片取一次，召回 / 调度段 / 技能路由三个读方共用——同一轮内
    // 它们看到的是同一刻的事实，不各开各的文件。
    let facts = context.session.facts_snapshot();
    turn.recall_view = super::super::memory_events::recall_view(
        context
            .session
            .run_op::<Vec<symbio_core::FleetSnapshot>>(symbio_core::run_ops::RECALL_SNAPSHOTS, ())
            .await
            .map_err(super::super::turn_exec::ActorError::Read)?,
        recall_query.as_deref(),
        &recall_viewer,
    );
    // 调度段的**数据**（S7）：就绪任务集只读一次、本轮各工具轮复用
    // ——与召回同一条口径（同一轮内集合不漂移）。
    //
    // 这里不再直接渲染：渲染搬进 `context::providers::ReadySetProvider`（DE-025 D-215），
    // 本函数只把「本轮的事实切片 + 容量」交给端口。于是「有哪几段」不再写在这里。
    // 技能路由（S9）：
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

    // ── 上下文段：判断在上面，渲染在端口后面（DE-025 D-208 / D-215）──────────
    //
    // 上面几步是**判断**（召回哪些条目、哪些技能过闸），它们住控制面；这里只把判完的
    // 值交成 `ContextRequest`，由 provider 自己决定「我这一段长什么样」。
    //
    // 于是「加一类每轮上下文」= 加一个 provider，本函数一行不动。两段现存贡献
    // （召回 / 调度）的渲染已搬到 `context::providers`，与端口化之前的输出逐字一致。
    //
    // `RecallLine` 借用 `turn.recall_view`：先把段收集进局部量，再写回 `turn`
    // ——否则「借视图」与「改本轮状态」会撞在同一处。
    let recall_lines: Vec<symbio_plugin_api::RecallLine<'_>> = turn
        .recall_view
        .as_ref()
        .map(|view| {
            view.entries
                .iter()
                .map(|e| symbio_plugin_api::RecallLine {
                    content: &e.content,
                    tag: &e.tag,
                })
                .collect()
        })
        .unwrap_or_default();
    let recall_facts = turn
        .recall_view
        .as_ref()
        .map(|view| symbio_plugin_api::RecallFacts {
            degraded: view.degraded,
            lines: &recall_lines,
        });
    let request = context_request(
        turn,
        &recall_viewer,
        recall_query.as_deref(),
        &facts,
        recall_facts.as_ref(),
        context.session.task_cap(),
        None,
    );
    let sections = symbio_plugin_api::context_sections(&request, Cadence::Turn);
    turn.context_sections = sections;

    // 「已做」与它的读视图同一时刻产生：下一问问表读它是为了不再做，故必须就地
    // 入日志（见本函数文档）。`result` 记技能命中与否——义务环的 `has_match` 只从
    // 日志读（输入闭包）。
    let hit = last_user_text(context).as_deref().is_some_and(|text| {
        super::super::skill_events::take_match(&turn.skill_hits, text).is_some()
    });
    super::super::stage_events::land(
        store,
        turn.turn_no,
        turn.user_seq,
        &symbio_core::request_principal(ctx.as_ref()),
        &[StageFact::recall(
            turn.request_round,
            turn.drive_seq,
            if hit { "hit" } else { "miss" },
        )],
    )
    .map_err(super::super::turn_exec::ActorError::Append)?;
    Ok(())
}

/// 快路格的执行体（环上的 `policy.reflex` 行）：命中即作答，**零模型往返**。
///
/// ## 它只落「已答」，不落收束
///
/// 出口格是 `turn.stage{stage:"reflex"}`（`result` = 命中的技能 id），正文在这里
/// 寄进本轮状态（[`TurnState::head_reflex`]）：收束格 `chat.assistant.final` 由
/// `close-turn` 的执行体落——两个执行体写同一批格会让依赖图成环（D-163）。
/// 收束行据此在零往返的轮上开账（`turn_head.reflex_answered`）。
///
/// ## 开账条件与命中判定同源
///
/// 本行动作只在 `turn_head.skill_hit` 成立时被选中，而那一格来自 [`run_recall`]
/// 对同一份 `skill_hits` / 用户发言的 `take_match`。因此这里命中为空是**不可达**的；
/// 真出现了也只落 `result:"miss"` 把账关掉，不拿空正文冒充作答（那一轮照常进模型往返）。
pub(crate) async fn run_reflex(
    ctx: &Arc<dyn PluginInvokeRequest>,
    store: &dyn symbio_core::Store<Event = symbio_core::Event>,
    context: &SessionContext,
    turn: &mut TurnState,
) -> Result<(), super::super::turn_exec::ActorError> {
    let text = last_user_text(context);
    let hit = text
        .as_deref()
        .and_then(|t| super::super::skill_events::take_match(&turn.skill_hits, t));
    let result = match hit {
        Some(h) => {
            symbio_core::plugin_info!(
                "session",
                "[Loop] 技能命中，走反射档（skill={}，turn={}）",
                h.skill_id,
                turn.turn_no
            );
            turn.head_reflex = Some(super::super::turn_runner::ReflexAnswer {
                text: h.content.clone(),
            });
            h.skill_id.clone()
        }
        None => "miss".to_string(),
    };
    super::super::stage_events::land(
        store,
        turn.turn_no,
        turn.user_seq,
        &symbio_core::request_principal(ctx.as_ref()),
        &[StageFact::reflex(
            turn.request_round,
            turn.drive_seq,
            &result,
        )],
    )
    .map_err(super::super::turn_exec::ActorError::Append)?;
    Ok(())
}

/// 压缩格的执行体（环上的 `policy.compress` 行）：进一次压缩产线并**自己落**「已做」。
///
/// ## 它是执行体，不是点问
///
/// 「欠不欠」由环向求解器要，本函数只在被选中时跑；出口那一格当场入日志
/// （与 [`run_recall`] 同一条理由：不入日志的账在下一次选债里照样是「没做过」）。
///
/// ## 顺序仍在产线里，不在调用点
///
/// 水位阈值读 [`TurnState::request_overhead`]（准备段 ①② 算出后寄存的那个数）。
/// 压不压得动（水位 / 熔断 / 失败回滚）是压缩内核自己的事，不在本行；失败**不失败
/// 整轮**——压缩是优化项，理由见函数体内的注释。
///
/// 55% 水位提醒（nudge）已随 `context_compact` 一并退役
/// （[D-072](../../../symbio-plugin-session/README.md#d-072-context_compact-工具与-55-水位提醒一并退役压缩只剩一个入口 "crates/symbio-plugin-session/src/README")）：它存在的唯一理由是引导模型主动调用那个工具，工具没了，提醒就是
/// 一条指向空处的指令。将来若要恢复「模型主动压」：注册成能力行走 v2 分发，再评估
/// 是否恢复提醒——不再回 v1 的 `close_turn` 拦截形态。
pub(crate) async fn run_compress(
    orchestrator: &ChatOrchestrator,
    ctx: &Arc<dyn PluginInvokeRequest>,
    store: &Arc<dyn symbio_core::Store<Event = symbio_core::Event>>,
    context: &mut SessionContext,
    turn: &mut TurnState,
) -> Result<(), super::super::turn_exec::ActorError> {
    // `done` 的口径是**本请求轮**：内核每轮重新取水位（未过水位返回 `Ok(None)`），
    // 下一轮照常再试；但同一幕内不重复进产线（D-107 判据 2）。
    //
    // 入账面：摘要那次模型往返走端口 `model`（D-122 决策 2）——三格由端口层落，
    // 这一次往返在账上看得见。没有事实源就没有账可记（临时会话连一幕都开不了，
    // 见 `assemble_turn`），本函数因此只在有事实源时跑。
    let compress = Compress::new(
        store.clone(),
        turn.turn_no,
        turn.user_seq,
        turn.drive_seq,
        turn.now_ms,
        turn.request_round,
    );
    match auto_compress_process(
        orchestrator,
        context,
        ctx,
        &turn.abort,
        turn.request_overhead,
        false,
        Some(&compress),
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
    super::super::stage_events::land(
        store.as_ref(),
        turn.turn_no,
        turn.user_seq,
        &symbio_core::request_principal(ctx.as_ref()),
        &[StageFact::compress(
            turn.request_round,
            compress.calls(),
            turn.drive_seq,
        )],
    )
    .map_err(super::super::turn_exec::ActorError::Append)?;
    Ok(())
}

#[cfg(test)]
#[path = "inputs.test.rs"]
mod tests;
