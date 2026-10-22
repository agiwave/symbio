//! v2 的工具分发实现（插件本地的 [`ToolDispatch`] 契约）。
//!
//! ## 为什么分发在插件侧
//!
//! 分发的**全部输入**都在这里：插件宿主（路由工具能力）、请求上下文（能力注册表 /
//! 会话 id / 工作区）、事件出口（结果节点写进对话图）、会话目录（超长结果存档）。
//! core 认识这些即违 E-009；契约与实现因此都留在本插件（唯一消费方是
//! [`super::turn_runner`]，ADR-118 决策 5）。
//!
//! ## 三个动作，不是一次打包调用
//!
//! 工具批次的三笔债各自由一个动作承担（ADR-118 决策 5）：
//!
//! 1. [`ToolDispatch::open`]：模型响应里的调用逐条写 `request(port=tool)`；
//! 2. [`ToolDispatch::authorize`]：对已开账的调用写 `authz.granted` /
//!    `authz.denied`（拒绝即止，不落 `sent`）；
//! 3. [`ToolDispatch::dispatch`]：为放行的调用落 `sent`、跑执行层、关账。
//!
//! 三步各自可观测，崩溃恢复的粒度因此从「整批」变成「已落哪一笔债」。
//!
//! ## 与工具执行层的关系：**不是第二条执行链**
//!
//! 实际执行走 `process_tool_calls_async`——超长结果存档、生命周期钩子、
//! 审批/问答的「待用户动作」、批尾未执行收口、中止提前收口全在里面。本模块只补
//! 两件事：
//!
//! 1. **补建助手轮消息**。工具调用节点不是流式逐帧广播出来的
//!    （`LlmAdapter` 交回的是收口后的 `LlmTurn`，没有那条帧流），由本处补建。
//!    节点形状与其余对话节点**同一份**（前端读同一套字段），
//!    因此复用 `llm_build_tool_call_nodes` / `llm_build_text_node`。
//! 2. **把结果映射成运行器的事实形状**（[`DispatchOutcome`]），供运行器落
//!    `artifact.added` 格与拼下一轮 prompt。
//!
//! ## 正文节点的定格
//!
//! 工具轮的中途正文（"我先查一下"）已经由 UI 桥逐片广播过。本模块在分发前把它
//! **定格**（`Completed`）并切节点（[`super::turn_exec::UiBridge::finalize_node`]）——
//! 不定格它永远转「流式中」，不切节点多轮正文会粘成一条。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::symbio_core::chat_node::{
    ChatNode, MessageContent, NodeKind, NodeRole, NodeStatus, CHAT_STYLE_TAG_REASONING,
};
use crate::symbio_core::facts::{Delegation, TaskDeclaration};
use crate::symbio_core::{
    capability_resolve, llm_emit_message, llm_emit_state, tool_metas, CapabilityRiskLevel, Entity,
    Event, ExecAbortSignal, ExecEventSink, LlmTurn, Plugin, PluginInvokeRequest,
    PluginInvokeRequestExt, Seq, Store, Verb, CAPABILITY_VISITOR, EVENT_AUTHZ_DENIED,
    EVENT_AUTHZ_GRANTED, EVENT_REQUEST, MODE, RISK_LEVEL,
};

use super::loop_rows;
use super::port;
use super::tools::{execute_tool_async, process_tool_calls_async};
use super::turn_exec::UiBridge;
use crate::symbio_core::node_build::{
    detach_if_flat, llm_build_text_node, llm_build_tool_call_nodes, PresentationTree,
};

/// 工具执行层交回的**收束派生事实**（承诺 / 任务表 / 熔断）。
///
/// 三者的数据来源都在工具执行层（`Delegation` / `TaskDeclaration` / 熔断理由），
/// 而写方（落格）在轮末——`turn_exec` 调 [`super::derived_facts::record_derived`] 写。
/// 本结构是那份出参从分发点回到运行器的**唯一通道**：不收集它，这三样就静默全丢
/// （与记忆 / 学习同一个坑——出参不交回，写方就无从落格）。
///
/// 形态仿 [`SessionDispatchPort::produced`]：`dispatch` 在 `&self` 上跑，出参却要
/// 跨 await 累积，故用 `Mutex` 收，调用方在桥释放前 `take_derived` 取走。
#[derive(Default)]
pub(crate) struct DerivedFacts {
    /// 代际立约出参（批⑧）：`agent_run` 的承诺记录。
    pub(crate) delegations: Vec<Delegation>,
    /// 任务表出参（批⑨）：本次成功的 `todo_write` 声明的清单状态。
    pub(crate) tasks: Vec<TaskDeclaration>,
    /// 熔断出参（批⑩ 步 20）：外部执行闸门判 `Break` 的理由。
    pub(crate) breaks: Vec<&'static str>,
}

/// 一笔已开账的工具端口调用（创建动作的产物）。
///
/// `granted = false` ⇒ 授权行已落 `authz.denied`（或授权事实落格失败），这笔调用
/// **没有** `sent`；执行层仍会跑它（审批 / 拒绝卡的节点由执行层唯一构造），但端口
/// 账已关。`sent = false` 且 `granted = true` ⇒ 派发行没有放行这笔账（行不欠 /
/// `sent` 落格失败），关账时按「没执行」处理。
#[derive(Clone)]
pub(crate) struct ToolPortCall {
    /// 模型交回的调用 id（执行层按它回结果；与端口 id 是两件事）。
    pub(crate) model_id: String,
    /// 端口账的逻辑 id（`request` / `sent` / 应答同 id）。
    pub(crate) call: port::Call,
    /// 工具名（LLM 可见名；授权事实的载荷要它）。
    pub(crate) name: String,
    /// 这次调用的风险档。`None` = 注册表里没有可判的能力（route-only 工具）。
    pub(crate) risk: Option<CapabilityRiskLevel>,
    /// 参数里显式带了 `approved: true`（已由人工确认过）。
    pub(crate) approved: bool,
    /// 授权动作的结论（创建时为 `false`，授权动作写入）。
    pub(crate) granted: bool,
    /// 派发动作是否已落 `sent`（派发动作写入）。
    pub(crate) sent: bool,
}

/// 一次工具调用结果的**事实形状**（[`ToolDispatch::dispatch`] 的元素类型）。
///
/// 刻意只装**结果**：消费方（运行器）要的是「拿什么回填给模型」与「要不要停」，
/// 不是插件侧的节点形状。节点怎么建、往哪发，是分发方自己的事——运行器不认识
/// `ChatNode` 的图语义（那在 `plugins/session`），只认识这段文本。
///
/// 它住在插件里而不是 core：生产消费方只有本插件的运行器（README §4 四问第 1 问）。
#[derive(Debug, Clone, Default)]
pub struct DispatchOutcome {
    /// 对应的工具调用 id（回填给模型时锚定 ToolCall；取 `TurnToolCallInfo::id`）。
    pub call_id: String,
    /// 工具名（LLM 可见名）。
    pub name: String,
    /// 结果正文。**失败也是正文**——工具失败是信息性的，模型据此改道，不中断会话。
    pub text: String,
    /// 成功与否。仅供可观测/展示；**不改变**本轮终态（信息性，见上）。
    pub ok: bool,
    /// 本次调用**收束于等待用户动作**（confirm / ask_user ⇒ 本轮停下等用户）。
    /// 运行器据此**停止工具循环**（恢复前提见
    /// [plan/11 批 2](../../../../docs/004-plan/009-多执行器与多主体加固实施方案.md "PL-009-多执行器与多主体加固实施方案")）。
    pub needs_user_action: bool,
    /// 等待用户时卡片载荷（工具自决的 `prompt` 形状：审批卡 / 提问卡不同）。
    ///
    /// **它是事实的一部分**：`needs_user_action` 只说明「这轮停在等用户」，而恢复
    /// （`resume`）要拿 `meta.prompt` 复现那张卡。T-056 起读路径走 WAL 重建 ⇒ 这张卡
    /// 必须能从事实折出来，故随载荷入格（`Some` 仅当 [`Self::needs_user_action`]）。
    /// 运行器不认识它的形状——原样透传（与 `text` 同一条：只搬数据，不解释）。
    pub prompt: Option<serde_json::Value>,
    /// 等待用户的原因码（`failure_kind` 里的 pending 取值：`needs_approval` /
    /// `needs_interaction`……）。与 [`Self::prompt`] 同批入库、同批复现。
    pub failure_kind: String,
    /// ToolCall 节点的 provider wire id（`tool_call_id` 字段）。节点身份的一部分：
    /// 事件载荷记录它，重建才能复刻同一个工具卡片（`call_id` 只是节点 id）。
    pub wire_id: String,
    /// ToolCall 节点的**定稿参数文本**（JSON 文本 / 解析失败的残破原文）。
    /// 重建直接回放这段文本，不经 `Value` 重序列化（键序漂移即失真）。
    pub args: String,
    /// 结果子节点（`role=Tool`）的节点 id——落库时由分发方分配，记进载荷供重建复用。
    pub result_id: String,
}

/// 工具派发的**三个动作**（ADR-118 决策 5）：创建 / 授权 / 派发。
///
/// 三笔债不再由一次调用代劳——运行器按顺序各调一次，每一步只写自己那一行的
/// 领域事实。契约住插件本地，core 不认识工具分发（E-009）。
#[async_trait]
pub(crate) trait ToolDispatch: Send + Sync {
    /// 创建：为模型响应里可派发的调用逐条写 `request(port=tool)`。
    async fn open(&self, turn: &LlmTurn) -> Vec<ToolPortCall>;

    /// 授权：为已开账的调用写 `authz.granted` / `authz.denied`。
    ///
    /// 拒绝即止：`granted` 保持 `false`，派发动作不会为它落 `sent`。
    async fn authorize(&self, calls: &mut [ToolPortCall]);

    /// 派发：为放行的调用落 `sent`、跑执行层、关账，返回结果事实。
    async fn dispatch(&self, turn: &LlmTurn, calls: &mut [ToolPortCall]) -> Vec<DispatchOutcome>;
}

/// 工具端口账的写方：批次分发与 resume 共用**同一套**开账 / 关账。
///
/// 为什么把它从 `SessionDispatchPort` 里拆出来：resume 的批准路径也要落
/// `request → granted → sent → response`（T-227 步 4 / T-226 判据 6）。如果
/// resume 自己再写一遍，两个写方就会在 id、授权判定、关账时机上各长一套——
/// 而那正是「旁路」的定义。
pub(crate) struct ToolPort {
    facts: Arc<dyn Store<Event = Event>>,
    turn: u64,
    user_seq: u64,
    drive_seq: u64,
}

/// 批次调用的**协议 id**：轮 + 幕 + 本次模型往返 + 模型交回的调用 id。
///
/// ## 为什么要带「本次模型往返」
///
/// 创建行 `open-tools` 的集合展开（ADR-117）从 `response.calls` 的元素里取 id，
/// 而 `response` 在派发**之前**就写好了——那时还没有「第几批」。要让展开出的键
/// 与随后的 `request` 同 id，id 必须只由**模型响应里已有的东西**决定：轮、幕、
/// 模型往返（`model-…-s{step}`）、模型调用 id。
///
/// 只带 `tc.id` 不够：本地模型常用 `call_0` / `call_1` 这种按回合重来的短 id，
/// 同一幕的下一轮就会撞上上一轮的键——创建债被旧 `request` 提前关掉，新调用根本
/// 不会派发。`round` 把「同一幕的不同轮」分开；缺失模型 id 的调用用序号兜底。
pub(crate) fn batch_call_id(
    turn: u64,
    drive: u64,
    idx: usize,
    model_id: Option<&str>,
    round: &str,
) -> String {
    match model_id.filter(|s| !s.trim().is_empty()) {
        Some(id) => format!("tool-{turn}-d{drive}-{round}-{id}"),
        None => format!("tool-{turn}-d{drive}-{round}-c{idx}"),
    }
}

/// 本幕最近一笔模型应答（`response(port=model)`）。
///
/// 派发侧不重算 id，而是读创建侧写下的 `calls`：那是**唯一真相**，也自然对齐
/// `turn.tool_calls` 的顺序。重算只能靠再次猜「这一轮是哪一轮」，而猜错一次的
/// 后果是创建债被旧事实提前关掉。
fn latest_model_response(facts: &[Event], turn: u64, drive: u64) -> Option<&Event> {
    let prefix = format!("model-{turn}-d{drive}-");
    facts.iter().rev().find(|e| {
        e.turn == turn
            && e.kind == crate::symbio_core::EVENT_RESPONSE
            && e.payload.get("port").and_then(|v| v.as_str()) == Some("model")
            && e.payload
                .get("id")
                .and_then(|v| v.as_str())
                .is_some_and(|id| id.starts_with(&prefix))
    })
}

/// 一次开账所需的请求级策略（批次内只读一次）。
struct ToolPortPolicy {
    known: Vec<String>,
    threshold: CapabilityRiskLevel,
    mode: String,
}

impl ToolPortPolicy {
    async fn read(ctx: &Arc<dyn PluginInvokeRequest>) -> Self {
        let known: Vec<String> = tool_metas(ctx).await.into_iter().map(|m| m.name).collect();
        // 阈值读法与执行闸门同一处口径：`ctx[RISK_LEVEL]`，缺席 = `Medium`。
        // 比较本身只问 `CapabilityRiskLevel::requires_approval`（唯一实现）。
        let threshold = ctx
            .get(RISK_LEVEL)
            .map(|s| CapabilityRiskLevel::from_name(&s))
            .unwrap_or_default();
        let mode = ctx.get(MODE).unwrap_or_default();
        Self {
            known,
            threshold,
            mode,
        }
    }
}

impl ToolPort {
    pub(crate) fn new(
        facts: Arc<dyn Store<Event = Event>>,
        turn: u64,
        user_seq: u64,
        drive_seq: u64,
    ) -> Self {
        Self {
            facts,
            turn,
            user_seq,
            drive_seq,
        }
    }

    /// 本次 drive 的 `now`：幕锚那条唤醒事实的 `ts`（判据 12）。
    ///
    /// 恢复幕在 `begin_drive` **之前**执行，`drive_seq` 填的是占位 0 ⇒ 这里得 0；
    /// 本行的三笔工具债（`open-tools` / `authorize-tool` / `dispatch-tools`）不读
    /// 带时间的视图，`now` 参与不了判据。要新增读时间的行，得先把真实幕锚传进来。
    fn now_ms(&self, facts: &[Event]) -> i64 {
        loop_rows::wake_now_ms(facts, self.drive_seq)
    }

    /// 批次级 prepass：为每个真正可派发的调用开一笔工具账。
    ///
    /// 入口先问创建行 `open-tools` 欠不欠（ADR-117）：一个模型响应是一笔集合债，
    /// 逐元素展开成 N 笔调用；全部 `request` 落完之后它就不再欠。这里不自己数
    /// 「第几批」——id 只由模型响应里已有的东西决定（[`batch_call_id`]），
    /// 否则创建行展开出的键与 `request` 的键对不上。
    pub(crate) async fn open_batch(
        &self,
        ctx: &Arc<dyn PluginInvokeRequest>,
        turn: &LlmTurn,
    ) -> Vec<ToolPortCall> {
        let facts = self.facts.range(Seq::new(0));
        let latest = latest_model_response(&facts, self.turn, self.drive_seq);
        // 旧存档里的模型应答没有 `calls`（ADR-117 之前写的）：没有创建债可言，
        // 按旧路径继续派发，不能因为升级把在飞的会话卡死。带 `calls` 的应答
        // 则必须由创建行先开账——它不欠就说明这一批已经落过 `request`。
        let has_calls = latest.is_some_and(|e| e.payload.get("calls").is_some());
        if has_calls && !loop_rows::owes("open-tools", &facts, self.turn, self.now_ms(&facts)) {
            crate::plugin_warn!(
                "session",
                "[Tool] 创建行不欠（turn={}），不落 request",
                self.turn
            );
            return Vec::new();
        }
        let policy = ToolPortPolicy::read(ctx).await;
        let round = latest
            .and_then(|e| e.payload.get("id"))
            .and_then(|v| v.as_str())
            .unwrap_or("r0");
        let created: &[serde_json::Value] = latest
            .and_then(|e| e.payload.get("calls"))
            .and_then(|v| v.as_array())
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let mut out = Vec::new();
        for (idx, tc) in turn.tool_calls.iter().enumerate() {
            let id = created
                .get(idx)
                .and_then(|c| c.get("id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    batch_call_id(self.turn, self.drive_seq, idx, tc.id.as_deref(), round)
                });
            let call = port::Call::new("tool", id, 1);
            let model_id = tc.id.as_deref().filter(|s| !s.trim().is_empty());
            let name = tc.name.as_deref().filter(|s| !s.trim().is_empty());
            let dispatchable = model_id.is_some()
                && name.is_some()
                && tc.parse_error.is_none()
                && tc.overflow_notice.is_none();
            if !dispatchable {
                // 创建行已经为它开了一笔债（响应里有这个元素），但它派不出去：
                // 落 `failed` 关账，而不是让那笔债永远欠着。
                let _ = port::failed(
                    self.facts.as_ref(),
                    &call,
                    self.turn,
                    self.user_seq,
                    "invalid_tool_call",
                );
                continue;
            }
            if let Some(open) = self
                .open_one(
                    ctx,
                    &policy,
                    &call,
                    model_id.unwrap_or_default(),
                    name.unwrap_or_default(),
                    &tc.arguments,
                )
                .await
            {
                out.push(open);
            }
        }
        out
    }

    /// 创建动作的单笔实体：只写 `request`（ADR-118 决策 5 的第一笔债）。
    ///
    /// 风险档在这里算定并写进请求载荷（`risk` / `threshold` / `approved` /
    /// `deferred`），授权动作只按它判放行——两处读的是**同一个判定输入**，
    /// 不各算一遍。
    async fn open_one(
        &self,
        ctx: &Arc<dyn PluginInvokeRequest>,
        policy: &ToolPortPolicy,
        call: &port::Call,
        model_id: &str,
        name: &str,
        args: &serde_json::Value,
    ) -> Option<ToolPortCall> {
        let approved = args
            .get("approved")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let risk = risk_of(ctx, name, args, &policy.known).await;
        // 注册表里没有能力可判（route-only 工具）⇒ 放行是**交给下游路由闸门**
        // 的延迟判定；不能在这里凭一个猜出来的风险档拒绝它。
        let deferred = risk.is_none();
        let risk_name = risk.map(risk_label).unwrap_or("unknown");

        if let Err(e) = port::announce_with(
            self.facts.as_ref(),
            call,
            self.turn,
            self.user_seq,
            serde_json::json!({
                "tool": name,
                "risk": risk_name,
                "threshold": risk_label(policy.threshold),
                "approved": approved,
                "deferred": deferred,
            }),
        ) {
            crate::plugin_warn!(
                "session",
                "[Tool] 工具请求事实未落格（turn={} / {model_id}）：{e:?}",
                self.turn
            );
            return None;
        }
        Some(ToolPortCall {
            model_id: model_id.to_string(),
            call: call.clone(),
            name: name.to_string(),
            risk,
            approved,
            granted: false,
            sent: false,
        })
    }

    /// 授权动作：为已开账的调用写 `authz.granted` / `authz.denied`（第二笔债）。
    pub(crate) async fn authorize(
        &self,
        ctx: &Arc<dyn PluginInvokeRequest>,
        calls: &mut [ToolPortCall],
    ) {
        if calls.is_empty() {
            return;
        }
        let policy = ToolPortPolicy::read(ctx).await;
        self.authorize_calls(&policy, calls);
    }

    /// 授权动作的实体（策略由调用方读出，resume 路径复用同一份）。
    fn authorize_calls(&self, policy: &ToolPortPolicy, calls: &mut [ToolPortCall]) {
        let facts = self.facts.range(Seq::new(0));
        if !loop_rows::owes("authorize-tool", &facts, self.turn, self.now_ms(&facts)) {
            crate::plugin_warn!(
                "session",
                "[Tool] 授权行不欠（turn={}），不判放行",
                self.turn
            );
            return;
        }
        for call in calls.iter_mut() {
            let granted = call
                .risk
                .is_none_or(|risk| !risk.requires_approval(policy.threshold, call.approved));
            let risk_name = call.risk.map(risk_label).unwrap_or("unknown");
            if granted {
                if let Err(e) = append_authz(
                    self.facts.as_ref(),
                    granted_event(
                        self.turn,
                        self.user_seq,
                        &call.call,
                        &call.name,
                        risk_name,
                        call.risk.is_none(),
                    ),
                ) {
                    crate::plugin_warn!(
                        "session",
                        "[Tool] 放行事实未落格（turn={} / {}）：{e:?}",
                        self.turn,
                        call.model_id
                    );
                    continue;
                }
                call.granted = true;
            } else {
                // 交互模式 ⇒ 执行层会建审批卡；auto ⇒ 执行层会回 `permission_denied`。
                // 两种都不是「已派发」，所以只有 `denied`、没有 `sent`。
                let why = if policy.mode == "auto" {
                    "permission_denied"
                } else {
                    "needs_approval"
                };
                if let Err(e) = append_authz(
                    self.facts.as_ref(),
                    denied_event(self.turn, self.user_seq, &call.call, &call.name, why),
                ) {
                    crate::plugin_warn!(
                        "session",
                        "[Tool] 拒绝事实未落格（turn={} / {}）：{e:?}",
                        self.turn,
                        call.model_id
                    );
                }
            }
        }
    }

    /// 派发动作的前半（第三笔债）：为放行的调用落 `sent`。
    ///
    /// 执行层由调用方跑（`SessionDispatchPort::dispatch`）——`sent` 的语义是
    /// 「已交给执行层」，所以它必须先于执行。
    pub(crate) fn begin_calls(&self, calls: &mut [ToolPortCall]) {
        if !calls.iter().any(|c| c.granted) {
            return;
        }
        let facts = self.facts.range(Seq::new(0));
        let owed = loop_rows::owes("dispatch-tools", &facts, self.turn, self.now_ms(&facts));
        for call in calls.iter_mut().filter(|c| c.granted) {
            if !owed {
                crate::plugin_warn!(
                    "session",
                    "[Tool] 派发行不欠（turn={} / {}），不落 sent",
                    self.turn,
                    call.model_id
                );
                let _ = port::failed(
                    self.facts.as_ref(),
                    &call.call,
                    self.turn,
                    self.user_seq,
                    "dispatch_row_not_owed",
                );
                continue;
            }
            if let Err(e) = port::begin(
                self.facts.as_ref(),
                loop_rows::PORTS,
                port::Dispatch {
                    call: &call.call,
                    turn: self.turn,
                    produced_by: self.user_seq,
                    owed,
                },
            ) {
                crate::plugin_warn!(
                    "session",
                    "[Tool] 工具 sent 未落格（turn={} / {}）：{e:?}",
                    self.turn,
                    call.model_id
                );
            } else {
                call.sent = true;
            }
        }
    }

    /// 下一次 resume 的尝试号：已有几笔同目标 `request`，下一笔就是几 + 1。
    ///
    /// 事实面是唯一计数器；重启后它仍然对得上。
    pub(crate) fn resume_attempt(&self, target_id: &str) -> u64 {
        let prefix = format!("tool-resume-{}-{}-", self.turn, target_id);
        let n = self
            .facts
            .range(Seq::new(0))
            .iter()
            .filter(|e| {
                e.kind == EVENT_REQUEST
                    && e.payload
                        .get("id")
                        .and_then(|v| v.as_str())
                        .is_some_and(|id| id.starts_with(&prefix))
            })
            .count() as u64;
        n + 1
    }

    /// 单笔开账（resume 用）：一次用户动作就是一个新批次。
    ///
    /// `attempt` 由调用方从事实面数出来（`tool-resume-{turn}-{target}-{n}`），
    /// 而不是从内存计数器来——进程重启后计数器归零，若拿它当 id 段号，第二次
    /// 恢复会撞上第一次的幂等键，事实面就把两次尝试当成了一次。
    pub(crate) async fn open_resume(
        &self,
        ctx: &Arc<dyn PluginInvokeRequest>,
        target_id: &str,
        attempt: u64,
        name: &str,
        args: &serde_json::Value,
    ) -> Option<ToolPortCall> {
        let policy = ToolPortPolicy::read(ctx).await;
        let call = port::Call::new(
            "tool",
            format!("tool-resume-{}-{}-{}", self.turn, target_id, attempt),
            attempt,
        );
        self.open_with(ctx, &policy, &call, target_id, name, args)
            .await
    }

    /// 单笔开账的实体（resume 用）：`request` → 授权 → 放行才落 `sent`。
    ///
    /// 批次路径按三个动作分开走；resume 的一次用户动作只涉及一笔调用，三步在
    /// 这里连续执行，**共用同一套实体**（不另写一份开账逻辑）。
    async fn open_with(
        &self,
        ctx: &Arc<dyn PluginInvokeRequest>,
        policy: &ToolPortPolicy,
        call: &port::Call,
        model_id: &str,
        name: &str,
        args: &serde_json::Value,
    ) -> Option<ToolPortCall> {
        let mut one = [self
            .open_one(ctx, policy, call, model_id, name, args)
            .await?];
        self.authorize_calls(policy, &mut one);
        if !one[0].granted {
            return one.into_iter().next();
        }
        self.begin_calls(&mut one);
        if !one[0].sent {
            return None;
        }
        one.into_iter().next()
    }

    /// 批次返回后关账：放行的每一笔都必须有 `response`（有结果）或 `failed`
    /// （执行层没跑到它）。拒绝的那几笔已经由 `authz.denied` 关掉，不在这里补。
    pub(crate) fn close_batch(&self, calls: &[ToolPortCall], outcomes: &[DispatchOutcome]) {
        for call in calls.iter().filter(|c| c.granted && c.sent) {
            let answered = outcomes.iter().any(|o| o.call_id == call.model_id);
            self.close(call, answered);
        }
    }

    /// 单笔关账（resume 用）：执行体已经给出成败。
    pub(crate) fn close(&self, call: &ToolPortCall, answered: bool) {
        if answered {
            if let Err(e) = port::succeed(self.facts.as_ref(), &call.call, self.turn, self.user_seq)
            {
                crate::plugin_warn!(
                    "session",
                    "[Tool] 工具应答事实未落格（turn={} / {}）：{e:?}",
                    self.turn,
                    call.model_id
                );
            }
        } else {
            self.fail(call, "not_executed");
        }
    }

    /// 单笔失败关账（resume 的中止路径用）：落 `failed{why}`，与「没执行」区分。
    pub(crate) fn fail(&self, call: &ToolPortCall, why: &str) {
        if !call.granted {
            return;
        }
        if let Err(e) = port::failed(
            self.facts.as_ref(),
            &call.call,
            self.turn,
            self.user_seq,
            why,
        ) {
            crate::plugin_warn!(
                "session",
                "[Tool] 工具失败事实未落格（turn={} / {}）：{e:?}",
                self.turn,
                call.model_id
            );
        }
    }
}

/// 单笔工具执行（resume 的 approve/retry/supply 用）：端口账 + 执行层。
///
/// 这是 resume 与批次**共用**的那一段执行语义：静默出口、`result_msg_id`、
/// `execute_tool_async` 的调用形状都在这里；resume 只负责把用户动作翻译成
/// `(tool_name, args)` 和结果节点的重建。执行层自己的审批闸门仍然跑
/// （defense in depth）——端口开账不替代它。
pub(crate) async fn execute_single_tool(
    parent: &Option<Arc<dyn Plugin>>,
    ctx: &Arc<dyn PluginInvokeRequest>,
    abort: &ExecAbortSignal,
    port: &ToolPort,
    target_id: &str,
    tool_name: &str,
    args: serde_json::Value,
) -> (String, bool) {
    let attempt = port.resume_attempt(target_id);
    let open = port
        .open_resume(ctx, target_id, attempt, tool_name, &args)
        .await;
    let result_msg_id = crate::symbio_core::llm_short_id();
    let silent = ExecEventSink::silent();
    let (res, success, _captured) = execute_tool_async(
        parent,
        tool_name,
        args,
        target_id,
        &silent,
        abort,
        result_msg_id,
        ctx.clone(),
    )
    .await;
    if let Some(call) = open.as_ref() {
        if abort.is_aborted() {
            port.fail(call, "aborted");
        } else {
            port.close(call, success);
        }
    }
    (res, success)
}

/// v2 的工具分发方：一轮工具调用的执行 + 落节点 + 结果映射。
pub(crate) struct SessionDispatchPort {
    /// 插件宿主（工具能力经它路由）。
    parent: Option<Arc<dyn Plugin>>,
    /// 本轮请求上下文（能力注册表 / 会话 id / 工作区）。
    ctx: Arc<dyn PluginInvokeRequest>,
    /// 事件出口（工具节点与结果都写到对话图上）。
    sink: ExecEventSink,
    /// 工具结果存档目录（session 在派发前算好的会话子目录）。
    tool_archive_dir: PathBuf,
    /// 本会话事实轨：闸门判据按值取自它（ADR-084 §2，能力面不见存储实现）。
    facts: Arc<dyn Store<Event = Event>>,
    /// 本轮中止信号：批内逐工具检查，未执行的批尾必须定格。
    abort: ExecAbortSignal,
    /// 本轮 Turn 根节点 id（助手轮消息的父节点）。
    root_id: String,
    /// 工具端口账的写方（批次与 resume 共用同一套开账 / 关账）。
    port: ToolPort,
    /// 流式桥：交出本轮正文节点并切下一轮。
    bridge: Arc<UiBridge>,
    /// 本轮产生的全部消息（供 chat_loop 并入落库权威镜像）。
    produced: Mutex<Vec<ChatNode>>,
    /// 本轮工具执行层交回的收束派生事实（承诺 / 任务表 / 熔断）。
    derived: Mutex<DerivedFacts>,
    /// 本轮建树策略（`ExecTurn` 的请求级快照，与流式发射同一个取值）。
    presentation_tree: PresentationTree,
    /// 淘汰闸的部署级判据（session 已折成**一个值**，见 ADR-086 §4）。
    elimination_revoked: bool,
}

impl SessionDispatchPort {
    #[allow(clippy::too_many_arguments)] // 构造点唯一（turn_exec），字段即上下文
    pub(crate) fn new(
        parent: Option<Arc<dyn Plugin>>,
        ctx: Arc<dyn PluginInvokeRequest>,
        sink: ExecEventSink,
        tool_archive_dir: PathBuf,
        facts: Arc<dyn Store<Event = Event>>,
        abort: ExecAbortSignal,
        root_id: String,
        turn: u64,
        user_seq: u64,
        drive_seq: u64,
        bridge: Arc<UiBridge>,
        presentation_tree: PresentationTree,
        elimination_revoked: bool,
    ) -> Self {
        let facts_for_port = facts.clone();
        Self {
            parent,
            ctx,
            sink,
            tool_archive_dir,
            facts,
            abort,
            root_id,
            port: ToolPort::new(facts_for_port, turn, user_seq, drive_seq),
            bridge,
            produced: Mutex::new(Vec::new()),
            derived: Mutex::new(DerivedFacts::default()),
            presentation_tree,
            elimination_revoked,
        }
    }

    /// 取走本轮产生的消息（调用方在桥释放前取）。
    pub(crate) fn take_produced(&self) -> Vec<ChatNode> {
        std::mem::take(&mut self.produced.lock().unwrap())
    }

    /// 取走本轮工具执行层交回的收束派生事实（调用方在桥释放前取）。
    pub(crate) fn take_derived(&self) -> DerivedFacts {
        std::mem::take(&mut self.derived.lock().unwrap())
    }

    /// 三动作契约的创建步（实体在 [`ToolPort::open_batch`]）。
    async fn open_action(&self, turn: &LlmTurn) -> Vec<ToolPortCall> {
        self.port.open_batch(&self.ctx, turn).await
    }

    /// 三动作契约的授权步（实体在 [`ToolPort::authorize`]）。
    async fn authorize_action(&self, calls: &mut [ToolPortCall]) {
        self.port.authorize(&self.ctx, calls).await;
    }

    /// 批次返回后关账（实体在 [`ToolPort::close_batch`]）。
    fn close_tool_calls(&self, calls: &[ToolPortCall], outcomes: &[DispatchOutcome]) {
        self.port.close_batch(calls, outcomes);
    }
}

/// 风险档的线格式（与 `CapabilityRiskLevel` 的 serde 字面量同一套词）。
fn risk_label(risk: CapabilityRiskLevel) -> &'static str {
    match risk {
        CapabilityRiskLevel::Low => "low",
        CapabilityRiskLevel::Medium => "medium",
        CapabilityRiskLevel::High => "high",
    }
}

/// 这次调用的风险档。`None` = 注册表里没有可判的能力（route-only 工具）。
async fn risk_of(
    ctx: &Arc<dyn PluginInvokeRequest>,
    name: &str,
    args: &serde_json::Value,
    known: &[String],
) -> Option<CapabilityRiskLevel> {
    let canonical = capability_resolve(name, known.iter().map(String::as_str))?;
    let visitor = ctx.get(CAPABILITY_VISITOR)?;
    let cap = visitor
        .get(canonical)
        .await
        .and_then(|r| r.cap().cloned())?;
    Some(cap.risk_for(args))
}

fn granted_event(
    turn: u64,
    produced_by: u64,
    call: &port::Call,
    tool: &str,
    risk: &str,
    deferred: bool,
) -> Event {
    let id = &call.id;
    Event::pending(
        format!("grant-{turn}-{id}"),
        EVENT_AUTHZ_GRANTED,
        Entity::Turn,
        Verb::Asserted,
        turn,
        "authorizer",
    )
    .with_produced_by(produced_by)
    .with_payload(serde_json::json!({
        "key": id,
        "port": "tool",
        "id": id,
        "tool": tool,
        "risk": risk,
        "deferred": deferred,
    }))
}

fn denied_event(turn: u64, produced_by: u64, call: &port::Call, tool: &str, why: &str) -> Event {
    let id = &call.id;
    Event::pending(
        format!("deny-tool-{turn}-{id}"),
        EVENT_AUTHZ_DENIED,
        Entity::Turn,
        Verb::Asserted,
        turn,
        "authorizer",
    )
    .with_produced_by(produced_by)
    .with_payload(serde_json::json!({
        "key": id,
        "port": "tool",
        "id": id,
        "tool": tool,
        "kind": "tool",
        "reason": crate::symbio_core::ClosureReason::Denied.as_str(),
        "detail": why,
    }))
}

fn append_authz<S>(store: &S, event: Event) -> Result<(), crate::symbio_core::AppendError>
where
    S: Store<Event = Event> + ?Sized,
{
    match store.append(event) {
        Ok(_) => Ok(()),
        Err(crate::symbio_core::AppendError::Duplicate) => Ok(()),
        Err(e) => Err(e),
    }
}

#[async_trait]
impl ToolDispatch for SessionDispatchPort {
    async fn open(&self, turn: &LlmTurn) -> Vec<ToolPortCall> {
        self.open_action(turn).await
    }

    async fn authorize(&self, calls: &mut [ToolPortCall]) {
        self.authorize_action(calls).await;
    }

    async fn dispatch(
        &self,
        turn: &LlmTurn,
        port_calls: &mut [ToolPortCall],
    ) -> Vec<DispatchOutcome> {
        let mut produced: Vec<ChatNode> = Vec::new();

        // ⓪ 第三笔债的前半：为放行的调用落 `sent`。必须在执行层之前——
        //    `sent` 的语义是「已交给执行层」，而不是「工具已经开始跑」。
        self.port.begin_calls(port_calls);

        // ① 定格本轮正文与推理（若有）：节点已由 UI 桥逐片广播，这里只迁状态并切节点。
        //    没有正文（纯工具调用轮）就不建节点——空节点是噪声。
        //
        //    两条流**一起定格**：都由桥建节点、都在本轮结束时收口，分两处做只是把同一个
        //    动作写两遍，且漏一处就有一条流永远转「流式中」（界面上表现为思考块卡住）。
        let text_node = if turn.text.trim().is_empty() {
            None
        } else {
            self.bridge.finalize_node(&turn.text)
        };
        let reason_node = self.bridge.finalize_reasoning_node();
        // 同步点：定格帧在桥的通道里排队，而下面的工具节点**直接**写出口
        // （`process_tool_calls_async` 只认 `ExecEventSink`）。不先排空队列，
        // 工具卡片会跑到正文之前。
        self.bridge.flush().await;
        if let Some(node_id) = text_node {
            let mut msg = llm_build_text_node(&self.root_id, &turn.text, None, node_id);
            detach_if_flat(&mut msg, &self.root_id, self.presentation_tree);
            // 状态帧：正文已由增量帧传过，重发正文会与队列里的增量打架。
            llm_emit_state(&self.sink, msg.clone()).await;
            produced.push(msg);
        }
        if let Some((node_id, text)) = reason_node {
            let mut msg = ChatNode {
                id: node_id,
                parent_id: Some(self.root_id.clone()),
                role: Some(NodeRole::Assistant),
                msg_type: Some(NodeKind::Notice),
                content: Some(MessageContent::Text(text)),
                status: Some(NodeStatus::Completed),
                ..Default::default()
            }
            .with_style_tag(CHAT_STYLE_TAG_REASONING);
            detach_if_flat(&mut msg, &self.root_id, self.presentation_tree);
            // 状态帧：推理正文已由增量帧传过，同上——重发会与队列里的增量打架。
            llm_emit_state(&self.sink, msg.clone()).await;
            produced.push(msg);
        }

        // ② 工具调用节点：这条路径没有流式工具帧，由本处补建。
        //    先以 `Streaming` 广播（工具还没跑，卡片应显示运行中），落库副本用终态。
        let mut call_nodes = llm_build_tool_call_nodes(&self.root_id, &turn.tool_calls);
        // 就地 detach：下面三处用同一份 `call_nodes`——广播帧、传给执行器作父查询源
        // （`process_tool_calls_async` 从它 clone 出运行态/终态帧，继承 detach 后的
        // parent_id），以及落库副本。一处收口，live↔存储自动锁步。
        for n in call_nodes.iter_mut() {
            detach_if_flat(n, &self.root_id, self.presentation_tree);
        }
        for node in &call_nodes {
            let mut frame = node.clone();
            frame.status = Some(NodeStatus::Streaming);
            llm_emit_message(&self.sink, frame).await;
        }

        // ③ 分发（工具执行层：存档 / 钩子 / 审批 / 批尾收口全在里面）。
        //    父节点查询用刚广播的这批节点——它们与执行器要求的形状同源。
        //
        // 三份收束派生事实出参（承诺 / 任务表 / 熔断）：数据来源在工具执行层，
        // 写方在轮末——由本分发方收下、`turn_exec` 轮末调
        // `derived_facts::record_derived` 落格。不收这三份，
        // 代际立约 / 任务表 / 熔断就静默全丢（见 [`DerivedFacts`]）。
        let mut delegations: Vec<Delegation> = Vec::new();
        let mut tasks: Vec<TaskDeclaration> = Vec::new();
        let mut breaks: Vec<&'static str> = Vec::new();
        let (tool_msgs, parent_updates) = process_tool_calls_async(
            turn.tool_calls.clone(),
            &self.parent,
            &self.sink,
            &self.abort,
            self.ctx.clone(),
            &call_nodes,
            &self.tool_archive_dir,
            self.facts.as_ref(),
            self.elimination_revoked,
            // 代际立约出参（批⑧）：`agent_run` 的承诺记录。
            &mut delegations,
            // 任务表出参（批⑨）：本次成功的 `todo_write` 声明的清单状态。
            &mut tasks,
            // 熔断出参（批⑩ 步 20）：外部执行闸门判 `Break` 的理由。
            &mut breaks,
        )
        .await;

        produced.extend(call_nodes.iter().cloned());
        // 父节点终态同步进落库副本——整条替换（执行器交回的就是完整快照）。
        // id 不存在的（协议失败兜底父节点）补入，使结果子节点不悬空。
        for full in &parent_updates {
            match produced.iter_mut().find(|m| m.id == full.id) {
                Some(msg) => *msg = full.clone(),
                None => produced.push(full.clone()),
            }
        }
        produced.extend(tool_msgs.iter().cloned());
        self.produced.lock().unwrap().extend(produced);
        // 三份收束派生事实出参收进取件面（`take_derived`）——调用方在桥释放前取走。
        {
            let mut d = self.derived.lock().unwrap();
            d.delegations.extend(delegations);
            d.tasks.extend(tasks);
            d.breaks.extend(breaks);
        }

        // ④ 映射为 core 的事实形状（运行器据此落 `artifact.added` 并拼下一轮 prompt）。
        let outcomes: Vec<DispatchOutcome> = tool_msgs
            .iter()
            .map(|m| {
                let call_id = m.parent_id.clone().unwrap_or_default();
                let name = m
                    .tool_name()
                    .map(str::to_string)
                    .or_else(|| {
                        turn.tool_calls
                            .iter()
                            .find(|tc| tc.id.as_deref() == Some(call_id.as_str()))
                            .and_then(|tc| tc.name.clone())
                    })
                    .unwrap_or_default();
                // 节点结构三字段（载荷补全）：`wire_id` / `args` 从 ToolCall 节点取
                //（结果子节点的 `parent_id` 就是它的节点 id；wire id 存 `tool_call_id`
                // 字段、定稿参数存 `content`），`result_id` 是结果子节点自己的 id——
                // 重建按这三个 id 复刻同一个工具卡片（与落库视图同源）。
                let call_node = call_nodes
                    .iter()
                    .chain(parent_updates.iter())
                    .find(|n| n.id == call_id && n.is_tool_call());
                DispatchOutcome {
                    call_id,
                    name,
                    text: m.content.as_ref().map(|c| c.to_text()).unwrap_or_default(),
                    ok: m
                        .meta
                        .as_ref()
                        .and_then(|meta| meta.get("success"))
                        .and_then(|v| v.as_bool())
                        .unwrap_or(true),
                    needs_user_action: m.is_interaction()
                        && m.status == Some(NodeStatus::WaitingUserAction),
                    // 卡片载荷与原因码：随事实入格，恢复（`resume`）据它复现那张卡。
                    // 只有等待用户的结果节点才带（其余为 `None` / 空串）。
                    prompt: m.meta.as_ref().and_then(|meta| meta.get("prompt")).cloned(),
                    failure_kind: m
                        .meta
                        .as_ref()
                        .and_then(|meta| meta.get("failure_kind"))
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    wire_id: call_node
                        .and_then(|n| n.tool_call_id.clone())
                        .unwrap_or_default(),
                    args: call_node
                        .and_then(|n| n.content.as_ref().map(|c| c.to_text()))
                        .unwrap_or_default(),
                    result_id: m.id.clone(),
                }
            })
            .collect();
        // ⑤ 关账：放行的每一笔都要有 `response` / `failed`（拒绝的那几笔由 `denied`
        //    关在授权行上）。放在结果映射之后：`call_id` 是执行层交回的权威 id。
        self.close_tool_calls(port_calls, &outcomes);
        outcomes
    }
}

// ── panic 面登记（PN-001…003）─────────────────────────────────────────
// 本文件每一处 `unwrap` / `expect` / `panic!` / `unreachable!` 的理由。登记放在
// 文件内而不是集中一张表：理由与它解释的那段代码会一起被 review、一起被删。
// 判据见 `scripts/panic-audit.mjs`。**加一处 panic 必须同时加一行登记，理由非空。**
// panic-allow symbio/src/plugins/loop/tool_dispatch.rs::take_produced: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
// panic-allow symbio/src/plugins/loop/tool_dispatch.rs::take_derived: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
// panic-allow symbio/src/plugins/loop/tool_dispatch.rs::dispatch: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
