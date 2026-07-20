//! SymbioKey - 类型安全的键定义 (V3.0)

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
define_string_key!(ToolCallIdKey, TOOL_CALL_ID, "tool_call_id");
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
/// ⚠️ 保留为编译期占位；不再有任何运行期使用点。如需 payload 键，请走 `set_payload` / `payload()` 方法。
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

pub struct CapabilityManagerKey;
impl SymbioKey for CapabilityManagerKey {
    type Value = Arc<dyn crate::symbio_core::CapabilityManager>;
    fn name(&self) -> &'static str {
        "tool_manager"
    }
    fn parse(&self, _s: &str) -> Option<Self::Value> {
        None
    }
    fn format(&self, _v: &Self::Value) -> String {
        "capability_manager".to_string()
    }
}
pub const CAPABILITY_MANAGER: CapabilityManagerKey = CapabilityManagerKey;
