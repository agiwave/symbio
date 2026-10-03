//! 压缩执行流水线：自动语义压缩（L2）与主动 `context_compact` 工具共用的实现。
//!
//! 本模块是 `context` 域唯一的**执行层**（LLM 请求 / 落库 / 广播）；域内其余兄弟
//! 模块（门面 / `view` / `prompt`）均为无副作用的纯策略。
//!
//! 两条入口（[`auto_compress_process`] / [`run_context_compact`]）共用
//! [`compress_with_snapshot_core`] 这一个执行内核——"何时压"有两个入口，
//! "怎么压"只有一个实现。

use super::*;

use super::super::chat_loop::{ChatOrchestrator, SessionContext};
use super::super::chat_session::PersistentChatSession;
use super::super::plugin::append_and_publish;
use super::super::tools::fire_hook;
use super::super::transcript::llm_emit_removed;
use crate::plugin_warn;
use crate::symbio_core::chat_message::{MessageContent, MessageStatus};
use crate::symbio_core::HookEvent;
use crate::symbio_core::{llm_short_id, ExecAbortSignal, ExecEnv, ExecEventSink, PluginError};

/// 被动自动压缩（L1）：阈值判定 → 切分 → 收益护栏 → 交执行内核。
///
/// `force` 为 `super::should_start_compression` 的公开契约（跳过阈值）。
/// 自动路径恒为 `false`（自动压缩必须走阈值）；`true` 只由**用户主动重试**
/// （`retry_compaction`）传入——此时同时**绕过熔断**：熔断约束的是"每轮自动白等
/// 一次注定失败的请求"，用户点重试是他的明确意愿，不该被冷却挡住。
///
/// 无 `extra_hints`：自动压缩没有"用户"在环内提供保留提示——`hints` 是
/// `context_compact` 工具的入参，只在主动路径有意义（见 `compress_with_snapshot_core`）。
pub(crate) async fn auto_compress_process(
    orchestrator: &ChatOrchestrator,
    context: &mut SessionContext,
    ctx: &Arc<dyn PluginInvokeRequest>,
    abort: &ExecAbortSignal,
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
                    "[Compress] 自动压缩熔断：连续失败已达阈值，本次跳过（冷却中），历史保持完整"
                );
                return Ok(None);
            }
        }
    }

    if !super::should_start_compression(&context.messages, effective_context_limit, force, overhead)
    {
        return Ok(None);
    }

    let (compression_request, history_to_compress, history_to_keep) =
        match super::prepare_compression(&context.messages) {
            Some(v) => v,
            None => return Ok(None),
        };

    // 压缩收益护栏（门槛的唯一出处：`super::has_compaction_payoff`）
    let compress_tokens: usize = history_to_compress
        .iter()
        .map(super::estimate_message_tokens)
        .sum();
    if !super::has_compaction_payoff(compress_tokens) {
        return Ok(None);
    }

    // 压缩核心与主动 context_compact 工具共用（compress_with_snapshot_core）：
    // 失败已在核心内就地回滚，这里只区分"成功/未压缩"两种结果。
    let original_count = context.messages.len();
    let post_tokens = compress_with_snapshot_core(
        orchestrator,
        context,
        ctx,
        abort,
        compression_request,
        history_to_keep,
        // 自动路径无用户保留提示（hints 只在主动路径有来源）
        None,
        "auto",
        if force { "retry" } else { "threshold" },
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
                em.state
                    .compression_record_failure(matches!(
                        f,
                        CompressionFailure::InputOverLimit { .. }
                    ))
                    .await;
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
    /// 这条路径**不裁剪历史**：请求体与上下文同源超限只说明"这次压缩放不下"，
    /// 不构成"可以静默丢历史"的授权——机械截断早期消息会让历史凭空变短且无重试入口。
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
    ctx: &Arc<dyn PluginInvokeRequest>,
    abort: &ExecAbortSignal,
    compression_request: Vec<ChatMessage>,
    keep_messages: Vec<ChatMessage>,
    extra_hints: Option<&str>,
    log_tag: &str,
    // 与包装层 begin/finish 同一个 id：摘要增量因此落到**同一个**节点上
    node_id: &str,
) -> Result<Option<usize>, CompressionFailure> {
    // 保存原始历史：压缩失败时回滚，绝不能让压缩请求残留在上下文里。
    let original_messages = context.messages.clone();

    // ── 输入超限预判（**前移到一切副作用之前**，跳过注定失败的巨型请求）──────
    // LLM 摘要请求的请求体**就携带完整待压缩历史**——若历史本身已超 Provider
    // 有效输入上限，摘要请求必然 400（"Input token exceed the limit"），
    // 且每轮自动压缩都会重发这条注定失败的巨型请求：压缩永不收敛、每轮开头
    // 多一段漫长的无响应。预判命中时**跳过这次 doomed 请求**（这是预判的
    // 唯一收益），然后如实报错。
    //
    // 预判必须在 PreCompact 钩子与 transcript 转存**之前**：前者有外部副作用，
    // 后者会把完整历史写成 0.87MB 级的存档文件——注定失败的压缩没有转存价值，
    // 实测会话 `09d74431` 因此攒下 18 份共 15MB 的废档。此前这段检查位于转存
    // 之后，属时序缺陷。
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
        super::estimate_request_overhead(&super::get_compression_prompt(), ctx).await;
    let pending_tokens: usize = compression_request
        .iter()
        .map(super::estimate_message_tokens)
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
        // 预判位于任何副作用之前，`context.messages` 仍是原始历史，无需回滚
        return Err(CompressionFailure::InputOverLimit {
            pending: pending_tokens + overhead_tokens,
            limit: effective_limit,
        });
    }

    let _ = fire_hook(&orchestrator.parent, HookEvent::PreCompact, ctx.clone()).await; // grep-audit-allow S-002-bonus: fire_hook 返回 HookOutput 非 Result，无错误可丢（见其文档）

    // 可回溯原则：压缩前把完整历史转存为 transcript，路径记入快照 meta。
    // 若跳过此步直接 replace_messages，被压掉的历史在物理层"凭空消失"，
    // 旧存档文件成为孤儿，事后无法审计。
    // 会话存储根 = **本插件自己的目录**（装配期由父插件经 `PLUGIN_DIR` 告知，
    // 已落在 orchestrator 上）——不从请求上下文反推，也不读全局系统根。
    let storage_root = orchestrator.session_dir.dir();
    let transcript_path = save_transcript_archive(
        &original_messages,
        context.session.session_id(),
        storage_root,
    );

    // 压缩请求的**全部内容**：待压缩历史（正常对话形态）+ 末尾压缩指令。
    // provider 的 `flatten_chat_messages` 会把它投影成模型该看的对话，
    // 而不是把存储字段原样倒给模型（见 `build_compression_request` 的说明）。
    context.messages = compression_request;

    // 专用压缩 system 提示词（模板只在本次请求出现，与主对话隔离）
    let compression_prompt = super::get_compression_prompt();
    let root_id = llm_short_id();
    let summary = match send_compression_request(
        orchestrator,
        &compression_prompt,
        &context.messages,
        &root_id,
        abort,
        // 摘要增量实时落到压缩节点上（节点 id 与 begin/finish 同源，见包装层）
        orchestrator.compression.as_deref(),
        node_id,
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
            plugin_warn!(
                "session",
                "[Compress] {log_tag} 压缩失败 ({})，回退到未压缩上下文",
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
    let mut validated = super::extract_snapshot(&summary_text(&summary));
    if validated.is_none() {
        // 重试：附纠正指令，要求严格按 XML 结构输出
        let retry_msg = ChatMessage {
            id: llm_short_id(),
            role: Some(MessageRole::User),
            msg_type: Some(MessageType::Text),
            content: Some(MessageContent::Text(
                "Your previous compose did not contain a valid <state_snapshot> XML block. \
                 Compose again with ONLY the <state_snapshot> block, following the requested structure."
                    .to_string(),
            )),
            status: Some(MessageStatus::Completed),
            ..Default::default()
        };
        context.messages.push(retry_msg);
        // 纠正重试沿用**同一个压缩节点**：两段摘要增量先后追加到同一节点正文上，
        // 与「这是一次压缩、重试是它的一部分」的观感一致。
        let retry = send_compression_request(
            orchestrator,
            &compression_prompt,
            &context.messages,
            &llm_short_id(),
            abort,
            orchestrator.compression.as_deref(),
            node_id,
        )
        .await;
        if let Ok(s) = retry {
            if let Some(snapshot) = super::extract_snapshot(&summary_text(&s)) {
                validated = Some(snapshot);
            }
        }
    }
    let Some(snapshot_text) = validated else {
        // 两次均未得到有效快照：保留原历史，禁止把未验证散文升级为唯一记忆。
        plugin_warn!(
            "session",
            "[Compress] {log_tag}: 重试后快照仍无效，保留原历史"
        );
        context.messages = original_messages;
        return Err(CompressionFailure::InvalidSnapshot);
    };
    context.messages.clear();

    // 落库前渲染为纯文本分节（历史中不残留 XML 标签，切断格式模仿链）
    let snapshot_display = super::render_snapshot_for_history(&snapshot_text);
    // 快照消息：meta 记录压缩标记、压缩后估算（迟滞依据）、转存路径。
    // post_tokens 口径 = 压缩完成后的内容水位（快照 + 保留区内容，不含请求级
    // overhead），与 should_start_compression 迟滞比较的读取侧对齐。若只算快照、
    // 漏掉保留区，迟滞地板被低估 → 压缩后很快再次越线 → 循环压缩。
    let post_tokens = super::estimate_message_tokens(&ChatMessage {
        content: Some(MessageContent::Text(snapshot_display.clone())),
        ..Default::default()
    }) + keep_messages
        .iter()
        .map(super::estimate_message_tokens)
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
    meta["protocol_version"] = serde_json::json!(super::COMPRESSION_PROTOCOL_VERSION);
    meta["prompt_fingerprint"] = serde_json::json!(super::compression_prompt_fingerprint());

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
    let snapshot_seq = super::snapshot_slot_seq(compressed, &keep_messages);

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
/// 压缩走的是 store 层的 `replace_messages`（`vdfs` 的三条 VDFS 写路由
/// 在 `replace_messages` 之后都要发变更，压缩这条路径原先漏了）。VDFS 变更流是
/// 前端转写的**唯一**入口，漏发即前端永久停留在压缩前的列表上：
/// 被压掉的历史仍在、快照不出现、被改号的消息仍持旧序号——而**没有任何机制会纠正**，
/// 因为两条链路互不校验（与 `plugin.rs` 消息级变更那三条路由同一个道理）。
///
/// 只发 VDFS 变更、不发前端帧：消息通道已归 VDFS 一处（见 `docs/node-state-streaming.md`），
/// 与 [`super::super::plugin::SessionPlugin::notify_change`] 同款。
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

/// 压缩节点的**结构化交代** —— 「这次压缩从哪里来、水位到哪去」的数字，
/// 随节点 `meta` 下发，由前端按字段渲染（缺字段就退回正文那一行）。
///
/// ## 口径（改之前先读）
///
/// - `before_tokens` / `after_tokens` 是**内容水位**：与快照 `meta.post_tokens` 同源
///   （包含历史与保留区内容，**不含**请求级固定开销）。与 `context_limit` 相除得到的
///   比例因此是一个**下限**——真实占用还要加上 system prompt 与工具定义。宁可少报
///   也不要多报：报多了会让用户以为"已经快满了"而做无谓的手动压缩。
/// - `after_tokens` 只在**成功**时写入。失败 / 未触发的路径没有可信的"压缩后水位"，
///   编一个 0 会在界面上显示成"水位已降到 0"——那是最坏的一种谎报。
///
/// 纯函数：可直接断言，不需要跑一整条压缩流水线。
fn compression_stats(
    trigger: &str,
    context_limit: usize,
    before_tokens: usize,
    after_tokens: Option<usize>,
    dropped: usize,
) -> serde_json::Value {
    let mut stats = serde_json::json!({
        "compact_trigger": trigger,
        "context_limit": context_limit,
        "before_tokens": before_tokens,
    });
    if let Some(after) = after_tokens {
        stats["after_tokens"] = serde_json::json!(after);
    }
    if dropped > 0 {
        stats["dropped"] = serde_json::json!(dropped);
    }
    stats
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
    ctx: &Arc<dyn PluginInvokeRequest>,
    abort: &ExecAbortSignal,
    compression_request: Vec<ChatMessage>,
    keep_messages: Vec<ChatMessage>,
    extra_hints: Option<&str>,
    log_tag: &str,
    // 触发来源（**面向用户的原因词**，进节点 `meta.compact_trigger`）：
    // `threshold`（越线自动）/ `tool`（模型主动调 `context_compact`）/ `retry`（用户重试）。
    // 与 `log_tag`（日志标签）分开：前者是线上契约的一部分，改它会改变前端呈现。
    trigger: &'static str,
) -> Result<Option<usize>, CompressionFailure> {
    // 压缩节点的 id 在**包装层**生成：这里才有发射器，而内层只管压缩逻辑。
    let node_id = llm_short_id();
    let before = context.messages.len();
    // 内容水位（不含请求级开销，口径见 `compression_stats`）——在上下文被改写**之前**读
    let before_tokens = super::estimate_context_tokens(&context.messages, 0);
    if let Some(e) = &orchestrator.compression {
        e.begin(&node_id).await;
    }
    let result = compress_snapshot_inner(
        orchestrator,
        context,
        ctx,
        abort,
        compression_request,
        keep_messages,
        extra_hints,
        log_tag,
        &node_id,
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
        // 成功才带「压缩后水位」；`dropped` = 这次被压掉的条数（前后条数之差）：
        // 用户关心的"我的历史少了多少"是一个数字，不该让他自己做减法。
        let stats = compression_stats(
            trigger,
            orchestrator.context_limit as usize,
            before_tokens,
            match &result {
                Ok(Some(post)) => Some(*post),
                _ => None,
            },
            before.saturating_sub(after),
        );
        let node = e.finish(&node_id, status, &text, kind, Some(stats)).await;
        // 落库 + 落库回包：压缩是会话里真实发生的一步，应当留下记录。否则用户刷新后
        // 只看到"历史突然变短了"，却没有任何东西说明发生过什么。回包那一步换入
        // `finish` 帧还缺的存储权威号（§3.4）；失败只记日志——前端已收到终态。
        if let Err(err) = append_and_publish(
            &context.session,
            vec![node],
            e.target(context.session.session_id()),
        )
        .await
        {
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
    ctx: &Arc<dyn PluginInvokeRequest>,
    abort: &ExecAbortSignal,
    split_user_idx: usize,
    hints: Option<&str>,
) -> (bool, usize, usize) {
    if split_user_idx == 0 {
        return (false, 0, 0);
    }

    // 待压缩历史 = [.., split_user_idx)，当前用户指令起的任务上下文整体留在保留区
    let history: Vec<ChatMessage> = context.messages[..split_user_idx].to_vec();
    let before_tokens: usize = history.iter().map(super::estimate_message_tokens).sum();
    // 压缩收益护栏（门槛的唯一出处：`super::has_compaction_payoff`）
    if !super::has_compaction_payoff(before_tokens) {
        return (false, before_tokens, before_tokens);
    }

    let keep_messages: Vec<ChatMessage> = context.messages[split_user_idx..].to_vec();
    let compression_request = super::build_compression_request(&history, hints);

    // 压缩流水线与被动自动压缩共用同一核心（transcript 转存 → LLM 压缩请求 →
    // 快照校验/纠正重试 → meta 构造 → 保留区拼接落库）；失败已在核心内
    // 就地回滚，这里只把它翻译为工具结果的 (compressed, before, after) 三元组。
    let post_tokens = compress_with_snapshot_core(
        orchestrator,
        context,
        ctx,
        abort,
        compression_request,
        keep_messages,
        hints,
        "manual",
        "tool",
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
/// 与工具恢复删除旧子节点同源：发一条删除帧（`status = removed`）。**不能**
/// 只删存储——那样前端转写会永久留着那个已被删掉的失败节点，且没有任何机制
/// 会纠正它。
pub(crate) async fn retry_compaction(
    orchestrator: &ChatOrchestrator,
    ctx: &Arc<dyn PluginInvokeRequest>,
    sink: &ExecEventSink,
    abort: &ExecAbortSignal,
    session: &Arc<PersistentChatSession>,
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
    // 删除帧：`status = removed`，一次状态迁移。
    llm_emit_removed(sink, target_id).await;

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
        super::estimate_request_overhead(&super::get_compression_prompt(), ctx).await;
    let outcome = auto_compress_process(
        orchestrator,
        &mut context,
        ctx,
        abort,
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
                let node_id = llm_short_id();
                em.begin(&node_id).await;
                // 仍然带上触发来源与当前水位：「未触发」也是用户点出来的结果，
                // 一行「无需压缩」之外应当能看出"现在离上限还有多远"。
                let stats = compression_stats(
                    "retry",
                    orchestrator.context_limit as usize,
                    super::estimate_context_tokens(&context.messages, 0),
                    None,
                    0,
                );
                let node = em
                    .finish(
                        &node_id,
                        MessageStatus::Completed,
                        "未触发压缩（当前历史无需压缩）",
                        None,
                        Some(stats),
                    )
                    .await;
                let sid = context.session.session_id();
                // 落库 + 回包：同上，换回存储分配的权威 `seq`（§3.4）。
                if let Err(err) =
                    append_and_publish(&context.session, vec![node], em.target(sid)).await
                {
                    plugin_warn!("session", "[Compress] 重试节点落库失败: {err}");
                }
            }
        }
        Err(f) => {
            if let Some(em) = &orchestrator.compression {
                em.state
                    .compression_record_failure(matches!(
                        f,
                        CompressionFailure::InputOverLimit { .. }
                    ))
                    .await;
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
    abort: &ExecAbortSignal,
    // 压缩节点的发射器与节点 id：有发射器时，摘要正文的逐帧增量会**改道**到这个
    // 节点上实时可见（[`compression_delta_gate`]）；`None` = 无前端场景，全帧静默。
    emitter: Option<&super::super::chat_loop::CompressionEmitter>,
    node_id: &str,
) -> Result<ChatMessage, PluginError> {
    // 压缩是**内部 LLM 请求**，不是对话轮次：其流式事件（Turn 起始 / 思考 / 正文 delta）
    // 绝不能以**自有身份**进入对话流——否则前端会多出一个永远停在"正在思考…"的
    // 空 Turn（压缩请求从不 finalize，快照也只落库不广播），且随每次自动压缩/主动
    // 压缩逐个累积。长会话才会触发压缩，因此该泄漏只在长任务后复现，极易误判为
    // 渲染层问题。
    //
    // 但「静默」不等于「用户全程面对一句占位文案」：长上下文的摘要请求可能耗时
    // 数分钟，用户看到的应当是摘要**正在长出来**——增量落在**压缩节点**上（它
    // 由 [`CompressionEmitter::begin`] 先行占位，不依赖压缩请求的任何帧）。
    //
    // 收口前这里靠**通道隔离的不对称设计**实现：tx 换成哑 sender + drain task
    // （出帧静默），rx 临时与主通道对调（入帧收真实 Abort）——因为当时「出口」
    // 只能是通道，「静默」只能靠换掉通道的一半来伪造。
    //
    // 现在这个窗口是**出口的一种取值**：有发射器时走[`ExecEventSink::filtered`]
    // （白名单把摘要文本帧改道到压缩节点，Turn 骨架帧照旧吞掉，见
    // [`compression_delta_gate`]），无发射器时退回 [`ExecEventSink::silent`]。
    // 中止则始终走**共享的 [`ExecAbortSignal`]**——压缩请求与对话轮次拿到的是同一
    // 个信号，用户停止时立即感知，不需要「把 rx 临时移交主通道」这种所有权交换。
    let sink = match emitter {
        Some(em) => {
            let writer = em.transcript_writer();
            ExecEventSink::filtered(std::sync::Arc::new(compression_delta_gate(node_id)), writer)
        }
        None => ExecEventSink::silent(),
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
        &ExecEnv::new(sink.clone(), abort.clone()),
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

    result
}

/// 摘要请求出口的**白名单**：把摘要正文的逐帧增量改道到压缩节点，其余帧吞掉。
///
/// ## 为什么是一道逐帧白名单，而不是「换一个出口」
///
/// 模型插件的流循环（`plugins/model/stream.rs`）发出的帧有三类：
///
/// 1. **Turn 组合节点骨架**（`msg_type = Turn`，`id = 请求 root_id`）——压缩请求
///    的 root 是一次性 id，这条帧若进转写，前端会多出一个**永远停在流式的空
///    Turn 骨架**（压缩不走 Turn 定稿路径），这是静默最初的动机，必须继续吞掉；
/// 2. **摘要正文子节点**（model 流循环自选 id 的 `Streaming` 正文）——它才是
///    摘要内容：首帧是「全量快照」（id + content + Streaming），后续是**纯窄
///    增量**（id + delta）。首帧被吞（它携带节点身份与全量正文），增量改写为
///    「压缩节点上的纯窄增量」放行；
/// 3. **推理增量**——同属摘要内容（reasoning-only 回退时 `effective_text` 取的
///    就是它），与正文增量同待遇。已知局限：工具参数增量与正文增量**帧形相同**
///    （`{id, delta}`），无法按形状区分；压缩请求本就不带工具清单（`tools: []`），
///    规约的模型不会产生工具调用，即使异常模型产生了，也只是把参数碎片追加到
///    预览正文后面——不落库（`finish` 用完整正文整条替换），无破坏性。
///
/// 于是判据落在**帧形状**上而不是节点 id 上：纯窄增量（`delta` 有、`content`
/// 与 `status` 皆无）⇒ 改写落点放行；其余（身份 / 全量 / 状态帧）⇒ 吞。帧面
/// 语义全在字段上（见 `transcript.rs` 的帧表），白名单因此可以**就地改写**而
/// 不需要新帧型。压缩节点本体由 [`CompressionEmitter::begin`] 先行占位，前端
/// 转写由此在占位正文后面逐帧长出摘要；`finish` 再用**完整消息帧**替换正文定稿
/// （正文从未经 delta 完整上线过——尾部不可能被 delta 续写，`finish` 的帧形
/// 无需变更）。
///
/// 返回 `false` = 吞掉（与静默出口完全同义）；`true` = 按（可能已改写的）帧放行。
fn compression_delta_gate(
    node_id: &str,
) -> impl Fn(&mut ChatMessage) -> bool + Send + Sync + 'static {
    let node_id = node_id.to_string();
    move |frame: &mut ChatMessage| {
        // 纯窄增量 ⇒ 摘要内容在增长：落点改到压缩节点（占位正文之后尾追加）。
        // 增量/全量语义互斥：改写后只留 `delta`，剥掉可能存在的 `meta`，
        // 防止上游帧形状演化时静默破坏转写不变量。
        if frame.delta.is_some() && frame.content.is_none() && frame.status.is_none() {
            frame.id = node_id.clone();
            frame.meta = None;
            return true;
        }
        // 其余（Turn 骨架 / 节点身份帧 / 全量快照 / 状态迁移）一律吞掉：
        // 压缩请求不得在流上产生任何可见节点。
        false
    }
}

/// 压缩摘要的实际 LLM 调用：出口由调用方给定（静默或过滤桥），中止走共享信号。
///
/// 注意：这里**绝不发射 Turn 帧**（不发 emit_streaming_start）。压缩是内部请求、
/// 不是对话轮次——若误走真实出口会在前端留下永远"正在思考…"的空 Turn 骨架
/// （每轮压缩尝试累积一个）。出口取值见 [`send_compression_request`]：无前端
/// 场景静默，有前端场景走过滤桥（摘要增量改道到压缩节点，骨架帧仍被吞）。
async fn run_compression_llm(
    orchestrator: &ChatOrchestrator,
    system_prompt: &str,
    messages: &[ChatMessage],
    root_id: &str,
    env: &ExecEnv,
) -> Result<ChatMessage, PluginError> {
    let abort = env.abort();

    // 压缩路径与对话轮次共用同一模型契约：provider.execute_turn（tools 为空）。
    // 出口静默、中止共享；协议细节（请求构造 / 重试 / SSE 解析）由 model 插件实现承担。
    let out = orchestrator
        .provider
        .execute_turn(system_prompt, messages, &[], root_id, env)
        .await?;

    if abort.is_aborted() {
        return Err(PluginError::Aborted);
    }

    let effective = out.effective_text().to_owned();
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
        role: Some(crate::symbio_core::chat_message::MessageRole::Assistant),
        msg_type: Some(MessageType::Text),
        content: Some(MessageContent::Text(effective)),
        status: Some(MessageStatus::Completed),
        ..Default::default()
    })
}

#[cfg(test)]
#[path = "pipeline.test.rs"]
mod tests;
