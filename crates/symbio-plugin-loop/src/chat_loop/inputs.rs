//! 一次 LLM 请求的**输入准备**：环前的纯读、轮首那几行，与它们的执行体。
//!
//! - [`resolve_system_prompt`]：系统提示词唯一真源
//! - [`collect_reads`]：提示词 + 工具 + 开销——环前的纯读，不含任何选债口
//! - [`prepare_request_sections`]：水位、根骨架上线路与请求视图——接缝动作在
//!   轮首行之后调它，任何请求之前
//! - [`run_recall`]：`policy.recall` 行的执行体（记忆 / 技能 / 调度段三段读视图）
//! - [`run_compress`]：`policy.compress` 行的执行体（上下文水位的响应，产线住在
//!   这里，**被选中才跑**）

use symbio_kernel::capability::CapabilityMeta;
use symbio_plugin_api::context::{Cadence, ContextSection, DelegateVerdict, LiveInputs};

use super::*;

use crate::session_port;

/// 本轮用户发言 = 轮首入列的最后一条 user 消息的正文。
///
/// **取值点与 `turn_exec` 的 `user_text` 同源**（该处晚于 `prepare_request_sections`，
/// 但用户消息在步骤 3 之前已入列，两次取到的是同一条）。混合打分
/// 的语义维由它驱动；取不到 ⇒ `None` = 「本轮没有问题」，投影退化为纯 ts 降序
/// （平凡值，J2）。用类型区分「没有问题」与「问题恰好是空串」：前者是事实，
/// 后者得写出来才成立。
pub(crate) fn last_user_text(context: &SessionContext) -> Option<String> {
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

/// 本轮读侧值包：两种节奏（发言级 / 请求级）两次收集共用同一构造口径。
///
/// 两种节奏是**同一请求的两半**，必须看到同一刻的值——各拼一份就会漂移成
/// 「召回读的是 A、委派读的是 B」。`live` 只在请求级收集时为 `Some`。
///
/// 这里没有轮号，也没有容量：端口只交值（DE-026 D-252）。段与段的先后由 owner 序
/// 决定，容量与开关由各自读那条已经落格的事实。
fn context_request<'a>(
    turn: &TurnState,
    principal: &'a str,
    query: Option<&'a str>,
    facts: &'a [symbio_kernel::event::Event],
    recall: Option<&'a symbio_plugin_api::RecallFacts<'a>>,
    live: Option<&'a LiveInputs<'a>>,
) -> symbio_plugin_api::ContextRequest<'a> {
    symbio_plugin_api::ContextRequest {
        request_round: u32::try_from(turn.round()).unwrap_or(u32::MAX),
        principal,
        query,
        facts,
        recall,
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

    if let Some(visitor) = ctx.get(symbio_sdk::capability::CAPABILITY_VISITOR) {
        for (_, text) in symbio_sdk::capability::capability_prompts(&*visitor).await {
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
) -> TurnReads {
    // ── ① 系统提示词 + 工具：同一函数、同一时刻、同一 Registry ──────
    let system_prompt = resolve_system_prompt(req.system_prompt.as_deref(), ctx).await;

    let tools = symbio_sdk::capability::tool_metas(ctx).await;

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
/// 4. **请求视图重建**（`build_request_view` 唯一入口）：段先按预算裁剪（D-243），
///    再拼成请求视图。
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
    req: &super::state::TurnRequest,
    root_id: &str,
    fresh_root: bool,
    system_prompt: &str,
    tools: &[CapabilityMeta],
) -> Result<Vec<ChatNode>, super::super::turn_exec::ActorError> {
    // 请求级固定开销（system prompt + 工具清单）：水位与预算共用同一把尺，
    // 就地算一次——不再寄存进 `TurnState`（`D-306` 裁决四：预算/租约的入格归行）。
    let overhead = context::estimate_overhead_with_tools(system_prompt, tools);
    // ── ③″ 取本轮的上下文水位（压缩**之后**：量的是这一轮真正发出去的那一体）──
    // 只在这里算一次，随 ④ 的骨架挂进 `meta.context`；判据是「一个请求级事实挂在
    // 它所属的那一轮上」，不是「前端要轮询」。取不到也不算异常：没有水位
    // ⇒ 前端不渲染水位条，而不是补一个 0。
    let watermark = context::context_watermark(
        &context.messages,
        overhead,
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
            Some(turn.round()),
            Some(watermark),
            &orchestrator.view.principal,
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
    // Q1 判定仍在本函数（控制面）：`turn.user_text()` 从本轮用户格（`user_seq`）
    // 读回正文，同一请求内不随工具轮变化；判定读的是**本会话当前配置**
    // （`worker_*` 三字段，出厂值真源在 core）。provider 只接到判完的
    // `DelegateVerdict`——判断闭集（D-208）。
    //
    // Q3：worker 进展现读磁盘（无子会话 ⇒ 零额外 IO）。整条读链住在
    // `session::worker_progress`（D-081 裁决一第 4 条），驱动器只调这一个入口。
    // 读失败向上传播——静默降级就是「没有观测 = 静默失效」：看不出是没 worker 还是没读到。
    let workers = session_port::call::<Vec<symbio_sdk::turn::facts::WorkerProgress>>(
        context.session.as_ref(),
        symbio_sdk::turn::dispatch::run_ops::WORKER_PROGRESS,
        (),
    )
    .await
    .map_err(super::super::turn_exec::ActorError::Read)?;
    let settings = &context.view.settings;
    let delegate_verdict = match turn.user_text().filter(|_| req.new_input) {
        Some(text) => {
            // 关键词存**逗号串**（面板无数组控件），切开即用；空串由
            // `worker_start_reason` 的「空关键词跳过」处理 ⇒ 空串 = 空集 = 本层关闭。
            let keywords: Vec<&str> = settings.worker_keywords.split(',').collect();
            match delegate::worker_start_reason(
                text.as_str(),
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

    // 段的**次序**在这里声明（= 模型读到的上下文次序）：发言级段在前、请求级段在后。
    // 加一段 provider 只需提交一条实现，本数组不动。
    let facts = context.facts_snapshot();
    let recall = recall_from_stage(&facts, turn);
    let live_lines: Vec<symbio_plugin_api::RecallLine<'_>> = recall
        .as_ref()
        .map(|view| {
            view.entries
                .iter()
                .map(|e| symbio_plugin_api::RecallLine {
                    seq: e.seq,
                    content: &e.content,
                    tag: &e.tag,
                })
                .collect()
        })
        .unwrap_or_default();
    let live_recall = recall.as_ref().map(|view| symbio_plugin_api::RecallFacts {
        degraded: view.degraded,
        lines: &live_lines,
    });
    let query = last_user_text(context);
    let principal = symbio_sdk::authz::request_principal(ctx.as_ref());
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
        Some(&live),
    );
    // ── ⑥″ 段预算：放不下就按优先级裁，读数自己出声（D-243）────────────────
    //
    // 预算与水位**同一把尺**（[`context::section_budget`]），成本也由同一把尺在
    // `fit_sections` 里回填——provider 侧不声明成本（那会变成第二套计量），只声明
    // 优先级。裁剪规则只认段自己声明的 `prio` 与收集次序，宿主不按插件名 / 段名特判：
    // 「谁先走」是声明方的判断，宿主只执行。
    let mut sections: Vec<ContextSection> =
        symbio_plugin_api::context_sections(&request, Cadence::Utterance);
    sections.extend(symbio_plugin_api::context_sections(
        &request,
        Cadence::Request,
    ));
    let budget = context::section_budget(
        &context.messages,
        overhead,
        orchestrator.context_limit as usize,
    );
    let (kept, fit) = {
        use symbio_plugin_api::text::{default_tokenizer, Tokenizer};
        let tok = default_tokenizer();
        symbio_plugin_api::context::fit_sections(sections, budget, |text| tok.count(text))
    };
    if fit.trimmed() {
        // 只在真的裁了才出声：各段皆空是常态，逐轮打印会把日志淹掉。
        symbio_plugin_api::plugin_info!(
            "session",
            "[section-budget] 预算 {} token、需求 {}、保留 {}；裁掉 {} 段（{}）；超界 {}",
            fit.budget,
            fit.demanded,
            fit.kept_cost,
            fit.dropped.len(),
            fit.dropped
                .iter()
                .map(|(slot, cost)| format!("{slot}={cost}"))
                .collect::<Vec<_>>()
                .join(","),
            fit.over()
        );
    }
    let sections: Vec<(&'static str, String)> =
        kept.into_iter().map(|s| (s.slot, s.text)).collect();
    Ok(context::build_request_sections(&sections))
}

/// 从 `policy.recall` 落下的召回格重建检索读数。
///
/// 宿主不再把读数寄存在 `TurnState`（DE-048 D-306）：这一格是事实，读侧与写侧
/// 都回到日志。逐条正文随格入载荷，重建不依赖再次检索。
fn recall_from_stage(
    facts: &[symbio_kernel::event::Event],
    turn: &TurnState,
) -> Option<symbio_plugin_api::retrieval::Retrieval> {
    let record = super::super::stage_events::recall_of(facts, turn.turn_no, turn.drive_seq)?;
    Some(symbio_plugin_api::retrieval::Retrieval {
        degraded: record.degraded,
        entries: record
            .entries
            .into_iter()
            .map(
                |(seq, tag, content)| symbio_plugin_api::retrieval::Retrieved { seq, content, tag },
            )
            .collect(),
    })
}

#[cfg(test)]
#[path = "inputs.test.rs"]
mod tests;
