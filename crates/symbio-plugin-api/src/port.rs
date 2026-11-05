//! 世界端口的注册面 —— 端口名字与契约静态，提供者随活插件的能力行动态收集（D-298）。
//!
//! ## 端口集开放（`DE-047` I-6）
//!
//! 世界端口种类是**开放**的：提供者申报自己服务的端口名（可多个），环只按端口名解析
//! provider；注册面（族 `multi("port")` + 行名 `<port:{port}>:{provider_id}`）不因新增
//! 端口而变。端口名是**数据**，不是类型——本文件不出现 `ModelProvider` / `ToolProvider`
//! 一类按世界命名的 trait，执行面只有通用的 [`PortExecutor`] / [`PortInput`] /
//! [`PortOutcome`]（I-6.2）。`PortDecl`、派发 / 等待两行模板、
//! `Call{port,id,attempt,via}` 的幂等键与记账住 `loop`。**提供者不静态注册**：
//! `submit_*!` 一类的编译期收集器在这里没有位置——它们链接期生效、没有启用过滤，
//! 插件停用后条目仍在（实例不构造不等于链接期条目消失），「停用即缺席」当场判红。
//! 这正是 `submit_port_provider!` 案被否的原因（D-298「为什么不是 (b)」）。
//!
//! ## 提供者随能力表走（单一 `port` 族，D-299）
//!
//! 提供者行由**活插件**在自己的 `traverse` 里 `put`（与工具 / 模型目录 / 挂载同一广播
//! 机制）：停用的插件不参与装配、不 traverse、不写行；收集器每次装配重建，旧行不残留。
//! `tool` 端口的提供者是独立插件（`crates/symbio-plugin-tool`），与 `notify` 同一条
//! 通路：在自己的 `traverse` 里 `put` 提供者行。生命周期同一条：收集器每次装配重建，
//! 行不残留。
//! 全部端口共用**一个 `multi` 族**，每（端口, 提供者）一行——注册层的族声明不认识端口名，
//! 加端口不加族；端口级唯一由读数与装载期检查裁（[`capability_port_of`] 按行名前缀
//! 分组），不靠表侧 `demote`。
//!
//! ## 判重与缺席
//!
//! - `0` 行 = 缺席（合法装配态：没装提供者插件）；响亮失败发生在调用点（消费方拿到
//!   [`PortReading::Absent`] 自行落事实并中止）。
//! - `>1` 行 = 判重：两行俱在（谁都没被顶掉），[`capability_port_check`] 落一条错误
//!   进 `CAPABILITY_ERRORS`（装载期判红）并指名两者，**不静默顶掉**。不做显式
//!   优先级——单 provider 场景没有挑选判据，「挑一个」正是要消灭的静默形态。
//! - 端口集开放：任何已注册的行名前缀就是一个合法端口名；0 行即 [`PortReading::Absent`]。
//!
//! ## 不出现在消费面
//!
//! `Registry` 的读方全部按族取（`list` 的签名要求族参数，仓内无全表遍历读点）；
//! 按名取的 `get` 由保留行名前缀挡住（工具名不含尖括号，同 `MOUNT_ROOT_ROW` 手法）。
//! 结构性用例钉住这条事实：port 行在表 ⇒ 六个消费族（工具 / 模型 / 提示词 / 挂载 /
//! 选项 / 可配置）仍然看不到它。

use crate::effects::FactDraft;
use serde_json::Value;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use symbio_kernel::capability::{CapabilityFamily, CapabilityRow, Registry};
use symbio_kernel::exec::{ExecAbortSignal, ExecEventSink};
use symbio_kernel::plugin::{PluginInvokeRequest, PluginInvokeRequestExt};
use symbio_sdk::capability::{capability_report_error, CAPABILITY_VISITOR};

/// 一个世界端口的提供者：申报端口名与提供者 id，并交出通用执行面。
///
/// 声明位（`ports` / `provider_id`）供表造行、消费方判缺席与判重；执行面
/// （[`Self::executor`]）是**唯一**的环 ↔ provider 合同——`Arc<dyn PortProvider>`
/// 在 Rust 里跨转型不出去，执行面必须以方法形式落在这里（D-298 的形式成立）。
///
/// 方法名与类型都**不具名任何世界**（`DE-047` I-6.2）：端口名是数据，载荷是通用的
/// [`PortInput`] / [`PortOutcome`]，能力位是 [`PortCapabilities`]。
pub trait PortProvider: Send + Sync + 'static {
    /// 本提供者服务的端口名（可多个，`DE-047` I-6.1）。环只按端口名解析 provider。
    fn ports(&self) -> &'static [&'static str];
    /// 提供者 id（行名后缀；判重读数里指名参与判重的各方）。
    fn provider_id(&self) -> &str;

    /// 可选能力位（`DE-047` I-6.6）：只允许通用词，默认全无。
    fn capabilities(&self) -> PortCapabilities {
        PortCapabilities::default()
    }

    /// 一笔债的执行合同（`DE-047` I-6.2 / I-6.4）。`None` = 本提供者没有执行面。
    fn executor(&self) -> Option<Arc<dyn PortExecutor>> {
        None
    }
}

/// 端口提供者的可选能力位（`DE-047` I-6.6）：词表只允许 `streaming` / `cancel` /
/// `idempotent` 这类通用词，不许出现某个世界的策略名。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PortCapabilities {
    /// 能按增量输出（帧经 [`PortExecContext::sink`] 写出）。
    pub streaming: bool,
    /// 能被 [`PortExecContext::abort`] 取消。
    pub cancel: bool,
    /// 同一 `idempotency_key` 重放安全。
    pub idempotent: bool,
}

/// 一笔债的输入（`DE-047` I-6.2）：**端口名 + 行 / 值草稿**。
///
/// 载荷是值不是类型：某端口有哪些行、字段如何校验，由该端口所属插件声明，
/// provider 执行时按行契约自校验（I-6.3）。接口不承载世界契约细节。
#[derive(Debug, Clone)]
pub struct PortInput {
    /// 端口名（与 `request` 事实载荷里的 `port` 同一个值）。
    pub port: &'static str,
    /// 载荷草稿：调用点的行 / 值。
    pub payload: Value,
}

/// 读面的一行：身份 + 端口 + 种类 + **不透明**载荷（本文件不定义任何世界词）。
///
/// `port` 是宿主落协议事实时按债身份盖的端口名；世界事实没有端口，给空串。
/// 执行体据此把自己端口的产物与其他端口的产物分开（端口名仍是数据）。
#[derive(Debug, Clone, PartialEq)]
pub struct PortFact {
    /// 事实身份（入库事件的 id）。
    pub id: String,
    /// 端口名；世界事实为空串。
    pub port: String,
    /// 事实种类（字符串透传，解释权在声明方）。
    pub kind: String,
    pub payload: Value,
}

/// 通用执行体上下文的事实读面：**宿主实现、执行体使用**；环不调用、不解释。
/// 不是具名世界接口——它只认「身份 / 端口 / 种类 / 载荷」四样通用形状。
pub trait PortFactReader: Send + Sync + 'static {
    /// 本会话的事实（读面在打开时就绑定了会话身份）。
    fn facts(&self) -> Vec<PortFact>;
}

/// 一笔债的执行上下文（`DE-047` I-6.5 / D-305 裁决一）：命名不带世界词。
#[derive(Clone)]
pub struct PortExecContext {
    /// 截止时刻（毫秒，相对时间轴）；`None` = 无截止。
    pub deadline_ms: Option<u64>,
    /// 取消信号。
    pub abort: ExecAbortSignal,
    /// 幂等键：同一笔债的重放按它去重。
    pub idempotency_key: String,
    /// 增量出口（`capabilities.streaming` 为真时使用）。
    pub sink: ExecEventSink,
    /// 事实草稿出口（D-305 裁决一）：provider 执行期间可追加事实，宿主唯一写口按
    /// 幂等键落行；环不解析、不识别落下的行。
    pub emit: PortFactEmit,
    /// 调用方 ctx（提供者据此读能力表）；`None` = 无。
    pub ctx: Option<Arc<dyn PluginInvokeRequest>>,
    /// 本次执行所服务的**触发事实标识**（`request` 事实的幂等键）：
    /// 同一批派发里的不同调用各有各的标识，可由执行体用作产物的命名基准。
    pub subject: String,
    /// 应答组（宿主原样交值，不透明；不参与配对）。
    pub group: String,
    /// 会话身份（宿主原样交值）：执行体需要会话身份时读这一格，不读 `subject`。
    pub session_id: String,
    /// 产生方（归属序号）；`None` = 无归属。
    pub origin: Option<u64>,
    /// 放行位（宿主侧的产物闸结论）；执行体只按结论记账，不自判。
    pub permitted: bool,
    /// 拒绝因（放行位为假时的整份交代）。
    pub deny_reason: String,
    /// 工作目录（宿主为本轮准备的可写目录）；`None` = 无。
    pub scratch_dir: Option<PathBuf>,
    /// 事实读面（宿主实现）：执行体按需读本可见域的事实；`None` = 无。
    pub reader: Option<Arc<dyn PortFactReader>>,
}

/// 宿主交给执行体的**不透明句柄束**：环只原样转交，不读取、不解释其中任何字段。
///
/// 这三个值不是环的知识——`ctx` 里装什么、`scratch_dir` 指向哪里、`sink` 把帧送去
/// 何处，都由宿主决定、由执行体自用（`D-316`）。环只保证「宿主给的，原样到达」。
#[derive(Clone)]
pub struct PortExecHandles {
    /// 调用方上下文（提供者据此读能力表 / 上游）；`None` = 无。
    pub ctx: Option<Arc<dyn PluginInvokeRequest>>,
    /// 宿主为本轮准备的可写目录；`None` = 无。
    pub scratch_dir: Option<PathBuf>,
    /// 增量出口（宿主决定去向）。
    pub sink: ExecEventSink,
    /// 会话身份（宿主原样交值；空串 = 无）。
    pub session_id: String,
}

impl Default for PortExecHandles {
    fn default() -> Self {
        Self {
            ctx: None,
            scratch_dir: None,
            sink: ExecEventSink::silent(),
            session_id: String::new(),
        }
    }
}

/// provider 执行一笔债期间追加事实草稿的通用出口（`DE-048` D-305 裁决一）。
///
/// 形状与 [`ExecEventSink`] 同族：**生产者只写，去处由宿主决定**。命名的「事实」
/// 是通用的记账单位（[`FactDraft`]），不是某个世界的事件词。
#[async_trait::async_trait]
pub trait PortFactWriter: Send + Sync + 'static {
    /// 追加一组事实草稿。宿主的唯一写口按草稿给的幂等键落行（重复投递被吸收）；
    /// provider 拿不到写句柄，也无权指定轮号 / 主体 / 时刻。
    async fn emit(&self, facts: Vec<FactDraft>);
}

/// 事实草稿出口的值形态（与 [`ExecEventSink`] 的 `Null` 同一套语义）。
#[derive(Clone, Default)]
pub struct PortFactEmit(Option<Arc<dyn PortFactWriter>>);

impl PortFactEmit {
    /// 静默出口：没有宿主写口时 `emit` 是 no-op（夹具 / 内部请求）。
    pub fn silent() -> Self {
        Self(None)
    }

    /// 宿主写口。
    pub fn new(writer: Arc<dyn PortFactWriter>) -> Self {
        Self(Some(writer))
    }

    /// 追加一组事实草稿。
    pub async fn emit(&self, facts: Vec<FactDraft>) {
        if let Some(writer) = &self.0 {
            writer.emit(facts).await;
        }
    }
}

impl fmt::Debug for PortFactEmit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            Some(_) => f.write_str("PortFactEmit(Host)"),
            None => f.write_str("PortFactEmit::Silent"),
        }
    }
}

/// 通用错误分类（`DE-047` I-6.2）：只有三档，不带世界语义。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortErrorClass {
    /// 可重试（租约看护可再举一次 `request`）。
    Retryable,
    /// 不可重试（判死）。
    NonRetryable,
    /// 需人介入（挂起等用户动作）。
    NeedsHuman,
}

/// 执行体交回的**应答承载**（中性契约）：环只按序追加、按 `links` 配对，
/// **不读其中的世界字段**——消息怎么组装是执行体的私事（D-319）。
#[derive(Debug, Clone, Default)]
pub struct PortReply {
    /// 应答消息（不透明承载；环只传递，不解释内容）。
    pub messages: Vec<symbio_kernel::llm::PromptMessage>,
    /// 应答配对：执行体内的调用键 → 应答里的配对键（环只按它配对）。
    pub links: Vec<(String, String)>,
}

/// 一笔债的产出（`DE-047` I-6.2）：事实草稿、错误与应答承载。
#[derive(Debug, Clone, Default)]
pub struct PortOutcome {
    /// 要写到行上的事实草稿（每项是一个 JSON 对象，字段由端口插件声明）。
    pub facts: Vec<Value>,
    /// `Some` ⇒ 这一笔失败；`facts` 仍可携带半截产物。
    pub error: Option<(PortErrorClass, String)>,
    /// 这一笔交回的应答承载（缺省为空；环只追加与配对）。
    pub reply: PortReply,
}

impl PortOutcome {
    /// 成功且无额外事实。
    pub fn ok() -> Self {
        Self::default()
    }

    /// 失败。
    pub fn failed(class: PortErrorClass, why: impl Into<String>) -> Self {
        Self {
            facts: Vec::new(),
            error: Some((class, why.into())),
            ..Default::default()
        }
    }

    /// 这一笔成功与否。
    pub fn success(&self) -> bool {
        self.error.is_none()
    }
}

/// 一笔债的执行合同（`DE-047` I-6.2 / I-6.4）。
///
/// 与 [`LlmAdapter`] 同形：合同方法是异步的，跨 `dyn` 边界走 `Arc<dyn PortExecutor>`。
/// **只承诺执行一笔债**：并发由宿主调度多笔债或 provider 内部自理；接口不出现批、
/// 并发度、熔断、gate 读数等策略参数。
#[async_trait::async_trait]
pub trait PortExecutor: Send + Sync + 'static {
    /// 执行一笔债：读 [`PortInput`] 的载荷，按端口插件声明的行契约自校验，
    /// 写 [`PortOutcome`] 的事实草稿。
    async fn execute(&self, ctx: PortExecContext, input: PortInput) -> PortOutcome;
}

/// 一次工具调用以「等待用户动作」结束时的载荷。
///
/// 工具只声明**意图与内容**（`failure_kind` + `prompt`），**不构造节点**：
/// user_prompt 节点的身份由调用 id派生（`{call_id}-result`），能力包不另立 id。
/// 让工具自己编 id 正是历史上「同一逻辑节点两个 id」的成因——前端出现重复审批卡，
/// resume 只删得掉一个，另一个永远不消失。
#[derive(Debug, Clone)]
pub struct PendingPrompt {
    /// 节点正文（审批卡 / 提问卡显示的那句话）
    pub text: String,
    /// 写进 `meta.prompt` 的载荷——卡片形状由**工具**决定（审批卡 / 提问卡不同）
    pub prompt: Value,
    /// [`symbio_kernel::capability::failure_kind`] 里的 pending 取值
    pub failure_kind: String,
}

/// port 行的载荷。
///
/// 消费方经 `row.payload::<PortPayload>()` 下转型取回——类型是声明方与消费方之间的
/// 私有约定，表不参与校验（D-175 的开放载荷形状）。
pub struct PortPayload(Arc<dyn PortProvider>);

impl PortPayload {
    pub fn new(provider: Arc<dyn PortProvider>) -> Self {
        Self(provider)
    }

    /// 取提供者。
    pub fn provider(&self) -> &Arc<dyn PortProvider> {
        &self.0
    }
}

/// 全部端口共用的唯一 `multi` 族（D-299）：族不区分端口，端口级分组由行名前缀
/// （[`port_row_name`]）在读数里做——加端口不加族。
pub const PORT_FAMILY: CapabilityFamily = CapabilityFamily::multi("port");

/// 任何端口都落在唯一族里（端口集开放，`DE-047` I-6.7）：族不区分端口，端口级
/// 分组由行名前缀（[`port_row_name`]）在读数里做——加端口不加族。
pub fn port_family(_port: &str) -> Option<CapabilityFamily> {
    Some(PORT_FAMILY)
}

/// port 行的行名：保留前缀，撞不上任何线上工具名。
///
/// 表键是全表唯一的名字空间（族不参与键），端口名当行名会与同名工具互相顶掉；
/// 工具名不含尖括号，前缀使碰撞不可表达——与 `MOUNT_ROOT_ROW` 同一条手法。
pub fn port_row_name(port: &str, provider_id: &str) -> String {
    format!("<port:{port}>:{provider_id}")
}

/// 提供者行：本提供者申报的**每个端口**一行（`multi` 族，`active = false`，不参与
/// 表侧降位——端口级唯一由读数与装载期检查裁，D-299）。
///
/// 一个 provider 申报 N 个端口就产生 N 行（`DE-047` I-6.1）：端口身份是数据，行数随
/// 数据走；新增端口只改提供者的 `ports()` 返回值，不改注册面、不改族。
pub fn port_rows(provider: &Arc<dyn PortProvider>) -> Vec<CapabilityRow> {
    provider
        .ports()
        .iter()
        .map(|port| {
            CapabilityRow::new(
                PORT_FAMILY,
                port_row_name(port, provider.provider_id()),
                PortPayload::new(provider.clone()),
                false,
            )
        })
        .collect()
}

/// 一个端口的装配期读数。
pub enum PortReading {
    /// 0 行：端口缺席（没装提供者插件——合法装配态，响亮失败在调用点）。
    Absent,
    /// 1 行：就绪。
    Ready(Arc<CapabilityRow>),
    /// `>1` 行：判重——同端口的多个提供者**两行俱在**（单族不降位），`dropped` 是
    /// 其余提供者的行名。
    ///
    /// 装载期已由 [`capability_port_check`] 落错误（不静默顶掉）；调用方拿到本读数
    /// 同样必须响亮处理，不许当作 [`PortReading::Ready`] 用。
    Duplicate {
        kept: Arc<CapabilityRow>,
        dropped: Vec<String>,
    },
}

impl fmt::Debug for PortReading {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Absent => write!(f, "Absent"),
            Self::Ready(row) => write!(f, "Ready({})", row.name()),
            Self::Duplicate { kept, dropped } => {
                write!(
                    f,
                    "Duplicate {{ kept: {}, dropped: {dropped:?} }}",
                    kept.name()
                )
            }
        }
    }
}

/// 读一个端口的提供者行（纯读数，不落错误）。
///
/// 判定的全部输入是**按行名前缀分组**后的行数（单一 `port` 族里其他端口的行不可
/// 见）：`0` 缺席、`1` 就绪、`>1` 判重。行不降位（`active = false`），行数让
/// 「多提供者」可判——读数不猜、不回落。
pub async fn capability_port_of(visitor: &dyn Registry, port: &str) -> PortReading {
    let prefix = port_row_name(port, "");
    let rows: Vec<_> = visitor
        .list(PORT_FAMILY)
        .await
        .into_iter()
        .filter(|r| r.name().starts_with(&prefix))
        .collect();
    match rows.len() {
        0 => PortReading::Absent,
        // panic-allow crates/symbio-plugin-api/src/port.rs::capability_port_of: 行数已判定为 1，next() 取不到说明 list 与 len 之间表被改写（同一次调用内不会发生），属代码缺陷。
        1 => PortReading::Ready(rows.into_iter().next().expect("len == 1")),
        _ => {
            let mut it = rows.into_iter();
            // panic-allow crates/symbio-plugin-api/src/port.rs::capability_port_of: 行数已判定为 >1，next() 取不到说明 list 与 len 之间表被改写（同一次调用内不会发生），属代码缺陷。
            let kept = it.next().expect("len > 1");
            let dropped = it.map(|r| r.name().to_string()).collect();
            PortReading::Duplicate { kept, dropped }
        }
    }
}

/// 从 `port` 族里已注册的行名派生端口集（`DE-047` I-6.7）。
///
/// 行名形状唯一来源是 [`port_row_name`]：`<port:{port}>:{provider_id}`。解析不出的行
/// （不是本族造的）由 `filter_map` 跳过——它们本来就不可见（`Registry::list` 按族取）。
async fn registered_ports(visitor: &dyn Registry) -> Vec<String> {
    let mut out = Vec::new();
    for row in visitor.list(PORT_FAMILY).await {
        let Some(rest) = row.name().strip_prefix("<port:") else {
            continue;
        };
        let Some((port, _)) = rest.split_once(">:") else {
            continue;
        };
        let port = port.to_string();
        if !out.contains(&port) {
            out.push(port);
        }
    }
    out
}

/// 装配期判重检查（D-298 / D-299 / `DE-047` I-6.7）：端口集从**已注册行**派生
/// （`<port:{port}>:` 前缀），每个端口按行名前缀读一次分组行数；判重 ⇒ 落一条错误
/// 进 `CAPABILITY_ERRORS`。
///
/// **时点契约**：必须在能力收集结束、编排方读错误桶（`capability_take_errors`）
/// 裁决**之前**调用——判重错误与收集期错误同桶同路，裁决即中止会话（装载期硬红）。
/// 落桶之后才裁决 ⇒ 错误留在桶里无人读，判重退化为「照常跑」，那是本检查要消灭的
/// 形态。缺席**不报错**——「没装提供者插件」是合法装配态（不装 model 插件的部署
/// 存在），响亮失败发生在调用点。没有 `CAPABILITY_VISITOR` 的 ctx 上本函数静默跳过：
/// 收集尚未开始时没有表可读。
pub async fn capability_port_check(ctx: &Arc<dyn PluginInvokeRequest>) {
    let Some(visitor) = ctx.get(CAPABILITY_VISITOR) else {
        return;
    };
    for port in registered_ports(&*visitor).await {
        if let PortReading::Duplicate { kept, dropped } = capability_port_of(&*visitor, &port).await
        {
            capability_report_error(
                ctx,
                "core",
                format!(
                    "世界端口 `{port}` 注册了多个提供者：`{}` 与 {dropped:?} 两行俱在",
                    kept.name(),
                ),
            )
            .await;
        }
    }
}

#[cfg(test)]
#[path = "port.test.rs"]
mod tests;
