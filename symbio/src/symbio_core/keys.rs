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
// 流式工具的结果消息 id：工具据此以该 id 广播 StreamEvent::Update 增量帧，
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
