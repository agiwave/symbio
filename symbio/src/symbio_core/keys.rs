//! SymbioKey - 类型安全的键定义

use serde_json::Value;
use std::sync::Arc;

/// 项目全局通用的 Key 特征
/// 支持类型安全的值获取与设置
pub trait SymbioKey {
    /// 该键对应的值类型
    type Value: Clone + Send + Sync + 'static;

    /// 键的唯一名称（用于在 Map 中查找）
    fn name(&self) -> &'static str;

    /// 从字符串解析值 (用于从请求头等字符串存储中恢复)
    fn parse(&self, s: &str) -> Option<Self::Value>;

    /// 将值格式化为字符串 (用于存储到请求头等字符串存储中)
    fn format(&self, v: &Self::Value) -> String;
}

macro_rules! define_string_key {
    ($struct_name:ident, $const_name:ident, $key_name:expr) => {
        pub struct $struct_name;
        impl SymbioKey for $struct_name {
            type Value = String;
            fn name(&self) -> &'static str {
                $key_name
            }
            fn parse(&self, s: &str) -> Option<Self::Value> {
                Some(s.to_string())
            }
            fn format(&self, v: &Self::Value) -> String {
                v.clone()
            }
        }
        pub const $const_name: $struct_name = $struct_name;
    };
}

define_string_key!(PathKey, PATH, "path");
define_string_key!(WorkdirKey, WORKDIR, "workdir");
define_string_key!(AgentIdKey, AGENT_ID, "agent_id");
define_string_key!(SessionIdKey, SESSION_ID, "session_id");
define_string_key!(TraceIdKey, TRACE_ID, "trace_id");
// 当前父地址：本插件在地址空间中挂载点的绝对地址。与 `WORKDIR` / `SESSION_ID`
// 同类的**上下文数据**：父插件把请求转发给子插件时（route / traverse）改写它，
// 子插件在少数需要协议级绝对地址的场合读它拼接（见 `symbio_core::vdfs::address`）。
// 缺省（顶层请求 / 无 vdfs 装配）为空。
define_string_key!(VdfsParentAddrKey, VDFS_PARENT_ADDR, "vdfs_parent_addr");
define_string_key!(ToolCallIdKey, TOOL_CALL_ID, "tool_call_id");
// 流式工具的结果消息 id：工具据此以该 id 广播消息帧（增量走 `delta`），
// 最终帧由 tool_executor 捕获为工具结果（与 result_msg_id 占位节点同 id 合并）。
define_string_key!(ResultMsgIdKey, RESULT_MSG_ID, "result_msg_id");
// 会话运行模式：auto（无人值守，失败不弹交互）| interactive（人在环，失败可交互，但 confirm/ask_user 仍不弹框）
define_string_key!(ModeKey, MODE, "mode");
// 会话选定的 Model Provider ID（与 agent_id 同级别：随 chat_send 传输 + session.metadata 持久化）
define_string_key!(ProviderIdKey, PROVIDER_ID, "provider_id");
// 会话执行风险等级阈值：low / medium / high（与 agent_id 同级别：随 chat_send 传输 + session.metadata 持久化）
define_string_key!(RiskLevelKey, RISK_LEVEL, "risk_level");

// 常用业务属性 Key
define_string_key!(IdKey, ID, "id");
define_string_key!(NameKey, NAME, "name");
define_string_key!(KindKey, KIND, "kind");
define_string_key!(ScopeKey, SCOPE, "scope");
define_string_key!(ContentKey, CONTENT, "content");
define_string_key!(DescriptionKey, DESCRIPTION, "description");

// 消息载荷 Key (JSON Value)
#[deprecated(
    since = "3.1.0",
    note = "请使用 ctx.payload::<T>() 或 ctx.set_payload() 代替，以保障编译期强类型安全"
)]
pub struct PayloadKey;

#[allow(deprecated)]
impl SymbioKey for PayloadKey {
    type Value = Value;
    fn name(&self) -> &'static str {
        "payload"
    }
    fn parse(&self, _s: &str) -> Option<Self::Value> {
        None // Payload 通常不从字符串解析
    }
    fn format(&self, v: &Self::Value) -> String {
        v.to_string()
    }
}

#[allow(deprecated)]
#[deprecated(
    since = "3.1.0",
    note = "请使用 ctx.payload::<T>() 或 ctx.set_payload() 代替，以保障编译期强类型安全"
)]
/// ⚠️ 仅作编译期占位；无任何运行期使用点。如需 payload 键，请走 `set_payload` / `payload()` 方法。
pub const PAYLOAD: PayloadKey = PayloadKey;

// 父插件弱引用 Key (Option<Weak<dyn Plugin>>)
pub struct ParentKey;
impl SymbioKey for ParentKey {
    type Value = Option<std::sync::Weak<dyn crate::symbio_core::Plugin>>;
    fn name(&self) -> &'static str {
        "parent"
    }
    fn parse(&self, _s: &str) -> Option<Self::Value> {
        None
    }
    fn format(&self, _v: &Self::Value) -> String {
        "weak_parent".to_string()
    }
}
pub const PARENT: ParentKey = ParentKey;

// 初始配置 Key (Value)
pub struct ConfigKey;
impl SymbioKey for ConfigKey {
    type Value = Value;
    fn name(&self) -> &'static str {
        "config"
    }
    fn parse(&self, _s: &str) -> Option<Self::Value> {
        None
    }
    fn format(&self, v: &Self::Value) -> String {
        v.to_string()
    }
}
pub const CONFIG: ConfigKey = ConfigKey;

// 插件自身目录 Key（`PluginDir`）
//
// 容器构造子插件时把它挂到子上下文上——插件据此知道「我的目录在哪」，
// 从而自己读写 `PLUGIN.yml`（父插件不再代管任何配置）。
pub struct PluginDirKey;
impl SymbioKey for PluginDirKey {
    type Value = crate::symbio_core::PluginDir;
    fn name(&self) -> &'static str {
        "plugin_dir"
    }
    fn parse(&self, _s: &str) -> Option<Self::Value> {
        None
    }
    fn format(&self, v: &Self::Value) -> String {
        v.dir().to_string_lossy().to_string()
    }
}
pub const PLUGIN_DIR: PluginDirKey = PluginDirKey;

/// 必需插件清单 Key —— **构造者**告诉容器「哪些插件即使没有目录也要补出来」
///
/// 容器是通用容器（可以嵌套另一个容器），因此它**不内置**任何插件清单：
/// 清单是构造者的策略，经本键随构造一起传入。缺省（未传）= 空清单 = 纯扫描。
pub struct RequiredPluginsKey;
impl SymbioKey for RequiredPluginsKey {
    type Value = Vec<String>;
    fn name(&self) -> &'static str {
        "required_plugins"
    }
    fn parse(&self, s: &str) -> Option<Self::Value> {
        // 逗号分隔（与其它字符串键在请求头等字符串载体中的表示一致）
        Some(
            s.split(',')
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .map(str::to_string)
                .collect(),
        )
    }
    fn format(&self, v: &Self::Value) -> String {
        v.join(",")
    }
}
pub const REQUIRED_PLUGINS: RequiredPluginsKey = RequiredPluginsKey;

/// 根（系统）Agent 挂载的**完整**插件清单 —— 父子的唯一真相源（机制级常量）。
///
/// 子 Agent 子树复用 [`SUB_AGENT_PLUGINS`]（本清单只多一个系统级单槽 `vdfs`）。
/// 两处都集中在 `symbio_core`，改一处即父子一致，杜绝「两套清单」漂移。
pub const SYSTEM_AGENT_PLUGINS: &[&str] = &[
    "plugin_manager",
    "event_bus",
    "model",
    "session",
    "local",
    "web",
    "mcp",
    "telegram",
    "hook",
    "agent",
    "skill",
    "gateway",
    "vdfs",
    "work",
];

/// 子 Agent 子树挂载的「默认插件」清单 —— 与父（系统）Agent **同构**的机制级常量。
///
/// 子 Agent 是一棵 composite 插件树（与系统 Agent 同构，agent-directory-spec §1.1），
/// 构造时经 ctx 键 [`REQUIRED_PLUGINS`] 告知容器「必须挂哪些插件」。这里集中声明清单，
/// 作为 `symbio_core` 的唯一真相源；[`SYSTEM_AGENT_PLUGINS`] 直接复用其超集，
/// 改一处即父子一致。
///
/// ## 与 [`SYSTEM_AGENT_PLUGINS`] 的关系：只差一个系统级单槽
///
/// 本清单 = 系统完整清单去掉 `vdfs`：
///
/// - `vdfs`（VDFS 根）是**单槽**注册，归系统 Agent 独占。
///   子树里的对应注册经 [`crate::plugins::agent::host::scope::SubAgentVisitor`]
///   丢弃（见其模块文档）；若在此列出，只会构造出无挂载点的空实例——既不生效、
///   又徒增开销。故子树不重复挂。
/// - `model` **在列**：子智能体有自己的模型服务——子树会话收集能力时以**子容器**
///   为 parent（`collect_capabilities(sub_composite, …)`），子树 `model` 实例
///   注册进**该次收集自己的**管理器，因此子会话用子智能体自己解析的模型。
///   父（系统）会话收集期，子树的 `model` 注册才经 `SubAgentVisitor` **丢弃**
///   （单槽，防子树模型劫持父会话——见 scope 模块文档）。
/// - 其余插件（含 `agent` 自身、`plugin_manager`、`work`）都在列：子树因此与父树**结构相同**，
///   前端看到的资源入口（含设置入口）与父 Agent 对齐。
///
/// ## 分形：任意层级复用同一常量
///
/// 本常量被 `plugins/agent` 的 `sub_agent` 用于构造**每一棵**子树。若某天子 Agent
/// 也能在其目录内再挂子 Agent（`<id>/agent/<sub-id>` 递归），同一常量 + 同一套构造
/// 逻辑自动套用——不存在「支持子 Agent 却不支持子 Agent 的子 Agent」的特例：任何一层
/// 都走同一条机制，且都同样只跳过 `vdfs` 单槽（单槽归系统 Agent，由
/// `SubAgentVisitor` 在每一层丢弃）。
pub const SUB_AGENT_PLUGINS: &[&str] = &[
    "plugin_manager", // 插件管理与配置入口（子 Agent 页同样需要）
    "event_bus",      // 事件总线
    "session",        // 会话
    "model",          // 模型服务（子智能体自己的模型；子树会话自行解析）
    "local",          // 本地文件
    "web",            // 网络访问
    "mcp",            // 工具
    "telegram",       // 消息渠道
    "hook",           // 钩子
    "agent",          // 智能体（含子子 Agent —— 分形）
    "skill",          // 技能
    "gateway",        // 外部 API 网关
    "vdfs",           // VDFS 根（单槽归系统 Agent）
    "work",           // 工作区记忆（WORKDIR 继承父会话，不再双重注入）
];

/// **不可停用的插件** —— 它们是**界面自身的底座**，不是普通功能。
///
/// 停用一个普通插件（如 `telegram`）少的是一个功能；停用这里的任何一个，少的是
/// **整个界面**：前端所有资源页都经 `vdfs` 取数，插件清单与启停按钮都长在
/// `plugin_manager` 的页面上。于是用户会看到一个再也点不到「启用」的界面，
/// 只能去磁盘上改 `PLUGIN.yml`——那不是权限设计，是自断其路。
///
/// 因此它是一条**机制级**判据（与 [`SYSTEM_AGENT_PLUGINS`] 同处）：装配方（容器）
/// 在执行停用前查它，而不是让每个插件自己声明「我不能被关」。插件的启停状态是
/// 装配方的事（见 [`crate::symbio_core::KEY_ENABLED`]），这条规则也该住在同一处。
pub const UNDISABLABLE_PLUGINS: &[&str] = &[
    crate::symbio_core::PLUGIN_MANAGER, // 插件管理入口：停用它就再也点不到「启用」
    crate::symbio_core::PLUGIN_VDFS,    // 资源访问层：停用它整棵资源树都取不到
];

pub struct CapabilityVisitorKey;
impl SymbioKey for CapabilityVisitorKey {
    type Value = Arc<dyn crate::symbio_core::CapabilityVisitor>;
    fn name(&self) -> &'static str {
        "tool_visitor"
    }
    fn parse(&self, _s: &str) -> Option<Self::Value> {
        None
    }
    fn format(&self, _v: &Self::Value) -> String {
        "capability_manager".to_string()
    }
}
pub const CAPABILITY_VISITOR: CapabilityVisitorKey = CapabilityVisitorKey;

/// 选项收集器 Key —— 与 [`CAPABILITY_VISITOR`] 平行的第二条收集通道
/// （能力 = 可调用对象；选项 = 可展示的数据节点，见 `symbio_core::option`）
pub struct OptionVisitorKey;
impl SymbioKey for OptionVisitorKey {
    type Value = Arc<dyn crate::symbio_core::OptionVisitor>;
    fn name(&self) -> &'static str {
        "option_visitor"
    }
    fn parse(&self, _s: &str) -> Option<Self::Value> {
        None
    }
    fn format(&self, _v: &Self::Value) -> String {
        "option_visitor".to_string()
    }
}
pub const OPTION_VISITOR: OptionVisitorKey = OptionVisitorKey;

/// 可配置声明收集器 Key —— 第三条收集通道
/// （能力 = 可调用对象；选项 = 可展示的数据节点；可配置 = 「我有配置文档」这一句话，
/// 见 `symbio_core::configurable`）
pub struct ConfigurableVisitorKey;
impl SymbioKey for ConfigurableVisitorKey {
    type Value = Arc<dyn crate::symbio_core::ConfigurableVisitor>;
    fn name(&self) -> &'static str {
        "configurable_visitor"
    }
    fn parse(&self, _s: &str) -> Option<Self::Value> {
        None
    }
    fn format(&self, _v: &Self::Value) -> String {
        "configurable_visitor".to_string()
    }
}
pub const CONFIG_VISITOR: ConfigurableVisitorKey = ConfigurableVisitorKey;

/// 执行期**事件出口** Key —— 第四条进程内通道
/// （能力 = 可调用对象；选项 = 可展示的数据节点；可配置 = 「我有配置文档」；
/// 出口 = 「本次调用的可见事件往这里写」，见 `symbio_core::exec`）。
///
/// ## 为什么走 `ctx` 而不是给 `Capability::execute` 加参数
///
/// `Capability::execute(ctx)` 的入参是**请求信封**（`PATH` / `trace_id` / `payload`
/// / 会话上下文），不是执行期上下文。出口与中止信号是**执行期**的，与「这次调用
/// 从哪条路径来」无关：同一个 `shell` 能力既可能被会话编排层调用（有出口），
/// 也可能被 `route()` 直接调用（无出口 ⇒ 静默）。用 `ctx` 承载，缺席即静默，
/// **不必为「有没有出口」造第二条调用路径**。
///
/// ## 为什么 `parse → None`
///
/// 与 [`CAPABILITY_VISITOR`] / [`OPTION_VISITOR`] / [`CONFIG_VISITOR`] 同款：
/// 进程内专用，没有字符串形态。`parse` 返回 `None` 是**刻意的**——它声明
/// 「本键不参与任何跨进程/字符串化的往返」，而不是「尚未实现」。
pub struct EventSinkKey;
impl SymbioKey for EventSinkKey {
    type Value = crate::symbio_core::EventSink;
    fn name(&self) -> &'static str {
        "event_sink"
    }
    fn parse(&self, _s: &str) -> Option<Self::Value> {
        None
    }
    fn format(&self, _v: &Self::Value) -> String {
        "event_sink".to_string()
    }
}
pub const EVENT_SINK: EventSinkKey = EventSinkKey;

/// 执行期**中止信号** Key —— 与 [`EVENT_SINK`] 成对。
///
/// 写入者是编排层（发起工具调用前）；读取者是工具实现体。缺席 ⇒ 得到一个
/// **永不中止**的独立信号（`AbortSignal::new()`）：直接 `route()` 调用没有编排层，
/// 也就没有中止来源，这是诚实表达而非兜底。
pub struct AbortSignalKey;
impl SymbioKey for AbortSignalKey {
    type Value = crate::symbio_core::AbortSignal;
    fn name(&self) -> &'static str {
        "abort_signal"
    }
    fn parse(&self, _s: &str) -> Option<Self::Value> {
        None
    }
    fn format(&self, _v: &Self::Value) -> String {
        "abort_signal".to_string()
    }
}
pub const ABORT_SIGNAL: AbortSignalKey = AbortSignalKey;
