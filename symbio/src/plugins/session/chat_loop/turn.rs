//! **单轮收尾**：推理产物并入上下文 → 工具分发 → 落库 → 流向判定。
//!
//! `close_turn` 是"单轮里发生了什么"的唯一实现；它的出口只有
//! [`TurnFlow`](super::TurnFlow)，**不做终态收尾**（那是 `finish_turn` 的职责）。

use super::*;

/// 输出被 max_tokens 截断时的**自动续写**上限。
const MAX_CONTINUE_ROUNDS: u32 = 3;

/// 推理收尾：定格 assistant 子节点状态 → 校准 token 估算 → 把本轮产出并入上下文。
///
/// 自 `run_chat_loop` 原样搬移（拆函数不拆行为）：语句、注释、调用顺序与搬移前
/// 逐字一致，仅把"提前取出的字段"收进 [`TurnResult`] 返回给调用方。
pub(crate) async fn settle_reasoning(
    orchestrator: &ChatOrchestrator,
    context: &mut SessionContext,
    sink: &ExecEventSink,
    root_id: &str,
    mut out: TurnOutput,
) -> TurnResult {
    let tools_done = out.tool_accumulator.get_completed();
    // 提前取出本轮的结束原因 / 用量 / 是否出现过工具调用 / 文本子节点 id，
    // 因为 `out.into_messages` 会按值消费 out，之后无法再读这些字段。
    let had_tool = out.tool_accumulator.had_any_tool_call();
    let finish = out.finish.clone();
    let usage = out.usage;
    let rtid = out.response_text_child_id.clone();
    let rrid = out.reasoning_child_id.clone();

    orchestrator
        .finalize_assistant_turn(root_id, &out, &tools_done, sink)
        .await;

    // 用 provider 返回的真实用量滚动校准 token 估算。
    feedback_estimate(usage, &out, &tools_done);

    let new_msgs = out.into_messages(root_id, tools_done.len());
    // 工具上下文保留策略：策略不 Stamp 到节点 meta 持久化，
    // 由 run_chat_loop 在构建 LLM 请求前从 CapabilityVisitor 动态解析，
    // 节点 name 即 LLM 可见工具名，与声明名直接匹配。
    context.messages.extend(new_msgs);

    // 被长度截断的 Turn 打标（供前端「继续」按钮与回溯），不得静默结束。
    if finish.is_length() {
        for m in context.messages.iter_mut() {
            if m.id == rtid || m.id == rrid {
                let mut meta = m.meta.clone().unwrap_or_else(|| serde_json::json!({}));
                meta["finish_reason"] = serde_json::json!("length");
                m.meta = Some(meta);
            }
        }
    }

    TurnResult {
        root_id: root_id.to_string(),
        tools_done,
        finish,
        had_tool,
    }
}

/// 本轮收尾阶段：截断续写 / 主动压缩拦截 / 工具分发 / 父节点状态落库 / 停等判定。
///
/// **不承担终态收尾**：`fire_stop_hook` 与最终 `persist_messages` 由 [`finish_turn`]
/// 统一执行（本函数返回 `Finish` 前会把 [`TurnState::last_saved`] 推进到最新，
/// 使 `finish_turn` 的落库成为 no-op）。
pub(crate) async fn close_turn(
    orchestrator: &ChatOrchestrator,
    ctx: Arc<dyn PluginInvokeRequest>,
    sink: &ExecEventSink,
    context: &mut SessionContext,
    turn: &mut TurnState,
    result: TurnResult,
    req: &TurnRequest,
) -> TurnFlow {
    let TurnResult {
        root_id,
        tools_done,
        finish,
        had_tool,
    } = result;
    let root_id = root_id.as_str();

    if tools_done.is_empty() {
        // 本轮无工具调用 —— 正常收尾，除非是被长度截断。
        if finish.is_length() && !had_tool {
            // 纯文本被 max_tokens 截断且参数完整 → 自动续写：
            // 已产出的（截断）文本已作为 assistant 消息进入上下文，下一轮请求时模型会
            // 自然从断点继续。最多续写 MAX_CONTINUE_ROUNDS 次，避免失控死循环。
            if turn.continuation_count < MAX_CONTINUE_ROUNDS {
                turn.continuation_count += 1;
                plugin_info!(
                    "session",
                    "[Turn] finish=Length，自动续写 ({}/{})",
                    turn.continuation_count,
                    MAX_CONTINUE_ROUNDS
                );
                persist_messages(context, turn.last_saved, sink).await;
                turn.last_saved = context.messages.len();
                return TurnFlow::NextTurn;
            }
            // 续写次数耗尽：明确告知，绝不静默结束（会话级告警状态，随下一轮请求清除）。
            sink.warn(Some(format!(
                "输出因达到长度上限而中断（已自动续写 {} 次仍超出）。请提高单次输出预算或缩小任务范围。",
                MAX_CONTINUE_ROUNDS
            )))
            .await;
        } else if finish.is_length() && had_tool {
            // 工具调用参数 JSON 被长度截断：参数残破无法通过续写修复，
            // 该次调用已丢弃 → 明确报错而非静默结束（会话级告警状态）。
            sink
                .warn(Some(
                    "输出在工具调用参数中途达到长度上限而中断。请提高单次输出预算，或把大任务拆小后重试。"
                        .to_string(),
                ))
                .await;
        }
        plugin_info!(
            "session",
            "[Turn] 结束（正常收尾，无工具调用）finish={:?}",
            finish
        );
        persist_messages(context, turn.last_saved, sink).await;
        turn.last_saved = context.messages.len();
        return TurnFlow::Finish(TurnExit::Completed);
    }

    // ── 主动压缩工具拦截 ─────────────────────────────────────────────
    // context_compact 不走 CapabilityVisitor 分发：它需要编排器内部的
    // 压缩链路（LLM 摘要 + 上下文替换 + 会话持久化）。
    // 在此拆分：压缩调用就地执行并生成合成工具结果；其余工具正常分发。
    // 门控：仅当工具压缩开关开启时拦截；开关关闭时工具不暴露，模型幻觉
    // 调用则归入标准工具链，以"未知路径"错误返回（不执行内部压缩链路）。
    let (compact_calls, other_calls): (Vec<_>, Vec<_>) = if req.enable_compact_tool {
        tools_done.into_iter().partition(|tc| {
            tc.name
                .as_deref()
                .map(|n| n == compression::CONTEXT_COMPACT_TOOL_NAME)
                .unwrap_or(false)
        })
    } else {
        (Vec::new(), tools_done)
    };

    let mut tool_results: Vec<ChatMessage> = Vec::new();
    let mut parent_updates: Vec<ChatMessage> = Vec::new();

    if !compact_calls.is_empty() {
        // 切分点前移到当前用户指令：保留区 = [用户指令, Turn 及其子节点...]，
        // Turn 子树 parent 链完整。切分点绝不能落在本 Turn 首个 ToolCall——
        // 那样用户指令与 Turn 根会被压进快照，保留区只剩 parent 悬空的
        // ToolCall → provider 400。
        // 返回 0 时 run_context_compact 以 split==0 视为中止，安全。
        let split_user_idx = compression::find_turn_user_split_idx(&context.messages, root_id);
        let first = compact_calls.first().cloned();
        if let Some(first) = first {
            let call_id = first.id.clone().unwrap_or_default();
            let hints = first
                .arguments
                .get("hints")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let (ok, before_t, after_t) = run_context_compact(
                orchestrator,
                context,
                &ctx,
                &turn.abort,
                split_user_idx,
                hints.as_deref(),
            )
            .await;
            if ok {
                // 压缩成功：上下文回落后允许再次水位提醒
                turn.nudged_this_request = false;
                // replace_messages 已整体重写会话存储，当前内存镜像即已落库状态；
                // 重置持久化锚点，避免末尾 persist_messages 用旧下标切片越界/重复落库
                turn.last_saved = context.messages.len();
                plugin_info!(
                    "session",
                    "[Compress] manual compaction done: ~{} -> ~{} tokens",
                    before_t,
                    after_t
                );
            }
            let result_text = if ok {
                format!(
                    "Context compacted: ~{before_t} -> ~{after_t} tokens. \
                     The session now starts from the state snapshot followed by the current \
                     task. Continue the task based on the snapshot; archived transcripts are \
                     referenced inside it if details are needed."
                )
            } else if before_t > 0 && before_t == after_t {
                format!(
                    "Compaction skipped: history to compress is only ~{before_t} tokens \
                     (below the useful threshold), context unchanged. Continue the task."
                )
            } else {
                "Compaction failed and was rolled back; context unchanged. \
                 Continue the task."
                    .to_string()
            };
            // 标准工具广播模式（与 process_tool_calls_async 一致）：
            // 前端实时可见 context_compact 的结果子节点与父节点状态——
            // 先广播 Tool 结果子节点，再广播父 ToolCall 的完整终态快照。
            let mut tool_msg = build_tool_message(&call_id, &result_text, Some(ok), None);
            if !ok {
                // 失败属信息性：结果以 Completed 定格（父节点同为 Completed），
                // 与普通工具结果的处理保持一致，避免孤儿 Failed 节点
                tool_msg.status = Some(MessageStatus::Completed);
            }
            emit_message(sink, tool_msg.clone()).await;
            // 父节点终态：从权威转写取完整副本应用终态（找不到 = 协议违例，跳过）。
            if let Some(mut parent) = context.messages.iter().find(|m| m.id == call_id).cloned() {
                parent.status = Some(MessageStatus::Completed);
                let mut meta = parent.meta.clone().unwrap_or_else(|| serde_json::json!({}));
                if let Some(obj) = meta.as_object_mut() {
                    obj.insert("success".into(), serde_json::json!(ok));
                    obj.insert("kind".into(), serde_json::json!("context_compact"));
                    if ok {
                        obj.insert("before_tokens".into(), serde_json::json!(before_t));
                        obj.insert("after_tokens".into(), serde_json::json!(after_t));
                    }
                }
                parent.meta = Some(meta);
                emit_state(sink, parent.clone()).await;
                parent_updates.push(parent);
            } else {
                plugin_error!(
                    "session",
                    "[Compress] 转写中不存在工具调用 {}（协议违例），跳过父状态广播",
                    call_id
                );
            }
            tool_results.push(tool_msg);
        }
        // 同批多余的 compact 调用：直接标记跳过
        for extra in compact_calls.iter().skip(1) {
            if let Some(cid) = &extra.id {
                // 同批多余调用同样走标准广播模式（跳过说明属信息性结果，定格 Completed）
                let mut tool_msg = build_tool_message(
                    cid,
                    "Skipped: another context_compact call in this batch was executed.",
                    Some(false),
                    None,
                );
                tool_msg.status = Some(MessageStatus::Completed);
                emit_message(sink, tool_msg.clone()).await;
                if let Some(mut parent) = context.messages.iter().find(|m| m.id == *cid).cloned() {
                    parent.status = Some(MessageStatus::Completed);
                    let mut meta = parent.meta.clone().unwrap_or_else(|| serde_json::json!({}));
                    if let Some(obj) = meta.as_object_mut() {
                        obj.insert("success".into(), serde_json::json!(false));
                        obj.insert("kind".into(), serde_json::json!("context_compact"));
                        obj.insert("skipped".into(), serde_json::json!(true));
                    }
                    parent.meta = Some(meta);
                    emit_state(sink, parent.clone()).await;
                    parent_updates.push(parent);
                } else {
                    plugin_error!(
                        "session",
                        "[Compress] 转写中不存在工具调用 {}（协议违例），跳过父状态广播",
                        cid
                    );
                }
                tool_results.push(tool_msg);
            }
        }
    }

    // 登记本轮派发的工具调用到在途集合（**启动条件的权威判据**）：
    // 级别 1 在 `process_tool_calls_async` 返回时同步清空——工具已全部跑完，
    // 回到闸门时集合为空；级别 2 会改为由每个工具的完成回调逐个移除，
    // 全部移除后才唤醒主循环（不完整不唤醒）。
    turn.in_flight_tools
        .extend(other_calls.iter().filter_map(|tc| tc.id.clone()));
    let (other_results, other_parent_updates) = process_tool_calls_async(
        other_calls,
        &orchestrator.parent,
        sink,
        &turn.abort,
        ctx.clone(),
        &context.messages,
    )
    .await;
    turn.in_flight_tools.clear();
    tool_results.extend(other_results);
    parent_updates.extend(other_parent_updates);
    context.messages.extend(tool_results.clone());

    // 父节点终态同步进 context.messages 内存镜像——**整条替换**（广播出去的
    // `parent_updates` 本就是完整快照）。id 不存在的（协议失败兜底父节点，
    // 从未广播过）补入转写，使其随下方的 persist_messages 落库，结果子节点
    // 不会悬空。落库由 persist_messages 统一承担（append 范围覆盖本轮全部
    // 新节点，父节点的终态已在镜像里），不再需要 update_messages 补写。
    for full in &parent_updates {
        match context.messages.iter_mut().find(|m| m.id == full.id) {
            Some(msg) => *msg = full.clone(),
            None => context.messages.push(full.clone()),
        }
    }

    persist_messages(context, turn.last_saved, sink).await;
    turn.last_saved = context.messages.len();

    // 检测工具待用户恢复 → 退出本轮：
    // 仅当存在 UserPrompt/WaitingUserAction（confirm/ask_user）时才算需要用户输入。
    // 普通的工具执行失败不使会话停摆：父节点已标 Completed、错误结果作为合法
    // tool 结果留在上下文喂回 LLM 继续处理，用户也可随时直接发新消息继续。
    let mode = ctx.get(crate::symbio_core::MODE).unwrap_or_default();
    let needs_user_action = tool_results.iter().any(|m| {
        m.msg_type == Some(MessageType::UserPrompt)
            && m.status == Some(MessageStatus::WaitingUserAction)
    }) || parent_updates
        .iter()
        .any(|p| p.status == Some(MessageStatus::WaitingUserAction));

    if needs_user_action {
        // 注：信息性策略下工具失败的父 ToolCall 已标 Completed（错误结果作为
        // 合法 tool 结果喂回 LLM，loop 不中断），不存在「Failed 父节点等待
        // 恢复」的场景；
        // user_prompt(WaitingUserAction) 驱动的暂停走 approve/reject/answer 恢复。
        plugin_info!(
            "session",
            "[Turn] 工具待用户恢复（mode={}），退出本轮",
            mode
        );
        return TurnFlow::Finish(TurnExit::Completed);
    }

    // 轮次计数：用户明确要求不设硬性上限，超长对话的规模控制由请求视图层的
    // fade / 骨架化（build_request_view）承担——存储保持完整历史，视图逐轮裁剪。
    turn.tool_rounds += 1;
    plugin_info!(
        "session",
        "[Turn] 第 {} 轮结束（工具轮，进入下一轮），工具调用 {} 个",
        turn.tool_rounds - 1,
        tool_results.len()
    );
    TurnFlow::NextTurn
}

/// 用 provider 返回的真实用量滚动校准 token 估算。
///
/// 中文/代码场景收益最大；估算长期偏低会直接导致 400 而非过早压缩。
///
/// 两条不变式（破坏任一条都会静默劣化水位判定，故单列为纯函数并配测试）：
/// 1. 反馈必须用**原始启发式估算**（`count_raw`）；用校准后的 `count()` 自反馈
///    会让校准系数收敛到 √(真实比值)（见 `CalibratedTokenizer::feedback` 文档）。
/// 2. 分母必须覆盖 provider 计入 `output_tokens` 的**全部**内容：文本 + 思考 +
///    工具调用名 + 参数 JSON。漏掉任一部分都会系统性低估估算值 → 校准比偏高
///    → 水位提前越过阈值 → 压缩被频繁触发。
fn feedback_estimate(usage: Option<ModelUsage>, out: &TurnOutput, tools: &[TurnToolCallInfo]) {
    let Some(u) = usage else { return };
    let tok = super::super::tokenizer::default_tokenizer();
    let mut estimated = tok.count_raw(&out.text) + tok.count_raw(&out.reasoning);
    for tc in tools {
        if let Some(name) = tc.name.as_ref().filter(|n| !n.is_empty()) {
            estimated += tok.count_raw(name);
        }
        estimated += tok.count_raw(&tc.arguments.to_string());
    }
    super::super::tokenizer::report_provider_usage(estimated, u.output);
}
