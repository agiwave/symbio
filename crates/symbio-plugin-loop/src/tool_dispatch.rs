//! v2 的工具分发实现（插件本地的 [`ToolDispatch`] 契约）。
//!
//! ## 为什么分发在插件侧
//!
//! 分发的**全部输入**都在这里：插件宿主（路由工具能力）、请求上下文（能力注册表 /
//! 会话 id / 工作区）、事件出口（结果节点写进对话图）、会话目录（超长结果存档）。
//! core 认识这些即违 E-009；契约与实现因此都留在本插件（唯一消费方是
//! [`super::turn_runner`]，D-118 决策 5）。
//!
//! ## 三个动作，不是一次打包调用
//!
//! 工具批次的三笔债各自由一个动作承担（D-118 决策 5）：
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

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use symbio_core::chat_node::{
    ChatNode, MessageContent, NodeKind, NodeStatus, CHAT_STYLE_TAG_REASONING,
};
use symbio_core::{
    capability_resolve, llm_emit_message, llm_emit_state, tool_metas, ApprovalBasis,
    ApprovalGrants, CapabilityRiskLevel, Entity, Event, ExecAbortSignal, ExecEventSink, LlmTurn,
    Plugin, PluginInvokeRequest, PluginInvokeRequestExt, Seq, Store, Verb, APPROVAL_GRANTS,
    CAPABILITY_VISITOR, EVENT_FAILED, EVENT_REQUEST, EVENT_RESPONSE, MODE, RISK_LEVEL,
};
use symbio_plugin_api::effects::EffectInput;
use symbio_plugin_api::events::EVENT_AUTHZ_DENIED;

use crate::control_decl::EVENT_AUTHZ_GRANTED;

use super::loop_rows;
use super::port;
use super::tools::{is_unanswered, process_tool_calls_async, NOT_EXECUTED};
use super::turn_exec::UiBridge;
use symbio_core::node_build::{
    detach_if_flat, llm_build_text_node, llm_build_tool_call_nodes, PresentationTree,
};

/// 工具执行层交回的**收束派生事实**（效果 / 熔断）。
///
/// 两者的数据来源都在工具执行层（`EffectInput` / 熔断理由），而写方（落格）在
/// 轮末——`turn_exec` 调 [`super::derived_facts::record_derived`] 写。本结构是那份
/// 出参从分发点回到运行器的**唯一通道**：不收集它，这两样就静默全丢
/// （与记忆 / 学习同一个坑——出参不交回，写方就无从落格）。
///
/// 任务表**不在这里**：它是一条效果（`task_events::TaskTable`），走 `effects`
/// 那一条通路（DE-025 D-207）。
///
/// 形态仿 [`SessionDispatchPort::produced`]：`dispatch` 在 `&self` 上跑，出参却要
/// 跨 await 累积，故用 `Mutex` 收，调用方在桥释放前 `take_derived` 取走。
#[derive(Default)]
pub(crate) struct DerivedFacts {
    /// 效果出参（DE-025 D-207）：每次终态调用一条，轮末由写口问能力包要草稿。
    pub(crate) effects: Vec<EffectInput>,
    /// 熔断出参（批⑩ 步 20）：外部执行闸门判 `Break` 的理由。
    pub(crate) breaks: Vec<&'static str>,
}

/// 一笔已开账的工具端口调用（创建动作的产物）。
///
/// `granted = false` ⇒ 授权行已落 `authz.denied`（或授权事实落格失败），这笔调用
/// **没有** `sent`；执行层仍会跑它（审批 / 拒绝卡的节点由执行层唯一构造），但端口
/// 账已关。`sent = false` 且 `granted = true` ⇒ 派发行没有放行这笔账（行不欠 /
/// `sent` 落格失败），关账时按「没执行」处理。
///
/// `deny_why` 是拒绝的**收尾凭据**（b）：`authz.denied` 关的是授权行，
/// 而被拒的调用还要在批尾落一条 `failed{why}`（`deny-tools` 行的出口）——否则
/// 「拒绝」在端口账上只留下半句话，创建行的集合债永远少一个终局。
#[derive(Clone)]
pub(crate) struct ToolPortCall {
    /// 模型交回的调用 id（执行层按它回结果；与端口 id 是两件事）。
    pub(crate) model_id: String,
    /// 端口账的逻辑 id（`request` / `sent` / 应答同 id）。
    pub(crate) call: port::Call,
    /// 工具名（LLM 可见名；授权事实的载荷要它）。
    pub(crate) name: String,
    /// 能力名（canonical name）——注册表里认回来的那一个。
    ///
    /// 永久放行清单按它记（D-170）：线上名只是协议字符集的投影，把线上名记进清单
    /// 会让同一个工具因字符集不同而有两份授权。`None` = 注册表里没有可判的能力。
    pub(crate) canonical: Option<String>,
    /// 这次调用的风险档。`None` = 注册表里没有可判的能力（route-only 工具）。
    pub(crate) risk: Option<CapabilityRiskLevel>,
    /// 参数里显式带了 `approved: true`（已由人工确认过）。
    pub(crate) approved: bool,
    /// 模型交回的调用本身不可派发（缺 id / 缺名字 / 参数解析失败 / 超长）。
    ///
    /// 它仍然要开账（创建行的集合里有这个元素），随后由授权动作以
    /// `invalid_tool_call` 拒绝——不能跳过开账，那样创建债就永远欠着。
    pub(crate) invalid: bool,
    /// 授权动作的结论（创建时为 `false`，授权动作写入）。
    pub(crate) granted: bool,
    /// 授权动作写下的拒绝原因（`None` = 放行或还没判）；批尾据此落 `failed`。
    pub(crate) deny_why: Option<&'static str>,
    /// 派发动作是否已落 `sent`（派发动作写入）。
    pub(crate) sent: bool,
    /// 这一笔是**恢复幕的唤醒调用**（d）：由用户动作（批准 / 重试 /
    /// 补充 / 回答 / 拒绝）落成的 `request`，不是某个模型响应里的调用。
    ///
    /// 派发动作据它决定「跑工具」还是「只合成结果」——两者的事实完全一样
    /// （`request → authz → sent → response / failed`），差别只在读侧。
    pub(crate) wake: Option<WakeIntent>,
}

/// 唤醒调用的种类（d）。
///
/// 恢复幕没有新的模型应答，工具批的入口是**用户动作**。用户动作先落成一笔
/// `request(port=tool)`（[`ToolPort::land_wake`]），之后与批次走同一条授权 / 派发
/// 链；本枚举只说明那一笔要不要真正跑工具。
#[derive(Debug, Clone)]
pub(crate) enum WakeIntent {
    /// approve / retry / supply：用户要求重跑一次工具。
    Execute,
    /// answer / reject：用户给出了结果（答案或拒绝理由），工具不再执行。
    ///
    /// `denied = true` ⇒ 授权行落 `authz.denied`、派发尾落 `failed{user_rejected}`；
    /// 其余 ⇒ 授权放行、派发落 `response`。
    Result {
        text: String,
        ok: bool,
        denied: bool,
    },
}

impl WakeIntent {
    /// 从 `request` 载荷的 `resume` 字段还原（[`Self::kind`] 的反函数）。
    fn from_payload(payload: &serde_json::Value) -> Option<Self> {
        match payload.get("resume").and_then(|v| v.as_str())? {
            "answer" => Some(Self::Result {
                text: payload
                    .get("result_text")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                ok: true,
                denied: false,
            }),
            "reject" => Some(Self::Result {
                text: payload
                    .get("result_text")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                ok: false,
                denied: true,
            }),
            _ => Some(Self::Execute),
        }
    }
}

/// 一次唤醒**要落的内容**（[`ToolPort::land_wake`] 的入参）。
///
/// 打包成一个形状而不是七个位置参数：这些字段同属「用户动作」这一件事，分成
/// 位置参数时调用点要靠顺序记住谁是谁，而顺序错了编译器不报错——载荷里
/// `wire_id` 与 `turn_id` 互换的后果是结果挂错卡片。
pub(crate) struct WakeSpec<'a> {
    /// 用户动作的种类（`approve` / `retry` / `supply` / `answer` / `reject`）——
    /// 落进载荷的 `resume` 标记，读侧据此还原意图。
    pub(crate) kind: &'static str,
    /// 被唤醒的那张卡（ToolCall 节点 id）：结果挂回它。
    pub(crate) target: &'a str,
    /// 工具名（LLM 可见名）。
    pub(crate) tool: &'a str,
    /// 这次调用的参数。
    pub(crate) args: &'a serde_json::Value,
    /// ToolCall 节点的 provider wire id。
    pub(crate) wire_id: &'a str,
    /// 本轮 Turn 根节点 id（UI 帧挂载的父节点）。
    pub(crate) turn_root: &'a str,
    /// 这一笔要不要真跑工具。
    pub(crate) intent: &'a WakeIntent,
}

/// 一次唤醒的**读侧形状**：把 `request` 载荷还原成派发 / 授权动作要的那份调用。
///
/// 为什么在派发时才还原、而不是像模型批次那样把创建结果寄存在草稿里：唤醒请求
/// 是在**开幕之前**落的（它要让 `turn_head` 看见「本幕有活没干完」），那时还没进
/// 执行体。行表选中授权 / 派发行时，草稿里没有 `pending_*`——这一格就是那两行
/// 动作的入口。
pub(crate) struct WakeBatch {
    /// 还原出的「模型往返」：`tool_calls` 里只有被唤醒的那一笔，节点 id 就是原
    /// ToolCall（结果因此挂回原来那张卡，而不是新长一张）。
    pub(crate) turn: LlmTurn,
    /// 还原出的端口账（一笔）。**意图与目标节点都在这一笔里**
    /// （`calls[0].wake` / `calls[0].model_id`）——不另立第二份形状。
    pub(crate) calls: Vec<ToolPortCall>,
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
    /// plan/11 批 2）。
    pub needs_user_action: bool,
    /// 等待用户时卡片载荷（工具自决的 `prompt` 形状：审批卡 / 提问卡不同）。
    ///
    /// **它是事实的一部分**：`needs_user_action` 只说明「这轮停在等用户」，而恢复
    /// （`resume`）要拿 `meta.prompt` 复现那张卡。 起读路径走 WAL 重建 ⇒ 这张卡
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
    /// 能力对**产物格**的补充字段（DE-025 D-207 的富化面）。
    ///
    /// 由工具声明的行为交出（今天唯一的生产者是 vdfs 写族的 `file_op`），随
    /// `artifact.added` 的载荷入格。运行器只负责把它们并进载荷——**不认识任何键名**；
    /// 读侧（工作目录变更账）认字段不认工具名，所以改一个工具名不影响任何读方。
    pub annotations: Vec<(&'static str, serde_json::Value)>,
}

/// 工具派发的**三个动作**（D-118 决策 5）：创建 / 授权 / 派发。
///
/// 三笔债不再由一次调用代劳——运行器按行各调一次，每一步只写自己那一行的
/// 领域事实。契约住插件本地，core 不认识工具分发（E-009）。
///
/// ## 为什么每个方法都收一个 `store`
///
/// 生产实现自己有写口（[`ToolPort`] 持 `Arc<dyn Store>`），这个参数在那里是重复的；
/// 它服务于**测试替身**：行表按事实判欠不欠，替身若不落协议事实，创建行永远欠着，
/// 派发器会在同一笔债上空转到步数上界。把「写哪一份日志」交给调用点（派发器手里
/// 正拿着这一轮的那份），替身就与生产走同一条循环。
#[async_trait]
pub(crate) trait ToolDispatch: Send + Sync {
    /// 创建：为模型响应里交回的调用逐条写 `request(port=tool)`。
    async fn open(&self, store: &dyn Store<Event = Event>, turn: &LlmTurn) -> Vec<ToolPortCall>;

    /// 授权：为已开账的调用写 `authz.granted` / `authz.denied`。
    ///
    /// 拒绝即止：`granted` 保持 `false`，派发动作不会为它落 `sent`。
    async fn authorize(&self, store: &dyn Store<Event = Event>, calls: &mut [ToolPortCall]);

    /// 派发：为放行的调用落 `sent`、跑执行层、关账，返回结果事实。
    async fn dispatch(
        &self,
        store: &dyn Store<Event = Event>,
        turn: &LlmTurn,
        calls: &mut [ToolPortCall],
    ) -> Vec<DispatchOutcome>;

    /// 读一笔**唤醒调用**（d）。
    ///
    /// 恢复幕没有模型应答可寄存：用户在等待卡上点了按钮，唤醒请求已在开幕之前
    /// 落格。行表选中授权 / 派发行时，动作从这里把那一笔取回来——没有唤醒请求
    /// ⇒ `None`，调用方按「引擎选错了账」处理（不写任何格）。
    async fn wake(&self, _store: &dyn Store<Event = Event>) -> Option<WakeBatch> {
        None
    }
}

/// 工具端口账的写方：批次分发与 resume 共用**同一套**开账 / 关账。
///
/// 为什么把它从 `SessionDispatchPort` 里拆出来：resume 的批准路径也要落
/// `request → granted → sent → response`（/ 判据 6）。如果
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
/// 创建行 `open-tools` 的集合展开（D-117）从 `response.calls` 的元素里取 id，
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
            && e.kind == symbio_core::EVENT_RESPONSE
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
    /// 跨会话的永久放行清单（工具 → 上限），与会话阈值同一处解析（D-170）。
    grants: ApprovalGrants,
    mode: String,
}

impl ToolPortPolicy {
    async fn read(ctx: &Arc<dyn PluginInvokeRequest>) -> Self {
        let known: Vec<String> = tool_metas(ctx).await.into_iter().map(|m| m.name).collect();
        // 阈值读法与执行闸门同一处口径：`ctx[RISK_LEVEL]`，缺席 = `Medium`。
        // 放行依据只问 `CapabilityRiskLevel::approval_basis`（唯一实现）。
        let threshold = ctx
            .get(RISK_LEVEL)
            .map(|s| CapabilityRiskLevel::from_name(&s))
            .unwrap_or_default();
        let grants = ctx
            .get(APPROVAL_GRANTS)
            .map(|s| ApprovalGrants::parse(&s))
            .unwrap_or_default();
        let mode = ctx.get(MODE).unwrap_or_default();
        Self {
            known,
            threshold,
            grants,
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
    /// 入口先问创建行 `open-tools` 欠不欠（D-117）：一个模型响应是一笔集合债，
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
        // 旧存档里的模型应答没有 `calls`（D-117 之前写的）：没有创建债可言，
        // 按旧路径继续派发，不能因为升级把在飞的会话卡死。带 `calls` 的应答
        // 则必须由创建行先开账——它不欠就说明这一批已经落过 `request`。
        let has_calls = latest.is_some_and(|e| e.payload.get("calls").is_some());
        // obligation-precheck OB-004: 执行体（OPENER）动手前的复核——同一批被 resume / 重放第二次带进来时，创建行已由 `until` 关掉，早退避免重复落 `request`（不是第二套调度：这里不选行、不排行，只问本行还欠不欠）
        if has_calls && !loop_rows::owes("open-tools", &facts, self.turn, self.now_ms(&facts)) {
            symbio_core::plugin_warn!(
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
            // 不可派发的调用**也开账**：创建行的集合里有这个元素，跳过它那一笔
            // 集合债就永远欠着。它的终局是「授权拒绝 + 批尾 failed」，不是「不存在」。
            let invalid = model_id.is_none()
                || name.is_none()
                || tc.parse_error.is_some()
                || tc.overflow_notice.is_some();
            if let Some(mut open) = self
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
                open.invalid = invalid;
                out.push(open);
            }
        }
        out
    }

    /// 创建动作的单笔实体：只写 `request`（D-118 决策 5 的第一笔债）。
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
        let resolved = risk_of(ctx, name, args, &policy.known).await;
        let risk = resolved.as_ref().map(|r| r.risk);
        // 注册表里没有能力可判（route-only 工具）⇒ 放行是**交给下游路由闸门**
        // 的延迟判定；不能在这里凭一个猜出来的风险档拒绝它。
        let deferred = risk.is_none();
        let risk_name = risk.map(CapabilityRiskLevel::as_str).unwrap_or("unknown");

        if let Err(e) = port::announce_with(
            self.facts.as_ref(),
            call,
            self.turn,
            self.user_seq,
            serde_json::json!({
                "tool": name,
                "risk": risk_name,
                "threshold": policy.threshold.as_str(),
                "approved": approved,
                "deferred": deferred,
            }),
        ) {
            symbio_core::plugin_warn!(
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
            canonical: resolved.map(|r| r.canonical),
            risk,
            approved,
            invalid: false,
            granted: false,
            deny_why: None,
            sent: false,
            wake: None,
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
        // obligation-precheck OB-004: 执行体（AUTHORIZER）动手前的复核——批内授权与策略变更后的续跑共用这份实体，第二次进入时授权行已落事实、不再欠，早退避免重复判放行
        if !loop_rows::owes("authorize-tool", &facts, self.turn, self.now_ms(&facts)) {
            symbio_core::plugin_warn!(
                "session",
                "[Tool] 授权行不欠（turn={}），不判放行",
                self.turn
            );
            return;
        }
        for call in calls.iter_mut() {
            // 非法调用先判：它没有可执行的名字 / 参数，风险档再低也不能放行。
            //
            // 放行依据只问 `approval_basis`（与执行闸门同一份判据）：三档分别是
            // 本次批准 / 会话阈值 / 该工具的永久放行上限。route-only 工具没有可判
            // 的风险（`None`），放行延迟给下游路由闸门。
            let (basis, grant) = match call.risk {
                Some(risk) => {
                    let grant = call
                        .canonical
                        .as_deref()
                        .and_then(|tool| policy.grants.ceiling(tool));
                    (
                        risk.approval_basis(policy.threshold, call.approved, grant),
                        grant,
                    )
                }
                None => (None, None),
            };
            let granted = !call.invalid && (call.risk.is_none() || basis.is_some());
            let risk_name = call
                .risk
                .map(CapabilityRiskLevel::as_str)
                .unwrap_or("unknown");
            if granted {
                // `grant` 只记生效的那一次上限：依据是会话阈值时，清单里有没有
                // 这一条与本次放行无关，写进去会让事后读日志的人以为靠的是它。
                let applied_grant = match basis {
                    Some(ApprovalBasis::Grant) => grant,
                    _ => None,
                };
                if let Err(e) = append_authz(
                    self.facts.as_ref(),
                    granted_event(
                        self.turn,
                        self.user_seq,
                        &call.call,
                        &call.name,
                        GrantFacts {
                            risk: risk_name,
                            deferred: call.risk.is_none(),
                            basis: basis.map(ApprovalBasis::as_str),
                            grant: applied_grant,
                        },
                    ),
                ) {
                    symbio_core::plugin_warn!(
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
                // 两种都不是「已派发」，所以只有 `denied`、没有 `sent`。非法调用
                // 不分模式：执行层会回一条协议错误，原因码独立。
                let why: &'static str = if call.invalid {
                    "invalid_tool_call"
                } else if policy.mode == "auto" {
                    "permission_denied"
                } else {
                    "needs_approval"
                };
                if let Err(e) = append_authz(
                    self.facts.as_ref(),
                    denied_event(self.turn, self.user_seq, &call.call, &call.name, why),
                ) {
                    symbio_core::plugin_warn!(
                        "session",
                        "[Tool] 拒绝事实未落格（turn={} / {}）：{e:?}",
                        self.turn,
                        call.model_id
                    );
                    continue;
                }
                call.deny_why = Some(why);
            }
        }
    }

    /// 派发动作的前半（第三笔债）：为放行的调用落 `sent`。
    ///
    /// 执行层由调用方跑（`SessionDispatchPort::dispatch`）——`sent` 的语义是
    /// 「已交给执行层」，所以它必须先于执行。
    ///
    /// 派发行欠不欠由引擎现读（D-136）：这里只交出授权行与读账的 `now`，
    /// 引擎回 [`port::DispatchError::NotOwed`] 就是「没有这笔账」，如实落 `failed`。
    pub(crate) fn begin_calls(&self, calls: &mut [ToolPortCall]) {
        if !calls.iter().any(|c| c.granted) {
            return;
        }
        let facts = self.facts.range(Seq::new(0));
        let now = self.now_ms(&facts);
        for call in calls.iter_mut().filter(|c| c.granted) {
            match port::begin(
                self.facts.as_ref(),
                loop_rows::PORTS,
                port::Dispatch {
                    call: &call.call,
                    turn: self.turn,
                    produced_by: self.user_seq,
                    row: "dispatch-tools",
                    now,
                    // 唤醒调用能由 `wake_batch` 从事实重建（有活着的等待方）；
                    // 普通批次把结果等在当前草稿里，进程一死就没人接了。
                    awaited: call.wake.is_some(),
                },
            ) {
                Ok(()) => call.sent = true,
                Err(port::DispatchError::NotOwed(row)) => {
                    symbio_core::plugin_warn!(
                        "session",
                        "[Tool] 派发行 `{row}` 不欠（turn={} / {}），不落 sent",
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
                }
                Err(e) => symbio_core::plugin_warn!(
                    "session",
                    "[Tool] 工具 sent 未落格（turn={} / {}）：{e:?}",
                    self.turn,
                    call.model_id
                ),
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

    /// 落一笔**唤醒请求**（d）：用户动作在开幕之前变成 `request(port=tool)`。
    ///
    /// ## 为什么只落 `request`，不顺便落授权
    ///
    /// 唤醒之后要走的是**同一条**债链：`authorize-tool`（0）判放行 / 拒绝，
    /// `dispatch-tools` / `deny-tools`（68 / 69）落 `sent` 并收尾。在这里把三笔
    /// 一起写掉，等于让恢复路径又长出一条绕过行表的旁路——那正是本步要退役的东西。
    ///
    /// ## 旧账先关
    ///
    /// 同轮里若还有一笔**开着**的唤醒请求（上次进程死在派发中途），先按
    /// `failed{superseded}` 关掉：它的结果永远不会来，而新的一次用户动作是新账。
    /// 不关的话 `turn_head.tools_open` 会看见两笔开着，`departed` 的集合债也永远
    /// 差一个终局。
    pub fn land_wake(&self, spec: &WakeSpec<'_>) -> Result<(), symbio_core::AppendError> {
        let facts = self.facts.range(Seq::new(0));
        for stale in facts.iter().filter(|e| {
            e.turn == self.turn && is_wake_request(e) && !wake_terminal(&facts, self.turn, e)
        }) {
            let call = port::Call::new(
                "tool",
                stale
                    .payload
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default(),
                stale
                    .payload
                    .get("attempt")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(1),
            );
            port::failed(
                self.facts.as_ref(),
                &call,
                self.turn,
                self.user_seq,
                "superseded",
            )?;
        }
        let attempt = self.resume_attempt(spec.target);
        let call = port::Call::new(
            "tool",
            format!("tool-resume-{}-{}-{}", self.turn, spec.target, attempt),
            attempt,
        );
        let mut extra = serde_json::json!({
            "tool": spec.tool,
            "args": spec.args,
            "wire_id": spec.wire_id,
            "turn_id": spec.turn_root,
            "target": spec.target,
            "resume": spec.kind,
        });
        if let WakeIntent::Result { text, ok, .. } = spec.intent {
            extra["result_text"] = serde_json::json!(text);
            extra["result_ok"] = serde_json::json!(ok);
        }
        port::announce_with(self.facts.as_ref(), &call, self.turn, self.user_seq, extra)
    }

    /// 读回**开着**的唤醒请求（最新一笔），还原成授权 / 派发动作的入参。
    ///
    /// 终局按**事实 seq** 判（`response` / `failed` 晚于请求即关闭）——与
    /// `turn_head.tools_open` 的请求面同一口径。已关账 ⇒ `None`：这一笔的派发
    /// 已经发生过，动作不该再跑第二遍。
    pub(crate) fn wake_batch(&self) -> Option<WakeBatch> {
        let facts = self.facts.range(Seq::new(0));
        let req = facts.iter().rev().find(|e| {
            e.turn == self.turn && is_wake_request(e) && !wake_terminal(&facts, self.turn, e)
        })?;
        let payload = &req.payload;
        let target = payload.get("target")?.as_str()?.to_string();
        let tool = payload
            .get("tool")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let args = payload
            .get("args")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        let wire_id = payload
            .get("wire_id")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let id = payload.get("id").and_then(|v| v.as_str())?.to_string();
        let attempt = payload.get("attempt").and_then(|v| v.as_u64()).unwrap_or(1);
        let intent = WakeIntent::from_payload(payload)?;
        let authz = facts.iter().find_map(|e| {
            // 授权事实按**同一次尝试**取（D-138）：重试后的 attempt 2 不能复用
            // attempt 1 的放行，否则 `dispatch-tools` 的新账永远不欠。
            // 旧日志里的授权没有 `attempt`：只对首次尝试按缺席兼容。
            let authz_attempt = e.payload.get("attempt").and_then(|v| v.as_u64());
            let same = e.payload.get("port").and_then(|v| v.as_str()) == Some("tool")
                && e.payload.get("id").and_then(|v| v.as_str()) == Some(id.as_str())
                && (authz_attempt == Some(attempt) || (authz_attempt.is_none() && attempt == 1));
            if !same {
                return None;
            }
            match e.kind.as_str() {
                EVENT_AUTHZ_GRANTED => Some((true, None)),
                EVENT_AUTHZ_DENIED => Some((false, Some("user_rejected"))),
                _ => None,
            }
        });
        let (granted, deny_why) = authz.unwrap_or((false, None));
        let turn = LlmTurn {
            tool_calls: vec![symbio_core::TurnToolCallInfo {
                id: Some(target.clone()),
                wire_id: (!wire_id.is_empty()).then(|| wire_id.clone()),
                name: Some(tool.clone()),
                arguments: args.clone(),
                parse_error: None,
                overflow_notice: None,
            }],
            ..Default::default()
        };
        Some(WakeBatch {
            turn,
            calls: vec![ToolPortCall {
                model_id: target.clone(),
                call: port::Call::new("tool", id, attempt),
                name: tool,
                canonical: None,
                risk: None,
                approved: args
                    .get("approved")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                invalid: false,
                granted,
                deny_why,
                sent: false,
                wake: Some(intent),
            }],
        })
    }

    /// 授权唤醒调用（`authorize-tool` 行的动作在草稿里没有批次时的入口）。
    ///
    /// 用户动作是**授权本身**：批准 / 重试 / 补充 / 回答一律放行；拒绝落
    /// `authz.denied{user_rejected}`。风险档比较不参与——用户已经做过这一次判断，
    /// 再按阈值拒一次会让审批卡永远点不动。
    pub(crate) fn authorize_wake(&self, calls: &mut [ToolPortCall]) {
        let facts = self.facts.range(Seq::new(0));
        // obligation-precheck OB-004: 执行体（AUTHORIZER）动手前的复核——唤醒路径与批内授权是两个入口进同一次判定，行已不欠说明这轮的放行落过事实，早退避免重复写授权格
        if !loop_rows::owes("authorize-tool", &facts, self.turn, self.now_ms(&facts)) {
            symbio_core::plugin_warn!(
                "session",
                "[Tool] 授权行不欠（turn={}），不判唤醒放行",
                self.turn
            );
            return;
        }
        for call in calls.iter_mut() {
            let denied = matches!(call.wake, Some(WakeIntent::Result { denied: true, .. }));
            let written = if denied {
                append_authz(
                    self.facts.as_ref(),
                    denied_event(
                        self.turn,
                        self.user_seq,
                        &call.call,
                        &call.name,
                        "user_rejected",
                    ),
                )
            } else {
                append_authz(
                    self.facts.as_ref(),
                    granted_event(
                        self.turn,
                        self.user_seq,
                        &call.call,
                        &call.name,
                        GrantFacts {
                            risk: "unknown",
                            deferred: true,
                            basis: Some(ApprovalBasis::Approved.as_str()),
                            grant: None,
                        },
                    ),
                )
            };
            if let Err(e) = written {
                symbio_core::plugin_warn!(
                    "session",
                    "[Tool] 唤醒授权事实未落格（turn={} / {}）：{e:?}",
                    self.turn,
                    call.model_id
                );
                continue;
            }
            if denied {
                call.deny_why = Some("user_rejected");
            } else {
                call.granted = true;
            }
        }
    }

    /// 批次返回后关账：**每一笔**都要有终局。
    ///
    /// - 放行且已派发：`response`（工具回了话）或 `failed`（执行层没跑到它）；
    /// - 被拒（`deny_why` 有值）：`failed{why}`——授权行已由 `authz.denied` 关掉，
    ///   但创建行的集合债与「拒绝收尾」这一笔债务（`deny-tools`）还等着终局；
    /// - 放行但没落 `sent`：`begin_calls` 已经替它落过 `failed`，这里不重复。
    ///
    /// 「有结果节点」≠「工具回了话」：批尾收口给没跑到的调用补的是**框架的交代**
    /// （一句话占位），它的成因写在 `meta.failure_kind` 上。占位按成因落
    /// `failed{why}` 而不是 `response`——否则中止的调用在日志里说「回来了」，而它
    /// 从未执行，`await-tools` 的 `failed` 出口就只剩 resume 一条路（[D-121](../../../docs/003-design/015-义务环.md#d-121-唤醒事实的行语义await-行的收清走协议格审批与补充重新开账 "DE-015-义务环")
    /// 的中止作用、DE-015 判据 6 场景三）。词表与判据同源：[`super::tools`] 门面的
    /// `is_unanswered` / `ABORTED` / `NOT_EXECUTED`。
    pub(crate) fn close_batch(&self, calls: &[ToolPortCall], outcomes: &[DispatchOutcome]) {
        for call in calls {
            if !call.granted {
                if let Some(why) = call.deny_why {
                    self.fail(call, why);
                }
                continue;
            }
            if !call.sent {
                continue;
            }
            match outcomes.iter().find(|o| o.call_id == call.model_id) {
                Some(o) if is_unanswered(&o.failure_kind) => self.fail(call, &o.failure_kind),
                Some(_) => self.close(call, true),
                None => self.close(call, false),
            }
        }
    }

    /// 单笔关账（resume 用）：执行体已经给出成败。
    ///
    /// `answered = false` 同样是「执行层没跑到」，词表与批尾收口共用
    /// [`NOT_EXECUTED`]。
    pub(crate) fn close(&self, call: &ToolPortCall, answered: bool) {
        if answered {
            if let Err(e) = port::succeed(self.facts.as_ref(), &call.call, self.turn, self.user_seq)
            {
                symbio_core::plugin_warn!(
                    "session",
                    "[Tool] 工具应答事实未落格（turn={} / {}）：{e:?}",
                    self.turn,
                    call.model_id
                );
            }
        } else {
            self.fail(call, NOT_EXECUTED);
        }
    }

    /// 单笔失败关账：落 `failed{why}`，与「应答」区分。
    ///
    /// 三个调用方：批尾关账（[`Self::close_batch`]，`why` 取结果节点的成因或被拒
    /// 原因）、resume 的中止路径（`why = ABORTED`）、以及「放行但没派出去」那几笔
    /// （`begin_calls` 自己落 `failed{dispatch_row_not_owed}`）。
    pub(crate) fn fail(&self, call: &ToolPortCall, why: &str) {
        if let Err(e) = port::failed(
            self.facts.as_ref(),
            &call.call,
            self.turn,
            self.user_seq,
            why,
        ) {
            symbio_core::plugin_warn!(
                "session",
                "[Tool] 工具失败事实未落格（turn={} / {}）：{e:?}",
                self.turn,
                call.model_id
            );
        }
    }
}

/// 这一格是不是一笔唤醒请求（用户动作落成的工具调用）。
fn is_wake_request(e: &Event) -> bool {
    e.kind == EVENT_REQUEST
        && e.payload.get("port").and_then(|v| v.as_str()) == Some("tool")
        && e.payload.get("resume").is_some()
}

/// 唤醒请求是否已有终局（同逻辑 id 的 `response` / `failed` 晚于它）。
fn wake_terminal(facts: &[Event], turn: u64, req: &Event) -> bool {
    let id = req.payload.get("id").and_then(|v| v.as_str()).unwrap_or("");
    let at = req.seq.map(|s| s.value()).unwrap_or(0);
    facts.iter().any(|e| {
        e.turn == turn
            && matches!(e.kind.as_str(), EVENT_RESPONSE | EVENT_FAILED)
            && e.payload.get("port").and_then(|v| v.as_str()) == Some("tool")
            && e.payload.get("id").and_then(|v| v.as_str()) == Some(id)
            && e.seq.map(|s| s.value()).unwrap_or(0) > at
    })
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
    /// 本会话事实轨：闸门判据按值取自它（D-084 §2，能力面不见存储实现）。
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
    /// 淘汰闸的部署级判据（session 已折成**一个值**，见 D-086 §4）。
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

    /// 本会话主体：节点诞生时写死的主体身份（D-147 档 2）。
    fn principal(&self) -> symbio_core::principal::Principal {
        symbio_core::principal::Principal::from_legacy(&symbio_core::request_principal(
            self.ctx.as_ref(),
        ))
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

    /// 用户结果（answer / reject）的派发：不跑执行层，直接合成调用卡与结果卡。
    ///
    /// 与正常派发共用**同一条关账**（`begin_calls` + `close_batch`）：回答是
    /// `sent → response`，拒绝是 `denied → failed`（授权行已落 `authz.denied`，
    /// `close_batch` 按 `deny_why` 收尾）。
    ///
    /// 调用卡沿用原节点 id（`turn.tool_calls[0].id == target`）：重建面按
    /// `artifact.added` 的 `tool_call_id` 复刻同一张卡，实时面这里发的是**终态
    /// 整条**——两面的 id、参数文本与状态因此一致。
    async fn dispatch_wake_result(
        &self,
        turn: &LlmTurn,
        port_calls: &mut [ToolPortCall],
    ) -> Vec<DispatchOutcome> {
        self.port.begin_calls(port_calls);
        let Some(call) = port_calls.first() else {
            return Vec::new();
        };
        let (text, ok) = match &call.wake {
            Some(WakeIntent::Result { text, ok, .. }) => (text.clone(), *ok),
            _ => (String::new(), false),
        };
        let failure_kind = if ok { "" } else { "error" };
        let name = call.name.clone();
        let final_status = if ok {
            NodeStatus::Completed
        } else {
            NodeStatus::Failed
        };

        let principal = self.principal();
        let mut call_nodes = llm_build_tool_call_nodes(&self.root_id, &turn.tool_calls, &principal);
        for node in call_nodes.iter_mut() {
            detach_if_flat(node, &self.root_id, self.presentation_tree);
            node.status = Some(final_status.clone());
            // **并入**而非整格替换：`meta.style_tag` / `tool_name` 是卡片身份
            // （与执行器 `emit_parent_finalized` 的合并口径同构）。
            let mut meta = node
                .meta
                .clone()
                .filter(|m| m.is_object())
                .unwrap_or_else(|| serde_json::json!({}));
            if let Some(obj) = meta.as_object_mut() {
                obj.insert("success".into(), serde_json::json!(ok));
                obj.insert("tool_name".into(), serde_json::json!(name));
                if !ok {
                    obj.insert("failure_kind".into(), serde_json::json!("error"));
                }
            }
            node.meta = Some(meta);
        }
        let call_id = call_nodes
            .first()
            .map(|n| n.id.clone())
            .unwrap_or_else(|| call.model_id.clone());
        let wire_id = call_nodes
            .first()
            .and_then(|n| n.tool_call_id.clone())
            .unwrap_or_default();
        let args = call_nodes
            .first()
            .and_then(|n| n.content.as_ref().map(|c| c.to_text()))
            .unwrap_or_default();
        let result_id = symbio_core::llm_short_id();
        let mut result_meta = serde_json::json!({ "success": ok, "tool_name": name });
        if !ok {
            result_meta["failure_kind"] = serde_json::json!("error");
        }
        let result = ChatNode {
            id: result_id.clone(),
            parent_id: Some(call_id.clone()),
            // 身份只写主体这一份（D-147）：工具结果的族由 `tool_principal` 折出，
            // 再写一遍 `role` 是两个表达说同一件事。
            principal: Some(symbio_core::principal::Principal::tool_principal(&name)),
            msg_type: Some(NodeKind::Body),
            content: Some(MessageContent::Text(text.clone())),
            status: Some(final_status),
            meta: Some(result_meta),
            timestamp: Some(symbio_core::clock_now_ms()),
            ..Default::default()
        };

        for node in call_nodes.iter().chain(std::iter::once(&result)) {
            llm_emit_message(&self.sink, node.clone()).await;
        }
        let outcome = DispatchOutcome {
            call_id,
            name,
            text,
            ok,
            needs_user_action: false,
            prompt: None,
            failure_kind: failure_kind.to_string(),
            wire_id,
            args,
            result_id,
            // 唤醒路径不跑工具（结果由调用方给），因此没有任何行为参与。
            annotations: Vec::new(),
        };
        let mut produced = call_nodes;
        produced.push(result);
        self.produced.lock().unwrap().extend(produced);
        self.close_tool_calls(port_calls, std::slice::from_ref(&outcome));
        vec![outcome]
    }
}

/// 一次调用的可判风险：能力名与本次风险档。
///
/// 两个值必须一起回来——授权级联既要按风险判放行，又要按**能力名**查永久放行清单；
/// 只回风险档会让记录方再认一次名字，两次解析就可能给出两个答案。
struct ToolRisk {
    canonical: String,
    risk: CapabilityRiskLevel,
}

/// 这次调用的风险档。`None` = 注册表里没有可判的能力（route-only 工具）。
async fn risk_of(
    ctx: &Arc<dyn PluginInvokeRequest>,
    name: &str,
    args: &serde_json::Value,
    known: &[String],
) -> Option<ToolRisk> {
    let canonical = capability_resolve(name, known.iter().map(String::as_str))?;
    let visitor = ctx.get(CAPABILITY_VISITOR)?;
    let cap = visitor
        .get(canonical)
        .await
        .and_then(|r| r.cap().cloned())?;
    Some(ToolRisk {
        canonical: canonical.to_string(),
        risk: cap.risk_for(args),
    })
}

/// `authz.granted` 里除「哪一笔调用」之外的判定字段。
///
/// 打包成一个形状而不是四个位置参数：`risk` / `deferred` / `basis` / `grant` 同属
/// 「这次凭什么放行」这一件事，散成位置参数时调用点要靠顺序记住谁是谁——顺序错了
/// 编译器不报错，而载荷里 `basis` 与 `grant` 互换的后果是日志说错放行依据。
struct GrantFacts<'a> {
    risk: &'a str,
    deferred: bool,
    /// 放行依据（三档）；`None` = 没有依据可记（route-only 工具的延迟判定）。
    basis: Option<&'a str>,
    /// 生效的工具上限；仅 `basis = grant` 时有值。
    grant: Option<CapabilityRiskLevel>,
}

fn granted_event(
    turn: u64,
    produced_by: u64,
    call: &port::Call,
    tool: &str,
    facts: GrantFacts<'_>,
) -> Event {
    let id = &call.id;
    let attempt = call.attempt;
    let mut payload = serde_json::json!({
        "key": id,
        "port": "tool",
        "id": id,
        "attempt": attempt,
        "tool": tool,
        "risk": facts.risk,
        "deferred": facts.deferred,
    });
    // 三档的差别就是这两个字段：`basis` 回答「凭什么没问」，`grant` 只在依据是
    // 永久放行时出现（生效上限）。route-only 工具没有依据，两个字段就都不出现。
    if let Some(basis) = facts.basis {
        payload["basis"] = serde_json::json!(basis);
    }
    if let Some(grant) = facts.grant {
        payload["grant"] = serde_json::json!(grant.as_str());
    }
    Event::pending(
        format!("grant-{turn}-{id}-{attempt}"),
        EVENT_AUTHZ_GRANTED,
        Entity::Turn,
        Verb::Asserted,
        turn,
        "authorizer",
    )
    .with_produced_by(produced_by)
    .with_payload(payload)
}

fn denied_event(turn: u64, produced_by: u64, call: &port::Call, tool: &str, why: &str) -> Event {
    let id = &call.id;
    let attempt = call.attempt;
    Event::pending(
        format!("deny-tool-{turn}-{id}-{attempt}"),
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
        "attempt": attempt,
        "tool": tool,
        "kind": "tool",
        "reason": symbio_core::ClosureReason::Denied.as_str(),
        "detail": why,
    }))
}

fn append_authz<S>(store: &S, event: Event) -> Result<(), symbio_core::AppendError>
where
    S: Store<Event = Event> + ?Sized,
{
    match store.append(event) {
        Ok(_) => Ok(()),
        Err(symbio_core::AppendError::Duplicate) => Ok(()),
        Err(e) => Err(e),
    }
}

#[async_trait]
impl ToolDispatch for SessionDispatchPort {
    async fn open(&self, _store: &dyn Store<Event = Event>, turn: &LlmTurn) -> Vec<ToolPortCall> {
        self.open_action(turn).await
    }

    async fn authorize(&self, _store: &dyn Store<Event = Event>, calls: &mut [ToolPortCall]) {
        if calls.is_empty() {
            // 恢复幕：草稿里没有批次，判断的是开幕前列队的那一笔唤醒调用。
            if let Some(mut batch) = self.port.wake_batch() {
                self.port.authorize_wake(&mut batch.calls);
            }
            return;
        }
        self.authorize_action(calls).await;
    }

    async fn wake(&self, _store: &dyn Store<Event = Event>) -> Option<WakeBatch> {
        self.port.wake_batch()
    }

    async fn dispatch(
        &self,
        _store: &dyn Store<Event = Event>,
        turn: &LlmTurn,
        port_calls: &mut [ToolPortCall],
    ) -> Vec<DispatchOutcome> {
        // 唤醒的用户结果（回答 / 拒绝）不跑工具：节点与事实在这里合成。
        // 批准 / 重试 / 补充仍然走下面的正常派发路径——执行层只知道
        // 「有一次调用要跑」，不关心它是模型要的，还是用户点的。
        if port_calls
            .iter()
            .any(|c| matches!(c.wake, Some(WakeIntent::Result { .. })))
        {
            return self.dispatch_wake_result(turn, port_calls).await;
        }
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
            let mut msg =
                llm_build_text_node(&self.root_id, &turn.text, None, node_id, &self.principal());
            detach_if_flat(&mut msg, &self.root_id, self.presentation_tree);
            // 状态帧：正文已由增量帧传过，重发正文会与队列里的增量打架。
            llm_emit_state(&self.sink, msg.clone()).await;
            produced.push(msg);
        }
        if let Some((node_id, text)) = reason_node {
            let mut msg = ChatNode {
                id: node_id,
                parent_id: Some(self.root_id.clone()),
                principal: Some(self.principal()),
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
        let mut call_nodes =
            llm_build_tool_call_nodes(&self.root_id, &turn.tool_calls, &self.principal());
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
        // 收束派生事实出参（效果 / 熔断）：数据来源在工具执行层，写方在轮末——
        // 由本分发方收下、`turn_exec` 轮末调 `derived_facts::record_derived` 落格。
        // 不收这两份，代际立约 / 任务表 / 文件变更 / 熔断就静默全丢（见 [`DerivedFacts`]）。
        let mut effects: Vec<EffectInput> = Vec::new();
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
            // 效果出参（DE-025 D-207）：每次终态调用一条。
            &mut effects,
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
        // 产物格的富化字段（DE-025 D-207）：按**调用 id** 归户，随 `artifact.added`
        // 入格。在这里算而不是在运行器里算，是因为「这个工具声明了什么行为」只有
        // 执行层读过（批首那份元数据），运行器没有注册表也没有 `declared`。
        let annotations: HashMap<String, Vec<(&'static str, serde_json::Value)>> = effects
            .iter()
            .map(|e| {
                (
                    e.call_id.clone(),
                    symbio_plugin_api::effects::annotations(e),
                )
            })
            .filter(|(_, a)| !a.is_empty())
            .collect();
        // 三份收束派生事实出参收进取件面（`take_derived`）——调用方在桥释放前取走。
        {
            let mut d = self.derived.lock().unwrap();
            d.effects.extend(effects);
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
                    annotations: annotations.get(&call_id).cloned().unwrap_or_default(),
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
// panic-allow crates/symbio-plugin-loop/src/tool_dispatch.rs::take_produced: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
// panic-allow crates/symbio-plugin-loop/src/tool_dispatch.rs::take_derived: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
// panic-allow crates/symbio-plugin-loop/src/tool_dispatch.rs::dispatch: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
// panic-allow crates/symbio-plugin-loop/src/tool_dispatch.rs::dispatch_wake_result: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。

#[cfg(test)]
#[path = "tool_dispatch.test.rs"]
mod tests;
