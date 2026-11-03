//! ②「逐条执行」。
//!
//! 一条顺序流程的中间段：对批内每条调用依序过「协议校验 → 同参提示 → 参数解析 →
//! 超限丢弃 → 宿主闸（外部执行）→ 声明闸（能力包自己的 `before`）→ Pre 钩子 →
//! 执行 → Post 钩子 → 记账 → 结果与父终态广播」。**没有批首读数**：闸门判据、声明位、
//! 主体、会话事实切片都在这一笔债的准备点现取；同参提示现读转写与本批前缀，不养台账。
//!
//! ## 批内执行策略
//!
//! 环**不做**风险档 → 并行/串行的映射（DE-047 D-311）：一条调用准备完就交给执行体，
//! 多笔可在途；并发上限与顺序归 provider 内部。环只保留**配对**职责——结果子节点、
//! 父节点终态、承诺 / 任务记账全部按**调用顺序**逐条收口（[`flush_one`]），所以
//! 「工具结果顺序 = 调用顺序」不变。每条调用的 `Streaming`（运行中）仍在本条开跑前
//! 广播，故「running 先于本条结果」这条关系也不变。
//!
//! 交互模式（`interactive`）例外：审批 / 提问是「一次只处理一张卡」的语义，前一条
//! 待用户动作时本批剩余不得启动——这是**交互语义**，不是并发策略，因此留环。
//!
//! 执行前就终结的出口（协议失败 / 超限丢弃 / 闸门或钩子拒止）**不在准备阶段发
//! 事件**：先 flush 掉此前的在途调用，再按调用顺序发（[`PreTerminal`]）。
//!
//! 判据（「这段错了谁会红」）：批内事件与父节点终态用例红——`tool_executor.test.rs`
//! 的 `process_tool_calls_in_env` 组（中止、交互 break、闸门拒止、协议失败、
//! 待用户审批的 `WaitingUserAction`）、事件批序断言，以及并行执行的用例组。

use futures::stream::{FuturesUnordered, StreamExt};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use symbio_plugin_api::effects::BeforeRequest;

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
    call_id: &str,
    error_text: &str,
    tool_messages: &mut Vec<ChatNode>,
    parent_updates: &mut Vec<ChatNode>,
    context_messages: &[ChatNode],
    principal: &symbio_sdk::chat::principal::Principal,
) {
    let result_node_id = symbio_kernel::event::result_node_id(call_id);
    let mut tool_msg = llm_build_tool_message(
        call_id,
        &format!("Error: {error_text}"),
        Some(false),
        Some(result_node_id),
        // 协议级失败：调用本身的 id/name 就非法，工具名**无从说起**——主体由
        // `attributed` 按会话主体兜底。
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
    //   终态节点（name/参数为 None 是诚实表达），收束那一格会把它补进转写并
    //   落库，结果子节点因此有真实的父节点，不再悬空。
    let parent_update = match context_messages.iter().find(|m| m.id == call_id) {
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
            id: call_id.to_string(),
            principal: Some(principal.clone()),
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
/// 状态机（`crates/symbio-plugin-session/src/docs/105-node-state-streaming.md` §2.3）：
/// `pending → streaming（参数流式 + 执行）→ waiting_user_action → completed / failed`。
/// 因此 `Streaming` 在这里**不是**"正在接收流"，而是"这次调用正在跑"。
///
/// 同时写入 `meta.started_at`（毫秒）：前端据此显示"已运行 47s"——「运行中」是断言，
/// 时长才是**判据**，用户靠它区分"还在跑"与"卡住了"。用节点属性承载而不是前端
/// 自己计时，切会话/重连后时长仍然连续（前端计时会从零重来）。
async fn emit_tool_running(sink: &ExecEventSink, context_messages: &[ChatNode], call_id: &str) {
    // 完整快照：从权威转写取父 ToolCall 副本（id / 父子关系 / name / 参数都在），
    // 应用「运行中」状态与 meta.started_at 后整条广播。找不到 = 协议违例
    // （ToolCallDelta 必然先广播过完整节点），报错并跳过——不造半截节点。
    let Some(parent) = context_messages.iter().find(|m| m.id == call_id) else {
        plugin_error!(
            "session",
            "[Tool] emit_tool_running: 转写中不存在工具调用 {}（协议违例），跳过运行态广播",
            call_id
        );
        return;
    };
    let mut running = parent.clone();
    running.status = Some(NodeStatus::Streaming);
    let mut meta = running.meta.clone().unwrap_or_else(|| json!({}));
    if let Some(obj) = meta.as_object_mut() {
        obj.insert(
            "started_at".into(),
            json!(symbio_sdk::clock::clock_now_ms()),
        );
    }
    running.meta = Some(meta);
    llm_emit_state(sink, running).await;
}

// ── 准备 / 执行 / 收口的三个阶段──────────────────────────────────

/// 一次执行的产出（执行体交回的文本 + 守卫元数据，P1b）。
type ExecOutcome = ExecResult;

/// 在途执行的 future。返回 `(seq, 产出)`——`seq` 是**启动顺序**，收口按它排队。
type ExecFuture<'a> =
    std::pin::Pin<Box<dyn std::future::Future<Output = (u64, ExecOutcome)> + Send + 'a>>;

/// 执行前的分岔：可以跑了，或本条就地终结。
enum PreCheck {
    Ready(Ready),
    Terminal(PreTerminal),
}

/// 过了协议校验 / 同参台账 / 参数解析 / 超限守卫 / 三道闸 / Pre 钩子的调用，
/// 只差执行。
struct Ready {
    id: String,
    name: String,
    args: Value,
    repeat_note: Option<String>,
}

/// 执行前就终结的出口。
///
/// **构造时不发任何事件**：事件由调用方在本批此前的在途调用都收口之后再发，
/// 于是「结果批序 = 调用顺序」不因并行而改变。
enum PreTerminal {
    /// 协议级失败（id / name 缺失或参数 JSON 残破）。
    ProtocolFailure {
        id: String,
        text: String,
        /// 参数残破同样「约了没履约」：(id, name, args)。
        promise: Option<(String, String, Value)>,
    },
    /// 单响应超限被流层丢弃（反馈而不是失败）。
    Overflow {
        id: String,
        name: String,
        args: Value,
        notice: String,
    },
    /// 闸门或 PreToolUse 钩子的拒止。
    Refused {
        id: String,
        name: String,
        args: Value,
        refusal: Refusal,
    },
}

/// 同一签名在此前出现过的**会话内序号**（1 基），没有则 `None`。
///
/// 台账有且只有两处**事实来源**：转写里已有的工具调用请求节点（骨架 + tag `tool`，
/// 正文即定稿参数文本），以及本批在这一次调用之前的调用。环**不养**可变台账——
/// 批首预读删掉后，判据按这一笔债现读（DE-047 D-311）。
///
/// `current_ids` 是本批调用的节点 id：转写在执行前**就已含**这一批 ToolCall 节点，
/// 不排除的话每条调用都会把自己数成「重复」。
fn prior_call_signatures(context_messages: &[ChatNode], current_ids: &[String]) -> Vec<String> {
    context_messages
        .iter()
        .filter(|m| m.is_tool_call() && !current_ids.contains(&m.id))
        .filter_map(|m| {
            let name = m.tool_name()?;
            let args = match m.content.as_ref()? {
                MessageContent::Text(s) => s.as_str(),
                MessageContent::Parts(_) => return None,
            };
            Some(call_signature(name, args))
        })
        .collect()
}

fn repeat_ordinal(
    context_messages: &[ChatNode],
    batch: &[TurnToolCallInfo],
    index: usize,
    current_ids: &[String],
    signature: &str,
) -> Option<usize> {
    let mut ledger = prior_call_signatures(context_messages, current_ids);
    ledger.extend(batch[..index].iter().filter_map(|tc| {
        let name = tc.name.as_deref()?;
        Some(call_signature(name, &tool_call_args_text(tc)))
    }));
    ledger.iter().position(|seen| seen == signature).map(|p| p + 1)
}

/// 一条调用的**准备**：逐字沿用拆分前循环体的前段，只把「发事件」的部分换成
/// [`PreTerminal`]（谁发、什么时候发由调用方按批序决定）。
///
/// 闸门判据在这一笔债自己的派发点上读（不预读整批）：熔断的授权 / 成本台账、
/// 自声明行为位、主体、会话事实切片都只属于这一次判定（DE-047 D-311）。
#[allow(clippy::too_many_arguments)]
async fn prepare_call(
    tc: &TurnToolCallInfo,
    batch: &[TurnToolCallInfo],
    index: usize,
    current_ids: &[String],
    parent: &Option<Arc<dyn Plugin>>,
    ctx: Arc<dyn PluginInvokeRequest>,
    context_messages: &[ChatNode],
    facts: &dyn Store<Event = Event>,
    elimination_revoked: bool,
    decls: &ToolDecls,
) -> PreCheck {
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
            return PreCheck::Terminal(PreTerminal::ProtocolFailure {
                id: llm_short_id(),
                text: "模型未返回有效的工具调用 ID（协议错误）".to_string(),
                promise: None,
            });
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
            return PreCheck::Terminal(PreTerminal::ProtocolFailure {
                id,
                text: "模型未返回有效的工具名称（协议错误）".to_string(),
                promise: None,
            });
        }
    };

    // 同参重复调用的判据：签名与落库节点里的参数文本同源
    // （[`tool_call_args_text`]），于是「上下文里的那一次」与「这一次」可比。
    // **不养可变台账**：此前出现过的调用现读转写节点 + 本批在本次调用之前的调用，
    // 只回报——不拦截、不改执行结果。
    let signature = call_signature(&name, &tool_call_args_text(tc));
    let repeat_note = repeat_ordinal(context_messages, batch, index, current_ids, &signature)
        .map(|first| {
            format!(
                "[重复调用] 与本次会话第 {first} 次工具调用同工具同参数（{name}），结果相同。原样重发不会带来新信息：\
                 需要后续结果请用 offset 翻页，需要缩小范围请换 pattern 或限定目录。"
            )
        });
    if repeat_note.is_some() {
        // 「要不要真的拦」留给实际数据判，先把绕圈的次数记进日志。
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
        return PreCheck::Terminal(PreTerminal::ProtocolFailure {
            text: format!(
                "工具参数 JSON 解析失败（可能被长度上限截断），本次调用未执行。\
                 只有完整且可解析的 JSON 参数才会被执行，请勿以相同内容重试。\n\
                 参数原文（{} 字符{}）：{preview}",
                raw.chars().count(),
                if truncated { "，已截断展示" } else { "" }
            ),
            // 约还没落地就执行不了 = 没履约（承诺的效果由 `record_effect` 交出去，
            // 实现住能力包，见 `symbio_plugin_api::effects`）。
            promise: Some((id.clone(), name.clone(), tc.arguments.clone())),
            id,
        });
    }

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
        return PreCheck::Terminal(PreTerminal::Overflow {
            id,
            name,
            args: tc.arguments.clone(),
            notice: notice.clone(),
        });
    }

    // ── 宿主闸：能不能开窗（外部执行） ───────────────────────────────
    //
    // 判定紧贴执行窗口之前、**先于 PreToolUse 钩子**：钩子是给「要执行的调用」
    // 准备的用户策略，对一通马上要被拒的调用先跑一遍钩子，等于让拒绝白白付一次
    // 钩子往返（还可能触发钩子自己的副作用）。
    //
    // 判据**按这一笔债现读**（不再批首一次读齐）：`gate_started` 从读判据之前
    // 起算——熔断的 `gate-timeout` 量的是「这份输入已经读了多久」。
    let gate_started = std::time::Instant::now();
    let session_facts = facts.range(Seq::new(0));
    let breaker = BreakerInputs::of(&session_facts, &ctx);
    let gate_ctx = GateContext {
        gate_started,
        breaker: &breaker,
    };
    if let Some(refusal) = first_refusal(&gate_ctx) {
        return PreCheck::Terminal(PreTerminal::Refused {
            id,
            name,
            args: tc.arguments.clone(),
            refusal,
        });
    }

    // ── 声明闸：这个工具自己声明「我受什么管辖」 ─────────────────────
    //
    // 淘汰 / 任务表写侧这类**按能力判**的闸不在宿主：它们各自是能力包的
    // `ToolBehavior::before`，按 `CapabilityMeta.behaviors` 选实现（DE-025 D-219）。
    // 宿主只交出这一笔的结论——主体、控制面撤销值、会话事实切片——不解释
    // 「谁该被拒」。
    let principal = batch_principal(&ctx);
    let before_req = BeforeRequest {
        tool: &name,
        call_id: &id,
        args: &tc.arguments,
        principal: &principal,
        declared: decls.behaviors_of(&name),
        facts: &session_facts,
        readings: symbio_plugin_api::effects::BeforeReadings {
            assign_work_revoked: elimination_revoked,
        },
    };
    if let Some(judged) = symbio_plugin_api::effects::apply_before(&before_req) {
        return PreCheck::Terminal(PreTerminal::Refused {
            id,
            name,
            args: tc.arguments.clone(),
            refusal: Refusal::from_before(judged),
        });
    }

    let pre_output = fire_hook(
        parent,
        HookEvent::PreToolUse {
            tool_name: name.clone(),
            tool_input: tc.arguments.clone(),
        },
        ctx,
    )
    .await;
    if !pre_output.should_proceed {
        let block_msg = pre_output
            .block_reason
            .unwrap_or_else(|| "Blocked by pre hook".to_string());
        return PreCheck::Terminal(PreTerminal::Refused {
            id,
            name,
            args: tc.arguments.clone(),
            refusal: Refusal::blocked(block_msg),
        });
    }

    PreCheck::Ready(Ready {
        id,
        name,
        args: tc.arguments.clone(),
        repeat_note,
    })
}

/// 发一条**执行前终结**的出口。调用方保证：此前的在途调用都已按序收口。
#[allow(clippy::too_many_arguments)]
async fn emit_terminal(
    sink: &ExecEventSink,
    context_messages: &[ChatNode],
    terminal: PreTerminal,
    tool_messages: &mut Vec<ChatNode>,
    parent_updates: &mut Vec<ChatNode>,
    effects: &mut Vec<EffectInput>,
    decls: &ToolDecls,
    breaks: &mut Vec<&'static str>,
    principal: &symbio_sdk::chat::principal::Principal,
) {
    match terminal {
        PreTerminal::ProtocolFailure { id, text, promise } => {
            record_protocol_failure(
                sink,
                &id,
                &text,
                tool_messages,
                parent_updates,
                context_messages,
                principal,
            )
            .await;
            if let Some((id, name, args)) = promise {
                record_effect(
                    effects,
                    decls,
                    &id,
                    &name,
                    &args,
                    false,
                    "",
                    "参数 JSON 解析失败，未执行",
                );
            }
        }
        PreTerminal::Overflow {
            id,
            name,
            args,
            notice,
        } => {
            let result_node_id = symbio_kernel::event::result_node_id(&id);
            let mut tool_msg =
                llm_build_tool_message(&id, &notice, Some(false), Some(result_node_id), Some(&name));
            // 与 `not_executed_result` 同口径：「没跑」不是「跑失败」，以 Completed
            // 留在上下文；成因由 `failure_kind` 承载（**并入**，不盖掉 tool_name）。
            tool_msg.status = Some(NodeStatus::Completed);
            if let Some(obj) = tool_msg.meta.as_mut().and_then(|m| m.as_object_mut()) {
                obj.insert("failure_kind".into(), json!("overflow_dropped"));
            }
            tool_messages.push(tool_msg);
            // 结果子节点必须有父节点终态配套（同工具终态不变量）：否则前端永远转圈。
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
            // 没跑 = 没履约（效果输入由 `record_effect` 交出，实现住能力包）。
            record_effect(
                effects,
                decls,
                &id,
                &name,
                &args,
                false,
                "",
                "单响应工具调用数超限，被系统丢弃未执行",
            );
        }
        PreTerminal::Refused {
            id,
            name,
            args,
            refusal,
        } => {
            refuse(
                sink,
                context_messages,
                &id,
                &name,
                &args,
                refusal,
                tool_messages,
                parent_updates,
                effects,
                decls,
                breaks,
            )
            .await;
        }
    }
}

/// 一条**已执行**调用的收口：记账 → Post 钩子 → 结果节点 → 父节点终态。
///
/// 逐字沿用拆分前循环体的后段；并行的唯一影响是**谁先到这里由 [`flush_one`]
/// 决定**（始终按调用顺序），本函数内部不感知并发。
#[allow(clippy::too_many_arguments)]
async fn finish_call(
    ready: Ready,
    out: ExecOutcome,
    parent: &Option<Arc<dyn Plugin>>,
    sink: &ExecEventSink,
    ctx: &Arc<dyn PluginInvokeRequest>,
    context_messages: &[ChatNode],
    effects: &mut Vec<EffectInput>,
    decls: &ToolDecls,
    tool_messages: &mut Vec<ChatNode>,
    parent_updates: &mut Vec<ChatNode>,
) {
    let Ready {
        id,
        name,
        args,
        repeat_note,
    } = ready;
    let result_node_id = symbio_kernel::event::result_node_id(&id);
    // 执行体交回的已是「节点正文 / 记账原文 / 守卫元数据」三份，本层不再裁剪
    // 结果（L0 守卫随执行体迁出，P1b / DE-047 D-308）。
    let ExecResult {
        text: guarded_text,
        raw: final_res,
        success,
        pending: mut pending_user_prompt,
        truncated,
        archive_path,
        origin_tokens,
    } = out;

    // 代际立约的了结：跑完即知履约与否。**待用户审批的不记**——
    // 承诺还没到可判时刻，抢跑记成履约或违约都是假事实（审批结束后随
    // resume 那一轮再记）。
    if pending_user_prompt.is_none() {
        // 效果（承诺 / 任务表 / 文件变更…）**全部**由这一条通路交出去：本处只问
        // 「这次调用发生了什么」，至于它算不算立约、算不算一次任务声明，由能力包
        // 自己声明并实现（DE-025 D-207）。宿主不认识任何工具名。
        record_effect(effects, decls, &id, &name, &args, success, &final_res, "");
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
            tool_input: args.clone(),
            tool_output,
        },
        ctx.clone(),
    )
    .await;

    // 工具以「等待用户动作」结束时，**节点在这里构造**：本函数是工具结果节点的
    // 唯一写入者（它持有 `result_node_id` 与父 ToolCall 的终态）。工具只给载荷
    // （`PendingPrompt`），因此不存在"工具造一个节点、这里再造一个"的双 id 问题。
    let mut tool_msg = if let Some(pending) = pending_user_prompt.take() {
        build_user_prompt_message(&result_node_id, &id, &pending, &name)
    } else {
        // 正文用执行体交回的**已守卫文本**（超长结果由执行体存档 + head/tail 摘要，
        // 存档目录跟随会话生命周期）。
        let mut tool_msg = llm_build_tool_message(
            &id,
            &guarded_text,
            Some(success),
            Some(result_node_id),
            Some(&name),
        );
        if truncated {
            let mut meta = tool_msg.meta.clone().unwrap_or_else(|| json!({}));
            meta["tool_result_truncated"] = json!(true);
            if let Some(p) = archive_path {
                meta["archive_path"] = json!(p);
            }
            meta["origin_tokens"] = json!(origin_tokens);
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
            .unwrap_or(symbio_kernel::capability::failure_kind::NEEDS_APPROVAL)
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
                    "args": args,
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

/// 启动一条执行（`seq` = 启动顺序，收口按它排队）。
fn start_execution<'a>(
    env: &'a RoundEnv,
    seq: u64,
    ready: &Ready,
    parent: &'a Option<Arc<dyn Plugin>>,
    sink: &'a ExecEventSink,
    abort: &'a ExecAbortSignal,
    ctx: &Arc<dyn PluginInvokeRequest>,
) -> ExecFuture<'a> {
    let name = ready.name.clone();
    let id = ready.id.clone();
    let args = ready.args.clone();
    let ctx = ctx.clone();
    Box::pin(async move {
        let out = execute_one(
            symbio_plugin_api::ports::TOOL,
            env,
            parent,
            &name,
            args,
            &id,
            sink,
            abort,
            ctx,
        )
        .await;
        (seq, out)
    })
}

/// 按**启动顺序**收口最早的一条在途调用（期间其余在途照常推进）。
#[allow(clippy::too_many_arguments)]
async fn flush_one<'a>(
    inflight: &mut FuturesUnordered<ExecFuture<'a>>,
    order: &mut VecDeque<u64>,
    started: &mut HashMap<u64, (Ready, ToolDecls)>,
    done: &mut HashMap<u64, ExecOutcome>,
    sink: &ExecEventSink,
    parent: &Option<Arc<dyn Plugin>>,
    ctx: &Arc<dyn PluginInvokeRequest>,
    context_messages: &[ChatNode],
    effects: &mut Vec<EffectInput>,
    tool_messages: &mut Vec<ChatNode>,
    parent_updates: &mut Vec<ChatNode>,
) {
    let Some(&target) = order.front() else {
        return;
    };
    while !done.contains_key(&target) {
        match inflight.next().await {
            Some((seq, out)) => {
                done.insert(seq, out);
            }
            // 在途集合已空但目标还没回来：调用方保证不会发生。静默返回好过
            // 在这里 panic（工具批处理不能把会话带崩）。
            None => return,
        }
    }
    let Some(out) = done.remove(&target) else {
        return;
    };
    // 声明位随这一笔债走（按债现读，不再整批复用一份）。
    let Some((ready, decls)) = started.remove(&target) else {
        return;
    };
    order.pop_front();
    finish_call(
        ready,
        out,
        parent,
        sink,
        ctx,
        context_messages,
        effects,
        &decls,
        tool_messages,
        parent_updates,
    )
    .await;
}

/// 把在途调用全部按序收口（串行边界、终结出口、中止与批尾共用）。
#[allow(clippy::too_many_arguments)]
async fn flush_all<'a>(
    inflight: &mut FuturesUnordered<ExecFuture<'a>>,
    order: &mut VecDeque<u64>,
    started: &mut HashMap<u64, (Ready, ToolDecls)>,
    done: &mut HashMap<u64, ExecOutcome>,
    sink: &ExecEventSink,
    parent: &Option<Arc<dyn Plugin>>,
    ctx: &Arc<dyn PluginInvokeRequest>,
    context_messages: &[ChatNode],
    effects: &mut Vec<EffectInput>,
    tool_messages: &mut Vec<ChatNode>,
    parent_updates: &mut Vec<ChatNode>,
) {
    while !order.is_empty() {
        flush_one(
            inflight,
            order,
            started,
            done,
            sink,
            parent,
            ctx,
            context_messages,
            effects,
            tool_messages,
            parent_updates,
        )
        .await;
    }
}

/// 交互模式下，前一条工具待用户恢复 / 失败 ⇒ 本批剩余不再跑（用户需逐个处理）。
fn interaction_blocked(parent_updates: &[ChatNode]) -> bool {
    parent_updates
        .last()
        .map(|p| {
            p.status == Some(NodeStatus::WaitingUserAction)
                || p.meta
                    .as_ref()
                    .and_then(|m| m.get("failure_kind"))
                    .and_then(|v| v.as_str())
                    .map(|k| {
                        // pending 两种 + error 都算「本批剩余不要继续跑」
                        symbio_kernel::capability::failure_kind::is_pending(k)
                            || k == symbio_kernel::capability::failure_kind::ERROR
                    })
                    .unwrap_or(false)
        })
        .unwrap_or(false)
}

/// 处理一批工具调用：准备 → 执行（只读并行）→ 按调用顺序收口。
///
/// 顺序保证与串行时逐项相同：
/// - **结果批序**：工具结果子节点与父节点终态按调用顺序收口（[`flush_one`]）；
/// - **每条调用的运行态先于其结果**：`Streaming` 在本条开跑前广播
///   （[`emit_tool_running`]）；
/// - **执行前终结的出口**先 flush 在途、再按序发（[`PreTerminal`]）。
///
/// 并行的代价只有跨调用的交错：同组只读调用会先各自亮起运行态，再按序收到
/// 结果（成员各自的 `Streaming` → 结果次序不变）。
///
/// 父 ToolCall 终态——**完整消息**（发射端从权威转写取副本应用终态）。
/// 返回给调用方做内存镜像整条替换 + 随 persist_messages 落库。
///
/// 闸门判据**按这一笔债现读**（不预读整批）：声明位随债走，熔断 / 主体 / 会话
/// 事实切片在 [`prepare_call`] 里现取（DE-047 D-311）。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_batch(
    tool_calls: Vec<TurnToolCallInfo>,
    parent: &Option<Arc<dyn Plugin>>,
    sink: &ExecEventSink,
    abort: &ExecAbortSignal,
    ctx: Arc<dyn PluginInvokeRequest>,
    context_messages: &[ChatNode],
    env: &RoundEnv,
    facts: &dyn Store<Event = Event>,
    elimination_revoked: bool,
    effects: &mut Vec<EffectInput>,
    breaks: &mut Vec<&'static str>,
) -> (Vec<ChatNode>, Vec<ChatNode>) {
    let mut tool_messages = Vec::new();
    let mut parent_updates: Vec<ChatNode> = Vec::new();
    // 本会话主体：协议失败兜底建出的 ToolCall 父节点也归它（结果子节点归工具自己）。
    let principal = symbio_sdk::chat::principal::Principal::from_legacy(
        &symbio_sdk::authz::request_principal(ctx.as_ref()),
    );
    // 本批调用的节点 id：转写里已含它们，同参判定要把自己排除（见
    // [`prior_call_signatures`]）。
    let current_ids: Vec<String> = tool_calls.iter().filter_map(|tc| tc.id.clone()).collect();

    // 在途执行（启动顺序 = 调用顺序）、待收口队列与已完成产出。
    let mut inflight: FuturesUnordered<ExecFuture<'_>> = FuturesUnordered::new();
    let mut order: VecDeque<u64> = VecDeque::new();
    let mut started: HashMap<u64, (Ready, ToolDecls)> = HashMap::new();
    let mut done: HashMap<u64, ExecOutcome> = HashMap::new();
    let mut seq: u64 = 0;
    // 交互模式中止 / 用户中止：不再启动新调用，先把在途的按序收完。
    let mut stop_starting = false;
    let mut i = 0usize;

    while i < tool_calls.len() || !order.is_empty() {
        if !stop_starting && !abort.is_aborted() && i < tool_calls.len() {
            // 交互语义留环：模式取**当前交互上下文**，不再由批首一次性读出。
            let interactive = ctx
                .get(symbio_sdk::keys::MODE)
                .map(|mode| mode == "interactive")
                .unwrap_or(false);
            // 交互模式下，前一个工具待审批/失败 → 中止本批剩余（用户需逐个处理）
            if interactive && !parent_updates.is_empty() && interaction_blocked(&parent_updates) {
                plugin_info!(
                    "session",
                    "[Tool] 交互模式下前一个工具待用户恢复，中止本批剩余工具"
                );
                stop_starting = true;
                continue;
            }

            // 声明位按这一笔债现读：没有 visitor ⇒ 声明为空 ⇒ 声明闸都不命中、
            // 效果端口也无事可做（缺声明与漏声明同形，e2e 兜行为）。
            let decls = ToolDecls::of_metas(&symbio_sdk::capability::tool_metas(&ctx).await);
            match prepare_call(
                &tool_calls[i],
                &tool_calls,
                i,
                &current_ids,
                parent,
                ctx.clone(),
                context_messages,
                facts,
                elimination_revoked,
                &decls,
            )
            .await
            {
                PreCheck::Terminal(terminal) => {
                    // 先让此前的在途调用按序收口，再发本条终结出口；结果批序因此
                    // 与调用顺序一致（判据）。
                    flush_all(
                        &mut inflight,
                        &mut order,
                        &mut started,
                        &mut done,
                        sink,
                        parent,
                        &ctx,
                        context_messages,
                        effects,
                        &mut tool_messages,
                        &mut parent_updates,
                    )
                    .await;
                    emit_terminal(
                        sink,
                        context_messages,
                        terminal,
                        &mut tool_messages,
                        &mut parent_updates,
                        effects,
                        &decls,
                        breaks,
                        &principal,
                    )
                    .await;
                    i += 1;
                }
                PreCheck::Ready(ready) => {
                    i += 1;
                    // 交互模式整批压成串行（审批 / 提问一次只处理一张卡）——这是
                    // 交互语义，不是并发策略；其余模式由执行体自己限流（D-311）。
                    if interactive {
                        // 串行边界：在途先全部按序收口，本调用独占执行窗口。
                        flush_all(
                            &mut inflight,
                            &mut order,
                            &mut started,
                            &mut done,
                            sink,
                            parent,
                            &ctx,
                            context_messages,
                            effects,
                            &mut tool_messages,
                            &mut parent_updates,
                        )
                        .await;
                        emit_tool_running(sink, context_messages, &ready.id).await;
                        let out = execute_one(
                            symbio_plugin_api::ports::TOOL,
                            env,
                            parent,
                            &ready.name,
                            ready.args.clone(),
                            &ready.id,
                            sink,
                            abort,
                            ctx.clone(),
                        )
                        .await;
                        finish_call(
                            ready,
                            out,
                            parent,
                            sink,
                            &ctx,
                            context_messages,
                            effects,
                            &decls,
                            &mut tool_messages,
                            &mut parent_updates,
                        )
                        .await;
                    } else {
                        // 非交互模式：准备完就交给执行体，多笔可在途；上限由
                        // provider 内部（端口声明的并发度）自己收（D-311）。
                        // 工具即将真正执行 → 父 ToolCall 置「运行中」。这是整段执行
                        // 窗口的**唯一**运行中信号来源，必须紧贴执行之前。
                        emit_tool_running(sink, context_messages, &ready.id).await;
                        seq += 1;
                        inflight.push(start_execution(env, seq, &ready, parent, sink, abort, &ctx));
                        order.push_back(seq);
                        started.insert(seq, (ready, decls));
                    }
                }
            }
            continue;
        }

        if !order.is_empty() {
            flush_one(
                &mut inflight,
                &mut order,
                &mut started,
                &mut done,
                sink,
                parent,
                &ctx,
                context_messages,
                effects,
                &mut tool_messages,
                &mut parent_updates,
            )
            .await;
            continue;
        }
        break;
    }

    // 中止 / 交互中断时仍在途的调用：按序收口（它们各自经 [`execute_one`]
    // 的中止臂返回「被用户中止」的结果），未启动的由未执行收口补终态。
    flush_all(
        &mut inflight,
        &mut order,
        &mut started,
        &mut done,
        sink,
        parent,
        &ctx,
        context_messages,
        effects,
        &mut tool_messages,
        &mut parent_updates,
    )
    .await;

    (tool_messages, parent_updates)
}
