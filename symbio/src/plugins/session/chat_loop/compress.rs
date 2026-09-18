//! **压缩流水线**：自动语义压缩（L2）与主动 `context_compact` 工具共用的实现。
//!
//! 自 `chat_loop.rs` 原样搬移（拆文件不拆行为）。
//!
//! 两条入口（[`auto_compress_process`] / [`run_context_compact`]）共用
//! [`compress_with_snapshot_core`] 这一个执行内核——"何时压"有两个入口，
//! "怎么压"只有一个实现。

use super::*;
use crate::symbio_core::{dir_from_ctx, PLUGIN_SESSION};

/// 被动自动压缩（L1）：阈值判定 → 切分 → 收益护栏 → 交执行内核。
///
/// `force` 为 `compression::should_start_compression` 的公开契约（跳过阈值），
/// 本路径恒为 `false`（自动压缩必须走阈值）；强制压缩属主动路径，由
/// `run_context_compact` 承担，不经此处。
///
/// 无 `extra_hints`：自动压缩没有"用户"在环内提供保留提示——`hints` 是
/// `context_compact` 工具的入参，只在主动路径有意义（见 `compress_with_snapshot_core`）。
pub(crate) async fn auto_compress_process(
    orchestrator: &ChatOrchestrator,
    context: &mut SessionContext,
    channel: &mut PluginChannel,
    ctx: &Arc<dyn InvokeRequest>,
    abort_flag: &Arc<AtomicBool>,
    overhead_tokens: usize,
    force: bool,
) -> Result<Option<usize>, PluginError> {
    let effective_context_limit = orchestrator.context_limit as usize;

    // 请求级固定开销（system prompt + 工具定义）必须计入阈值判断，
    // 否则上下文实际占用被低估，压缩触发过晚 → 撞 provider 的 context-length 400。
    // 口径由调用方（`prepare_turn_inputs`）算好后传入：本函数原先只拿它算这一处，
    // 却因此需要自己再取一次工具清单，与主循环的收集重复。
    let overhead = overhead_tokens;

    if !compression::should_start_compression(
        &context.messages,
        effective_context_limit,
        force,
        overhead,
    ) {
        return Ok(None);
    }

    let (compression_msg, history_to_compress, history_to_keep) =
        match compression::prepare_compression(&context.messages) {
            Some(v) => v,
            None => return Ok(None),
        };

    // 压缩收益护栏（门槛的唯一出处：`compression::has_compaction_payoff`）
    let compress_tokens: usize = history_to_compress
        .iter()
        .map(compression::estimate_message_tokens)
        .sum();
    if !compression::has_compaction_payoff(compress_tokens) {
        return Ok(None);
    }

    // 压缩核心与主动 context_compact 工具共用（compress_with_snapshot_core）：
    // 失败已在核心内就地回滚，这里只区分"成功/未压缩"两种结果。
    let original_count = context.messages.len();
    let post_tokens = compress_with_snapshot_core(
        orchestrator,
        context,
        channel,
        ctx,
        abort_flag,
        compression_msg,
        history_to_keep,
        // 自动路径无用户保留提示（hints 只在主动路径有来源）
        None,
        "auto",
    )
    .await;
    match post_tokens {
        Some(_) => Ok(Some(original_count)),
        None => Ok(None),
    }
}

/// 快照压缩核心 —— 被动 L2 自动压缩（[`auto_compress_process`]）与主动
/// `context_compact` 工具（[`run_context_compact`]）共用的唯一实现。
///
/// 单一实现约束：被动与主动两条入口**必须**共用本函数，各自只保留调用方特有的
/// 切分点与呈现语义。两处各维护一份流水线（PreCompact 钩子 → transcript
/// 转存 → LLM 压缩请求 → 快照校验/纠正重试/降级兜底 → meta 与快照消息构造 →
/// 保留区拼接落库）会带来行为漂移风险。
///
/// 职责：
/// 1. PreCompact 钩子 + 压缩前完整历史 transcript 转存（可回溯原则）；
/// 2. 上下文临时替换为 `[compression_msg]`，以专用压缩提示词发起 LLM 请求；
/// 3. 快照校验：提取 `<state_snapshot>` → 缺失则附纠正指令重试一次 →
///    仍失败 → 保留原历史，回滚并放弃本次压缩；
/// 4. 构造快照消息（meta：compacted/post_tokens/transcript_path/compact_hints/
///    protocol_version/prompt_fingerprint）并与保留区拼接，replace_messages 落库。
///
/// 失败语义：**任何失败都不向上冒泡**（回滚到调用前历史后返回 None），由调用方
/// 决定对外呈现（auto → `Ok(None)` 继续本轮 Turn；manual → 工具结果"压缩失败已回滚"）。
///
/// `keep_messages`：压缩后原样保留在快照之后的消息（auto = 未压缩尾段；
/// manual = 当前用户指令 + 进行中 Turn 及之后，保证 Turn 子树 parent 链完整，
/// 详见 [`run_context_compact`] 文档中的切分点约束）。
/// 成功返回 `Some(post_tokens)`（压缩后内容水位：快照 + 保留区，不含请求级 overhead）。
#[allow(clippy::too_many_arguments)]
async fn compress_snapshot_inner(
    orchestrator: &ChatOrchestrator,
    context: &mut SessionContext,
    channel: &mut PluginChannel,
    ctx: &Arc<dyn InvokeRequest>,
    abort_flag: &Arc<AtomicBool>,
    compression_msg: ChatMessage,
    keep_messages: Vec<ChatMessage>,
    extra_hints: Option<&str>,
    log_tag: &str,
) -> Option<usize> {
    // 保存原始历史：压缩失败时回滚，绝不能让 `[compression_msg]` 残留在上下文里。
    let original_messages = context.messages.clone();
    let _ = fire_hook(&orchestrator.parent, HookEvent::PreCompact, ctx.clone()).await;

    // 可回溯原则：压缩前把完整历史转存为 transcript，路径记入快照 meta。
    // 若跳过此步直接 replace_messages，被压掉的历史在物理层"凭空消失"，
    // 旧存档文件成为孤儿，事后无法审计。
    // 会话存储根 = 本插件自己的目录（装配态由父插件经 `PLUGIN_DIR` 告知）
    let storage_root = dir_from_ctx(&**ctx, PLUGIN_SESSION);
    let transcript_path = save_transcript_archive(
        &original_messages,
        context.session.session_id(),
        storage_root.dir(),
    );

    // ── 输入超限死锁预判（日志实证的恶性循环）──────────────────────────
    // LLM 摘要请求的请求体**就携带完整待压缩历史**——若历史本身已超 Provider
    // 有效输入上限，摘要请求必然 400（"Input token exceed the limit"），
    // 且每轮自动压缩都会重发这条注定失败的巨型请求：压缩永不收敛、每轮开头
    // 多一段漫长的无响应。预判命中时跳过 doomed 请求，直接本地机械兜底
    // （尾部保留 + 说明头，不依赖 LLM），让上下文水位立即回落到可工作区间。
    let pending: Vec<ChatMessage> = {
        let mut v = Vec::with_capacity(1 + keep_messages.len());
        v.push(compression_msg.clone());
        v.extend(keep_messages.iter().cloned());
        v
    };
    let overhead_tokens =
        compression::estimate_request_overhead(&compression::get_compression_prompt(), ctx).await;
    let pending_tokens: usize = pending
        .iter()
        .map(compression::estimate_message_tokens)
        .sum();
    let effective_limit = orchestrator.context_limit as usize;
    if pending_tokens + overhead_tokens > effective_limit {
        plugin_warn!(
            "session",
            "[Compress] {log_tag}: summary request itself exceeds input limit ({} + {} > {}), \
             applying local emergency tail compression instead of a doomed LLM call",
            pending_tokens,
            overhead_tokens,
            effective_limit
        );
        // 机械兜底目标：压到有效上限的一半（给后续对话留出增长空间，
        // 避免刚兜底完又立刻越线）
        let target = effective_limit / 2;
        let (mut new_messages, removed) = compression::emergency_tail_compression(
            &original_messages,
            target,
            transcript_path.as_deref(),
        );
        if removed > 0 {
            // 与 LLM 快照同款的 meta 指纹（协议版本标记 emergency 路径）
            if let Some(head) = new_messages.first_mut() {
                let mut meta = head.meta.clone().unwrap_or_else(|| serde_json::json!({}));
                meta["protocol_version"] =
                    serde_json::json!(super::super::compression::COMPRESSION_PROTOCOL_VERSION);
                head.meta = Some(meta);
            }
            context.messages = new_messages;
            let _ = context
                .session
                .replace_messages(context.messages.clone())
                .await;
            plugin_info!(
                "session",
                "[Compress] {log_tag}: emergency tail compression removed {removed} messages"
            );
            // post_tokens 按兜底后的内容水位返回（迟滞比较的读取侧口径）
            let post_tokens: usize = context
                .messages
                .iter()
                .map(compression::estimate_message_tokens)
                .sum();
            return Some(post_tokens);
        }
        // 兜底也无需截断（理论上不可达：能进压缩说明已越线）——回滚放弃
        context.messages = original_messages;
        return None;
    }

    context.messages = vec![compression_msg];

    // 专用压缩 system 提示词（模板只在本次请求出现，与主对话隔离）
    let compression_prompt = compression::get_compression_prompt();
    let root_id = short_id();
    let summary = match send_compression_request(
        orchestrator,
        &compression_prompt,
        &context.messages,
        &root_id,
        channel,
        abort_flag,
    )
    .await
    {
        Ok(s) => s,
        // **任何失败都不在压缩阶段向上冒泡**（用户中止 / 限流 / 空摘要 /
        // 流中断 / 网关错误等一律优雅降级）：
        //
        // 压缩发生在本轮 Turn 创建之前（调用点的 emit_streaming_start 在其后）。
        // 若在此处向上冒泡，消费循环的 Error 帧分支会调用 persist_failure，
        // 而 collected 中最后一个根级 Turn 是上一轮已成功定稿的 Turn——
        // 其"已落库 Completed 强制回滚 Failed"逻辑会误回滚成功 Turn。
        //
        // 就地回滚到未压缩历史后返回 Ok(None)，让主循环继续创建本轮
        // Turn；真正的 LLM 请求若同样中止 / 限流 / 失败，会以标准链路
        // （在途 Turn → persist_failure）呈现在本轮 Turn 上：
        // - Aborted → 消费循环 ABORTED 分支 → Failed + "用户手动中止了
        //   本次回复"（前端错误条 + 重试入口）；
        // - RateLimited / 其他 → 消费循环非中止分支 → Failed + 错误原因。
        Err(e) => {
            plugin_warn!("session",
                "[Compress] {log_tag} compression failed ({}), falling back to uncompressed context",
                e
            );
            context.messages = original_messages;
            return None;
        }
    };

    // 快照校验：从输出提取 <state_snapshot>；缺失则纠正重试一次；
    // 仍失败则保留原历史，不把未验证原文作为快照落库。
    // 只检查非空是不够的——模型输出散文/scratchpad 泄漏/截断时，
    // 残缺内容会原样成为唯一记忆。
    let mut validated = compression::extract_snapshot(&summary_text(&summary));
    if validated.is_none() {
        // 重试：附纠正指令，要求严格按 XML 结构输出
        let retry_msg = ChatMessage {
            id: short_id(),
            role: Some(MessageRole::User),
            msg_type: Some(MessageType::Text),
            content: Some(MessageContent::Text(
                "Your previous reply did not contain a valid <state_snapshot> XML block. \
                 Reply again with ONLY the <state_snapshot> block, following the requested structure."
                    .to_string(),
            )),
            status: Some(MessageStatus::Completed),
            ..Default::default()
        };
        context.messages.push(retry_msg);
        let retry = send_compression_request(
            orchestrator,
            &compression_prompt,
            &context.messages,
            &short_id(),
            channel,
            abort_flag,
        )
        .await;
        if let Ok(s) = retry {
            if let Some(snapshot) = compression::extract_snapshot(&summary_text(&s)) {
                validated = Some(snapshot);
            }
        }
    }
    let Some(snapshot_text) = validated else {
        // 两次均未得到有效快照：保留原历史，禁止把未验证散文升级为唯一记忆。
        plugin_warn!(
            "session",
            "[Compress] {log_tag}: invalid snapshot after retry; preserving history"
        );
        context.messages = original_messages;
        return None;
    };
    context.messages.clear();

    // 落库前渲染为纯文本分节（历史中不残留 XML 标签，切断格式模仿链）
    let snapshot_display = compression::render_snapshot_for_history(&snapshot_text);
    // 快照消息：meta 记录压缩标记、压缩后估算（迟滞依据）、转存路径。
    // post_tokens 口径 = 压缩完成后的内容水位（快照 + 保留区内容，不含请求级
    // overhead），与 should_start_compression 迟滞比较的读取侧对齐。若只算快照、
    // 漏掉保留区，迟滞地板被低估 → 压缩后很快再次越线 → 循环压缩。
    let post_tokens = compression::estimate_message_tokens(&ChatMessage {
        content: Some(MessageContent::Text(snapshot_display.clone())),
        ..Default::default()
    }) + keep_messages
        .iter()
        .map(compression::estimate_message_tokens)
        .sum::<usize>();
    let mut meta = serde_json::json!({
        "compacted": true,
        "post_tokens": post_tokens,
    });
    if let Some(p) = &transcript_path {
        meta["transcript_path"] = serde_json::json!(p);
    }
    if let Some(hints) = extra_hints {
        if !hints.trim().is_empty() {
            meta["compact_hints"] = serde_json::json!(hints);
        }
    }

    // 快照指纹 —— 记录压缩协议版本与提示词指纹，
    // 使"提示词强化是否生效"可从产物侧（快照 meta）验证。
    meta["protocol_version"] =
        serde_json::json!(super::super::compression::COMPRESSION_PROTOCOL_VERSION);
    meta["prompt_fingerprint"] =
        serde_json::json!(super::super::compression::compression_prompt_fingerprint());

    let snapshot_message = ChatMessage {
        id: root_id.to_string(),
        // 快照作为压缩后的首条消息，必须是 user 角色（多数 provider 要求对话以 user 开头）
        role: Some(MessageRole::User),
        msg_type: Some(MessageType::Text),
        content: Some(MessageContent::Text(format!(
            "[CONTEXT SNAPSHOT — 压缩的历史记忆，基于它继续任务]\n{snapshot_display}"
        ))),
        status: Some(MessageStatus::Completed),
        meta: Some(meta),
        ..Default::default()
    };

    let mut new_messages = vec![snapshot_message];
    new_messages.extend(keep_messages);
    context.messages = new_messages;

    let _ = context
        .session
        .replace_messages(context.messages.clone())
        .await;

    Some(post_tokens)
}

/// 压缩内核的**包装层**：只负责「正在压缩」这个会话阶段的置位与清位。
///
/// ## 为什么清位必须收在这一层
///
/// 内核有**六个**出口（成功 / LLM 失败回滚 / 快照校验失败 / 输入超限紧急兜底 /
/// 兜底亦无收益 / 用户中止），漏掉任何一个，会话就会在回到空闲后仍挂着
/// "正在压缩上下文"——而且没有任何机制会纠正它（`is_working` 已经归位，
/// 后续也不会再有压缩相关的变更）。
///
/// 置位/清位散落到每个 `return` 前的写法，每加一个出口就要记得补一次，
/// 正是最容易静默失效的一类结构；收成包装层之后，新增出口自动被覆盖。
#[allow(clippy::too_many_arguments)]
async fn compress_with_snapshot_core(
    orchestrator: &ChatOrchestrator,
    context: &mut SessionContext,
    channel: &mut PluginChannel,
    ctx: &Arc<dyn InvokeRequest>,
    abort_flag: &Arc<AtomicBool>,
    compression_msg: ChatMessage,
    keep_messages: Vec<ChatMessage>,
    extra_hints: Option<&str>,
    log_tag: &str,
) -> Option<usize> {
    if let Some(e) = &orchestrator.phase {
        e.set(Some(crate::plugins::session::plugin::PHASE_COMPRESSING))
            .await;
    }
    let result = compress_snapshot_inner(
        orchestrator,
        context,
        channel,
        ctx,
        abort_flag,
        compression_msg,
        keep_messages,
        extra_hints,
        log_tag,
    )
    .await;
    if let Some(e) = &orchestrator.phase {
        e.clear().await;
    }
    result
}

/// 取消息纯文本（快照校验用）
fn summary_text(m: &ChatMessage) -> String {
    m.content.as_ref().map(|c| c.to_text()).unwrap_or_default()
}

/// 主动压缩工具（context_compact）执行体。
///
/// 关键正确性约束：**进行中的 Turn 必须从其用户指令起整体保留**。
/// 调用时本 Turn 的 ToolCall 消息已在上下文中（请求后 extend），但其工具结果
/// 子节点尚未产生。切分点必须落在当前用户指令（Turn 根之前）：
/// - 保留区 = [用户指令, Turn, 子节点...]，Turn 子树 parent 链完整；
/// - 若切在 Turn 根与 ToolCall 之间：Turn 根入快照、子 ToolCall 留保留区 →
///   parent 悬空 → 孤儿消息被 drop_orphan_messages 剔除 → 请求包里 Tool 结果
///   失去父节点 → provider 400。
///
/// `split_user_idx`：主循环传入的切分下标（当前用户指令，见 find_turn_user_split_idx）。
/// 返回 `(是否执行了压缩, 估算压缩前 tokens, 估算压缩后 tokens)`。
pub(crate) async fn run_context_compact(
    orchestrator: &ChatOrchestrator,
    context: &mut SessionContext,
    channel: &mut PluginChannel,
    ctx: &Arc<dyn InvokeRequest>,
    abort_flag: &Arc<AtomicBool>,
    split_user_idx: usize,
    hints: Option<&str>,
) -> (bool, usize, usize) {
    if split_user_idx == 0 {
        return (false, 0, 0);
    }

    // 待压缩历史 = [.., split_user_idx)，当前用户指令起的任务上下文整体留在保留区
    let history: Vec<ChatMessage> = context.messages[..split_user_idx].to_vec();
    let before_tokens: usize = history
        .iter()
        .map(compression::estimate_message_tokens)
        .sum();
    // 压缩收益护栏（门槛的唯一出处：`compression::has_compaction_payoff`）
    if !compression::has_compaction_payoff(before_tokens) {
        return (false, before_tokens, before_tokens);
    }

    let keep_messages: Vec<ChatMessage> = context.messages[split_user_idx..].to_vec();
    let compression_msg = compression::build_compression_request(&history, hints);

    // 压缩流水线与被动自动压缩共用同一核心（transcript 转存 → LLM 压缩请求 →
    // 快照校验/纠正重试/降级兜底 → meta 构造 → 保留区拼接落库）；失败已在核心内
    // 就地回滚，这里只把它翻译为工具结果的 (compressed, before, after) 三元组。
    let post_tokens = compress_with_snapshot_core(
        orchestrator,
        context,
        channel,
        ctx,
        abort_flag,
        compression_msg,
        keep_messages,
        hints,
        "manual",
    )
    .await;

    match post_tokens {
        Some(after) => (true, before_tokens, after),
        // 未执行 / 失败：压缩放弃（历史已回滚），工具结果如实反映无收益
        None => (false, before_tokens, before_tokens),
    }
}

/// 压缩前把完整历史转存为 JSON transcript（best-effort）。
/// 落在会话存储目录内（`<会话存储根>/<id>/transcripts/`，跟随会话生命周期），
/// 而非系统临时目录（临时目录无 GC、跨会话堆积、脱离会话管理）。
/// 路径派生统一走 paths 模块（safe_id / 会话根目录的唯一权威实现）。
fn save_transcript_archive(
    messages: &[ChatMessage],
    session_id: &str,
    root: &std::path::Path,
) -> Option<String> {
    let root = super::super::paths::session_subdir(
        root,
        session_id,
        super::super::paths::TRANSCRIPTS_SUBDIR,
    );
    std::fs::create_dir_all(&root).ok()?;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let n = messages.len();
    let path = root.join(format!("transcript_{ts}_{n}.json"));
    let body = serde_json::to_string_pretty(messages).ok()?;
    std::fs::write(&path, body).ok()?;
    path.to_str().map(|s| s.to_string())
}

async fn send_compression_request(
    orchestrator: &ChatOrchestrator,
    system_prompt: &str,
    messages: &[ChatMessage],
    root_id: &str,
    channel: &mut PluginChannel,
    abort_flag: &Arc<AtomicBool>,
) -> Result<ChatMessage, PluginError> {
    // 压缩是**内部 LLM 请求**，不是对话轮次：其流式帧（Turn 起始 / 思考 / 正文 delta）
    // 绝不能进入对话流——否则前端会多出一个永远停在"正在思考…"的空 Turn（压缩请求
    // 从不 finalize，快照也只落库不广播），且随每次自动压缩/主动压缩逐个累积。
    // 长会话才会触发压缩，因此该泄漏只在长任务后复现，极易误判为渲染层问题。
    //
    // 通道隔离的不对称设计：
    // - **tx（出帧）完全静默**：哑 sender + drain task，压缩 delta 一律丢弃（编译期
    //   不可泄漏——本函数内所有 emit 都走 muted.tx）；
    // - **rx（入帧）临时移交真实主通道**：用户停止时 Abort 帧只会进入主通道队列，
    //   而消费循环此刻正 await 在压缩请求上——若 rx 也是哑的，Abort 永远收不到，
    //   压缩请求将无视中止跑完整整轮 LLM 流；哑 rx 也不能立即关闭，否则会被
    //   误判 Aborted，导致每轮重试巨型压缩请求。压缩结束后 rx 归还主通道。
    let (mute_tx, mut mute_rx) = tokio::sync::mpsc::channel::<PluginFrame>(64);
    tokio::spawn(async move { while mute_rx.recv().await.is_some() {} });
    let dummy_rx = tokio::sync::mpsc::channel::<PluginFrame>(1).1;
    // 出栈时通过 mem::replace 归还真实 rx（下方统一在请求结束后归还）
    let real_rx = std::mem::replace(&mut channel.rx, dummy_rx);
    let mut muted = PluginChannel {
        tx: mute_tx,
        rx: real_rx,
        cancel_token: tokio_util::sync::CancellationToken::new(),
    };

    // 压缩请求窗口日志：此窗口内出帧静默、消费循环收不到任何流式帧，
    // 若无日志，长压缩请求表现为"整段时间无任何输出"（用户视角的卡死）。
    // 压缩走完整 LLM 流，长上下文时可能耗时数分钟，必须显式标注开始/结束。
    let compression_started = std::time::Instant::now();
    crate::plugin_info!(
        "session",
        "[Compress] 压缩 LLM 请求开始：root_id={root_id}, 历史消息数={}，约 {} 字符（此窗口内前端无流式输出属正常）",
        messages.len(),
        messages.iter().map(|m| m.content.as_ref().map(|c| c.to_text().len()).unwrap_or(0)).sum::<usize>()
    );

    let result = run_compression_llm(
        orchestrator,
        system_prompt,
        messages,
        root_id,
        &mut muted,
        abort_flag,
    )
    .await;

    match &result {
        Ok(msg) => {
            let text_len = msg.content.as_ref().map(|c| c.to_text().len()).unwrap_or(0);
            crate::plugin_info!(
                "session",
                "[Compress] 压缩 LLM 请求完成：摘要 {} 字符，耗时 {}s",
                text_len,
                compression_started.elapsed().as_secs()
            );
        }
        Err(e) => {
            crate::plugin_error!(
                "session",
                "[Compress] 压缩 LLM 请求失败（耗时 {}s）：{e}",
                compression_started.elapsed().as_secs()
            );
        }
    }

    // 无论成败，立即把真实 rx 归还主通道（Abort 帧的消费权交还消费循环）
    let dummy_rx = tokio::sync::mpsc::channel::<PluginFrame>(1).1;
    channel.rx = std::mem::replace(&mut muted.rx, dummy_rx);

    result
}

/// 压缩摘要的实际 LLM 调用：出帧全部静默（muted.tx），入帧收真实主通道 Abort。
///
/// 注意：这里**绝不发射 Turn 帧**（不发 emit_streaming_start）。压缩是内部请求、
/// 不是对话轮次——Turn 帧在哑通道上是纯死代码；若误走主通道则会在前端留下永远
/// "正在思考…"的空 Turn 骨架（每轮压缩尝试累积一个）。不设 Turn 帧调用点，
/// 使"内部请求泄漏可见帧"这一类问题在结构上不可能发生。
async fn run_compression_llm(
    orchestrator: &ChatOrchestrator,
    system_prompt: &str,
    messages: &[ChatMessage],
    root_id: &str,
    muted: &mut PluginChannel,
    abort_flag: &Arc<AtomicBool>,
) -> Result<ChatMessage, PluginError> {
    use crate::symbio_core::schemas::session::chat_message::MessageContent;

    // 压缩路径与对话轮次共用同一模型契约：provider.execute_turn（tools 为空）。
    // 出帧仍全部静默（muted.tx），入帧收真实主通道 Abort；
    // 协议细节（请求构造 / 重试 / SSE 解析）由 model 插件实现承担。
    let out = orchestrator
        .provider
        .execute_turn(system_prompt, messages, &[], root_id, muted, abort_flag)
        .await?;

    if abort_flag.load(Ordering::SeqCst) {
        return Err(PluginError::Aborted);
    }

    let effective = out.effective_text(0).to_owned();
    if effective.is_empty() {
        return Err(PluginError::InternalError(
            // 语义说明：压缩摘要请求的 SSE 流正常结束，但未产出任何文本/推理内容
            // （常见于上游网关错误被吞掉、或模型只回了空流）。调用方
            // auto_compress_process 对这类失败优雅降级，不中断整个 turn。
            "Compression summary request returned no content".to_string(),
        ));
    }

    Ok(ChatMessage {
        id: root_id.to_string(),
        role: Some(crate::symbio_core::schemas::session::chat_message::MessageRole::Assistant),
        msg_type: Some(MessageType::Text),
        content: Some(MessageContent::Text(effective)),
        status: Some(MessageStatus::Completed),
        ..Default::default()
    })
}
