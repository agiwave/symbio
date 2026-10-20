//! 阶段②「逐条执行」——`process_tool_calls_async` 三段拆分的第二段
//! （T-179 / [PL-015 §5 动作 B](../../../../../../docs/004-plan/015-可扩充性与插件切分.md "PL-015-可扩充性与插件切分")）。
//!
//! 一条顺序流程的中间段：对批内每条调用依序过「协议校验 → 同参台账 → 参数解析 →
//! 超限丢弃 → 外部执行闸 → 淘汰闸 → 任务表闸 → Pre 钩子 → 执行 → Post 钩子 → 记账
//! → 结果与父终态广播」。批首读数经 [`GateReadings`] 传入，台账（`seen_calls`）在
//! 本段逐条登记。
//!
//! 判据（「这段错了谁会红」）：批内事件与父节点终态用例红——`tool_executor.test.rs`
//! 的 `process_tool_calls_async` 组（中止、交互 break、闸门拒止零事件、协议失败、
//! 待用户审批的 `WaitingUserAction`）与事件批序断言。

use super::*;

// ── 本段私有的广播/记账原语（逐字来自拆分前的 tool_executor.rs）──────────────
/// 记录协议级工具调用失败（工具调用 id/name 缺失或非法）。
///
/// 不再简单跳过：跳过会让已落库的 ToolCall 节点没有结果子节点，
/// 下一轮请求携带"无结果的 tool_call"触发 provider 400（Bug 2 同类问题）。
/// 处理口径与普通工具执行失败一致（失败属信息性）：
/// - 生成一条 `role=Tool` 的错误结果子节点（Completed + success=false），错误内容喂回 LLM；
/// - 生成父 ToolCall 节点的失败补丁（failure_kind=error）并广播。
///
/// 两条消息分别加入 tool_messages / parent_updates，由调用方统一持久化。
async fn record_protocol_failure(
    sink: &ExecEventSink,
    tool_call_id: &str,
    error_text: &str,
    tool_messages: &mut Vec<ChatNode>,
    parent_updates: &mut Vec<ChatNode>,
    context_messages: &[ChatNode],
) {
    let result_msg_id = uuid::Uuid::new_v4().to_string();
    let mut tool_msg = llm_build_tool_message(
        tool_call_id,
        &format!("Error: {error_text}"),
        Some(false),
        Some(result_msg_id),
        // 协议级失败：调用本身的 id/name 就非法，工具名**无从说起**——主体由
        // `attributed` 按会话主体兜底（[T-147d-iii]）。
        None,
    );
    // 失败属信息性：结果以 Completed 留在上下文（Failed 会被 get_context_messages
    // 过滤，导致"孤儿 tool 结果"使下一轮 LLM 请求非法）。
    tool_msg.status = Some(NodeStatus::Completed);

    llm_emit_message(sink, tool_msg.clone()).await;

    // 父 ToolCall 终态——**完整快照**，两种情形：
    // - id 合法但 name/参数非法：ToolCallDelta 已广播过完整节点（在权威转写里），
    //   取副本应用终态；
    // - id 本身缺失（兜底 llm_short_id）：不存在任何父节点——构造**完整**的 ToolCall
    //   终态节点（name/参数为 None 是诚实表达），`close_turn` 会把它补进转写并
    //   落库，结果子节点因此有真实的父节点，不再悬空。
    let parent_update = match context_messages.iter().find(|m| m.id == tool_call_id) {
        Some(p) => {
            let mut full = p.clone();
            full.status = Some(NodeStatus::Completed);
            let mut meta = full.meta.clone().unwrap_or_else(|| json!({}));
            if let Some(obj) = meta.as_object_mut() {
                obj.insert("success".into(), json!(false));
                obj.insert("failure_kind".into(), json!("error"));
            }
            full.meta = Some(meta);
            full.error = Some(error_text.to_string());
            full
        }
        None => ChatNode {
            id: tool_call_id.to_string(),
            role: Some(NodeRole::Assistant),
            msg_type: Some(NodeKind::Skeleton),
            status: Some(NodeStatus::Completed),
            error: Some(error_text.to_string()),
            meta: Some(json!({
                "success": false,
                "failure_kind": "error",
                "style_tag": CHAT_STYLE_TAG_TOOL,
            })),
            ..Default::default()
        },
    };
    llm_emit_state(sink, parent_update.clone()).await;

    plugin_info!(
        "session",
        "[Tool] Protocol failure recorded as failed tool call: {}",
        error_text
    );

    tool_messages.push(tool_msg);
    parent_updates.push(parent_update);
}

/// 广播父 ToolCall 的「执行中」状态（`Streaming`）。
///
/// ## 为什么需要它
///
/// ToolCall 节点的生命周期**不在模型停止输出参数时结束**——那只是"参数齐了"。
/// 它此后还要经历一段**执行窗口**：一次编译、一次网络请求、一个子智能体跑完，
/// 往往比参数流式本身长得多。若此处不下发状态，前端在整段窗口里没有任何
/// 「运行中」迹象：参数流完画面静止，直到结果突然出现——用户无法判断
/// 是"还在跑"还是"卡死了"。
///
/// 状态机（`symbio/src/plugins/session/docs/018-node-state-streaming.md` §2.3）：
/// `pending → streaming（参数流式 + 执行）→ waiting_user_action → completed / failed`。
/// 因此 `Streaming` 在这里**不是**"正在接收流"，而是"这次调用正在跑"。
///
/// 同时写入 `meta.started_at`（毫秒）：前端据此显示"已运行 47s"——「运行中」是断言，
/// 时长才是**判据**，用户靠它区分"还在跑"与"卡住了"。用节点属性承载而不是前端
/// 自己计时，切会话/重连后时长仍然连续（前端计时会从零重来）。
async fn emit_tool_running(
    sink: &ExecEventSink,
    context_messages: &[ChatNode],
    tool_call_id: &str,
) {
    // 完整快照：从权威转写取父 ToolCall 副本（id / 父子关系 / name / 参数都在），
    // 应用「运行中」状态与 meta.started_at 后整条广播。找不到 = 协议违例
    // （ToolCallDelta 必然先广播过完整节点），报错并跳过——不造半截节点。
    let Some(parent) = context_messages.iter().find(|m| m.id == tool_call_id) else {
        plugin_error!(
            "session",
            "[Tool] emit_tool_running: 转写中不存在工具调用 {}（协议违例），跳过运行态广播",
            tool_call_id
        );
        return;
    };
    let mut running = parent.clone();
    running.status = Some(NodeStatus::Streaming);
    let mut meta = running.meta.clone().unwrap_or_else(|| json!({}));
    if let Some(obj) = meta.as_object_mut() {
        obj.insert(
            "started_at".into(),
            json!(crate::symbio_core::clock_now_ms()),
        );
    }
    running.meta = Some(meta);
    llm_emit_state(sink, running).await;
}

/// 顺序处理一批工具调用（阶段②）：以下 `for` 循环逐字来自拆分前的
/// `process_tool_calls_async` 主体，`if` 条件零变化。
///
/// 父 ToolCall 终态——**完整消息**（发射端从权威转写取副本应用终态）。
/// 返回给调用方做内存镜像整条替换 + 随 persist_messages 落库。
// 拆分段以局部绑定沿用拆分前的变量名（`decls` 等），循环体逐字未动——
// `&decls` 这类取放在拆分前是 `&Vec`/`&T`、现在是 `&&T`，语义同一，clippy 的
// needless_borrow 在这里是对「纯结构拆分」的误报（T-179 判据 1：不动代码行）。
#[allow(clippy::needless_borrow)]
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_batch(
    tool_calls: Vec<TurnToolCallInfo>,
    parent: &Option<Arc<dyn Plugin>>,
    sink: &ExecEventSink,
    abort: &ExecAbortSignal,
    ctx: Arc<dyn PluginInvokeRequest>,
    context_messages: &[ChatNode],
    tool_archive_dir: &std::path::Path,
    readings: &mut GateReadings,
    delegations: &mut Vec<Delegation>,
    tasks: &mut Vec<TaskDeclaration>,
    breaks: &mut Vec<&'static str>,
) -> (Vec<ChatNode>, Vec<ChatNode>) {
    let mut tool_messages = Vec::new();
    let mut parent_updates: Vec<ChatNode> = Vec::new();
    // 阶段①的读数按本段原变量名接线——循环体内的代码因此与拆分前逐字一致。
    let mode = readings.mode.as_str();
    let gate_started = readings.gate_started;
    let breaker = &readings.breaker;
    let elimination = &readings.elimination;
    let task_table = &readings.task_table;
    let decls = &readings.decls;
    let seen_calls = &mut readings.seen_calls;

    for tc in tool_calls {
        if abort.is_aborted() {
            break;
        }
        // 交互模式下，前一个工具待审批/失败 → 中止本批剩余（用户需逐个处理）
        if mode == "interactive" && !parent_updates.is_empty() {
            let last_blocked = parent_updates
                .last()
                .map(|p| {
                    p.status == Some(NodeStatus::WaitingUserAction)
                        || p.meta
                            .as_ref()
                            .and_then(|m| m.get("failure_kind"))
                            .and_then(|v| v.as_str())
                            .map(|k| {
                                // pending 两种 + error 都算「本批剩余不要继续跑」
                                crate::symbio_core::failure_kind::is_pending(k)
                                    || k == crate::symbio_core::failure_kind::ERROR
                            })
                            .unwrap_or(false)
                })
                .unwrap_or(false);
            if last_blocked {
                plugin_info!(
                    "session",
                    "[Tool] 交互模式下前一个工具待用户恢复，中止本批剩余工具"
                );
                break;
            }
        }

        // 工具调用 id 缺失/非法 → 作为工具调用失败处理（不跳过）。
        // 正常情况下 TurnToolCallAccumulator 已保证 id 非空；此分支为兜底防御。
        // 注意：兜底 id 仅用于挂载失败结果与父节点补丁（保持结构完整）。
        let id = match tc.id.as_ref() {
            Some(id) if !id.trim().is_empty() => id.clone(),
            _ => {
                plugin_error!(
                    "session",
                    "[Tool] 协议错误：工具调用 ID 缺失/非法，记为失败调用"
                );
                record_protocol_failure(
                    sink,
                    &llm_short_id(),
                    "模型未返回有效的工具调用 ID（协议错误）",
                    &mut tool_messages,
                    &mut parent_updates,
                    context_messages,
                )
                .await;
                continue;
            }
        };
        // 工具名缺失/非法 → 同样作为工具调用失败处理（不跳过），
        // 避免已落库的 ToolCall 节点没有结果子节点。
        let name = match tc.name.as_ref() {
            Some(name) if !name.trim().is_empty() => name.clone(),
            _ => {
                plugin_error!("session",
                    format!(
                        "Protocol Error: Tool call name missing/invalid, recording as failed tool call. ID: {id}"
                    )
                );
                record_protocol_failure(
                    sink,
                    &id,
                    "模型未返回有效的工具名称（协议错误）",
                    &mut tool_messages,
                    &mut parent_updates,
                    context_messages,
                )
                .await;
                continue;
            }
        };

        // 同参重复调用的判据（[T-140] 步 2）：签名与落库节点里的参数文本同源
        // （[`tool_call_args_text`]），于是「上下文里的那一次」与「这一次」可比。
        // 只登记、只回报——不拦截、不改执行结果。
        let signature = call_signature(&name, &tool_call_args_text(&tc));
        let repeat_note = seen_calls
            .iter()
            .position(|seen| *seen == signature)
            .map(|first| {
                format!(
                    "[重复调用] 与本次会话第 {} 次工具调用同工具同参数（{name}），结果相同。原样重发不会带来新信息：\
                     需要后续结果请用 offset 翻页，需要缩小范围请换 pattern 或限定目录。",
                    first + 1
                )
            });
        seen_calls.push(signature);
        if repeat_note.is_some() {
            // 「要不要真的拦」留给实际数据判（见 T-140），先把绕圈的次数记进日志。
            plugin_warn!("session", "[Tool] 同参重复调用：{name}（调用 id={id}）");
        }

        // 参数 JSON 非空却解析失败（典型：被 max_tokens 截断）→ **拒绝执行**。
        // 旧行为是带着占位 `{}` 继续执行，工具必然报「缺少必填参数」，而这条错误
        // 对模型毫无信息量（它认为自己发了完整参数），于是原样重试 → 卡思考死循环。
        // 这里以协议错误形态回报，明确告诉模型「参数残破，需重发完整调用」。
        if let Some(raw) = tc.parse_error.as_ref() {
            let preview: String = raw.chars().take(400).collect();
            let truncated = raw.chars().count() > 400;
            plugin_error!(
                "session",
                "[Tool] 协议错误：工具调用参数 JSON 解析失败，拒绝执行。ID: {id}, name: {name}"
            );
            record_protocol_failure(
                sink,
                &id,
                &format!(
                    "工具参数 JSON 解析失败（可能被长度上限截断），本次调用未执行。\
                     只有完整且可解析的 JSON 参数才会被执行，请勿以相同内容重试。\n\
                     参数原文（{} 字符{}）：{preview}",
                    raw.chars().count(),
                    if truncated { "，已截断展示" } else { "" }
                ),
                &mut tool_messages,
                &mut parent_updates,
                context_messages,
            )
            .await;
            // 约还没落地就执行不了 = 没履约（`agent_run` 记账见 `note_delegation`）。
            note_delegation(
                delegations,
                &decls,
                &id,
                &name,
                &tc.arguments,
                false,
                "参数 JSON 解析失败，未执行",
            );
            continue;
        }

        let result_msg_id = uuid::Uuid::new_v4().to_string();

        // ── 单响应超限丢弃（megabatch 守卫的**反馈面**）────────────────────
        //
        // 上限判在流的源头（model 插件 stream.rs），但处置是反馈而不是失败：
        // 被丢弃的调用各落一条结果子节点，正文即流层给的提示——模型在下一轮从
        // 工具结果里读到「超限了、后面的调用被丢了、该分批发起」。整轮 Err 的
        // 旧方案会让模型对发生了什么一无所知（流程无故中断），已否决。
        // 放在闸门之前：丢弃的调用不该消耗熔断预算，也不该跑钩子。
        if let Some(notice) = tc.overflow_notice.as_ref() {
            plugin_warn!(
                "session",
                "[Tool] 单响应工具调用数超限，调用被丢弃（不执行）：{id} / {name}"
            );
            let mut tool_msg = llm_build_tool_message(
                &id,
                notice,
                Some(false),
                Some(result_msg_id.clone()),
                Some(&name),
            );
            // 与 `not_executed_result` 同口径：「没跑」不是「跑失败」，以 Completed
            // 留在上下文；成因由 `failure_kind` 承载（**并入**，不盖掉 tool_name）。
            tool_msg.status = Some(NodeStatus::Completed);
            if let Some(obj) = tool_msg.meta.as_mut().and_then(|m| m.as_object_mut()) {
                obj.insert("failure_kind".into(), json!("overflow_dropped"));
            }
            tool_messages.push(tool_msg);
            // 结果子节点必须有父节点终态配套（同 S20.1 不变量）：否则前端永远转圈。
            if let Some(parent_update) = emit_parent_finalized(
                sink,
                context_messages,
                &id,
                NodeStatus::Completed,
                json!({ "success": false, "failure_kind": "overflow_dropped" }),
                None,
            )
            .await
            {
                parent_updates.push(parent_update);
            }
            // 没跑 = 没履约（`agent_run` 的承诺记账见 `note_delegation`）。
            note_delegation(
                delegations,
                &decls,
                &id,
                &name,
                &tc.arguments,
                false,
                "单响应工具调用数超限，被系统丢弃未执行",
            );
            continue;
        }

        // ── 闸门表（T-196）：外部执行 / 淘汰 / 任务表，先命中者胜 ────────────
        //
        // 判定紧贴执行窗口之前、**先于 PreToolUse 钩子**：钩子是给「要执行的调用」
        // 准备的用户策略，对一通马上要被拒的调用先跑一遍钩子，等于让拒绝白白付一次
        // 钩子往返（还可能触发钩子自己的副作用）。
        //
        // 四道闸的出口判据**不对称**（S09 §6），差异经 `RefusalTrace` 表达：
        // `Refuse` 零事件、`Break` 入格、淘汰 / 钩子可审计拒绝、任务表不记承诺。
        // 骨架与差异的唯一实现在 `dispatch::gates`（[`GATES`] 是闸门的登记表）。
        let gate_ctx = GateContext {
            gate_started,
            breaker,
            elimination,
            task_table,
            decls,
            name: &name,
        };
        if let Some(refusal) = first_refusal(&gate_ctx) {
            refuse(
                sink,
                context_messages,
                &id,
                &name,
                &tc.arguments,
                &result_msg_id,
                refusal,
                &mut tool_messages,
                &mut parent_updates,
                delegations,
                decls,
                breaks,
            )
            .await;
            continue;
        }

        let pre_output = fire_hook(
            parent,
            HookEvent::PreToolUse {
                tool_name: name.clone(),
                tool_input: tc.arguments.clone(),
            },
            ctx.clone(),
        )
        .await;
        if !pre_output.should_proceed {
            let block_msg = pre_output
                .block_reason
                .unwrap_or_else(|| "Blocked by pre hook".to_string());
            refuse(
                sink,
                context_messages,
                &id,
                &name,
                &tc.arguments,
                &result_msg_id,
                Refusal::blocked(block_msg),
                &mut tool_messages,
                &mut parent_updates,
                delegations,
                decls,
                breaks,
            )
            .await;
            continue;
        }

        // 工具即将真正执行 → 父 ToolCall 置「运行中」。这是整段执行窗口的**唯一**
        // 运行中信号来源，必须紧贴 `execute_tool_async` 之前（`finalize_assistant_turn`
        // 已不再在参数流结束时提前定格）。
        emit_tool_running(sink, context_messages, &id).await;

        let (res, success, mut pending_user_prompt) = execute_tool_async(
            parent,
            &name,
            tc.arguments.clone(),
            &id,
            sink,
            abort,
            result_msg_id.clone(),
            ctx.clone(),
        )
        .await;

        let final_res = res;

        // 代际立约的了结（批⑧）：跑完即知履约与否。**待用户审批的不记**——
        // 承诺还没到可判时刻，抢跑记成履约或违约都是假事实（审批结束后随
        // resume 那一轮再记）。
        if pending_user_prompt.is_none() {
            note_delegation(
                delegations,
                &decls,
                &id,
                &name,
                &tc.arguments,
                success,
                &final_res,
            );
            // 任务表声明（批⑨）：同一条「跑完了才判」的口径。`note_tasks` 自己
            // 过滤工具名与成败，非 `todo_write` / 失败的调用在这里一并被挡掉。
            note_tasks(tasks, &decls, &name, &tc.arguments, success);
        }

        let tool_output = if success {
            serde_json::json!({ "content": final_res.clone() })
        } else {
            serde_json::json!({ "error": final_res.clone() })
        };
        let _post_output = fire_hook(
            parent,
            HookEvent::PostToolUse {
                tool_name: name.clone(),
                tool_input: tc.arguments.clone(),
                tool_output,
            },
            ctx.clone(),
        )
        .await;

        // 工具以「等待用户动作」结束时，**节点在这里构造**：本函数是工具结果节点的
        // 唯一写入者（它持有 `result_msg_id` 与父 ToolCall 的终态）。工具只给载荷
        // （`PendingPrompt`），因此不存在"工具造一个节点、这里再造一个"的双 id 问题。
        let mut tool_msg = if let Some(pending) = pending_user_prompt.take() {
            build_user_prompt_message(&result_msg_id, &id, &pending, &name)
        } else {
            // L0 守卫：超长工具结果存档 + head/tail 摘要，避免单条撑爆上下文窗口
            // （对应"单次工具调用内容太长"的压缩诉求；物理字节上限不再是唯一防线）。
            // 存档目录由 session 预计算（跟随会话生命周期，历史可取回不被 OS 清理）。
            let guarded =
                guard_tool_result(&final_res, DEFAULT_TOOL_RESULT_TOKEN_CAP, tool_archive_dir);
            let mut tool_msg = llm_build_tool_message(
                &id,
                &guarded.text,
                Some(success),
                Some(result_msg_id),
                Some(&name),
            );
            if guarded.truncated {
                let mut meta = tool_msg.meta.clone().unwrap_or_else(|| json!({}));
                meta["tool_result_truncated"] = json!(true);
                if let Some(p) = guarded.archive_path {
                    meta["archive_path"] = json!(p);
                }
                meta["origin_tokens"] = json!(guarded.original_tokens);
                tool_msg.meta = Some(meta);
            }
            tool_msg
        };
        // 重复调用的提示挂在**结果正文末尾**（待用户动作的交互节点不挂——它不是结果）。
        // 放在 L0 守卫之后：超长结果被截断存档时，这条提示仍然完整可见。
        if !tool_msg.is_interaction() {
            if let Some(note) = repeat_note.as_deref() {
                if let Some(MessageContent::Text(text)) = tool_msg.content.as_mut() {
                    text.push('\n');
                    text.push_str(note);
                }
            }
        }
        // 任何模式：工具失败属"信息性"，结果仍以合法 tool 结果（Completed）留在上下文，
        // 让 LLM 看到错误并继续；其父节点在下方也标 Completed（不暂停会话）。
        // 若此处仍标 Failed，则会被 get_context_messages 过滤，导致"孤儿 tool 结果"
        // （父 tool_call 被过滤、结果残留）使下一轮 LLM 请求非法（Bug 2 同类问题）。
        // 仅对普通工具结果生效（user_prompt 走 WaitingUserAction 分支，不受此覆盖）。
        if !success && !tool_msg.is_interaction() {
            tool_msg.status = Some(NodeStatus::Completed);
        }

        if tool_msg.is_interaction() {
            // user_prompt 节点本身即是工具"结果"（待用户审批/回答）：
            // 广播该节点（WaitingUserAction），并把父节点 ToolCall 标为
            // WaitingUserAction（持久化 failure_kind 供 resume 提取）。
            let failure_kind = tool_msg
                .meta
                .as_ref()
                .and_then(|m| m.get("failure_kind"))
                .and_then(|v| v.as_str())
                .unwrap_or(crate::symbio_core::failure_kind::NEEDS_APPROVAL)
                .to_string();

            llm_emit_message(sink, tool_msg.clone()).await;

            // 父 ToolCall 置 WaitingUserAction（完整快照；meta.failure_kind 供 resume 提取）
            if let Some(parent_update) = emit_parent_finalized(
                sink,
                context_messages,
                &id,
                NodeStatus::WaitingUserAction,
                json!({
                    "success": false,
                    "failure_kind": failure_kind,
                }),
                None,
            )
            .await
            {
                parent_updates.push(parent_update);
            }
        } else {
            // 广播工具结果子节点：**一次性节点单帧完成**——内容以 delta 首次传输，
            // 与状态、meta 同帧（meta 一并带全：截断标记 / 存档路径不再只活在存储里）。
            llm_emit_message(sink, tool_msg.clone()).await;

            // 标记父节点最终状态（完整快照）：
            // - 成功 => Completed
            // - 普通工具失败 => Completed（失败属信息性，错误结果作为合法 tool 结果留在
            //   上下文，父节点不再标 Failed，不暂停会话、不触发重试/补参渲染）。
            //   仅真正需要用户输入的 UserPrompt 场景在上方以 WaitingUserAction 处理。
            let (status, meta_extra, error) = if success {
                (NodeStatus::Completed, json!({ "success": true }), None)
            } else {
                (
                    NodeStatus::Completed,
                    json!({
                        "success": false,
                        "failure_kind": "error",
                        "tool_name": name,
                        "args": tc.arguments,
                    }),
                    Some(final_res.clone()),
                )
            };
            if let Some(parent_update) =
                emit_parent_finalized(sink, context_messages, &id, status, meta_extra, error).await
            {
                parent_updates.push(parent_update);
            }
        }

        tool_messages.push(tool_msg);
    }
    (tool_messages, parent_updates)
}
