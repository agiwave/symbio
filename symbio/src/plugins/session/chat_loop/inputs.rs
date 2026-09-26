//! 一次 LLM 请求的**输入准备**与**压缩响应点**（收口 ②③）。
//!
//! - [`resolve_system_prompt`]：系统提示词唯一真源
//! - [`prepare_turn_inputs`]：提示词 + 工具 + 压缩 + 请求视图的唯一收集点
//! - [`apply_compaction`]：上下文水位的唯一响应点（自动语义压缩 → 水位提醒）

use super::*;

/// 系统提示词的唯一真源。
///
/// 结果 = **请求显式段（若有）** + **全部注册段**，按「先显式、后注册顺序」拼接。
///
/// ## 只有一个集合，没有竞争
///
/// 插件经 `CapabilityVisitor::register_system_prompt` 注册的每一段都**一定会送达
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
        for (_, text) in visitor.list_system_prompts().await {
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
/// ——同一个 `CapabilityVisitor`、同一个时刻；下游（推理 / 收尾）只读本结构，
/// 不再各自去 visitor 取值。
pub(crate) struct TurnInputs {
    /// Turn 根节点 id（本步内创建，供流式占位与推理产物挂载）
    pub(crate) root_id: String,
    /// 系统提示词（`resolve_system_prompt` 为唯一真源）
    pub(crate) system_prompt: String,
    /// 本轮到模型的完整工具清单（含条件注入的 `context_compact`）
    pub(crate) tools: Vec<CapabilityMeta>,
    /// 请求视图（`build_request_view` 唯一入口的产物，**不落库**）
    pub(crate) request_view: Vec<ChatMessage>,
}

/// 收口 ②：本轮 LLM 请求输入的**唯一准备点**。
///
/// ## 硬约束：系统提示词与工具必须同机制、同时机、始终一起收集
///
/// 两者都由插件经 `CapabilityVisitor` 注册、经同一次 `traverse` 汇集到同一个
/// visitor 上——来源与生命周期完全一致。拆成两处、两个时机收集，会让"这一轮模型
/// 看到的人格"与"这一轮模型能调的工具"出现不一致窗口。故本函数是它们的唯一取值
/// 点，且两步紧邻、中间不插入任何其它动作。
///
/// ## 时机：每轮 Turn 开头（循环内），**不**提升到前步骤
///
/// `list_capability` / `list_system_prompts` 都是纯内存读 visitor 注册表、不触发
/// I/O，相对一次 LLM 推理可忽略；换来的是"每轮可动态调整人格与工具集"（例如中途
/// 新连上的 MCP 工具，下一轮即可见）。若将来要提升到循环外（例如单轮工具轮次极多
/// 使查询成为可观开销，或需要"一次请求内人格与工具集冻结"），**必须两者一起提升**，
/// 不允许只提升其一。
///
/// ## 本步内的顺序（与收口前逐条对齐，语义不变）
///
/// 1. 系统提示词 + 工具（同函数、同一时刻）
/// 2. 派生请求级开销 `overhead_tokens`（口径：**不含**下面条件注入的
///    `context_compact`，与历史一致 → 压缩阈值 / 水位提醒的判定行为不变）
/// 3. 压缩（收口 ③：[`apply_compaction`]，自动语义压缩 + 水位提醒的唯一响应点）
/// 4. Turn 根节点流式占位（**必须在压缩之后**：压缩失败时不留半截 Turn 节点）
/// 5. 请求视图重建（`build_request_view` 唯一入口）
pub(crate) async fn prepare_turn_inputs(
    orchestrator: &ChatOrchestrator,
    ctx: &Arc<dyn PluginInvokeRequest>,
    context: &mut SessionContext,
    sink: &ExecEventSink,
    turn: &mut TurnState,
    req: &TurnRequest,
) -> Result<TurnInputs, TurnExit> {
    // ── ① 系统提示词 + 工具：同一函数、同一时刻、同一 CapabilityVisitor ──────
    let system_prompt = resolve_system_prompt(req.system_prompt.as_deref(), ctx).await;

    let mut tools = match ctx.get(crate::symbio_core::CAPABILITY_VISITOR) {
        Some(visitor) => visitor.list_capability().await,
        None => Vec::new(),
    };

    // ── ② 请求级固定开销：一轮只算一次，供下方压缩（阈值与提醒）共用
    //        （收口前两处各算一次，且各自要重新取一遍工具清单）────────────────
    let overhead_tokens = compression::estimate_overhead_with_tools(&system_prompt, &tools);

    // 主动压缩工具：仅当工具压缩启用时暴露给模型（独立于自动压缩开关）。
    // 执行不走 CapabilityVisitor 分发，由 `close_turn` 拦截处理（需要编排器内部链路）。
    if req.enable_compact_tool {
        tools.push(compression::context_compact_tool_meta());
    }

    // ── ③ 压缩（收口 ③：自动语义压缩 + 水位提醒，唯一响应点）───────────────
    let inject_nudge =
        apply_compaction(orchestrator, ctx, context, turn, req, overhead_tokens).await?;

    // ── ④ Turn 根节点流式占位 ────────────────────────────────────────────
    let root_id: String = llm_short_id();
    emit_streaming_start(sink, &root_id, Some(turn.tool_rounds)).await;

    // ── ⑤ 请求视图（唯一入口 build_request_view）──────────────────────────
    // 在存储视图之上叠加四项**不落库**的裁剪，全部只作用于本次 execute_turn 的
    // 请求包，不回写 context.messages——存储保持完整历史，last_saved 锚点与
    // persist_messages 切片不会错位。
    // 1) 内容节点淡化：B1 保护窗口（末条 + 最近 N 个内容节点）外的超大正文/思考
    //    做 head/tail 摘要（阈值取会话配置 line_threshold / token 上限 2048）；
    // 2) fade：轮次过多时淡化较早的工具结果（存储保留全文，视图每轮重建，天然幂等）；
    // 3) 工具级骨架化：从 CapabilityVisitor 的能力声明（context_retention）动态解析
    //    保留策略，LastOnly/LastN → 更早调用的参数与结果替换为占位文案
    //    （ToolCall↔Tool 配对完整保留，不会造成大模型逻辑断联）；
    // 4) nudge：水位提醒请求级注入（不落库、不占轮次窗口的 User 计数）。
    let retention: HashMap<String, crate::symbio_core::CapabilityToolContextRetention> = tools
        .iter()
        .filter_map(|t| {
            t.context_retention
                .filter(|r| !matches!(r, crate::symbio_core::CapabilityToolContextRetention::All))
                .map(|r| {
                    let short = t.name.rsplit('/').next().unwrap_or(&t.name);
                    (short.to_string(), r)
                })
        })
        .collect();
    let request_view = compression::build_request_view(
        &context.messages,
        req.tool_context_window,
        &retention,
        turn.tool_rounds > context.session.fade_activate_rounds(),
        context.session.fade_keep_recent_turns(),
        context.session.compress_keep_recent(),
        context.session.line_threshold(),
        inject_nudge,
    );

    Ok(TurnInputs {
        root_id,
        system_prompt,
        tools,
        request_view,
    })
}

/// 收口 ③：上下文水位的**唯一响应点**——自动语义压缩与水位提醒只在这里判定与执行。
///
/// ## 顺序不可交换（收口前由主循环体的行序隐含保证，现在由本函数内部保证）
///
/// 先按 70% 阈值执行 L1 自动语义压缩（就地改写 `context.messages`），再按**压缩
/// 之后**的真实水位判 55% 水位提醒。反过来会让"刚压完就提醒"自相矛盾；两个阈值
/// 分处两地时，一次行序调整就能无声破坏该语义，故必须收在同一个函数内、顺序写死。
///
/// ## 与 `context_compact`（模型主动调用）的分工
///
/// 那条路径由模型在任务阶段间隙自行发起，判定点在 `close_turn` 的工具拦截处
/// （`run_context_compact`），**不走本函数**；两条路径共用同一执行内核
/// [`compress_with_snapshot_core`]——"何时压"有两个入口，"怎么压"只有一个实现。
///
/// 返回：是否需要在请求视图末尾注入一次性水位提醒。
pub(crate) async fn apply_compaction(
    orchestrator: &ChatOrchestrator,
    ctx: &Arc<dyn PluginInvokeRequest>,
    context: &mut SessionContext,
    turn: &mut TurnState,
    req: &TurnRequest,
    overhead_tokens: usize,
) -> Result<bool, TurnExit> {
    // ── ① 自动语义压缩（L1：70% 触发）────────────────────────────────────
    if req.auto_compress {
        match auto_compress_process(
            orchestrator,
            context,
            ctx,
            &turn.abort,
            overhead_tokens,
            false,
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
    }

    // ── ② 水位提醒（nudge：55% 触发）─────────────────────────────────────
    // 估算用量 ≥ 55% 有效上限时，在请求视图末尾注入一条一次性系统提示
    // （请求级、不落库，由 build_request_view 统一追加），引导模型在
    // "阶段间隙"主动调用 context_compact（比 70% 硬触发更早、时机更优）。
    // 门控：提醒只为引导工具调用，跟随工具开关（enable_compact_tool），
    // 与自动压缩开关解耦（关自动压缩、开工具压缩时仍需提醒）。
    // 去重：每次用户请求生命周期内最多注入一次（主动压缩成功后重置）；
    // 提醒不落库，无需扫描历史做去重，也不占用轮次窗口的 User 计数。
    if !req.enable_compact_tool || turn.nudged_this_request {
        return Ok(false);
    }
    let effective_limit = orchestrator.context_limit as usize;
    if !compression::should_emit_context_nudge(&context.messages, effective_limit, overhead_tokens)
    {
        return Ok(false);
    }
    turn.nudged_this_request = true;
    plugin_info!(
        "session",
        "[Compress] context nudge emitted (~55% of limit), suggesting context_compact"
    );
    Ok(true)
}

#[cfg(test)]
#[path = "inputs.test.rs"]
mod tests;
