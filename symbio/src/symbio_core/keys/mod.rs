//! SymbioKey - 类型安全的键定义
//!
//! ## 命名规则（`symbio_core/README.md` §3 豁免表的判据）
//!
//! 本域里两种形态**互不重叠**，所以裸名不是「风格不统一」，而是一条有判别力的规则：
//!
//! | 形态 | 写成 | 例 |
//! |---|---|---|
//! | 字符串常量 —— **名字**（跨进程 / 跨文件的字面量） | **带前缀** | `PLUGIN_SESSION` · `KEY_PAYLOAD`（本文件）· `VDFS_ROOT`（`paths`）· `EMBEDDING_LOCAL`（`ids`） |
//! | `SymbioKey` **实例** —— **键对象**（`ctx.get(&…)` 的凭据） | **裸名** | `PATH` · `ID` · `CAPABILITY_VISITOR` · `PARENT` |
//!
//! 消费形态也不同：字符串是**值**（`name == KEY_PAYLOAD`），实例是**取值的凭据**。
//! 新增时照上表选形态——实例用裸名（类型名已带 `Key` 后缀，常量再加前缀会读成
//! 「键的键」：`ctx.get(&KEY_PATH)`），字面量带前缀。

mod ids;
mod paths;

// 域内子模块私有，公开面在此显式重导出（逐符号列出，可在一屏读完域的对外面）
pub use ids::{
    EMBEDDING_LOCAL, EMBEDDING_NOOP, PLUGIN_AGENT, PLUGIN_COMPOSITE, PLUGIN_EVENT_BUS,
    PLUGIN_GATEWAY, PLUGIN_HOME, PLUGIN_HOOK, PLUGIN_LOCAL, PLUGIN_MANAGER, PLUGIN_MCP,
    PLUGIN_MODEL, PLUGIN_SESSION, PLUGIN_SKILL, PLUGIN_TELEGRAM, PLUGIN_VDFS, PLUGIN_WEB,
    PLUGIN_WORK,
};
pub use paths::{
    EVENT_BUS_SUBSCRIBE, HOOK_FIRE, SESSION_CHAT_ABORT, SESSION_CHAT_SEND, VDFS_ROOT, VDFS_UNWATCH,
    VDFS_WATCH,
};

use serde_json::Value;
use std::sync::Arc;

/// 项目全局通用的 Key 特征
/// 支持类型安全的值获取与设置
pub trait SymbioKey {
    /// 该键对应的值类型
    type Value: Clone + Send + Sync + 'static;

    /// 本键能否在**进程外**表达（跨进程可传输）
    ///
    /// - `true`（缺省）：值可序列化为字符串或 JSON，可进线上 `metadata`；
    /// - `false`：进程内专用对象（trait object / 闭包 / 弱引用），无字符串形态。
    ///
    /// ## 为什么需要它
    ///
    /// 从前「这个键能不能跨进程」只能靠 `parse` 是否返回 `None` **倒推**，
    /// 且那是个约定、不是可枚举的事实——要判断一次调用能否送到进程外插件，
    /// 得逐个键去读注释。第三方插件体系的**准入判据**必须可机检，故把它提成
    /// 类型事实：`SymbioKey::WIRE` 可被静态读取、可被守卫脚本核对。
    ///
    /// ## 与 `parse → None` 的关系
    ///
    /// 两者是**声明与结果**：`WIRE = false` ⇒ `parse` 必返 `None`（无字符串形态）。
    /// 反向不成立——`parse → None` 不必然是 `WIRE = false`（如 `CONFIG` /
    /// `PLUGIN_DIR` 的值可以序列化，只是不做「从字符串恢复」这件事）。
    const WIRE: bool = true;

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

// ==================== 载荷键 ====================
//
// `payload` 是**事实上的核心键**：`PluginInvokeRequestExt::payload` / `set_payload`
// 读写的就是它。但它**不是一个 `SymbioKey`**——它的值类型由调用方决定
// （`payload::<T>()` 的 `T`），而 `SymbioKey` 要求一个固定的关联 `Value` 类型。
//
// 历史上这里有一个 `PayloadKey`（`Value` 型）+ `PAYLOAD` 常量，标着
// `#[deprecated]`（指向 `ctx.payload::<T>()`）。那是一个**名不副实的标注**：
// 该废弃的是「用 `PAYLOAD` 键直接存取 `Value` 这种用法」，不是 `payload`
// 这个概念——后者恰恰是核心路径。于是它成了一个「废弃但仍被需要」的悬置
// 常量，而真正的读写方（`plugin.rs` / `gateway`）为了规避告警，只好写**裸
// 字符串** `"payload"`——正是 `grep-audit` S-009 想拦的形态。
//
// 现在收口为一个纯字符串常量：键名只有这一处定义，读写双方都引它。

/// 载荷键名（`PluginInvokeRequestExt::payload` / `set_payload` 读写的桶）
pub const KEY_PAYLOAD: &str = "payload";

// 父插件弱引用 Key (Option<Weak<dyn Plugin>>)
pub struct ParentKey;
impl SymbioKey for ParentKey {
    type Value = Option<std::sync::Weak<dyn crate::symbio_core::Plugin>>;
    /// 进程内专用：`Weak<dyn Plugin>` 是进程内引用，无字符串 / JSON 形态
    const WIRE: bool = false;
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

// 注：插件**装配策略**常量（子树清单、不可停用清单）不在本域——它们不是上下文键，
// 已迁到 `symbio_core::assembly`（见该模块文档的判据说明）。

pub struct CapabilityVisitorKey;
impl SymbioKey for CapabilityVisitorKey {
    type Value = Arc<dyn crate::symbio_core::CapabilityVisitor>;
    /// 进程内专用：trait object，无字符串 / JSON 形态
    const WIRE: bool = false;
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
/// （能力 = 可调用对象；选项 = 可展示的数据节点，见 `symbio_core::capability::option`）
pub struct OptionVisitorKey;
impl SymbioKey for OptionVisitorKey {
    type Value = Arc<dyn crate::symbio_core::OptionVisitor>;
    /// 进程内专用：trait object，无字符串 / JSON 形态
    const WIRE: bool = false;
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
/// 见 `symbio_core::capability::configurable`）
pub struct ConfigurableVisitorKey;
impl SymbioKey for ConfigurableVisitorKey {
    type Value = Arc<dyn crate::symbio_core::ConfigurableVisitor>;
    /// 进程内专用：trait object，无字符串 / JSON 形态
    const WIRE: bool = false;
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
pub struct ExecEventSinkKey;
impl SymbioKey for ExecEventSinkKey {
    type Value = crate::symbio_core::ExecEventSink;
    /// 进程内专用：闭包，无字符串 / JSON 形态
    const WIRE: bool = false;
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
pub const EVENT_SINK: ExecEventSinkKey = ExecEventSinkKey;

/// 执行期**中止信号** Key —— 与 [`EVENT_SINK`] 成对。
///
/// 写入者是编排层（发起工具调用前）；读取者是工具实现体。缺席 ⇒ 得到一个
/// **永不中止**的独立信号（`ExecAbortSignal::new()`）：直接 `route()` 调用没有编排层，
/// 也就没有中止来源，这是诚实表达而非兜底。
pub struct ExecAbortSignalKey;
impl SymbioKey for ExecAbortSignalKey {
    type Value = crate::symbio_core::ExecAbortSignal;
    /// 进程内专用：`CancellationToken`，无字符串 / JSON 形态
    const WIRE: bool = false;
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
pub const ABORT_SIGNAL: ExecAbortSignalKey = ExecAbortSignalKey;
