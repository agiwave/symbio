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
//! 2. 授权：把已开账的调用交给端口执行体的 gate 输入（`authorize_only`），
//!    由执行体落 `authz.granted` / `authz.denied`（拒绝即止，不落 `sent`）；
//! 3. [`ToolDispatch::dispatch`]：为放行的调用落 `sent`、跑执行层、关账。
//!
//! 三步各自可观测，崩溃恢复的粒度因此从「整批」变成「已落哪一笔债」。
//!
//! ## 与工具执行层的关系：**不是第二条执行链**
//!
//! 实际执行走 `process_tool_calls_in_env`——超长结果存档、生命周期钩子、
//! 审批/问答的「待用户动作」、批尾未执行收口、中止提前收口全在里面。本模块只补
//! 两件事：
//!
//! 1. **补建助手轮消息**。工具调用节点不是流式逐帧广播出来的
//!    （`LlmAdapter` 交回的是收口后的 `LlmTurn`，没有那条帧流），由本处补建。
//!    节点形状与其余对话节点**同一份**（前端读同一套字段），
//!    因此复用 `llm_build_tool_call_nodes` / `llm_build_text_node`。
//! 2. **把结果映射成运行器的事实形状**（[`CallOutcome`]），供运行器拼下一轮
//!    prompt；`artifact.added` 由执行体在收尾时落（P1b / DE-048 D-308）。
//!
//! ## 正文节点的定格
//!
//! 工具轮的中途正文（"我先查一下"）已经由 UI 桥逐片广播过。本模块在分发前把它
//! **定格**（`Completed`）并切节点（[`super::turn_exec::UiBridge::finalize_node`]）——
//! 不定格它永远转「流式中」，不切节点多轮正文会粘成一条。

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use symbio_kernel::capability::CapabilityRiskLevel;
use symbio_kernel::event::{Event, Seq, EVENT_FAILED, EVENT_REQUEST, EVENT_RESPONSE};
use symbio_kernel::exec::{ExecAbortSignal, ExecEventSink};
use symbio_kernel::plugin::{Plugin, PluginInvokeRequest, PluginInvokeRequestExt};
use symbio_kernel::store::Store;
use symbio_plugin_api::effects::EffectInput;
use symbio_plugin_api::events::EVENT_AUTHZ_DENIED;
use symbio_plugin_api::port::{ports, PortExecContext};
use symbio_sdk::adapters::LlmTurn;
use symbio_sdk::capability::ApprovalGrants;
use symbio_sdk::chat::chat_node::{
    ChatNode, MessageContent, NodeKind, NodeStatus, CHAT_STYLE_TAG_REASONING,
};
use symbio_sdk::clock::clock_now_ms;
use symbio_sdk::keys::{APPROVAL_GRANTS, MODE, RISK_LEVEL};
use symbio_sdk::llm::llm_emit_state;
use symbio_sdk::llm::turn::llm_emit_message;

use crate::control_decl::EVENT_AUTHZ_GRANTED;

use super::loop_rows;
use super::port;
use super::tool_exec::{is_unanswered, process_tool_calls_in_env, ABORTED, NOT_EXECUTED};
use super::turn_exec::UiBridge;
use symbio_kernel::actors::ActorSpec;
use symbio_sdk::turn::node_build::{
    detach_if_flat, llm_build_text_node, llm_build_tool_call_nodes, PresentationTree,
};

/// 工具执行层交回的**收束派生事实**（效果）。
///
/// 数据来源在工具执行层（`EffectInput`），而写方（落格）在轮末——`turn_exec` 调
/// [`super::derived_facts::record_derived`] 写。本结构是那份出参从分发点回到运行器的
/// **唯一通道**：不收集它，效果就静默全丢（与记忆 / 学习同一个坑——出参不交回，
/// 写方就无从落格）。熔断事实由执行体随债自己落（DE-039 §6），不在这条通路。
///
/// 任务表**不在这里**：它是一条效果（`symbio-plugin-task` 的 `TaskTable`），走
/// `effects` 那一条通路（DE-025 D-207）。
///
/// `dispatch` 在 `&self` 上跑，出参却要跨 await 累积，故由 [`PortOutput`] 的
/// `Mutex` 收，调用方在桥释放前一次取走。
#[derive(Default)]
pub(crate) struct DerivedFacts {
    /// 效果出参（DE-025 D-207）：每次终态调用一条，轮末由写口问能力包要草稿。
    pub(crate) effects: Vec<EffectInput>,
}

/// 一次工具批次交回运行器的**全部出参**（DE-044 `E-3`）。
///
/// 两条出参——节点（供会话镜像）与收束派生事实（供轮末写口落格）——形状同构：
/// 都由执行层产生、写方在别处、且必须跨 `await` 累积。分成两个 `Mutex` 只是历史
/// 分层（一个来自 UI 桥、一个来自执行层），不是两种语义；合成一处后取用点只有
/// 一次加锁，也不会出现「取了一条忘了另一条」。
///
/// [`CallOutcome`] 不在本结构里：它是**同步返回值**，一次调用即交齐，
/// 没有跨 `await` 累积的问题——把它塞进 `Mutex` 只会把返回值变成第二条通道。
#[derive(Default)]
pub(crate) struct PortOutput {
    /// 本轮产生的全部消息（供 chat_loop 并入落库权威镜像）。
    pub(crate) nodes: Vec<ChatNode>,
    /// 本轮工具执行层交回的收束派生事实。
    pub(crate) derived: DerivedFacts,
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
    /// 应答配对键（执行体交回的 `PortReply::links`，`D-319`）：环只搬运它，
    /// 让工具结果与模型侧的 assistant 声明用同一个配对键。
    pub(crate) reply_key: Option<String>,
    /// 授权动作的结论（创建时为 `false`，gate 调用写入）。
    pub(crate) granted: bool,
    /// 授权动作写下的拒绝措辞（`authz.denied` 的 `detail`；`None` = 放行或还没判）；
    /// 批尾据此落 `failed{why}`。
    pub(crate) deny_why: Option<String>,
    /// 派发动作是否已落 `sent`（派发动作写入）。
    pub(crate) sent: bool,
    /// 这一笔是**恢复幕的唤醒调用**（d）：由用户动作（批准 / 重试 /
    /// 补充 / 回答 / 拒绝）落成的 `request`，不是某个模型响应里的调用。
    ///
    /// 派发动作据它决定「跑工具」还是「只合成结果」——两者的事实完全一样
    /// （`request → authz → sent → response / failed`），差别只在读侧。
    pub(crate) wake: Option<Wake>,
}

/// 唤醒调用的种类（d）。
///
/// 恢复幕没有新的模型应答，工具批的入口是**用户动作**。用户动作先落成一笔
/// `request(port=tool)`（[`ToolPort::land_wake`]），之后与批次走同一条授权 / 派发
/// 链；本枚举只说明那一笔要不要真正跑工具。
#[derive(Debug, Clone)]
pub(crate) enum Wake {
    /// approve / retry / supply：用户要求重跑一次工具。
    Execute,
    /// answer / reject：用户给出了结果（答案或拒绝理由），工具不再执行。
    ///
    /// 批准 / 拒绝的差别只写进请求载荷的 `resume`（gate 读那份），这里只留结果。
    Result { text: String, ok: bool },
}

impl Wake {
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
            }),
            "reject" => Some(Self::Result {
                text: payload
                    .get("result_text")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                ok: false,
            }),
            _ => Some(Self::Execute),
        }
    }
}

/// 一次唤醒**要落的内容**（[`ToolPort::land_wake`] 的入参）。
///
/// 打包成一个形状而不是七个位置参数：这些字段同属「用户动作」这一件事，分成
/// 位置参数时调用点要靠顺序记住谁是谁，而顺序错了编译器不报错——载荷里
/// `target` 与 `turn_root` 互换的后果是结果挂错卡片。
pub(crate) struct WakeSpec<'a> {
    /// 用户动作的种类（`approve` / `retry` / `supply` / `answer` / `reject`）——
    /// 落进载荷的 `resume` 标记，读侧据此还原唤醒数据。
    pub(crate) kind: &'static str,
    /// 被唤醒的那张卡（ToolCall 节点 id）：结果挂回它。
    pub(crate) target: &'a str,
    /// 工具名（LLM 可见名）。
    pub(crate) tool: &'a str,
    /// 这次调用的参数。
    pub(crate) args: &'a serde_json::Value,
    /// 本轮 Turn 根节点 id（UI 帧挂载的父节点）。
    pub(crate) turn_root: &'a str,
    /// 这一笔要不要真跑工具。
    pub(crate) wake: &'a Wake,
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
    /// 还原出的端口账（一笔）。**唤醒数据与目标节点都在这一笔里**
    /// （`calls[0].wake` / `calls[0].model_id`）——不另立第二份形状。
    pub(crate) calls: Vec<ToolPortCall>,
}

/// 一次工具调用结果的**事实形状**（[`ToolDispatch::dispatch`] 的元素类型）。
///
/// 刻意只装**结果**：消费方（运行器）要的是「拿什么回填给模型」与「要不要停」，
/// 不是插件侧的节点形状。节点怎么建、往哪发，是分发方自己的事——运行器不认识
/// `ChatNode` 的图语义（那在 `crates/symbio-plugin-session`），只认识这段文本。
///
/// 它住在插件里而不是 core：生产消费方只有本插件的运行器（README §4 四问第 1 问）。
#[derive(Debug, Clone)]
pub struct CallOutcome {
    /// 这笔结果对应的**端口账**（`request` / `sent` / 终局同 id）。
    ///
    /// 配对按它做（DE-048 D-311）：日志里 `request(port=tool)`、`response` / `failed`
    /// 的键就是这一个 id；`call_id` 是节点身份，不参与配对。
    pub call: port::Call,
    /// 对应的工具调用 id（回填给模型时锚定 ToolCall；取 `TurnToolCallInfo::id`）。
    pub call_id: String,
    /// 工具名（LLM 可见名）。
    pub name: String,
    /// 结果正文。**失败也是正文**——工具失败是信息性的，模型据此改道，不中断会话。
    pub text: String,
    /// 成功与否。仅供可观测/展示；**不改变**本轮终态（信息性，见上）。
    pub ok: bool,
    /// 本次调用**收束于等待用户动作**（confirm / ask_user ⇒ 本轮停下等用户）。
    /// 运行器据此**停止工具循环**。
    pub needs_user_action: bool,
    /// 等待用户时卡片载荷（工具自决的 `prompt` 形状：审批卡 / 提问卡不同）。
    ///
    /// **它是事实的一部分**：`needs_user_action` 只说明「这轮停在等用户」，而恢复
    /// （`resume`）要拿 `meta.prompt` 复现那张卡。读路径走 WAL 重建 ⇒ 这张卡
    /// 必须能从事实折出来，故随载荷入格（`Some` 仅当 [`Self::needs_user_action`]）。
    /// 运行器不认识它的形状——原样透传（与 `text` 同一条：只搬数据，不解释）。
    pub prompt: Option<serde_json::Value>,
    /// 等待用户的原因码（`failure_kind` 里的 pending 取值：`needs_approval` /
    /// `needs_interaction`……）。与 [`Self::prompt`] 同批入库、同批复现。
    pub failure_kind: String,
    /// ToolCall 节点的**定稿参数文本**（JSON 文本 / 解析失败的残破原文）。
    /// 重建直接回放这段文本，不经 `Value` 重序列化（键序漂移即失真）。
    pub args: String,
    /// 这一笔执行层交回的效果出参（DE-025 D-207）。
    ///
    /// 随结果携带而不是当场写进共用的取件面：并发执行时落地顺序由 provider 决定，
    /// 而效果事实的顺序必须等于**调用顺序**——由环在整轮收口时按序交回。
    pub effects: Vec<EffectInput>,
    /// 应答承载（`D-319`）：工具结果消息由执行体组装，环只按序追加。
    pub reply: symbio_plugin_api::port::PortReply,
}

impl CallOutcome {
    /// 空结果（仅在执行层没交回任何结果节点时兜底；`failure_kind` 由调用点补）。
    fn empty() -> Self {
        Self {
            call: port::Call {
                port: "tool",
                id: String::new(),
                attempt: 0,
                via: None,
            },
            call_id: String::new(),
            name: String::new(),
            text: String::new(),
            ok: false,
            needs_user_action: false,
            prompt: None,
            failure_kind: String::new(),
            args: String::new(),
            effects: Vec::new(),
            reply: symbio_plugin_api::port::PortReply::default(),
        }
    }
}

/// 工具派发的**两个动作**（`D-124` 订正 1）：派出 / 收尾。
///
/// 创建（`open-tools`）与授权（`authorize-tool`）两段已是**行承接体**：写方是
/// [`open_calls`] / [`gate_authorize`]，由行动作直接调用，不经本契约（P2-4）。
/// 契约住插件本地，core 不认识工具分发（E-009）。
///
/// ## 为什么派出与收尾是两半
///
/// 与 `model` 端口逐字同形（`D-222` 订正 2）：`begin` 落 `sent` 就把「跑执行层 + 收结果」
/// 整段寄存成一个 **`'static`** future（[`ToolInFlight`]），`finish` 在下一个环步取走它的
/// 结论关账。两个环步之间那一圈「`sent` 有、终局无」因此是**正常跑**里读得到的形状，
/// 而不是只在崩溃恢复的输入上才有值——`waiting` 视图、`stale_sent`、`reap` 读的都是它。
///
/// ## 为什么每个方法都收一个 `store`
///
/// 生产实现自己有写口（[`ToolPort`] 持 `Arc<dyn Store>`），这个参数在那里是重复的；
/// 它服务于**测试替身**：行表按事实判欠不欠，替身若不落协议事实，创建行永远欠着，
/// 派发器会在同一笔债上空转到步数上界。把「写哪一份日志」交给调用点（派发器手里
/// 正拿着这一轮的那份），替身就与生产走同一条循环。
///
/// `begin` 收的是 `Arc<dyn Store>` 而不是引用：它交回的那个 future 要活过这一次调用
/// （寄存进草稿、下一圈才 poll），引用进不去 `'static` 的框。
#[async_trait]
pub(crate) trait ToolDispatch: Send + Sync {
    /// 幕内**一次**的正文定格（正文 / 推理收口并切节点）。
    ///
    /// 逐笔派发之后，每笔调用各走一次 [`Self::begin`]；正文只属于那一轮模型往返，
    /// 因此把「定格 + 排空」提成独立入口，由运行器在开账之后调一次。默认空实现：
    /// 测试替身没有流式桥。
    async fn prepare(&self, _store: &dyn Store<Event = Event>, _turn: &LlmTurn) {}

    /// **派出半**：为这一笔落 `sent`，把「跑执行层 + 收结果」寄存成一个 future。
    ///
    /// 每笔调用一个环步（`dispatch-tool` 行的键就是这一笔）：N 笔可以同时在途，
    /// 但环里没有「批」这个对象（DE-048 D-311）。
    async fn begin(
        self: Arc<Self>,
        store: Arc<dyn Store<Event = Event>>,
        turn: &LlmTurn,
        call: &mut ToolPortCall,
    ) -> ToolBegin;

    /// **收尾半**：把这一笔的结论落成端口账的终局格。
    ///
    /// `outcome = None` ⇒ 执行层没交出结果：环按「没收到应答的债」写
    /// `failed{NOT_EXECUTED}`——租约超时 / 未执行的终局由环负责。
    fn finish<'a>(
        &'a self,
        store: &'a dyn Store<Event = Event>,
        call: &'a ToolPortCall,
        outcome: Option<&'a CallOutcome>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

    /// 把整轮的效果出参按**调用顺序**交回取件面（轮末写口从这里取）。
    ///
    /// 逐笔派发时各笔完成次序由 provider 决定；环只在整轮收口时按调用顺序交回，
    /// 世界事实的次序因此与调用顺序一致。
    fn land_derived(&self, _effects: Vec<EffectInput>) {}

    /// 读一笔**唤醒调用**（d）。
    ///
    /// 恢复幕没有模型应答可寄存：用户在等待卡上点了按钮，唤醒请求已在开幕之前
    /// 落格。行表选中授权 / 派发行时，动作从这里把那一笔取回来——没有唤醒请求
    /// ⇒ `None`，调用方按「引擎选错了账」处理（不写任何格）。
    async fn wake(&self, _store: &dyn Store<Event = Event>) -> Option<WakeBatch> {
        None
    }
}

/// 在飞的一笔工具调用：`sent` 已落、结果还没回来。
///
/// `'static` 是硬要求（与 `stream::ModelInFlight` 同一条）：它寄存在本轮草稿里，跨两次
/// 选债活着；草稿不带生命周期参数，否则 `Action` 的 `for<'a>` 要对两个生命周期成立，
/// 而绑定表要求它 `'static`。
pub(crate) type ToolInFlight =
    std::pin::Pin<Box<dyn std::future::Future<Output = CallOutcome> + Send + 'static>>;

/// [`ToolDispatch::begin`] 的两种结论。
pub(crate) enum ToolBegin {
    /// 草稿里没有可派的批次（引擎选错了账）：一格没落，也不登记在飞槽。
    ///
    /// 派发器的空账本守卫会把这种形态显形（`exhausted{no_progress}`）——不猜一个
    /// 空批次去掩盖它。
    None,
    /// 已在飞：`sent` 落了，`await-tool` 行会来收。
    InFlight(ToolInFlight),
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

/// 一次工具调用的**协议 id**：轮 + 幕 + 本次模型往返 + 响应内序号 + 模型交回的调用 id。
///
/// ## 为什么要带「本次模型往返」
///
/// 创建行 `open-tools` 的集合展开（D-117）从 `response.calls` 的元素里取 id，
/// 而 `response` 在派发**之前**就写好了。要让展开出的键与随后的 `request` 同 id，
/// id 必须只由**模型响应里已有的东西**决定：轮、幕、模型往返（`model-…-s{step}`）、
/// 响应内序号、模型调用 id。
///
/// 只带 `tc.id` 不够：本地模型常用 `call_0` / `call_1` 这种按回合重来的短 id，
/// 同一幕的下一轮就会撞上上一轮的键——创建债被旧 `request` 提前关掉，新调用根本
/// 不会派发。`round` 把「同一幕的不同轮」分开；缺失模型 id 的调用用序号兜底。
///
/// ## 为什么序号要**定宽**前置
///
/// 逐笔派发后环按债键 `(port, id, attempt)` 选最小的一笔（`loop_rows` 的行序），
/// 键的字典序就是派发序。模型交回的 id 是任意字符串，按它排序会把调用次序打乱；
/// 定宽序号放在前面，键序就等于模型响应里 `calls` 的次序。这个次序**只来自响应
/// 自身**，环不引入「批」的概念，也不据此决定同时派发几笔（并发在 provider 内部）。
pub(crate) fn call_key(
    turn: u64,
    drive: u64,
    idx: usize,
    model_id: Option<&str>,
    round: &str,
) -> String {
    match model_id.filter(|s| !s.trim().is_empty()) {
        Some(id) => format!("{}-{turn}-d{drive}-{round}-c{idx:03}-{id}", ports::TOOL),
        None => format!("{}-{turn}-d{drive}-{round}-c{idx:03}", ports::TOOL),
    }
}

/// 本幕最近一笔模型应答（`response(port=model)`）。
///
/// 派发侧不重算 id，而是读创建侧写下的 `calls`：那是**唯一真相**，也自然对齐
/// `turn.tool_calls` 的顺序。重算只能靠再次猜「这一轮是哪一轮」，而猜错一次的
/// 后果是创建债被旧事实提前关掉。
fn latest_model_response(facts: &[Event], turn: u64, drive: u64) -> Option<&Event> {
    let prefix = format!("{}-{turn}-d{drive}-", ports::MODEL);
    facts.iter().rev().find(|e| {
        e.scope == turn
            && e.kind == symbio_kernel::event::EVENT_RESPONSE
            && e.payload.get("port").and_then(|v| v.as_str()) == Some(ports::MODEL)
            && e.payload
                .get("id")
                .and_then(|v| v.as_str())
                .is_some_and(|id| id.starts_with(&prefix))
    })
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
    /// 恢复幕在 `resume-turn` **之前**执行，`drive_seq` 填的是占位 0 ⇒ 这里得 0；
    /// 本行的三笔工具债（`open-tools` / `authorize-tool` / `dispatch-tool`）不读
    /// 带时间的视图，`now` 参与不了判据。要新增读时间的行，得先把真实幕锚传进来。
    fn now_ms(&self, facts: &[Event]) -> i64 {
        loop_rows::wake_now_ms(facts, self.drive_seq)
    }
}

// ── 行承接体：创建 / 授权（P2-4） ────────────────────────────────
// 两段从 `ToolDispatch` 契约搬到行动作直接调用：它们只写自己那一行的领域事实，
// 不再借道分发包。派出与收尾仍在 `impl ToolDispatch`（P2-5 的边界）。

/// 创建行 prepass：问创建行欠不欠，欠就把调用清单交给 `port=tool` 执行体开账。
///
/// 环不再装配请求载荷：线上名解析、风险档、永久放行上限与「不可派发」判定都在
/// 执行体（DE-048 D-311）。环只交纯值（应答原文、调用清单、策略读数字串），
/// 跑完再读回新落的 `request` 事实，按**响应顺序**与模型调用清单配对成草稿。
pub(crate) async fn open_calls(
    store: Arc<dyn Store<Event = Event>>,
    ctx: Option<&Arc<dyn PluginInvokeRequest>>,
    subject: u64,
    user_seq: u64,
    drive_seq: u64,
    model_turn: &LlmTurn,
    reply_links: &[(String, String)],
) -> Vec<ToolPortCall> {
    let facts = store.range(Seq::new(0));
    let latest = latest_model_response(&facts, subject, drive_seq);
    // 旧存档里的模型应答没有 `calls`（D-117 之前写的）：没有创建债可言，
    // 按旧路径继续派发，不能因为升级把在飞的会话卡死。带 `calls` 的应答
    // 则必须由创建行先开账——它不欠就说明这一批已经落过 `request`。
    let has_calls = latest.is_some_and(|e| e.payload.get("calls").is_some());
    // obligation-precheck OB-004: 执行体（OPENER）动手前的复核——同一批被 resume / 重放第二次带进来时，创建行已由 `until` 关掉，早退避免重复落 `request`（不是第二套调度：这里不选行、不排行，只问本行还欠不欠）
    if has_calls
        && !loop_rows::owes(
            "open-tools",
            &facts,
            subject,
            loop_rows::wake_now_ms(&facts, drive_seq),
        )
    {
        symbio_plugin_api::plugin_warn!(
            "session",
            "[Tool] 创建行不欠（subject={}），不落 request",
            subject
        );
        return Vec::new();
    }
    let Some(ctx) = ctx else {
        symbio_plugin_api::plugin_warn!(
            "session",
            "[Tool] 创建行没有宿主面（subject={}），不落 request",
            subject
        );
        return Vec::new();
    };
    // 策略读数字串（值，不是判定）：阈值 / 永久放行清单 / 模式由执行体自己解释。
    let threshold = ctx
        .get(RISK_LEVEL)
        .map(|s| CapabilityRiskLevel::from_name(&s))
        .unwrap_or_default();
    let grants = ctx
        .get(APPROVAL_GRANTS)
        .map(|s| ApprovalGrants::parse(&s))
        .unwrap_or_default();
    let mode = ctx.get(MODE).unwrap_or_default();
    let payload = serde_json::json!({
        "open_only": true,
        "epoch": drive_seq,
        "policy": {
            "threshold": threshold.as_str(),
            // 已按 owner 的容错规则解析过，执行体只反序列化合法档位。
            "grants": grants.to_json(),
            "mode": mode,
        },
        "response": latest
            .map(|e| e.payload.clone())
            .unwrap_or(serde_json::Value::Null),
        "tool_calls": model_turn
            .tool_calls
            .iter()
            .map(|tc| {
                serde_json::json!({
                    "id": tc.id,
                    "name": tc.name,
                    // 参数原文搬运（值，不是装配请求载荷）：执行体才解释它。
                    "arguments": tc.arguments,
                    "parse_error": tc.parse_error.is_some(),
                    "overflow_notice": tc.overflow_notice.is_some(),
                })
            })
            .collect::<Vec<_>>(),
    });
    let Some(provider) = port::provider_of(ctx, ports::TOOL).await else {
        return Vec::new();
    };
    let Some(executor) = provider.executor() else {
        symbio_plugin_api::plugin_warn!(
            "session",
            "[Tool] 工具端口没有执行面（subject={}），不落 request",
            subject
        );
        return Vec::new();
    };
    let now = loop_rows::wake_now_ms(&facts, drive_seq);
    let exec = PortExecContext {
        deadline_ms: None,
        abort: ExecAbortSignal::default(),
        idempotency_key: format!("open-{subject}-d{drive_seq}"),
        sink: ExecEventSink::Null,
        // 与行表里的执行体名同字面量（`loop_rows::OPENER` 是私有的）。
        emit: port::host_fact_emit(store.clone(), subject, "opener", now, Some(user_seq), None),
        ctx: Some(ctx.clone()),
        subject,
        group: String::new(),
        origin: Some(user_seq),
        permitted: true,
        deny_reason: String::new(),
        scratch_dir: None,
    };
    let outcome = executor
        .execute(
            exec,
            symbio_plugin_api::port::PortInput {
                port: ports::TOOL,
                payload,
            },
        )
        .await;
    if let Some((class, why)) = outcome.error {
        symbio_plugin_api::plugin_warn!(
            "session",
            "[Tool] 建账调用未交回结论（subject={}）：{class:?} {why}",
            subject
        );
    }
    let after = store.range(Seq::new(0));
    after
        .iter()
        .skip(facts.len())
        .filter(|e| {
            e.kind == EVENT_REQUEST
                && e.payload.get("port").and_then(|v| v.as_str()) == Some(ports::TOOL)
        })
        .zip(model_turn.tool_calls.iter())
        .enumerate()
        .filter_map(|(index, (event, tc))| {
            let id = event.payload.get("id").and_then(|v| v.as_str())?;
            let attempt = event
                .payload
                .get("attempt")
                .and_then(|v| v.as_u64())
                .unwrap_or(1);
            // 应答配对键：优先按调用 id 命中；缺 id 的调用按声明顺序取（模型侧
            // 的 `tool_calls` 与创建行逐条对齐，顺序因此是稳定配对面）。
            let reply_key = tc
                .id
                .as_deref()
                .and_then(|key| {
                    reply_links
                        .iter()
                        .find(|(k, _)| k == key)
                        .map(|(_, v)| v.clone())
                })
                .or_else(|| reply_links.get(index).map(|(_, v)| v.clone()));
            Some(ToolPortCall {
                model_id: tc.id.clone().unwrap_or_default(),
                call: port::Call::new(ports::TOOL, id, attempt),
                name: tc.name.clone().unwrap_or_default(),
                reply_key,
                granted: false,
                deny_why: None,
                sent: false,
                wake: None,
            })
        })
        .collect()
}

/// 授权动作：把这一笔**交给端口执行体的 gate 输入**（第二笔债）。
///
/// 判定不在环里：环把 `request` 载荷原样交出并打上 `authorize_only` 标记，执行体
/// 判完只 `emit` 一条 `authz.granted` / `authz.denied`——**不执行、不写 `sent`**。
/// 环跑完再读回那条授权事实，写入草稿的两格（`granted` / `deny_why`）。
///
/// 行表的键就是调用标识：一次动作只判一笔，`owes` 复核因此仍按行整体判
/// （还有别的调用没判时这一行仍欠，逐笔进入各判各的）。
pub(crate) async fn gate_authorize(
    store: Arc<dyn Store<Event = Event>>,
    ctx: Option<&Arc<dyn PluginInvokeRequest>>,
    turn: u64,
    user_seq: u64,
    drive_seq: u64,
    call: &mut ToolPortCall,
) {
    let facts = store.range(Seq::new(0));
    // obligation-precheck OB-004: 执行体（AUTHORIZER）动手前的复核——第二次进入时
    // 授权行已落事实、不再欠，早退避免重复判放行
    if !loop_rows::owes(
        "authorize-tool",
        &facts,
        turn,
        loop_rows::wake_now_ms(&facts, drive_seq),
    ) {
        symbio_plugin_api::plugin_warn!("session", "[Tool] 授权行不欠（turn={}），不判放行", turn);
        return;
    }
    let Some(request) = facts.iter().rev().find(|e| {
        e.kind == EVENT_REQUEST
            && e.payload.get("port").and_then(|v| v.as_str()) == Some(ports::TOOL)
            && e.payload.get("id").and_then(|v| v.as_str()) == Some(call.call.id.as_str())
            && e.payload.get("attempt").and_then(|v| v.as_u64()) == Some(call.call.attempt)
    }) else {
        symbio_plugin_api::plugin_warn!(
            "session",
            "[Tool] 授权找不到请求事实（turn={} / {}），不判放行",
            turn,
            call.model_id
        );
        return;
    };
    let mut payload = request.payload.clone();
    payload["authorize_only"] = serde_json::json!(true);
    let request_id = request.event_id.clone();
    let Some(ctx) = ctx else {
        symbio_plugin_api::plugin_warn!(
            "session",
            "[Tool] 授权没有宿主面（turn={} / {}），不判放行",
            turn,
            call.model_id
        );
        return;
    };
    let Some(provider) = port::provider_of(ctx, ports::TOOL).await else {
        return;
    };
    let Some(executor) = provider.executor() else {
        symbio_plugin_api::plugin_warn!(
            "session",
            "[Tool] 工具端口没有执行面（turn={} / {}），不判放行",
            turn,
            call.model_id
        );
        return;
    };
    let now = loop_rows::wake_now_ms(&facts, drive_seq);
    let exec = PortExecContext {
        deadline_ms: None,
        abort: ExecAbortSignal::default(),
        idempotency_key: request_id,
        sink: ExecEventSink::Null,
        // 与行表里的执行体名同字面量（`loop_rows::AUTHORIZER` 是私有的）。
        emit: port::host_fact_emit(store.clone(), turn, "authorizer", now, Some(user_seq), None),
        ctx: Some(ctx.clone()),
        subject: turn,
        group: String::new(),
        origin: Some(user_seq),
        permitted: true,
        deny_reason: String::new(),
        scratch_dir: None,
    };
    let outcome = executor
        .execute(
            exec,
            symbio_plugin_api::port::PortInput {
                port: ports::TOOL,
                payload,
            },
        )
        .await;
    if let Some((class, why)) = outcome.error {
        symbio_plugin_api::plugin_warn!(
            "session",
            "[Tool] gate 调用未交回结论（turn={} / {}）：{class:?} {why}",
            turn,
            call.model_id
        );
    }
    let after = store.range(Seq::new(0));
    match authz_verdict(&after, &call.call) {
        Some((true, _)) => call.granted = true,
        Some((false, detail)) => call.deny_why = Some(detail),
        None => symbio_plugin_api::plugin_warn!(
            "session",
            "[Tool] gate 未落授权事实（turn={} / {}）",
            turn,
            call.model_id
        ),
    }
}

/// 读回一笔债的授权结论：`(放行?, 拒绝措辞)`。
///
/// 结论只认执行体写下的那条事实（环不缓存判定）：`authz.denied` 的 `detail` 就是
/// 线上契约的措辞，原样带进草稿供 `deny-tools` 落 `failed`。
fn authz_verdict(facts: &[Event], call: &port::Call) -> Option<(bool, String)> {
    let of = |kind: &str| {
        facts.iter().rev().find(|e| {
            e.kind == kind
                && e.payload.get("port").and_then(|v| v.as_str()) == Some(ports::TOOL)
                && e.payload.get("id").and_then(|v| v.as_str()) == Some(call.id.as_str())
                && e.payload.get("attempt").and_then(|v| v.as_u64()) == Some(call.attempt)
        })
    };
    if let Some(denied) = of(EVENT_AUTHZ_DENIED) {
        let detail = denied
            .payload
            .get("detail")
            .and_then(|v| v.as_str())
            .unwrap_or("denied");
        return Some((false, detail.to_string()));
    }
    of(EVENT_AUTHZ_GRANTED).map(|_| (true, String::new()))
}

/// 一笔**被拒且没有派发**的债：`(拒绝措辞, attempt)`。
///
/// 只认 `authz.denied` 且该逻辑 id 还没有终局——这是 `deny-tools` 的入口形状
/// （`request → authz.denied → failed`，中间没有 `sent`）。
pub(crate) fn denied_debt(facts: &[Event], id: &str) -> Option<(String, u64)> {
    let denied = facts.iter().rev().find(|e| {
        e.kind == EVENT_AUTHZ_DENIED
            && e.payload.get("port").and_then(|v| v.as_str()) == Some(ports::TOOL)
            && e.payload.get("id").and_then(|v| v.as_str()) == Some(id)
    })?;
    if facts.iter().any(|e| {
        e.kind == EVENT_FAILED
            && e.payload.get("port").and_then(|v| v.as_str()) == Some(ports::TOOL)
            && e.payload.get("id").and_then(|v| v.as_str()) == Some(id)
            && e.payload.get("attempt").and_then(|v| v.as_u64())
                == denied.payload.get("attempt").and_then(|v| v.as_u64())
    }) {
        return None;
    }
    Some((
        denied
            .payload
            .get("detail")
            .and_then(|v| v.as_str())
            .unwrap_or("denied")
            .to_string(),
        denied
            .payload
            .get("attempt")
            .and_then(|v| v.as_u64())
            .unwrap_or(1),
    ))
}

impl ToolPort {
    /// 派发动作的前半（第三笔债）：为这一笔放行的调用落 `sent`。
    ///
    /// 执行层由调用方跑（`SessionDispatchPort::dispatch`）——`sent` 的语义是
    /// 「已交给执行层」，所以它必须先于执行。
    ///
    /// 派发行欠不欠由引擎现读（D-136）：这里只交出授权行与读账的 `now`，
    /// 引擎回 [`port::DispatchError::NotOwed`] 就是「没有这笔账」，如实落 `failed`。
    pub(crate) fn begin_call(&self, call: &mut ToolPortCall) {
        if !call.granted {
            return;
        }
        let facts = self.facts.range(Seq::new(0));
        let now = self.now_ms(&facts);
        // 端口声明从冻结表按端口名取——环不持有端口清单（DE-047 不变量 6）。
        let ports = loop_rows::ports();
        match port::begin_with_facts(
            self.facts.as_ref(),
            &ports,
            port::Dispatch {
                call: &call.call,
                turn: self.turn,
                produced_by: self.user_seq,
                row: "dispatch-tool",
                now,
                // 唤醒调用能由 `wake_batch` 从事实重建（有活着的等待方）；
                // 普通调用把结果等在当前草稿里，进程一死就没人接了。
                awaited: call.wake.is_some(),
            },
            &facts,
        ) {
            Ok(()) => call.sent = true,
            Err(port::DispatchError::NotOwed(row)) => {
                symbio_plugin_api::plugin_warn!(
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
            Err(e) => symbio_plugin_api::plugin_warn!(
                "session",
                "[Tool] 工具 sent 未落格（turn={} / {}）：{e:?}",
                self.turn,
                call.model_id
            ),
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
    /// `dispatch-tool` / `deny-tools`（68 / 69）落 `sent` 并收尾。在这里把三笔
    /// 一起写掉，等于让恢复路径又长出一条绕过行表的旁路——那正是本步要退役的东西。
    ///
    /// ## 旧账先关
    ///
    /// 同轮里若还有一笔**开着**的唤醒请求（上次进程死在派发中途），先按
    /// `failed{superseded}` 关掉：它的结果永远不会来，而新的一次用户动作是新账。
    /// 不关的话 `turn_head.tools_open` 会看见两笔开着，`departed` 的集合债也永远
    /// 差一个终局。
    pub fn land_wake(&self, spec: &WakeSpec<'_>) -> Result<(), symbio_kernel::store::AppendError> {
        let facts = self.facts.range(Seq::new(0));
        for stale in facts.iter().filter(|e| {
            e.scope == self.turn && is_wake_request(e) && !wake_terminal(&facts, self.turn, e)
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
            "turn_id": spec.turn_root,
            "target": spec.target,
            "resume": spec.kind,
        });
        if let Wake::Result { text, ok, .. } = spec.wake {
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
            e.scope == self.turn && is_wake_request(e) && !wake_terminal(&facts, self.turn, e)
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
        let id = payload.get("id").and_then(|v| v.as_str())?.to_string();
        let attempt = payload.get("attempt").and_then(|v| v.as_u64()).unwrap_or(1);
        let intent = Wake::from_payload(payload)?;
        let authz = facts.iter().find_map(|e| {
            // 授权事实按**同一次尝试**取（D-138）：重试后的 attempt 2 不能复用
            // attempt 1 的放行，否则 `dispatch-tool` 的新账永远不欠。
            // 旧日志里的授权没有 `attempt`：只对首次尝试按缺席兼容。
            let authz_attempt = e.payload.get("attempt").and_then(|v| v.as_u64());
            let same = e.payload.get("port").and_then(|v| v.as_str()) == Some("tool")
                && e.payload.get("id").and_then(|v| v.as_str()) == Some(id.as_str())
                && (authz_attempt == Some(attempt) || (authz_attempt.is_none() && attempt == 1));
            if !same {
                return None;
            }
            match e.kind.as_str() {
                EVENT_AUTHZ_GRANTED => Some((true, String::new())),
                EVENT_AUTHZ_DENIED => Some((
                    false,
                    e.payload
                        .get("detail")
                        .and_then(|v| v.as_str())
                        .unwrap_or("denied")
                        .to_string(),
                )),
                _ => None,
            }
        });
        let (granted, deny_why) = authz.unwrap_or((false, String::new()));
        let turn = LlmTurn {
            tool_calls: vec![symbio_kernel::llm::TurnToolCallInfo {
                id: Some(target.clone()),
                wire_id: None,
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
                // 唤醒批没有模型往返；配对键随 P3 的「唤醒归端口」一并交回。
                reply_key: None,
                granted,
                deny_why: (!granted).then_some(deny_why),
                sent: false,
                wake: Some(intent),
            }],
        })
    }

    /// 一笔调用返回后关账：**这一笔**的终局。
    ///
    /// 配对按**调用标识**（`outcome.call` 与 `call.call` 同一个端口账）：日志里
    /// `request` / `response` / `failed` 共享的键就是它，不存在按批按下标的配对。
    ///
    /// - 放行且已派发：`response`（工具回了话）或 `failed`（执行层没跑到它）；
    /// - 被拒（`deny_why` 有值）：`failed{why}`——授权行已由 `authz.denied` 关掉，
    ///   但创建行的集合债与「拒绝收尾」这一笔债务（`deny-tools`）还等着终局；
    /// - 放行但没落 `sent`：`begin_call` 已经替它落过 `failed`，这里不重复；
    /// - **没有结果**（`outcome = None`）：环写 `failed{NOT_EXECUTED}`——「未收到
    ///   `response` 的债」本身就是终局判据，不需要一个专门的未执行模块。
    ///
    /// 「有结果节点」≠「工具回了话」：收口给没跑到的调用补的是**框架的交代**
    /// （一句话占位），它的成因写在 `meta.failure_kind` 上。占位按成因落
    /// `failed{why}` 而不是 `response`——否则中止的调用在日志里说「回来了」，而它
    /// 从未执行，`await-tool` 的 `failed` 出口就只剩 resume 一条路（[D-121](../../../docs/003-design/015-义务环.md#d-121-唤醒事实的行语义await-行的收清走协议格审批与补充重新开账 "DE-015-义务环")
    /// 的中止作用、DE-015 判据 6 场景三）。词表与判据同源：[`super::tool_exec`] 门面的
    /// `is_unanswered` / `ABORTED` / `NOT_EXECUTED`。
    pub(crate) fn close_call(&self, call: &ToolPortCall, outcome: Option<&CallOutcome>) {
        if !call.granted {
            if let Some(why) = call.deny_why.as_deref() {
                self.fail_denied(call, why);
            }
            return;
        }
        if !call.sent {
            return;
        }
        // 配对键：结果自带的端口账与这一笔同一个 id。对不上 = 执行层答的
        // 不是这一笔，按「没收到应答」收口（不猜另一个名字）。
        match outcome.filter(|o| o.call.id == call.call.id) {
            Some(o) if is_unanswered(&o.failure_kind) => self.fail(call, &o.failure_kind),
            Some(_) => self.close(call, true),
            None => self.fail(call, NOT_EXECUTED),
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
                symbio_plugin_api::plugin_warn!(
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
    /// 三个调用方：逐笔关账（[`Self::close_call`]，`why` 取结果节点的成因或被拒
    /// 原因）、resume 的中止路径（`why = ABORTED`）、以及「放行但没派出去」那几笔
    /// （`begin_call` 自己落 `failed{dispatch_row_not_owed}`）。
    pub(crate) fn fail(&self, call: &ToolPortCall, why: &str) {
        // 三种成因要能被**关账判据**直接认出来（`failed.cause`）：中止、超限、
        // 拒绝。其余失败只带原样的 `why`，不硬凑一个成因。
        let cause = match why {
            ABORTED => Some("aborted"),
            w if w.starts_with("overflow") => Some("overflow"),
            _ => None,
        };
        self.fail_with(call, why, cause);
    }

    /// 被拒的债：`why` 是线上契约的措辞（`detail`），成因固定为 `denied`。
    fn fail_denied(&self, call: &ToolPortCall, detail: &str) {
        self.fail_with(call, detail, Some("denied"));
    }

    fn fail_with(&self, call: &ToolPortCall, why: &str, cause: Option<&str>) {
        let extra = match cause {
            Some(cause) => serde_json::json!({ "cause": cause }),
            None => serde_json::json!({}),
        };
        if let Err(e) = port::failed_with_extra(
            self.facts.as_ref(),
            &call.call,
            self.turn,
            self.user_seq,
            why,
            extra,
        ) {
            symbio_plugin_api::plugin_warn!(
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
        e.scope == turn
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
    /// 轮号（产物格幂等键的一部分，随执行信封下发）。
    round: u64,
    /// 归属序号（事实的 `produced_by`）。
    user_seq: u64,
    /// 本轮主体规格（宿主写口的 actor 用它）。
    actor: ActorSpec,
    /// 产物闸的结论（轮首由环判定，执行体只按结论记账；P1b / D-308）。
    artifact_allowed: bool,
    artifact_denied_reason: String,
    /// 工具端口账的写方（批次与 resume 共用同一套开账 / 关账）。
    port: ToolPort,
    /// 流式桥：交出本轮正文节点并切下一轮。
    bridge: Arc<UiBridge>,
    /// 本轮全部出参（消息 + 收束派生事实），幕尾一次取走。
    out: Mutex<PortOutput>,
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
        round: u64,
        user_seq: u64,
        drive_seq: u64,
        actor: ActorSpec,
        artifact_allowed: bool,
        artifact_denied_reason: String,
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
            round,
            user_seq,
            actor,
            artifact_allowed,
            artifact_denied_reason,
            port: ToolPort::new(facts_for_port, round, user_seq, drive_seq),
            bridge,
            out: Mutex::new(PortOutput::default()),
            presentation_tree,
            elimination_revoked,
        }
    }

    /// 本会话主体：节点诞生时写死的主体身份（D-147 档 2）。
    fn principal(&self) -> symbio_sdk::chat::principal::Principal {
        symbio_sdk::chat::principal::Principal::from_legacy(&symbio_sdk::authz::request_principal(
            self.ctx.as_ref(),
        ))
    }

    /// 取走本轮全部出参（消息 + 收束派生事实），调用方在桥释放前取一次。
    pub(crate) fn take_output(&self) -> PortOutput {
        std::mem::take(&mut self.out.lock().unwrap())
    }

    /// 本轮的通用执行信封：轮号 / 轮根 / 归属 / 产物闸结论 / 宿主写口。
    ///
    /// 闸在轮首由环判一次（只认 actor，见 `turn_runner::artifact_verdict`）；执行体
    /// 只按结论记账，拒绝理由随信封下发，由它在 `land_only` 出口落幂等的否定事实
    /// （P1b / D-308）。
    fn round_exec(&self) -> PortExecContext {
        PortExecContext {
            deadline_ms: None,
            abort: ExecAbortSignal::default(),
            idempotency_key: String::new(),
            sink: ExecEventSink::Null,
            emit: port::host_fact_emit(
                self.facts.clone(),
                self.round,
                self.actor.principal.clone(),
                clock_now_ms(),
                Some(self.user_seq),
                None,
            ),
            ctx: None,
            subject: self.round,
            group: self.root_id.clone(),
            origin: Some(self.user_seq),
            permitted: self.artifact_allowed,
            deny_reason: self.artifact_denied_reason.clone(),
            scratch_dir: Some(self.tool_archive_dir.clone()),
        }
    }

    /// 一笔返回后关账（实体在 [`ToolPort::close_call`]）。
    fn close_tool_calls(&self, call: &ToolPortCall, outcome: Option<&CallOutcome>) {
        self.port.close_call(call, outcome);
    }

    /// 用户结果（answer / reject）的**派出半**：不跑执行层，直接合成调用卡与结果卡。
    ///
    /// 与正常派发共用**同一条关账**（`begin_call` 在调用方 + `finish` 的
    /// `close_call`）：回答是 `sent → response`，拒绝是 `denied → failed`（授权行已落
    /// `authz.denied`，`close_call` 按 `deny_why` 收尾）。
    ///
    /// 交回的是**结论**而不是节点：收尾半只认 `CallOutcome`（它按端口账配对），
    /// 而这一条路上的结论在用户点按钮那一刻就已经定了——没有执行层要等，所以
    /// [`ToolDispatch::begin`] 交出的 future 第一次 poll 就 Ready。
    ///
    /// 调用卡沿用原节点 id（`turn.tool_calls[0].id == target`）：重建面按
    /// `artifact.added` 的 `tool_call_id` 复刻同一张卡，实时面这里发的是**终态
    /// 整条**——两面的 id、参数文本与状态因此一致。
    async fn wake_result_begin(&self, turn: &LlmTurn, call: &ToolPortCall) -> Option<CallOutcome> {
        let (text, ok) = match &call.wake {
            Some(Wake::Result { text, ok, .. }) => (text.clone(), *ok),
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
        let args = call_nodes
            .first()
            .and_then(|n| n.content.as_ref().map(|c| c.to_text()))
            .unwrap_or_default();
        let result_id = symbio_kernel::event::result_node_id(&call_id);
        let mut result_meta = serde_json::json!({ "success": ok, "tool_name": name });
        if !ok {
            result_meta["failure_kind"] = serde_json::json!("error");
        }
        let result = ChatNode {
            id: result_id.clone(),
            parent_id: Some(call_id.clone()),
            // 身份只写主体这一份（D-147）：工具结果的族由 `tool_principal` 折出，
            // 再写一遍 `role` 是两个表达说同一件事。
            principal: Some(symbio_sdk::chat::principal::Principal::tool_principal(
                &name,
            )),
            msg_type: Some(NodeKind::Body),
            content: Some(MessageContent::Text(text.clone())),
            status: Some(final_status),
            meta: Some(result_meta),
            timestamp: Some(symbio_sdk::clock::clock_now_ms()),
            ..Default::default()
        };

        for node in call_nodes.iter().chain(std::iter::once(&result)) {
            llm_emit_message(&self.sink, node.clone()).await;
        }
        let outcome = CallOutcome {
            call: call.call.clone(),
            call_id,
            name,
            text,
            ok,
            needs_user_action: false,
            prompt: None,
            failure_kind: failure_kind.to_string(),
            args,
            effects: Vec::new(),
            // 唤醒结果没有模型往返；结果消息仍由环按调用节点配对兜底（P3 迁往端口）。
            reply: symbio_plugin_api::port::PortReply::default(),
        };
        let mut produced = call_nodes;
        produced.push(result);
        self.out.lock().unwrap().nodes.extend(produced);
        // 关账归 [`ToolDispatch::finish`]（下一个环步）——与正常批次同一条分工。
        Some(outcome)
    }
}

#[async_trait]
impl ToolDispatch for SessionDispatchPort {
    async fn prepare(&self, _store: &dyn Store<Event = Event>, turn: &LlmTurn) {
        self.finalize_turn(turn).await;
    }

    async fn wake(&self, _store: &dyn Store<Event = Event>) -> Option<WakeBatch> {
        self.port.wake_batch()
    }

    async fn begin(
        self: Arc<Self>,
        _store: Arc<dyn Store<Event = Event>>,
        turn: &LlmTurn,
        call: &mut ToolPortCall,
    ) -> ToolBegin {
        // 唤醒的用户结果（回答 / 拒绝）不跑工具：节点与事实在这里合成。
        // 批准 / 重试 / 补充仍然走下面的正常派发路径——执行层只知道
        // 「有一次调用要跑」，不关心它是模型要的，还是用户点的。
        if matches!(call.wake, Some(Wake::Result { .. })) {
            self.port.begin_call(call);
            // 结论已经在手上：这个 future 第一次 poll 就 Ready（没有跨边界的往返，
            // 也就没有真正「在等」的那一圈）。
            return match self.wake_result_begin(turn, call).await {
                Some(outcome) => ToolBegin::InFlight(Box::pin(async move { outcome })),
                // 空：一格没落，也不登记在飞槽（派发器会把它显形）。
                None => ToolBegin::None,
            };
        }

        // ⓪ 第三笔债的前半：为这一笔落 `sent`。必须在执行层之前——
        //    `sent` 的语义是「已交给执行层」，而不是「工具已经开始跑」。
        self.port.begin_call(call);

        // ① 广播**这一笔**的工具节点。在派出这一侧：它在执行层之前就该出现在
        //    界面上（卡片显示「运行中」），而不是等结果回来才补。正文定格是一次
        //    每轮的（[`ToolDispatch::prepare`]），不在这里重复。
        let call_nodes = self.dispatch_window(turn).await;

        let this = self;
        let tool_calls = turn.tool_calls.clone();
        let outcome_call = call.call.clone();
        let reply_key = call.reply_key.clone();
        ToolBegin::InFlight(Box::pin(async move {
            this.execute_one(tool_calls, call_nodes, outcome_call, reply_key)
                .await
        }))
    }

    fn land_derived(&self, effects: Vec<EffectInput>) {
        let mut out = match self.out.lock() {
            Ok(out) => out,
            Err(poisoned) => poisoned.into_inner(),
        };
        out.derived.effects.extend(effects);
    }

    fn finish<'a>(
        &'a self,
        _store: &'a dyn Store<Event = Event>,
        call: &'a ToolPortCall,
        outcome: Option<&'a CallOutcome>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            self.close_tool_calls(call, outcome);
            // 产物落格：这一笔结果交回执行体的 `land_only` 出口（含唤醒结果与没跑到的
            // 占位结果）——环不再自己装配产物载荷（P1b / DE-048 D-308）。
            if let Some(outcome) = outcome {
                let env = self.round_exec();
                super::tool_exec::land_outcome_async(
                    &env,
                    &self.parent,
                    &self.sink,
                    &self.abort,
                    self.ctx.clone(),
                    outcome,
                )
                .await;
            }
        })
    }
}

/// 派发的**派出窗口**：定格正文 / 推理、广播工具调用节点。
///
/// 从 [`ToolDispatch::begin`] 里拆出来只为一件事：整段要 `await`（桥的排空与事件出口
/// 都是异步），而 `begin` 返回的那个 future 必须是 `'static`——把 `&self` 与 `&LlmTurn`
/// 的借用留在窗口里，future 只带走**已经建好的节点**。
impl SessionDispatchPort {
    /// 一轮**一次**的正文收口：把本轮正文 / 推理定格并切节点。
    ///
    /// 逐笔派发让 [`Self::dispatch_window`] 每笔调用跑一次；正文属于那一轮模型往返，
    /// 只能收口一次，因此单独的入口由 [`ToolDispatch::prepare`] 在开账之后调。
    async fn finalize_turn(&self, turn: &LlmTurn) {
        let mut produced: Vec<ChatNode> = Vec::new();

        // 定格本轮正文与推理（若有）：节点已由 UI 桥逐片广播，这里只迁状态并切节点。
        // 没有正文（纯工具调用轮）就不建节点——空节点是噪声。
        //
        // 两条流**一起定格**：都由桥建节点、都在本轮结束时收口，分两处做只是把同一个
        // 动作写两遍，且漏一处就有一条流永远转「流式中」（界面上表现为思考块卡住）。
        let text_node = if turn.text.trim().is_empty() {
            None
        } else {
            self.bridge.finalize_node(&turn.text)
        };
        let reason_node = self.bridge.finalize_reasoning_node();
        // 同步点：定格帧在桥的通道里排队，而下面的工具节点**直接**写出口
        // （`process_tool_calls_in_env` 只认 `ExecEventSink`）。不先排空队列，
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
        self.out
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .nodes
            .extend(produced);
    }

    /// 广播**这一笔**的工具调用节点（这条路径没有流式工具帧，由本处补建）。
    ///
    /// 先以 `Streaming` 广播（工具还没跑，卡片应显示运行中），落库副本用终态。
    async fn dispatch_window(&self, turn: &LlmTurn) -> Vec<ChatNode> {
        // 就地 detach：下面三处用同一份 `call_nodes`——广播帧、传给执行器作父查询源
        // （`process_tool_calls_in_env` 从它 clone 出运行态/终态帧，继承 detach 后的
        // parent_id），以及落库副本。一处收口，live↔存储自动锁步。
        let mut call_nodes =
            llm_build_tool_call_nodes(&self.root_id, &turn.tool_calls, &self.principal());
        for n in call_nodes.iter_mut() {
            detach_if_flat(n, &self.root_id, self.presentation_tree);
        }
        for node in &call_nodes {
            let mut frame = node.clone();
            frame.status = Some(NodeStatus::Streaming);
            llm_emit_message(&self.sink, frame).await;
        }
        call_nodes
    }

    /// 派发的**收尾窗口**：跑执行层、收派生事实、把结果映射成运行器的事实形状。
    ///
    /// 它就是 [`ToolDispatch::begin`] 交出的那个 future 的本体。为什么这一段必须是
    /// `'static` 的：它在**下一个环步**才被 poll（`await-tool` 行的动作），中间那一圈
    /// 「`sent` 有、终局无」正是 `waiting` 读的形状。所以它只带走**已经建好的东西**
    /// （`Arc<Self>`、模型交回的调用、广播过的节点），不带走任何借用。
    async fn execute_one(
        &self,
        tool_calls: Vec<symbio_kernel::llm::TurnToolCallInfo>,
        call_nodes: Vec<ChatNode>,
        call: port::Call,
        reply_key: Option<String>,
    ) -> CallOutcome {
        // ③ 分发（工具执行层：存档 / 钩子 / 审批 / 未执行收口全在里面）。
        //    父节点查询用刚广播的这批节点——它们与执行器要求的形状同源。
        //
        // 执行信封（P1b / DE-048 D-308）：产物格与授权事实的**宿主写口**随批走。
        // 产物闸在轮首判一次（`turn_runner::artifact_verdict`），结论以 `allowed` 交给执行体；
        // 落格本身由执行体的 `land_only` 出口做，环不装配产物载荷。
        let env = self.round_exec();
        let mut effects: Vec<EffectInput> = Vec::new();
        let (tool_msgs, parent_updates, replies) = process_tool_calls_in_env(
            tool_calls.clone(),
            &self.parent,
            &self.sink,
            &self.abort,
            self.ctx.clone(),
            &call_nodes,
            &env,
            self.facts.as_ref(),
            self.elimination_revoked,
            // 效果出参（DE-025 D-207）：每次终态调用一条。
            &mut effects,
            reply_key.as_deref(),
        )
        .await;

        let mut produced: Vec<ChatNode> = Vec::new();
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
        self.out.lock().unwrap().nodes.extend(produced);

        // ④ 映射为 core 的事实形状（运行器据此拼下一轮 prompt；`finish` 再交回执行体
        //    落 `artifact.added`）。逐笔执行 ⇒ 这一笔只有一条结果节点；执行层没交回
        //    结果（理论上由未执行收口补占位）时按 `NOT_EXECUTED` 收口，环据此写
        //    `failed`——「未收到 response 的债」就是终局判据。
        let mut outcome = match tool_msgs.iter().find(|m| {
            m.parent_id.as_deref() == Some(call.id.as_str())
                || tool_calls
                    .iter()
                    .any(|tc| tc.id.as_deref() == m.parent_id.as_deref())
        }) {
            Some(m) => map_outcome(m, &call, &tool_calls, &call_nodes, &parent_updates),
            None => CallOutcome {
                call,
                failure_kind: NOT_EXECUTED.to_string(),
                call_id: tool_calls
                    .first()
                    .and_then(|tc| tc.id.clone())
                    .unwrap_or_default(),
                ..CallOutcome::empty()
            },
        };
        // 效果出参随这一笔走（不写进共用取件面）：环在整轮收口时按**调用顺序**
        // 交回，并发完成的次序因此不会改变事实顺序。
        outcome.effects = effects;
        // 应答承载（`D-319`）：一笔调用一条结果消息；环只按序追加。
        outcome.reply = replies.into_iter().next().unwrap_or_default();
        outcome
    }
}

/// 结果节点 → 运行器事实形状（逐笔：一笔一条）。
fn map_outcome(
    m: &ChatNode,
    call: &port::Call,
    tool_calls: &[symbio_kernel::llm::TurnToolCallInfo],
    call_nodes: &[ChatNode],
    parent_updates: &[ChatNode],
) -> CallOutcome {
    let call_id = m.parent_id.clone().unwrap_or_default();
    let name = m
        .tool_name()
        .map(str::to_string)
        .or_else(|| {
            tool_calls
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
    CallOutcome {
        call: call.clone(),
        call_id,
        name,
        text: m.content.as_ref().map(|c| c.to_text()).unwrap_or_default(),
        ok: m
            .meta
            .as_ref()
            .and_then(|meta| meta.get("success"))
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
        needs_user_action: m.is_interaction() && m.status == Some(NodeStatus::WaitingUserAction),
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
        args: call_node
            .and_then(|n| n.content.as_ref().map(|c| c.to_text()))
            .unwrap_or_default(),
        effects: Vec::new(),
        reply: symbio_plugin_api::port::PortReply::default(),
    }
}

// ── panic 面登记（PN-001…003）─────────────────────────────────────────
// 本文件每一处 `unwrap` / `expect` / `panic!` / `unreachable!` 的理由。登记放在
// 文件内而不是集中一张表：理由与它解释的那段代码会一起被 review、一起被删。
// 判据见 `scripts/panic-audit.mjs`。**加一处 panic 必须同时加一行登记，理由非空。**
// panic-allow crates/symbio-plugin-loop/src/tool_dispatch.rs::take_output: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
// panic-allow crates/symbio-plugin-loop/src/tool_dispatch.rs::execute_one: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
// panic-allow crates/symbio-plugin-loop/src/tool_dispatch.rs::wake_result_begin: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。

#[cfg(test)]
#[path = "tool_dispatch.test.rs"]
mod tests;
