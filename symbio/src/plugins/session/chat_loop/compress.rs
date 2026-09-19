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
/// `force` 为 `compression::should_start_compression` 的公开契约（跳过阈值）。
/// 自动路径恒为 `false`（自动压缩必须走阈值）；`true` 只由**用户主动重试**
/// （`retry_compaction`）传入——此时同时**绕过熔断**：熔断约束的是"每轮自动白等
/// 一次注定失败的请求"，用户点重试是他的明确意愿，不该被冷却挡住。
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
) -> Result<Option<usize>, CompressionFailure> {
    let effective_context_limit = orchestrator.context_limit as usize;

    // 请求级固定开销（system prompt + 工具定义）必须计入阈值判断，
    // 否则上下文实际占用被低估，压缩触发过晚 → 撞 provider 的 context-length 400。
    // 口径由调用方（`prepare_turn_inputs`）算好后传入：本函数原先只拿它算这一处，
    // 却因此需要自己再取一次工具清单，与主循环的收集重复。
    let overhead = overhead_tokens;

    // 自动压缩熔断：连续失败达到阈值后，跳过本次自动压缩——历史已回滚、本轮仍
    // 能正常回复，不必每轮再白等一次注定失败的 LLM 请求（实测会话 `09d74431` 的
    // 反复失败就是这个形态）。手动 `context_compact` 与用户主动重试（`force`）
    // 都不受影响，且任一次压缩成功都会清零计数（半开冷却期内放行一次重试，
    // 让瞬时错误自愈）。
    if !force {
        if let Some(em) = &orchestrator.compression {
            if em.state.compression_should_skip().await {
                plugin_warn!(
                    "session",
                    "自动压缩熔断：连续失败已达阈值，本次跳过（冷却中），历史保持完整"
                );
                return Ok(None);
            }
        }
    }

    if !compression::should_start_compression(
        &context.messages,
        effective_context_limit,
        force,
        overhead,
    ) {
        return Ok(None);
    }

    let (compression_request, history_to_compress, history_to_keep) =
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
        compression_request,
        history_to_keep,
        // 自动路径无用户保留提示（hints 只在主动路径有来源）
        None,
        "auto",
    )
    .await;
    match post_tokens {
        Ok(Some(_)) => {
            if let Some(em) = &orchestrator.compression {
                em.state.compression_record_success().await;
            }
            Ok(Some(original_count))
        }
        Ok(None) => Ok(None),
        Err(f) => {
            if let Some(em) = &orchestrator.compression {
                em.state.compression_record_failure().await;
            }
            Err(f)
        }
    }
}

/// 快照压缩核心 —— 被动 L2 自动压缩（[`auto_compress_process`]）与主动
/// `context_compact` 工具（[`run_context_compact`]）共用的唯一实现。
///
/// 单一实现约束：被动与主动两条入口**必须**共用本函数，各自只保留调用方特有的
/// 切分点与呈现语义。两处各维护一份流水线（PreCompact 钩子 → transcript
/// 转存 → LLM 压缩请求 → 快照校验/纠正重试 → meta 与快照消息构造 →
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
/// 失败语义：**任何失败都不向上冒泡**（回滚到调用前历史后返回 Err），由调用方
/// 决定对外呈现（auto → `Ok(None)` 继续本轮 Turn；manual → 工具结果"压缩失败已回滚"）。
///
/// **失败一律不改动历史**：四个出口（LLM 失败 / 快照校验失败 / 输入超限 / 用户中止）
/// 全部把 `context.messages` 还原为调用前的 `original_messages`。历史是用户的资产，
/// 不因为"这次压缩没做成"而被丢弃——见 [`CompressionFailure`] 关于 `InputOverLimit`
/// 的说明（那里曾是唯一的例外，现已移除）。
///
/// `keep_messages`：压缩后原样保留在快照之后的消息（auto = 未压缩尾段；
/// manual = 当前用户指令 + 进行中 Turn 及之后，保证 Turn 子树 parent 链完整，
/// 详见 [`run_context_compact`] 文档中的切分点约束）。
/// 成功返回 `Some(post_tokens)`（压缩后内容水位：快照 + 保留区，不含请求级 overhead）。
/// 压缩为什么没完成。
///
/// ## 为什么需要它
///
/// 此前所有失败出口统一返回 `None`，调用方只拿到"失败了"，节点写死一句
/// "压缩未完成，已保留完整历史"，`error` 与 `meta` 全空。实测会话
/// `09d74431` 的两条 failed 压缩节点就是这副样子——**无法区分**是 LLM 请求
/// 失败、模型没输出合法快照、还是待压缩历史超出模型输入上限。用户看到的是同一个
/// 结果，但三者的应对完全不同（重试 / 换模型 / 手动清理），不可诊断就无法处理。
///
/// 它同时给出两个视图：`message()` 面向用户（进节点 content），
/// `kind()` 机器可读（进 `meta.failure_kind`，与未执行工具节点同一字段约定）。
pub(crate) enum CompressionFailure {
    /// LLM 请求本身失败（网络 / 限流 / provider 报错）。
    /// 携带 provider 的错误文本——它是唯一能区分"限流"与"参数错误"的东西。
    Llm(String),
    /// 两次尝试都未取得合法 `<state_snapshot>`（模型输出散文 / scratchpad
    /// 泄漏 / 被截断）。
    InvalidSnapshot,
    /// **摘要请求自身**就超出 Provider 有效输入上限（`pending + overhead > limit`）。
    ///
    /// 这条路径**不裁剪历史**。曾经的写法是"本地机械兜底截断早期消息"并**回报成功**
    /// （压缩节点显示"已压缩上下文（N → M 条）"），于是历史凭空变短、后续内容与
    /// 截断前失联，而用户拿不到任何原因，也没有重试入口——这正是要否决的行为。
    /// 请求体与上下文同源超限只说明"这次压缩放不下"，不构成"可以静默丢历史"的授权。
    InputOverLimit { pending: usize, limit: usize },
}

impl std::fmt::Display for CompressionFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

impl CompressionFailure {
    /// 机器可读的原因码（节点 `meta.failure_kind`）
    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Self::Llm(_) => "llm_error",
            Self::InvalidSnapshot => "invalid_snapshot",
            Self::InputOverLimit { .. } => "input_over_limit",
        }
    }

    /// 面向用户的短说明（节点 content）
    ///
    /// 三种失败都明示「已保留完整历史」——这是失败路径的**硬不变式**：压缩失败
    /// 不改变历史，一条消息都不丢。
    pub(crate) fn message(&self) -> String {
        match self {
            Self::Llm(e) => format!("压缩未完成（模型请求失败：{e}），已保留完整历史"),
            Self::InvalidSnapshot => {
                "压缩未完成（模型未按要求输出摘要），已保留完整历史".to_string()
            }
            Self::InputOverLimit { pending, limit } => format!(
                "压缩未完成（待压缩历史约 {pending} tokens，已超出模型输入上限 {limit}），\
                 已保留完整历史。请改用上下文更大的模型后重试，或手动精简会话。"
            ),
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn compress_snapshot_inner(
    orchestrator: &ChatOrchestrator,
    context: &mut SessionContext,
    channel: &mut PluginChannel,
    ctx: &Arc<dyn InvokeRequest>,
    abort_flag: &Arc<AtomicBool>,
    compression_request: Vec<ChatMessage>,
    keep_messages: Vec<ChatMessage>,
    extra_hints: Option<&str>,
    log_tag: &str,
) -> Result<Option<usize>, CompressionFailure> {
    // 保存原始历史：压缩失败时回滚，绝不能让压缩请求残留在上下文里。
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

    // ── 输入超限预判（跳过注定失败的巨型请求，**但不裁剪历史**）──────────
    // LLM 摘要请求的请求体**就携带完整待压缩历史**——若历史本身已超 Provider
    // 有效输入上限，摘要请求必然 400（"Input token exceed the limit"），
    // 且每轮自动压缩都会重发这条注定失败的巨型请求：压缩永不收敛、每轮开头
    // 多一段漫长的无响应。预判命中时**跳过这次 doomed 请求**（这是预判的唯一
    // 收益），然后如实报错。
    //
    // 此前这里走的是"本地机械兜底截断"（尾部保留 + 说明头，不依赖 LLM）并
    // **回报成功**：压缩节点显示"已压缩上下文（N → M 条）"，用户看到的是历史
    // 突然变短、后续内容与截断前失联，却拿不到原因、也没有重试入口。历史是
    // 用户的资产，不能因为"压缩放不下"就被静默丢掉——只报错、不动历史，
    // 由用户决定下一步（换更大上下文的模型后重试 / 手动精简会话）。
    // 请求体的实际内容 = 压缩 system 提示词 + `compression_request`（历史对话 + 末尾指令）。
    //
    // **不含 `keep_messages`**：保留区根本不发往模型，把它算进来只会高估请求体，
    // 让本可成功的 LLM 摘要被误判成"注定超限"。
    let overhead_tokens =
        compression::estimate_request_overhead(&compression::get_compression_prompt(), ctx).await;
    let pending_tokens: usize = compression_request
        .iter()
        .map(compression::estimate_message_tokens)
        .sum();
    let effective_limit = orchestrator.context_limit as usize;
    if pending_tokens + overhead_tokens > effective_limit {
        plugin_warn!(
            "session",
            "[Compress] {log_tag}: summary request itself exceeds input limit ({} + {} > {}); \
             skipping the doomed LLM call and preserving history intact",
            pending_tokens,
            overhead_tokens,
            effective_limit
        );
        // 历史一条不动（此时 `context.messages` 尚未被替换为压缩请求，赋值只为把
        // "失败即原样"这条不变式写在代码里，而非依赖上面的时序）
        context.messages = original_messages;
        return Err(CompressionFailure::InputOverLimit {
            pending: pending_tokens + overhead_tokens,
            limit: effective_limit,
        });
    }

    // 压缩请求的**全部内容**：待压缩历史（正常对话形态）+ 末尾压缩指令。
    // provider 的 `flatten_chat_messages` 会把它投影成模型该看的对话，
    // 而不是把存储字段原样倒给模型（见 `build_compression_request` 的说明）。
    context.messages = compression_request;

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
            return Err(CompressionFailure::Llm(e.to_string()));
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
        return Err(CompressionFailure::InvalidSnapshot);
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

    // 被压掉的那一段（`original_messages` 的前缀）与快照的**槽位序号**。
    // 槽位号 = 保留区首条 seq − 1 ⇒ 新列表天然单调，`assign_seq` 只补缺号，
    // 保留区任何一条消息的 seq 都不会被改写（"压缩不改变既有序号"）。
    let compressed = {
        let cut = original_messages
            .len()
            .saturating_sub(keep_messages.len())
            .min(original_messages.len());
        &original_messages[..cut]
    };
    let snapshot_seq = compression::snapshot_slot_seq(compressed, &keep_messages);

    let snapshot_message = ChatMessage {
        id: root_id.to_string(),
        // 快照作为压缩后的首条消息，必须是 user 角色（多数 provider 要求对话以 user 开头）
        role: Some(MessageRole::User),
        msg_type: Some(MessageType::Text),
        content: Some(MessageContent::Text(format!(
            "[CONTEXT SNAPSHOT — 压缩的历史记忆，基于它继续任务]\n{snapshot_display}"
        ))),
        status: Some(MessageStatus::Completed),
        // 顺序锚点：接替被压缩内容的槽位（**不是**新号，否则历史记忆会被排到末尾）
        seq: snapshot_seq,
        meta: Some(meta),
        ..Default::default()
    };

    let kept_ids: std::collections::HashSet<&str> =
        keep_messages.iter().map(|m| m.id.as_str()).collect();
    let dropped: Vec<String> = original_messages
        .iter()
        .filter(|m| !kept_ids.contains(m.id.as_str()))
        .map(|m| m.id.clone())
        .collect();
    let head = snapshot_message.clone();

    let mut new_messages = vec![snapshot_message];
    new_messages.extend(keep_messages);
    context.messages = new_messages;

    let _ = context
        .session
        .replace_messages(context.messages.clone())
        .await;
    // 整表重写 → 广播（否则前端转写停在压缩前：被压掉的消息不消失、快照不出现）
    emit_transcript_rewrite(orchestrator, context, &dropped, &head).await;

    Ok(Some(post_tokens))
}

/// 整表重写后的**消费者收敛**：逐条 `deleted`（已消失的消息）+ `created`（新首条）。
///
/// ## 为什么必须有这一步
///
/// 压缩走的是 store 层的 `replace_messages`（`vdfs_provider` 的三条 VDFS 写路由
/// 在 `replace_messages` 之后都要发变更，压缩这条路径原先漏了）。VDFS 变更流是
/// 前端转写的**唯一**入口，漏发即前端永久停留在压缩前的列表上：
/// 被压掉的历史仍在、快照不出现、被改号的消息仍持旧序号——而**没有任何机制会纠正**，
/// 因为两条链路互不校验（与 `plugin.rs` 消息级变更那三条路由同一个道理）。
///
/// 只发 VDFS 变更、不发前端帧：消息通道已归 VDFS 一处（见 `docs/node-state-streaming.md`），
/// 与 [`super::super::plugin::SessionPlugin::emit_message_updated`] 同款。
async fn emit_transcript_rewrite(
    orchestrator: &ChatOrchestrator,
    context: &SessionContext,
    dropped: &[String],
    head: &ChatMessage,
) {
    let Some(emitter) = &orchestrator.compression else {
        // 无发射器 = 无前端场景（单测 / 无订阅者）：压缩照常完成，只是不广播。
        return;
    };
    emitter
        .emit_rewrite(context.session.session_id(), dropped, head)
        .await;
}

/// 压缩内核的**包装层**：只负责「正在压缩」这个会话阶段的置位与清位。
///
/// ## 为什么清位必须收在这一层
///
/// 内核有**四个**出口（成功 / 输入超限 / LLM 失败（含用户中止）/ 快照校验失败），
/// 漏掉任何一个，会话就会在回到空闲后仍挂着"正在压缩上下文"——而且没有任何机制
/// 会纠正它（`is_working` 已经归位，后续也不会再有压缩相关的变更）。
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
    compression_request: Vec<ChatMessage>,
    keep_messages: Vec<ChatMessage>,
    extra_hints: Option<&str>,
    log_tag: &str,
) -> Result<Option<usize>, CompressionFailure> {
    // 压缩节点的 id 在**包装层**生成：这里才有发射器，而内层只管压缩逻辑。
    let node_id = short_id();
    let before = context.messages.len();
    if let Some(e) = &orchestrator.compression {
        e.begin(&node_id).await;
    }
    let result = compress_snapshot_inner(
        orchestrator,
        context,
        channel,
        ctx,
        abort_flag,
        compression_request,
        keep_messages,
        extra_hints,
        log_tag,
    )
    .await;
    if let Some(e) = &orchestrator.compression {
        let after = context.messages.len();
        // 失败必须带上**原因**：三种失败的应对完全不同（重试 / 换模型 / 手动清理），
        // 只说"压缩未完成"等于让用户和排查者都无从下手——实测会话 `09d74431`
        // 的两条 failed 节点就是只有这句话、`error` 与 `meta` 全空。
        let (status, text, kind) = match &result {
            Ok(Some(_)) => (
                MessageStatus::Completed,
                format!("已压缩上下文（{before} → {after} 条）"),
                None,
            ),
            Ok(None) => (
                MessageStatus::Completed,
                "未触发压缩（内容未达阈值）".to_string(),
                None,
            ),
            Err(f) => (MessageStatus::Failed, f.message(), Some(f.kind())),
        };
        let node = e.finish(&node_id, status, &text, kind).await;
        // 落库：压缩是会话里真实发生的一步，应当留下记录。否则用户刷新后只看到
        // "历史突然变短了"，却没有任何东西说明发生过什么。
        if let Err(err) = context.session.append_messages(vec![node]).await {
            plugin_warn!(
                "session",
                "[Compress] 压缩节点落库失败（前端已收到终态）: {err}"
            );
        }
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
    let compression_request = compression::build_compression_request(&history, hints);

    // 压缩流水线与被动自动压缩共用同一核心（transcript 转存 → LLM 压缩请求 →
    // 快照校验/纠正重试 → meta 构造 → 保留区拼接落库）；失败已在核心内
    // 就地回滚，这里只把它翻译为工具结果的 (compressed, before, after) 三元组。
    let post_tokens = compress_with_snapshot_core(
        orchestrator,
        context,
        channel,
        ctx,
        abort_flag,
        compression_request,
        keep_messages,
        hints,
        "manual",
    )
    .await;

    match post_tokens {
        Ok(Some(after)) => (true, before_tokens, after),
        // 未执行（内容未达阈值）：工具结果如实反映无收益
        Ok(None) => (false, before_tokens, before_tokens),
        // 失败：历史已在核心内回滚；具体原因已写入压缩节点（meta.failure_kind +
        // error），这里只如实反映工具无收益。失败不再冒泡为整轮失败。
        Err(_) => (false, before_tokens, before_tokens),
    }
}

/// 用户对**失败的压缩节点**发起重试（`ResumeAction::RetryCompaction` 的执行体）。
///
/// ## 为什么"重试"是这套失败语义的必要一环
///
/// 压缩失败**不改动历史**（失败路径的硬不变式），因此用户看到失败原因之后唯一
/// 能做的事就是"再试一次"——`input_over_limit` 尤其如此：换一个上下文更大的模型，
/// 同一次压缩就能成功。没有这个入口，失败节点就只是一个死胡同。
///
/// ## 与自动路径的两点差异
///
/// 1. **绕过熔断**（`force = true`）：熔断约束的是"每轮自动白等一次注定失败的
///    请求"，用户点重试是他的明确意愿，不该被冷却挡住；
/// 2. **删除旧失败节点**：删除-重建（与 `resume.rs` 的其它恢复动作同款）——
///    重试成功不留旧失败痕迹，重试失败则留下一条新的失败节点。
///
/// ## 删除帧走哪条通道
///
/// 与工具恢复删除旧子节点同源：发 `StreamEvent::Delete`，由消费循环转译成 VDFS
/// `deleted` 变更（见 `orchestrator/consume.rs`）。**不能**只删存储——那样前端
/// 转写会永久留着那个已被删掉的失败节点，且没有任何机制会纠正它。
pub(crate) async fn retry_compaction(
    orchestrator: &ChatOrchestrator,
    ctx: &Arc<dyn InvokeRequest>,
    channel: &mut PluginChannel,
    abort_flag: &Arc<AtomicBool>,
    session: &Arc<dyn ChatSession>,
    target_id: &str,
) -> Result<(), PluginError> {
    let mut messages = session.get_messages().await?;
    // 只接受**压缩节点**：重试入口长在压缩失败节点上，目标错位说明前端与存储已经
    // 不一致——此时按 NotFound 拒绝，比"顺着 target_id 猜一个节点"安全。
    let found = messages
        .iter()
        .any(|m| m.id == target_id && m.msg_type == Some(MessageType::Compression));
    if !found {
        return Err(PluginError::NotFound(format!(
            "未找到压缩节点: {target_id}"
        )));
    }

    // 删除旧失败节点必须针对**原始列表**（`get_messages`）：失败节点在
    // `get_context_messages` 的过滤里本就看不见（它会滤掉 Failed 消息），
    // 拿过滤视图去删会「删了个空气」，节点反而留在存储里。
    messages.retain(|m| m.id != target_id);
    session.replace_messages(messages).await?;
    let _ = channel
        .tx
        .send(PluginFrame::Data(serde_json::json!(
            session_chat_response::StreamEvent::Delete {
                message_id: target_id.to_string()
            }
        )))
        .await;

    // 压缩本身则必须跑在与**自动路径完全同一份视图**上：`get_context_messages`
    // 会做三层清理（滤 Failed / 剔孤儿 / content 归一）并施加轮次窗口，
    // 用 `get_messages` 的原始列表会让切分点与首次失败时不同，重试的结论
    // 与首次失败对不上（"换个视图再试一次"不是重试）。
    let mut context = SessionContext {
        messages: session.get_context_messages(None).await.unwrap_or_default(),
        session: session.clone(),
    };
    // 请求级固定开销与自动路径同源（压缩提示词 + 工具定义）：口径不一致会让
    // "是否超限"的预判比自动路径乐观，重试的结论就与首次失败对不上。
    let overhead_tokens =
        compression::estimate_request_overhead(&compression::get_compression_prompt(), ctx).await;
    let outcome = auto_compress_process(
        orchestrator,
        &mut context,
        channel,
        ctx,
        abort_flag,
        overhead_tokens,
        // force = true：跳过阈值判定并绕过熔断（用户主动重试）
        true,
    )
    .await;

    match &outcome {
        Ok(Some(_)) => {
            if let Some(em) = &orchestrator.compression {
                em.state.compression_record_success().await;
            }
        }
        Ok(None) => {
            // 无切分点 / 无收益：内核根本没被调用，于是不会产出任何节点。旧节点已经
            // 删掉了，若就这么返回，用户点"重试"的结果是"节点消失了，什么都没发生"。
            // 补一条 Completed 说明，让这次点击有交代。
            if let Some(em) = &orchestrator.compression {
                let node_id = short_id();
                em.begin(&node_id).await;
                let node = em
                    .finish(
                        &node_id,
                        MessageStatus::Completed,
                        "未触发压缩（当前历史无需压缩）",
                        None,
                    )
                    .await;
                if let Err(err) = context.session.append_messages(vec![node]).await {
                    plugin_warn!("session", "[Compress] 重试节点落库失败: {err}");
                }
            }
        }
        Err(f) => {
            if let Some(em) = &orchestrator.compression {
                em.state.compression_record_failure().await;
            }
            // 失败节点已由内核落库并广播（含 `meta.failure_kind` + 原因），这里只记日志
            plugin_warn!("session", "[Compress] 用户重试仍失败: {f}");
        }
    }
    Ok(())
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
