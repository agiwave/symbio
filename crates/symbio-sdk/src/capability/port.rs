//! 世界端口的收集面：端口名校验、装配期的收集容器，以及提供者 / 执行体的通用契约
//! （`DE-047` I-6，D-445 / D-468）。
//!
//! ## 端口集是装配期的收集结果，不是能力行（D-468）
//!
//! 端口由活插件在自己的 `traverse` 里申报（[`register_ports`]），写进本次装配的
//! [`PortBindings`] 容器；容器挂在 ctx 上（[`PORT_BINDINGS`]），装配方读快照折执行体表。
//! 它**不借能力表的行存在**：端口名不是表键，于是行名编码、`<port:…>` 前缀，以及
//! 「端口名与同名工具互相顶掉」这一族隐含约束一起消失。
//!
//! 端口名仍是**数据**：本文件不出现 `ModelProvider` / `ToolProvider` 一类按世界命名的
//! trait，执行面只有通用的 [`PortExecutor`] / [`PortInput`] / [`PortOutcome`]（I-6.2）。
//! `PortDecl`、派发 / 等待两行模板、`Call{port,id,attempt,via}` 的幂等键与记账住 `loop`。
//!
//! ## 「停用即缺席」靠同一个前提
//!
//! 容器每次装配新建（宿主与会话各建各的）：停用的插件不 traverse、不登记，旧条目不会
//! 残留。**提供者不静态注册**：`submit_*!` 一类的编译期收集器在端口面没有位置——它们
//! 链接期生效、没有启用过滤，插件停用后条目仍在（实例不构造不等于链接期条目消失），
//! 「停用即缺席」当场判红。
//!
//! ## 判重与缺席
//!
//! - **未登记 = 缺席**（合法装配态：没装提供者插件）。响亮失败发生在调用点。
//! - **同名再登记 = 冲突**：该键从此**永久**不可执行，两方提供者都在说明里被指名，
//!   装配方读快照时整份失败。不静默顶掉、不先到者胜、也不靠第二次读表才看得见。
//! - **端口集开放**：任何登记过的端口名都合法；加一个端口只改提供者的 `ports()`。

use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::capability::capability_report_error;
use crate::plugin::effects::FactDraft;
use symbio_kernel::exec::{ExecAbortSignal, ExecEventSink};
use symbio_kernel::keys::SymbioKey;
use symbio_kernel::plugin::{PluginInvokeRequest, PluginInvokeRequestExt};

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

/// 一条有效绑定：提供者 id + 执行体。
type Binding = (String, Arc<dyn PortExecutor>);
/// 一个端口名的条目：有效绑定，或一行冲突说明（键永久不可执行）。
type Entry = Result<Binding, String>;
/// 一次装配收集到的全部条目。
type Entries = BTreeMap<String, Entry>;

/// 一次装配的端口收集容器（D-468）。
///
/// 条目是 `Ok((提供者 id, 执行体))` 或 `Err(冲突说明)`：一条被拒的键**不会**被后来的
/// 登记救回来——同一端口两个 owner 是装配错误，不是覆盖问题。
///
/// 只此一格数据（CS-009）：绑定用元组，不为「提供者 id + 执行体」再造一个公开结构。
#[derive(Default)]
pub struct PortBindings {
    entries: Mutex<Entries>,
}

impl PortBindings {
    /// 按名取绑定：未申报 `Ok(None)`，冲突 `Err(说明)`。
    pub fn get(&self, port: &str) -> Result<Option<Binding>, String> {
        let entries = self.entries();
        match entries.get(port) {
            None => Ok(None),
            Some(Ok(binding)) => Ok(Some(binding.clone())),
            Some(Err(error)) => Err(error.clone()),
        }
    }

    /// 收集结束后的有效绑定快照：任一冲突都让**整份**快照失败（装配期硬红）。
    ///
    /// 快照固定本次装配结果；下一次装配必须新建容器，复用等于把旧绑定带进来。
    pub fn snapshot(&self) -> Result<BTreeMap<String, Binding>, String> {
        self.entries()
            .iter()
            .map(|(port, entry)| match entry {
                Ok(binding) => Ok((port.clone(), binding.clone())),
                Err(error) => Err(error.clone()),
            })
            .collect()
    }

    /// 有效绑定的端口名（不含冲突键）：消费方「清单 = 显式暴露 ∩ 有效绑定」用。
    pub fn bound(&self) -> Vec<String> {
        self.entries()
            .iter()
            .filter(|(_, entry)| entry.is_ok())
            .map(|(port, _)| port.clone())
            .collect()
    }

    fn entries(&self) -> std::sync::MutexGuard<'_, Entries> {
        self.entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 登记一组端口；返回要让调用方落进错误桶的说明（冲突 / 无执行体）。
    ///
    /// 同名再登记**永久**把该键转成不可执行：先登记的那一份不再"胜出"——两份都可能被
    /// 派发到，挑一个就是让描述与执行漂移。
    ///
    /// 公开的是「往容器里写」这一个动作：插件的端口集走 [`register_ports`]，工具端口
    /// 走 `register_tool`（它要同时写工具行），两条都落在本方法上。
    pub fn bind(
        &self,
        ports: &[String],
        provider_id: &str,
        executor: Option<Arc<dyn PortExecutor>>,
    ) -> Vec<String> {
        let mut entries = self.entries();
        let mut errors = Vec::new();
        for port in ports {
            match entries.get(port) {
                Some(existing) => {
                    // 先前那次已经落错的键不再被当成提供者名读：错误里保留原来的说明，
                    // 并把本次登记方指名——两方都在，键仍然永久不可执行。
                    let error = match existing {
                        Ok((previous, _)) => format!(
                            "世界端口 `{port}` 重复登记：提供者 [{previous:?}, {provider_id:?}]；该端口不可执行"
                        ),
                        Err(previous) => format!(
                            "世界端口 `{port}` 重复登记：提供者 `{provider_id}` 与先前的登记冲突（{previous}）；该端口不可执行"
                        ),
                    };
                    entries.insert(port.clone(), Err(error.clone()));
                    errors.push(error);
                }
                None => match executor.clone() {
                    Some(executor) => {
                        entries.insert(port.clone(), Ok((provider_id.to_string(), executor)));
                    }
                    None => {
                        let error = format!(
                            "世界端口 `{port}` 的提供者 `{provider_id}` 没有执行体，登记失败"
                        );
                        entries.insert(port.clone(), Err(error.clone()));
                        errors.push(error);
                    }
                },
            }
        }
        errors
    }
}

/// 端口收集容器的 ctx 槽（D-468）：一次装配一个实例，装配方读快照折执行体表。
pub struct PortBindingsKey;
impl SymbioKey for PortBindingsKey {
    type Value = Arc<PortBindings>;
    const WIRE: bool = false;
    fn name(&self) -> &'static str {
        "port_bindings"
    }
    fn parse(&self, _s: &str) -> Option<Self::Value> {
        None
    }
    fn format(&self, _v: &Self::Value) -> String {
        "port_bindings".to_string()
    }
}
pub const PORT_BINDINGS: PortBindingsKey = PortBindingsKey;

/// 作用域前缀槽（D-468）：子 Agent 的作用域给每个端口名加同一个前缀。
///
/// **端口名与工具名同源**：作用域里工具行被改名为 `agent_<id>_<name>`，端口名必须用
/// 同一个前缀——否则派发按前缀名查不到绑定。前缀为空（系统树）⇒ 端口名原样。
pub struct PortScopeKey;
impl SymbioKey for PortScopeKey {
    type Value = String;
    const WIRE: bool = false;
    fn name(&self) -> &'static str {
        "port_scope"
    }
    fn parse(&self, _s: &str) -> Option<Self::Value> {
        None
    }
    fn format(&self, _v: &Self::Value) -> String {
        "port_scope".to_string()
    }
}
pub const PORT_SCOPE: PortScopeKey = PortScopeKey;

/// 把端口名交出去之前套上当前作用域的前缀（未设前缀 ⇒ 原样）。
pub fn scoped_port_name(ctx: &Arc<dyn PluginInvokeRequest>, port: &str) -> String {
    format!("{}{port}", ctx.get(PORT_SCOPE).unwrap_or_default())
}

/// 活插件在 `traverse` 里申报自己的端口（D-468）。
///
/// 容器缺席（夹具 / 内部请求没有装收集）时静默跳过：没有容器就没有端口集，那不是
/// 装配错误，与 `CAPABILITY_VISITOR` 缺席同一条口径。登记错误写进既有能力错误桶，
/// 与收集期错误同桶同路（装配方裁决即中止）。
pub async fn register_ports(ctx: &Arc<dyn PluginInvokeRequest>, provider: &dyn PortProvider) {
    let Some(bindings) = ctx.get(PORT_BINDINGS) else {
        return;
    };
    let ports: Vec<String> = provider
        .ports()
        .into_iter()
        .map(|port| {
            // 端口名先进事件 id：非法名在这里 fail-closed（D-336）。
            let name = scoped_port_name(ctx, &port);
            check_port_name(&name);
            name
        })
        .collect();
    let errors = bindings.bind(&ports, provider.provider_id(), provider.executor());
    for error in errors {
        capability_report_error(ctx, provider.provider_id(), error).await;
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
/// **不读其中的世界字段**——消息怎么组装是执行体的私事（D-319 / D-446 ②）。
///
/// 通用端口不认识模型协议：`messages` 是**不透明值**，不是模型侧的消息类型或任何
/// 角色 / 调用 id 形状。结果先成为事实，模型插件自己把事实投影成模型消息；
/// 环与宿主只传递这一格，不解释、不反序列化。
#[derive(Debug, Clone, Default)]
pub struct PortReply {
    /// 应答内容（不透明值；环只传递，不解释内容）。
    pub messages: Vec<Value>,
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

/// 端口执行体的**缺上下文守卫**（`D-493`）：没有调用方上下文 ⇒ 非重试失败。
///
/// 这条判断此前在 17 个工具端口的 `impl PortExecutor::execute` 里各抄一份，连失败文案
/// 一起——改一句文案要改 17 处，漏一处就出现两种说法。它和 [`port_result`] 同址，因为
/// 两者都是「执行体入口 / 出口」的约定，不是某个工具的知识。
pub fn missing_caller() -> PortOutcome {
    PortOutcome::failed(
        PortErrorClass::NonRetryable,
        "工具端口没有调用方上下文，无法拆信封".to_string(),
    )
}
