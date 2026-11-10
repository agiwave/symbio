//! 世界端口的注册面：族与行名、端口名校验、按行名前缀分组的读数与装配期判重，
//! 以及提供者 / 执行体的通用契约（`DE-047` I-6）。
//!
//! ## 为什么整张端口面住 sdk
//!
//! 读/校验半是**宿主与编排方问表**的机制，D-404 / D-414 先搬；`D-440` ②把**注册面**
//! （`PortProvider` / `PortExecutor` / 载荷与读数）也搬了进来——门面（plugin-api）
//! 是架构词汇层，自有实现只许降（FA-001）；端口契约不认识任何世界词，与本域其余
//! 读口同一条边界。两半必须同源：`PORT_FAMILY` 的声明、`port_row_name` 的拼法、
//! `capability_port_of` 的分组、`port_rows` 的造行各自分家，读数与注册就会各说各话。
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
//!
//! ## 提供者随能力表走（单一 `port` 族，D-299）
//!
//! 提供者行由**活插件**在自己的 `traverse` 里 `put`（与工具 / 模型目录 / 挂载同一广播
//! 机制）：停用的插件不参与装配、不 traverse、不写行；收集器每次装配重建，旧行不残留。
//! 全部端口共用**一个 `multi` 族**，每（端口, 提供者）一行；端口级唯一由读数与装载期
//! 检查裁（[`capability_port_of`] 按行名前缀分组），不靠表侧 `demote`。
//!
//! ## 判重与缺席
//!
//! - `0` 行 = 缺席（合法装配态：没装提供者插件）；响亮失败发生在调用点（消费方拿到
//!   [`PortReading::Absent`] 自行落事实并中止）。
//! - `>1` 行 = 判重：两行俱在（谁都没被顶掉），[`capability_port_check`] 落一条错误
//!   进 `CAPABILITY_ERRORS`（装载期判红）并指名两者，**不静默顶掉**。
//! - 端口集开放：任何已注册的行名前缀就是一个合法端口名；0 行即 [`PortReading::Absent`]。

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;

use crate::capability::{capability_report_error, CAPABILITY_VISITOR};
use crate::llm::PromptMessage;
use crate::plugin::effects::FactDraft;
use symbio_kernel::capability::{CapabilityFamily, CapabilityRow, Registry};
use symbio_kernel::exec::{ExecAbortSignal, ExecEventSink};
use symbio_kernel::plugin::{PluginInvokeRequest, PluginInvokeRequestExt};

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

/// 端口名的保留字符：环事件 id 的分隔符。
///
/// 事件 id 形如 `ring:{port}:{id}:{attempt}:{kind}`，端口位是首个字段；端口名一旦
/// 含 `:`，解析会把端口位截断（`model:assist` 会被读成 `model`），事实键与债键对不上。
/// 这里 **fail-closed**：注册期直接拒绝，不转义、不猜（D-336「已知编码风险」）。
const PORT_NAME_RESERVED: &[char] = &[':'];

/// 端口名校验：种进事件 id 之前就把保留字符挡在注册期。
pub fn check_port_name(port: &str) {
    if let Some(ch) = port.chars().find(|ch| PORT_NAME_RESERVED.contains(ch)) {
        // panic-allow crates/symbio-sdk/src/capability/port.rs::check_port_name: 注册期 fail-closed——含保留字符的端口名会让环事件 id 的端口位被截断，注册当场拒绝；带病启动比崩掉更难诊断（D-336）。
        panic!(
            "端口名 {port:?} 含有环事件 id 的保留字符 {ch:?}，端口注册失败。\
             事件 id 是 `ring:{{port}}:{{id}}:{{attempt}}:{{kind}}`，端口位按 `:` 定界；\
             命名端口时请避开保留集 {PORT_NAME_RESERVED:?}（D-336）。"
        );
    }
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
    /// 装载期已由门面侧的 `capability_port_check` 落错误（不静默顶掉）；调用方拿到本读数
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
        // panic-allow crates/symbio-sdk/src/capability/port.rs::capability_port_of: 行数已判定为 1，next() 取不到说明 list 与 len 之间表被改写（同一次调用内不会发生），属代码缺陷。
        1 => PortReading::Ready(rows.into_iter().next().expect("len == 1")),
        _ => {
            let mut it = rows.into_iter();
            // panic-allow crates/symbio-sdk/src/capability/port.rs::capability_port_of: 行数已判定为 >1，next() 取不到说明 list 与 len 之间表被改写（同一次调用内不会发生），属代码缺陷。
            let kept = it.next().expect("len > 1");
            let dropped = it.map(|r| r.name().to_string()).collect();
            PortReading::Duplicate { kept, dropped }
        }
    }
}

/// 装配期判重检查（D-298 / D-299 / `DE-047` I-6.7）：**一次遍历**把已收集的
/// provider 行按端口名分组，同名多提供者 ⇒ 落一条错误进 `CAPABILITY_ERRORS`。
///
/// 端口集是装配期的**收集结果**（活插件 traverse 时申报，停用即缺席），不再有
/// 「先按行名前缀派生端口集、再逐个回表分组」那层反推（D-440）。
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
    // 端口集 = 本轮收集到的 provider 行一次遍历（不再有「先派生端口清单、再逐个
    // 回表分组」那层反推，D-440）；分组判重仍走唯一读数，保证两行俱在不静默顶掉。
    let mut ports: std::collections::BTreeSet<String> = Default::default();
    for row in visitor.list(PORT_FAMILY).await {
        let Some(rest) = row.name().strip_prefix("<port:") else {
            continue;
        };
        let Some((port, _)) = rest.split_once(">:") else {
            continue;
        };
        ports.insert(port.to_string());
    }
    for port in ports {
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

// ============================================================
// 注册面与执行契约（自 plugin-api 搬入，D-440 ②；门面只留转发）。
// ============================================================

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
    fn ports(&self) -> Vec<String>;
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
    pub port: String,
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

    /// 事件原样（含入库 `seq`）：行执行体按它构造 `RowRequest`（D-338）。
    ///
    /// 默认空：只实现 `facts()` 的宿主仍可工作，读不到 `seq` 的行按「无区间」自处。
    fn events(&self) -> Vec<symbio_kernel::event::Event> {
        Vec::new()
    }
}

/// 一笔债的执行上下文（`DE-047` I-6.5 / D-305 裁决一）：命名不带世界词。
///
/// **不含租约截止**（D-370）：租约是环的簿记（落在 `sent{deadline_ms}`），不是执行体的
/// 预算；执行体各自持有自己的超时常数。
#[derive(Clone)]
pub struct PortExecContext {
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
    /// 工作目录（宿主为本轮准备的可写目录）；`None` = 无。
    pub scratch_dir: Option<PathBuf>,
    /// 事实读面（宿主实现）：执行体按需读本可见域的事实；`None` = 无。
    pub reader: Option<Arc<dyn PortFactReader>>,
    /// 入站来源主体（D-067）：**宿主**从本轮 ctx 取好并解析完（缺席 ⇒ 会话身份），
    /// 环只原样转交。效果面拿它填 `EffectEnv.source`——承诺的 `to` 是数据，不是
    /// 让扩展插件自己去读某个 ctx 键。
    pub source: String,
}

/// 宿主交给执行体的**不透明句柄束**：环只原样转交，不读取、不解释其中任何字段。
///
/// 这几个值不是环的知识——`ctx` 里装什么、`scratch_dir` 指向哪里、`sink` 把帧送去
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
    /// 入站来源主体（宿主原样交值；空串 = 本机用户发起 ⇒ 效果面回落会话身份）。
    pub source: String,
}

impl Default for PortExecHandles {
    fn default() -> Self {
        Self {
            ctx: None,
            scratch_dir: None,
            sink: ExecEventSink::silent(),
            session_id: String::new(),
            source: String::new(),
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

    /// 追加一组事实草稿，并**显式带 owner**（落格信封的归属位）。
    ///
    /// `owner` **只从声明表来**：机制侧按草稿的 `kind` 查出格的申报方
    /// （`Frozen::event_owner`），再交给写口。它**不从草稿来**——`FactDraft` 的信封
    /// 三格（轮号 / 归属 / 时刻）归宿主，行为无权指定（DE-026 D-252）；给草稿加
    /// `actor` 字段会把这条分界重新打开。
    ///
    /// 缺省实现丢弃归属、退化为 [`PortFactWriter::emit`]（只读夹具与内部请求仍可工作）。
    async fn emit_owned(&self, owner: &str, facts: Vec<FactDraft>) {
        let _ = owner;
        self.emit(facts).await;
    }
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

    /// 追加一组事实草稿，归属位由机制侧给的 `owner` 填（见
    /// [`PortFactWriter::emit_owned`] 的口径：owner 只从声明表来，不从草稿来）。
    pub async fn emit_owned(&self, owner: &str, facts: Vec<FactDraft>) {
        if let Some(writer) = &self.0 {
            writer.emit_owned(owner, facts).await;
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
    pub messages: Vec<PromptMessage>,
    /// 应答配对：执行体内的调用键 → 应答里的配对键（环只按它配对）。
    pub links: Vec<(String, String)>,
}

/// 一次派发的**授权结论**（D-385）：执行体自判、环不代填。
///
/// 它没有「未表态」的变体，也没有 `Default`：任何 [`PortExecutor`] 都必须选一个变体
/// 才能交回 [`PortOutcome`]，漏判在类型上不可表达。环不构造它，也不解释它的理由。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthzVerdict {
    /// 执行体自己的闸门通过（对不需要授权判定的端口是「无待判事项」）。
    Authorized,
    /// 执行体自己的闸门拒绝；`reason` 是拒绝因，必须写清。
    Denied { reason: String },
}

impl AuthzVerdict {
    /// 拒绝结论（构造时就把理由带上，不留空理由的路径）。
    pub fn denied(reason: impl Into<String>) -> Self {
        Self::Denied {
            reason: reason.into(),
        }
    }

    /// 是否被拒绝。
    pub fn is_denied(&self) -> bool {
        matches!(self, Self::Denied { .. })
    }

    /// 拒绝因（通过 ⇒ `None`）。
    pub fn denial(&self) -> Option<&str> {
        match self {
            Self::Authorized => None,
            Self::Denied { reason } => Some(reason.as_str()),
        }
    }
}

/// 一笔债的产出（`DE-047` I-6.2）：授权结论、事实草稿、错误与应答承载。
#[derive(Debug, Clone)]
pub struct PortOutcome {
    /// 执行体自判的授权结论（D-385）；环只读这一格决定是否落它交来的事实。
    pub authz: AuthzVerdict,
    /// 要写到行上的事实草稿（每项是一个 JSON 对象，字段由端口插件声明）。
    pub facts: Vec<Value>,
    /// `Some` ⇒ 这一笔失败；`facts` 仍可携带半截产物。
    pub error: Option<(PortErrorClass, String)>,
    /// 这一笔交回的应答承载（缺省为空；环只追加与配对）。
    pub reply: PortReply,
}

impl PortOutcome {
    /// 成功、无额外事实、执行体的闸门通过。
    pub fn ok() -> Self {
        Self {
            authz: AuthzVerdict::Authorized,
            facts: Vec::new(),
            error: None,
            reply: PortReply::default(),
        }
    }

    /// 失败（失败不是授权拒绝：错误类已经交代了原因）。
    pub fn failed(class: PortErrorClass, why: impl Into<String>) -> Self {
        Self {
            authz: AuthzVerdict::Authorized,
            facts: Vec::new(),
            error: Some((class, why.into())),
            reply: PortReply::default(),
        }
    }

    /// 执行体的闸门拒绝：不落任何事实 / 产物。
    pub fn denied(reason: impl Into<String>) -> Self {
        Self {
            authz: AuthzVerdict::denied(reason),
            facts: Vec::new(),
            error: None,
            reply: PortReply::default(),
        }
    }

    /// 覆盖授权结论（执行体自判后把结论放进结果）。
    pub fn with_authz(mut self, verdict: AuthzVerdict) -> Self {
        self.authz = verdict;
        self
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

/// 提供者行：本提供者申报的**每个端口**一行（`multi` 族，`active = false`，不参与
/// 表侧降位——端口级唯一由读数与装载期检查裁，D-299）。
///
/// 一个 provider 申报 N 个端口就产生 N 行（`DE-047` I-6.1）：端口身份是数据，行数随
/// 数据走；新增端口只改提供者的 `ports()` 返回值，不改注册面、不改族。
///
/// 端口名先过 [`check_port_name`]（fail-closed）；非法端口名在这里就注册失败。
pub fn port_rows(provider: &Arc<dyn PortProvider>) -> Vec<CapabilityRow> {
    provider
        .ports()
        .into_iter()
        .map(|port| {
            check_port_name(&port);
            CapabilityRow::new(
                PORT_FAMILY,
                port_row_name(&port, provider.provider_id()),
                PortPayload::new(provider.clone()),
                false,
            )
        })
        .collect()
}

/// 执行体交回**主结果值**的约定键：`facts` 里带这个键的项是调用方要的纯值，其余
/// 事实是端口插件自己声明的世界事实。（`D-440` ②：工具行执行体的结果经它交回，
/// 派发侧按名取出后继续自己的编排。）
pub const RESULT_KEY: &str = "result";

/// 工具世界的通用出口换算（`D-440` ③）：**工具结果本身**（任意 JSON）或
/// 工具错误 ⇒ [`PortOutcome`]。
///
/// 这是「信封 ↔ 结果」换算的唯一收口：执行体内部按工具自己的形状干活，出口
/// 一律经本函数装载——成功值放 `facts` 的 [`RESULT_KEY`] 项，失败走 `error`
/// （非重试类：工具错误是不是可重试，判据在编排侧的失败语义里，不在工具侧）。
/// 取代了旧 `capability_invoke` 的信封拆装位置。
pub fn port_result(result: Result<Value, symbio_kernel::plugin::PluginError>) -> PortOutcome {
    match result {
        Ok(value) => PortOutcome {
            authz: AuthzVerdict::Authorized,
            facts: vec![serde_json::json!({ RESULT_KEY: value })],
            error: None,
            reply: Default::default(),
        },
        Err(e) => PortOutcome::failed(PortErrorClass::NonRetryable, e.to_string()),
    }
}

/// 按端口名从收集表取执行体（`D-440`：端口集是装配期收集出来的，查表即得）。
///
/// 遍历 `PORT_FAMILY` 行、按提供者申报的端口名匹配——与宿主 `gather` 折执行体表
/// 是同一条读法，只是不要求调用方预折整张表：按名一次取一个。工具世界的派发步
/// （一笔 `port.tool` 债内的多次调用）按此逐名解析，行上不再持执行对象。
pub async fn port_executor_of(visitor: &dyn Registry, port: &str) -> Option<Arc<dyn PortExecutor>> {
    for row in visitor.list(PORT_FAMILY).await {
        let Some(payload) = row.payload::<PortPayload>() else {
            continue;
        };
        let provider = payload.provider();
        if provider.ports().iter().any(|p| p == port) {
            return provider.executor();
        }
    }
    None
}
