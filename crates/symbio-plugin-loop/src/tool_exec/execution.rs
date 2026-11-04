//! 工具**逐笔债**执行：准备 → 运行态 → 单笔执行 → 收口。
//!
//! 环只保留两个职务：超限丢弃的预算反馈与交互模式下「前一笔待用户/失败 ⇒ 后续
//! 不再启动」；协议失败、钩子与快慢判定都在执行体（DE-048 D-311 / D-312）。
//! 「没跑到」的占位结果子节点不在这里收口：节点是事实的投影，环只写
//! `failed{why}`，展示由投影器渲染。
//!
//! 载荷装配也住在本模块（环只交值，不拼世界信封键）。

use std::sync::Arc;

use serde_json::{json, Value};
use symbio_kernel::event::{Event, Seq};
use symbio_kernel::exec::{ExecAbortSignal, ExecEventSink};
use symbio_kernel::llm::TurnToolCallInfo;
use symbio_kernel::plugin::{Plugin, PluginInvokeRequest, PluginInvokeRequestExt, PLUGIN_PARENT};
use symbio_kernel::store::Store;
use symbio_plugin_api::effects::{BeforeRefusal, BeforeRequest, EffectInput};
use symbio_plugin_api::port::{PendingPrompt, PortExecContext, PortInput};
use symbio_plugin_api::ports;
use symbio_sdk::chat::chat_node::{
    ChatNode, MessageContent, NodeKind, NodeStatus, CHAT_STYLE_TAG_TOOL,
};
use symbio_sdk::llm::llm_emit_state;
use symbio_sdk::llm::turn::{llm_emit_message, llm_short_id};
use symbio_sdk::turn::node_build::{llm_build_tool_message, tool_call_args_text};
use {
    symbio_plugin_api::plugin_error, symbio_plugin_api::plugin_info, symbio_plugin_api::plugin_warn,
};

use super::decls::ToolDecls;
use super::effects::record_effect;
use super::gate_inputs::GateInputs;

/// 调用信封的协议读数：环只搬原始值，判定在执行体（D-312）。
#[derive(Debug, Clone, Default)]
pub(crate) struct ProtocolInput {
    pub(crate) id_missing: bool,
    pub(crate) name_missing: bool,
    pub(crate) parse_error: Option<String>,
}

impl ProtocolInput {
    pub(crate) fn is_failure(&self) -> bool {
        self.id_missing || self.name_missing || self.parse_error.is_some()
    }
}

/// 中止的成因词：与 [`NOT_EXECUTED`] 同属「没跑到」的词表。
pub(crate) const ABORTED: &str = "aborted";
/// 未执行的成因词：租约超时 / 等待用户前没派到的调用由环写这条 `failed` 事实。
pub(crate) const NOT_EXECUTED: &str = "not_executed";

/// 这笔结果是否属于「没跑到」（中止 / 未执行）——决定关账写 `failed` 还是 `response`。
pub(crate) fn is_unanswered(failure_kind: &str) -> bool {
    failure_kind == ABORTED || failure_kind == NOT_EXECUTED
}

// ── 共用收口原语（逐字来自拆分前的 tool_executor.rs）────────────────────────
/// 把父 ToolCall 的**完整副本**定稿为终态并广播。
///
/// 完整快照纪律：从权威转写取副本（id / 父子关系 / name / 参数都在）→
/// 应用状态、合并 meta、设置 error → 整条广播。找不到 = 协议违例
/// （ToolCallDelta 必然先广播过完整节点），报错并返回 `None`——调用方跳过
/// 父状态帧，结果子节点照常给出。返回值是已广播的完整消息，调用方把它
/// 收进 `parent_updates` 供内存镜像同步与落库（整条替换，非补丁合并）。
async fn emit_parent_finalized(
    sink: &ExecEventSink,
    context_messages: &[ChatNode],
    tool_call_id: &str,
    status: NodeStatus,
    meta_extra: Value,
    error: Option<String>,
) -> Option<ChatNode> {
    let Some(parent) = context_messages.iter().find(|m| m.id == tool_call_id) else {
        plugin_error!(
            "session",
            "[Tool] 终态广播: 转写中不存在工具调用 {}（协议违例），跳过父状态帧",
            tool_call_id
        );
        return None;
    };
    let mut full = parent.clone();
    full.status = Some(status);
    // meta 在**发射端**并入权威副本（读-改-写发生在状态所有者处），
    // 广出去的 meta 因此是完整对象，接收端只做整条替换。
    let mut meta = full.meta.clone().unwrap_or_else(|| json!({}));
    if let (Some(obj), Some(new)) = (meta.as_object_mut(), meta_extra.as_object()) {
        for (k, v) in new {
            obj.insert(k.clone(), v.clone());
        }
    } else {
        meta = meta_extra;
    }
    full.meta = Some(meta);
    full.error = error;
    // 终态帧只带状态 / 元数据 / 错误，**不带正文**：ToolCall 的参数正文已在
    // 流式阶段以 delta 逐帧上线。
    llm_emit_state(sink, full.clone()).await;
    Some(full)
}

// ── 拒止骨架（原 `gates.rs`；能力包 `before` 的出口）─────────────────────
/// 本条债的**主体**：声明闸按它问（`AGENT_ID` 的会话族地址，未绑定即 `session/main`）。
///
/// 与 `request_principal` 同源：判断与落笔因此指向同一个主体，不会出现「按 A 判、
/// 以 B 记」的两套主体。
pub(crate) fn batch_principal(ctx: &Arc<dyn PluginInvokeRequest>) -> String {
    symbio_sdk::authz::principal_of(ctx.get(symbio_sdk::keys::AGENT_ID).as_deref())
}

/// 一道闸的拒因：放行是 `None`，拒止是这里的一整份交代。
#[derive(Debug, Clone)]
pub(crate) struct Refusal {
    /// 闸名（日志与测试的稳定标识）。
    pub(crate) gate: &'static str,
    /// 回灌模型的完整拒因（结果正文 = `"{prefix}: {reason}"`）。
    pub(crate) reason: String,
    /// 结果正文前缀：`Refused`（闸门）。
    pub(crate) prefix: &'static str,
    /// `meta.failure_kind`（约定字段，控制流不靠猜正文）。
    pub(crate) kind: &'static str,
    /// 是否记违约承诺（任务表闸明确不记：工具没跑，声明无从说起）。
    pub(crate) notes_delegation: bool,
    /// 违约承诺的记账理由；`None` = 用拒因本身。钩子保留自己的原措辞。
    pub(crate) note_why: Option<String>,
}

impl Refusal {
    /// 声明闸的判定 → 拒止骨架。实现的判定只说「拒 / 为什么 / 记不记承诺」，
    /// 骨架要的 `prefix` 都由宿主按自己的口径补。
    ///
    /// 出口语义由实现自报（`notes_delegation`）：这一位不是宿主对某道闸的特判，
    /// 而是实现对自己这次拒绝**知道多少**的交代——先说「约没见过」的工具，拒了
    /// 也不该记一笔没履约的承诺（DE-025 D-219）。
    pub(crate) fn from_before(refusal: BeforeRefusal) -> Self {
        Self {
            gate: refusal.gate,
            reason: refusal.reason,
            prefix: "Refused",
            kind: refusal.kind,
            notes_delegation: refusal.notes_delegation,
            note_why: None,
        }
    }
}

/// 拒止骨架（**唯一实现**）：拒因 → 结果子节点 → 父节点收敛 → 记账 → 调用方 continue。
///
/// 不变量 1「每个调用以终态收场」与不变量 2「有调用必有结果」在这里兑现一次；
/// 各闸的差异只经 [`Refusal::notes_delegation`] 表达。开窗判据搬进执行体后，
/// 本函数不产生任何入格理由（熔断事实由执行体自己落）。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn refuse(
    sink: &ExecEventSink,
    context_messages: &[ChatNode],
    id: &str,
    name: &str,
    args: &Value,
    refusal: Refusal,
    tool_messages: &mut Vec<ChatNode>,
    parent_updates: &mut Vec<ChatNode>,
    effects: &mut Vec<EffectInput>,
    decls: &ToolDecls,
) {
    plugin_warn!(
        "session",
        "[Tool] {}：{id} / {name}——{reason}",
        refusal.gate,
        reason = refusal.reason
    );

    let tool_msg = llm_build_tool_message(
        id,
        &format!("{}: {}", refusal.prefix, refusal.reason),
        Some(false),
        Some(symbio_kernel::event::result_node_id(id)),
        Some(name),
    );
    tool_messages.push(tool_msg);

    // 拒掉的调用同样必须收敛父节点：只推结果子节点，父节点就永远停在
    // 「运行中」（那条兜底 `finalize_assistant_turn` 已移除）。
    if let Some(parent_update) = emit_parent_finalized(
        sink,
        context_messages,
        id,
        NodeStatus::Completed,
        json!({ "success": false, "failure_kind": refusal.kind }),
        None,
    )
    .await
    {
        parent_updates.push(parent_update);
    }

    // 没跑 = 没履约（效果输入由 [`record_effect`] 交出，实现住能力包）；任务表闸
    // 明确不记——工具没跑，声明无从说起。
    if refusal.notes_delegation {
        let why = refusal.note_why.as_deref().unwrap_or(&refusal.reason);
        record_effect(effects, decls, id, name, args, false, "", why);
    }
}

// ── 本段私有的广播/记账原语（逐字来自拆分前的 tool_executor.rs）──────────────
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

/// 执行前的分岔：可以跑了，或本条就地终结。
enum PreCheck {
    Ready(Ready),
    Terminal(PreTerminal),
}

/// 过了超限守卫 / 声明闸的调用，只差执行。
struct Ready {
    id: String,
    name: String,
    args: Value,
    repeat_note: Option<String>,
    /// 授权 / 成本的中性输入：随这笔债交给执行体，环不据此拒发。
    gate: GateInputs,
    /// 协议读数：环只搬原始值，判与措辞在执行体（DE-048 D-312）。
    protocol: ProtocolInput,
}

/// 执行前就终结的出口（构造时不发事件；逐笔驱动下由调用方立即发出）。
enum PreTerminal {
    /// 单响应超限被流层丢弃（反馈而不是失败）。
    Overflow {
        id: String,
        name: String,
        args: Value,
        notice: String,
    },
    /// 声明闸的拒止（能力包自己的 `before`）。
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
/// 批首预读删掉后，判据按这一笔债现读（DE-048 D-311）。
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
    ledger
        .iter()
        .position(|seen| seen == signature)
        .map(|p| p + 1)
}

/// 一条调用的**准备**：协议读数与超限守卫在这里收，世界判定在执行体。
///
/// 授权 / 成本的中性输入在这一笔债自己的派发点上读（不预读整批）：自声明行为位、
/// 主体、会话事实切片都只属于这一次判定（DE-048 D-311）；开窗判定在执行体。
#[allow(clippy::too_many_arguments)]
async fn prepare_call(
    tc: &TurnToolCallInfo,
    batch: &[TurnToolCallInfo],
    index: usize,
    current_ids: &[String],
    ctx: Arc<dyn PluginInvokeRequest>,
    context_messages: &[ChatNode],
    facts: &dyn Store<Event = Event>,
    elimination_revoked: bool,
    decls: &ToolDecls,
) -> PreCheck {
    // 协议读数只搬原始值：id 缺失时给一个兜底 id 挂结果，判在执行体（D-312）。
    let id_missing = !matches!(tc.id.as_ref(), Some(id) if !id.trim().is_empty());
    let id = if id_missing {
        llm_short_id()
    } else {
        tc.id.clone().unwrap_or_default()
    };
    let name = tc.name.clone().unwrap_or_default();
    let protocol = ProtocolInput {
        id_missing,
        name_missing: name.trim().is_empty(),
        parse_error: tc.parse_error.clone(),
    };
    let protocol_failed = protocol.is_failure();

    // 同参重复调用只在**可执行的调用**上提示；协议级失败不进入世界判断。
    // 判据与落库节点里的参数文本同源（[`tool_call_args_text`]），不养可变台账：
    // 此前出现过的调用现读转写节点 + 本批在本次调用之前的调用，只回报、不拦截。
    let repeat_note = if protocol_failed {
        None
    } else {
        let signature = call_signature(&name, &tool_call_args_text(tc));
        let note = repeat_ordinal(context_messages, batch, index, current_ids, &signature).map(
            |first| {
                format!(
                    "[重复调用] 与本次会话第 {first} 次工具调用同工具同参数（{name}），结果相同。原样重发不会带来新信息：\
                     需要后续结果请用 offset 翻页，需要缩小范围请换 pattern 或限定目录。"
                )
            },
        );
        if note.is_some() {
            // 「要不要真的拦」留给实际数据判，先把绕圈的次数记进日志。
            plugin_warn!("session", "[Tool] 同参重复调用：{name}（调用 id={id}）");
        }
        note
    };

    // ── 单响应超限丢弃（megabatch 守卫的**反馈面**）────────────────────
    //
    // 上限判在流的源头（model 插件 stream.rs），但处置是反馈而不是失败：
    // 被丢弃的调用各落一条结果子节点，正文即流层给的提示——模型在下一轮从
    // 工具结果里读到「超限了、后面的调用被丢了、该分批发起」。整轮 Err 的
    // 旧方案会让模型对发生了什么一无所知（流程无故中断），已否决。
    // 放在闸门之前：丢弃的调用不该消耗熔断预算，也不该跑钩子。
    if !protocol_failed {
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
    }

    // ── 开窗判据：环只读账，不下判 ───────────────────────────────────
    //
    // 授权矩阵与成本台账是环的职务（能力包读不到的 E 类语法 + 声明视图），但它们
    // 只折成**中性输入**；快慢、开窗与「熔断中不发」的事实归执行体（DE-039 §6，
    // DE-048 D-311）。环按端口名派发，不预判、不拒发。
    let session_facts = facts.range(Seq::new(0));
    let gate = GateInputs::of(&session_facts, &ctx);

    // ── 声明闸：这个工具自己声明「我受什么管辖」 ─────────────────────
    //
    // 淘汰 / 任务表写侧这类**按能力判**的闸不在宿主：它们各自是能力包的
    // `ToolBehavior::before`，按 `CapabilityMeta.behaviors` 选实现（DE-025 D-219）。
    // 宿主只交出这一笔的结论——主体、控制面撤销值、会话事实切片——不解释
    // 「谁该被拒」。协议级失败的调用信封本身不成立，世界判断无从谈起，直接交给
    // 执行体判格式。
    if !protocol_failed {
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
    }

    PreCheck::Ready(Ready {
        id,
        name,
        args: tc.arguments.clone(),
        repeat_note,
        gate,
        protocol,
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
) {
    match terminal {
        PreTerminal::Overflow {
            id,
            name,
            args,
            notice,
        } => {
            let result_node_id = symbio_kernel::event::result_node_id(&id);
            let mut tool_msg = llm_build_tool_message(
                &id,
                &notice,
                Some(false),
                Some(result_node_id),
                Some(&name),
            );
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
            )
            .await;
        }
    }
}

/// 一条**已执行**调用的收口：记账 → 结果节点 → 父节点终态。
///
/// 钩子（PostToolUse）已在执行体触发；环只按这一笔债的产出配对节点。
#[allow(clippy::too_many_arguments)]
async fn finish_call(
    ready: Ready,
    out: ExecOutcome,
    sink: &ExecEventSink,
    context_messages: &[ChatNode],
    principal: &symbio_sdk::chat::principal::Principal,
    effects: &mut Vec<EffectInput>,
    decls: &ToolDecls,
    tool_messages: &mut Vec<ChatNode>,
    parent_updates: &mut Vec<ChatNode>,
    replies: &mut Vec<symbio_plugin_api::port::PortReply>,
) {
    let Ready {
        id,
        name,
        args,
        repeat_note,
        gate: _,
        protocol,
    } = ready;
    let result_node_id = symbio_kernel::event::result_node_id(&id);
    // 执行体交回的已是「节点正文 / 记账原文 / 守卫元数据」三份，本层不再裁剪
    // 结果（L0 守卫随执行体迁出，P1b / DE-048 D-308）。
    let ExecResult {
        text: guarded_text,
        raw: final_res,
        success,
        pending: mut pending_user_prompt,
        truncated,
        archive_path,
        origin_tokens,
        failure_kind: fail_kind,
        why,
        reply,
    } = out;

    // 代际立约的了结：跑完即知履约与否。**待用户审批的不记**——
    // 承诺还没到可判时刻，抢跑记成履约或违约都是假事实（审批结束后随
    // resume 那一轮再记）。
    if pending_user_prompt.is_none() {
        // 效果（承诺 / 任务表 / 文件变更…）**全部**由这一条通路交出去：本处只问
        // 「这次调用发生了什么」，至于它算不算立约、算不算一次任务声明，由能力包
        // 自己声明并实现（DE-025 D-207）。宿主不认识任何工具名。
        record_effect(effects, decls, &id, &name, &args, success, &final_res, &why);
    }

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
    // 应答承载（`D-319`）：只交执行体产出的那一份；没有端口承载的路径
    // （假分发方 / 唤醒结果）由环侧 `land_round` 按旧规则兜底。
    replies.push(reply);

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
        if let Some(parent_update) = finalize_parent(
            sink,
            context_messages,
            &id,
            principal,
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
        } else if protocol.is_failure() {
            // 协议级失败：调用信封本身不成立（缺 id / 缺 name / 参数解析失败），
            // 工具名与参数**无从说起**；只落失败事实，由展示面按事实渲染。
            (
                NodeStatus::Completed,
                json!({
                    "success": false,
                    "failure_kind": fail_kind.as_deref().unwrap_or(
                        symbio_kernel::capability::failure_kind::ERROR
                    ),
                }),
                Some(final_res.clone()),
            )
        } else {
            (
                NodeStatus::Completed,
                json!({
                    "success": false,
                    // 成因由执行体交回（中性词）；环不解释原因，缺省仍是通用失败。
                    "failure_kind": fail_kind.as_deref().unwrap_or(
                        symbio_kernel::capability::failure_kind::ERROR
                    ),
                    "tool_name": name,
                    "args": args,
                }),
                Some(final_res.clone()),
            )
        };
        if let Some(parent_update) = finalize_parent(
            sink,
            context_messages,
            &id,
            principal,
            status,
            meta_extra,
            error,
        )
        .await
        {
            parent_updates.push(parent_update);
        }
    }

    tool_messages.push(tool_msg);
}

/// 父 ToolCall 终态：转写里有它就从副本应用终态；没有（协议级兜底 id）就构造
/// 一条完整终态节点，保证「有调用必有终态」、结果子节点不悬空。
#[allow(clippy::too_many_arguments)]
async fn finalize_parent(
    sink: &ExecEventSink,
    context_messages: &[ChatNode],
    id: &str,
    principal: &symbio_sdk::chat::principal::Principal,
    status: NodeStatus,
    meta_extra: Value,
    error: Option<String>,
) -> Option<ChatNode> {
    if context_messages.iter().any(|m| m.id == id) {
        return emit_parent_finalized(sink, context_messages, id, status, meta_extra, error).await;
    }
    let mut meta = meta_extra;
    if let Some(obj) = meta.as_object_mut() {
        obj.insert("style_tag".into(), json!(CHAT_STYLE_TAG_TOOL));
    }
    let node = ChatNode {
        id: id.to_string(),
        principal: Some(principal.clone()),
        msg_type: Some(NodeKind::Skeleton),
        status: Some(status),
        error,
        meta: Some(meta),
        ..Default::default()
    };
    llm_emit_state(sink, node.clone()).await;
    Some(node)
}

/// 交互模式下，前一条工具待用户恢复 / 失败 ⇒ 后续不再跑（用户需逐个处理）。
/// 判定只读**具体某一笔**的父终态（最后一条），不引入批概念。
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

/// 逐笔驱动这批工具调用：准备 → 运行态 → 执行 → 收口，一笔收完再走下一笔。
///
/// 没有在途表、没有批首读数：并发、顺序、快照都在端口 provider 内部
/// （DE-048 D-311）。环只留两个职务：超限丢弃的预算反馈（[`PreTerminal::Overflow`]）
/// 与交互模式下「前一笔待用户 / 失败 ⇒ 后续不再启动」的逐笔语义
/// （[`interaction_blocked`]）。
///
/// 父 ToolCall 终态——**完整消息**（发射端从权威转写取副本应用终态）。
/// 返回给调用方做内存镜像整条替换 + 随 persist_messages 落库。
///
/// 读数**按这一笔债现取**（不预读整批）：声明位随债走，授权 / 成本的中性输入、
/// 主体、会话事实切片在 [`prepare_call`] 里现取（DE-048 D-311）。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_calls(
    tool_calls: Vec<TurnToolCallInfo>,
    parent: &Option<Arc<dyn Plugin>>,
    sink: &ExecEventSink,
    abort: &ExecAbortSignal,
    ctx: Arc<dyn PluginInvokeRequest>,
    context_messages: &[ChatNode],
    env: &PortExecContext,
    facts: &dyn Store<Event = Event>,
    elimination_revoked: bool,
    effects: &mut Vec<EffectInput>,
    reply_key: Option<&str>,
) -> (
    Vec<ChatNode>,
    Vec<ChatNode>,
    Vec<symbio_plugin_api::port::PortReply>,
) {
    let mut tool_messages = Vec::new();
    let mut parent_updates: Vec<ChatNode> = Vec::new();
    let mut replies: Vec<symbio_plugin_api::port::PortReply> = Vec::new();
    // 本会话主体：协议级兜底建出的 ToolCall 父节点也归它（结果子节点归工具自己）。
    let principal = symbio_sdk::chat::principal::Principal::from_legacy(
        &symbio_sdk::authz::request_principal(ctx.as_ref()),
    );
    // 本批调用的节点 id：转写里已含它们，同参判定要把自己排除（见
    // [`prior_call_signatures`]）。
    let current_ids: Vec<String> = tool_calls.iter().filter_map(|tc| tc.id.clone()).collect();

    for (i, tc) in tool_calls.iter().enumerate() {
        // 用户中止：本笔之后的调用不再启动；未启动的由未执行收口补终态。
        if abort.is_aborted() {
            break;
        }
        let interactive = ctx
            .get(symbio_sdk::keys::MODE)
            .map(|mode| mode == "interactive")
            .unwrap_or(false);
        // 交互语义留环：前一笔待审批/失败 ⇒ 本笔之后的调用不再启动（逐笔判定）。
        if interactive && !parent_updates.is_empty() && interaction_blocked(&parent_updates) {
            plugin_info!(
                "session",
                "[Tool] 交互模式下前一个工具待用户恢复，中止本批剩余工具"
            );
            break;
        }

        // 声明位按这一笔债现读：没有 visitor ⇒ 声明为空 ⇒ 声明闸都不命中、
        // 效果端口也无事可做（缺声明与漏声明同形，e2e 兜行为）。
        let decls = ToolDecls::of_metas(&symbio_sdk::capability::tool_metas(&ctx).await);
        match prepare_call(
            tc,
            &tool_calls,
            i,
            &current_ids,
            ctx.clone(),
            context_messages,
            facts,
            elimination_revoked,
            &decls,
        )
        .await
        {
            PreCheck::Terminal(terminal) => {
                emit_terminal(
                    sink,
                    context_messages,
                    terminal,
                    &mut tool_messages,
                    &mut parent_updates,
                    effects,
                    &decls,
                )
                .await;
            }
            PreCheck::Ready(ready) => {
                // 协议级失败没有「运行中」这一段：调用信封不成立，结果直接收口。
                if !ready.protocol.is_failure() {
                    emit_tool_running(sink, context_messages, &ready.id).await;
                }
                let out = execute_one(
                    symbio_plugin_api::ports::TOOL,
                    env,
                    &ready.gate,
                    parent,
                    &ready.name,
                    ready.args.clone(),
                    &ready.id,
                    &ready.protocol,
                    sink,
                    abort,
                    ctx.clone(),
                    reply_key,
                    ready.repeat_note.as_deref(),
                )
                .await;
                finish_call(
                    ready,
                    out,
                    sink,
                    context_messages,
                    &principal,
                    effects,
                    &decls,
                    &mut tool_messages,
                    &mut parent_updates,
                    &mut replies,
                )
                .await;
            }
        }
    }

    (tool_messages, parent_updates, replies)
}

/// 同参重复调用的判据键：`(工具名, 参数文本)`。
///
/// 参数取 `Value::to_string()`（serde_json 对象按键序规范化），于是模型两次发出
/// 的同一调用——空白与键序不同也无妨——落到同一个键上。
pub(super) fn call_signature(name: &str, args: &str) -> String {
    format!("{name}\u{1}{args}")
}

/// 由载荷构造 user_prompt 节点（`WaitingUserAction`）。
///
/// **本文件是工具结果节点的唯一写入者**：普通结果（`llm_build_tool_message`）与
/// 等待用户（本函数）都出自这里，两者共享同一套 id 约定
/// （`id = result_node_id`、`parent_id = tool_call_id`）。
pub(super) fn build_user_prompt_message(
    result_node_id: &str,
    tool_call_id: &str,
    pending: &PendingPrompt,
    tool_name: &str,
) -> ChatNode {
    ChatNode {
        id: result_node_id.to_string(),
        parent_id: Some(tool_call_id.to_string()),
        // 等待用户的交互卡同样是**工具的结果**：族归工具自己。
        // 身份只写这一份——`role` 那格不再跟着写（D-147：主体在场时折叠读主体，
        // 第二份写了也不生效，只会在改写时留下「名实不符」的机会）。
        principal: Some(symbio_sdk::chat::principal::Principal::tool_principal(
            tool_name,
        )),
        msg_type: Some(NodeKind::Interaction),
        content: Some(MessageContent::Text(pending.text.clone())),
        status: Some(NodeStatus::WaitingUserAction),
        meta: Some(json!({
            "prompt": pending.prompt,
            "failure_kind": pending.failure_kind,
            "tool_name": tool_name,
        })),
        ..Default::default()
    }
}

/// 执行体交回的结果：节点正文用已守卫的 [`Self::text`]，记账与钩子用 [`Self::raw`]。
#[derive(Debug, Clone, Default)]
pub struct ExecResult {
    pub text: String,
    pub raw: String,
    pub success: bool,
    pub pending: Option<PendingPrompt>,
    pub truncated: bool,
    pub archive_path: Option<String>,
    pub origin_tokens: usize,
    /// 执行体在结果事实里交回的 `failure_kind`（中性词；环不解释原因）。
    pub failure_kind: Option<String>,
    /// 执行体交回的「为什么没履约」（中性文本；环只交给能力包的效果端口）。
    pub why: String,
    /// 执行体交回的应答承载（`D-319`）：环只搬运、按配对键挂回调用。
    pub reply: symbio_plugin_api::port::PortReply,
}

/// 执行**一笔债**（不阻塞等待用户）：按端口名取提供者，交给端口执行合同跑。
///
/// 这是环与执行体之间的**通用单笔入口**（DE-048 D-311）：一笔调用一笔债，入参只有
/// 端口名与这笔债的载荷；并发、顺序、熔断全在 provider 内部，本函数不认识它们。
///
/// 本函数只是**端口取用面**：按端口名从能力表读提供者，把调用现场整份交给
/// 端口执行合同，收回文本 + 守卫元数据——线上名怎么解析、挂死怎么判（空闲而不是
/// 总时长）、任意 JSON 怎么压成一段文本、超预算结果怎么裁剪与存档、产物格怎么落，
/// 都是提供者一侧的私事，行为等价由 e2e 全量守着。
///
/// 端口缺席（没装提供者插件——零工具的装配没有工具世界的提供者）或合同缺席
/// （提供者不实现该端口的合同）都**响亮失败**：错误文本回传给模型（失败属信息性，
/// 模型据此改道）、`success = false`、日志出声——与既有 route 错误同形。不许
/// 静默回退、不许按「判不出来」继续：装配缺了提供者，继续转只会把缺失往后推。
#[allow(clippy::too_many_arguments)]
pub async fn execute_one(
    port: &'static str,
    env: &PortExecContext,
    gate: &GateInputs,
    parent: &Option<Arc<dyn Plugin>>,
    name: &str,
    args: Value,
    call_id: &str,
    protocol: &ProtocolInput,
    sink: &ExecEventSink,
    abort: &ExecAbortSignal,
    ctx: Arc<dyn PluginInvokeRequest>,
    reply_key: Option<&str>,
    reply_note: Option<&str>,
) -> ExecResult {
    let absent = |why: &str| {
        let text = format!("Error: `port.{port}` 缺席：{why}（{name} 未执行）");
        plugin_error!("session", "[Tool] {text}");
        ExecResult {
            text: text.clone(),
            raw: text,
            success: false,
            ..Default::default()
        }
    };
    let Some(provider) = crate::port::provider_of(&ctx, port).await else {
        return absent("没有可用的端口提供者");
    };
    let Some(executor) = provider.executor() else {
        return absent("提供者不实现执行合同");
    };

    // 宿主插件必须让提供者拿得到：`ctx.parent()` 是提供者解析失败时的回落面。
    // 在 fork 上写 `PLUGIN_PARENT`，不改调用方共享的那份 ctx。
    let exec_ctx = ctx.fork();
    if let Some(p) = parent {
        exec_ctx.set(PLUGIN_PARENT, Some(Arc::downgrade(p)));
    }
    // 模板只承载本轮不变的六值；每笔各自覆盖执行期句柄与幂等键。
    let mut per_call = env.clone();
    per_call.deadline_ms = None;
    per_call.abort = abort.clone();
    per_call.idempotency_key = call_id.to_string();
    per_call.sink = sink.clone();
    per_call.ctx = Some(exec_ctx);
    let out = executor
        .execute(
            per_call,
            PortInput {
                port,
                payload: json!({
                    "name": name,
                    "args": args,
                    "call_id": call_id,
                    "reply_key": reply_key,
                    "reply_note": reply_note,
                    "protocol": {
                        "id_missing": protocol.id_missing,
                        "name_missing": protocol.name_missing,
                        "parse_error": protocol.parse_error,
                    },
                    "gate": {
                        "authorized": gate.authorized,
                        "spent_ms": gate.spent_ms,
                        "requested_ms": gate.requested_ms,
                        "budget_ms": gate.budget_ms,
                    },
                }),
            },
        )
        .await;

    // 端口协议失败与工具自身失败是两件事：前者由 `error` 交回，后者住在
    // `facts` 的 `success=false` 里（工具失败是信息性事实，模型据此自我纠正）。
    if let Some((class, why)) = out.error {
        plugin_error!(
            "session",
            format!("[Tool] 端口执行失败（{class:?}）：{why}")
        );
        let text = format!("Error: {why}");
        return ExecResult {
            text: text.clone(),
            raw: text,
            success: false,
            ..Default::default()
        };
    }
    let fact = out.facts.first().cloned().unwrap_or(Value::Null);
    let result_text = fact
        .get("result_text")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let raw = fact
        .get("raw_text")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| result_text.clone());
    let success = fact
        .get("success")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let pending = fact
        .get("pending")
        .filter(|v| !v.is_null())
        .map(|v| PendingPrompt {
            text: v
                .get("text")
                .and_then(|t| t.as_str())
                .unwrap_or("需要用户操作")
                .to_string(),
            prompt: v.get("prompt").cloned().unwrap_or(Value::Null),
            failure_kind: v
                .get("failure_kind")
                .and_then(|k| k.as_str())
                .unwrap_or_default()
                .to_string(),
        });
    let guard = fact.get("guard").cloned().unwrap_or(Value::Null);
    ExecResult {
        text: result_text,
        raw,
        success,
        pending,
        truncated: guard
            .get("truncated")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        archive_path: guard
            .get("archive_path")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        origin_tokens: guard
            .get("origin_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize,
        failure_kind: fact
            .get("failure_kind")
            .and_then(|v| v.as_str())
            .filter(|k| !k.is_empty())
            .map(str::to_string),
        why: fact
            .get("why")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        reply: out.reply,
    }
}

/// 产物落格的执行体出口：只交回**结果纯值**，不再执行工具。
///
/// 唤醒结果与没跑到的占位结果没有一次「执行」可挂，但它们同样是产物；环把结果字段
/// 按通用载荷给出，执行体负责装配产物载荷、富化字段与幂等的拒绝事实（P1b /
/// DE-048 D-308）。环不按世界分支，也不在环里留第二条落格路径。
pub(crate) async fn land_outcome_async(
    env: &PortExecContext,
    parent: &Option<Arc<dyn Plugin>>,
    sink: &ExecEventSink,
    abort: &ExecAbortSignal,
    ctx: Arc<dyn PluginInvokeRequest>,
    outcome: &crate::tool_dispatch::CallOutcome,
) {
    let Some(provider) = crate::port::provider_of(&ctx, ports::TOOL).await else {
        return;
    };
    let Some(executor) = provider.executor() else {
        return;
    };
    let exec_ctx = ctx.fork();
    if let Some(p) = parent {
        exec_ctx.set(PLUGIN_PARENT, Some(Arc::downgrade(p)));
    }
    let args = serde_json::from_str(&outcome.args).unwrap_or(Value::Null);
    // 落格出口的幂等键必须按**债务 id**：同一张调用卡的等待态与答复态是两笔债，
    // 按节点 id（`call_id`）去重会把后一笔吞掉。`call_id` 只供结果锚定 `tool_call_id`。
    let mut per_call = env.clone();
    per_call.deadline_ms = None;
    per_call.abort = abort.clone();
    per_call.idempotency_key = outcome.call.id.clone();
    per_call.sink = sink.clone();
    per_call.ctx = Some(exec_ctx);
    let _ = executor
        .execute(
            per_call,
            PortInput {
                port: ports::TOOL,
                payload: json!({
                    "name": outcome.name,
                    "args": args,
                    "call_id": outcome.call_id,
                    "land_only": {
                        "text": outcome.text,
                        "success": outcome.ok,
                        "needs_user_action": outcome.needs_user_action,
                        "prompt": outcome.prompt,
                        "failure_kind": outcome.failure_kind,
                    },
                }),
            },
        )
        .await;
}

// 批量工具调用处理

/// 带执行信封的批量入口：顺序处理一批工具调用（产物格 / 宿主写口 / 轮归属随
/// [`PortExecContext`] 走），向 channel 广播每个工具的结果，
/// 并返回 `(tool_messages, parent_updates)`：
/// - `tool_messages`：工具结果子节点（用于追加到对话历史）
/// - `parent_updates`：ToolCall 父节点的**完整终态快照**（发射端从
///   `context_messages` 取权威副本应用终态，整条广播），供调用方做内存镜像
///   整条替换并随 `persist_messages` 落库。
///
/// 交互模式（interactive）下，若前一个工具产出 user_prompt（待审批/询问）或失败，
/// 则中止本批剩余工具（用户需逐个处理）；auto 模式不中止，失败结果传 LLM 继续。
///
/// ## 四条不变量（本函数负责保证）
///
/// 1. **本批每一个工具调用都必须以终态收场**（`Completed` / `WaitingUserAction`）：
///    调用前广播 `Streaming`（`dispatch::batch::emit_tool_running`），调用后广播终态。
///    「没跑到」的调用由环写 `failed{why}` 事实，投影器按事实渲染调用卡终态与
///    「未执行」结果子项，环不再补节点（DE-048 D-312）。
///
/// 2. **外部执行的授权 / 成本读数在每个调用点各取一次**（[`GateInputs`]），随债交给
///    执行体：判据与开窗不在环——`Refuse` 零事件、`Break` 由执行体自己 `emit`
///    `control × opened`、`Allow` 照旧开窗（DE-039 §6，D-311）。
/// 3. **声明闸由能力包自己判**（DE-025 D-219）：工具在 `CapabilityMeta.behaviors`
///    上声明「我受哪项管辖」，对应实现从
///    `ToolBehavior::before` 回答能不能跑（淘汰 / 任务表写侧即此类）。宿主只交这一笔
///    的结论（主体 / 控制面撤销值 / 会话事实切片），本层不含任何「哪个工具名
///    受什么管辖」的分支。
///
/// 阶段分层见 `dispatch::batch`；**没有批首读数**——编排顺序（空批早退 →
/// 逐条准备 → 逐条执行）不变。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn process_tool_calls_in_env(
    tool_calls: Vec<TurnToolCallInfo>,
    parent: &Option<Arc<dyn Plugin>>,
    sink: &ExecEventSink,
    abort: &ExecAbortSignal,
    ctx: Arc<dyn PluginInvokeRequest>,
    context_messages: &[ChatNode],
    env: &PortExecContext,
    facts: &dyn Store<Event = Event>,
    elimination_revoked: bool,
    effects: &mut Vec<EffectInput>,
    reply_key: Option<&str>,
) -> (
    Vec<ChatNode>,
    Vec<ChatNode>,
    Vec<symbio_plugin_api::port::PortReply>,
) {
    // 空批不读闸门、不发日志——与拆分前的早退位置逐字一致。
    if tool_calls.is_empty() {
        return (Vec::new(), Vec::new(), Vec::new());
    }

    // 逐条执行：批内顺序流程（协议失败 / 闸门拒止 / 钩子拦下 / 正常执行四条出口）。
    // - `tool_messages`：工具结果子节点（用于追加到对话历史）
    // - `parent_updates`：ToolCall 父节点的**完整终态快照**（发射端从
    //   `context_messages` 取权威副本应用终态），供调用方做内存镜像整条替换并随
    //   `persist_messages` 落库。
    let (tool_messages, parent_updates, replies) = run_calls(
        tool_calls,
        parent,
        sink,
        abort,
        ctx,
        context_messages,
        env,
        facts,
        elimination_revoked,
        effects,
        reply_key,
    )
    .await;

    (tool_messages, parent_updates, replies)
}

#[cfg(test)]
#[path = "execution.test.rs"]
mod tests;
