//! 收集管线：三条收集**通道**的键。
//!
//! ## 为什么三条通道而不是一张表
//!
//! 能力 = 可调用对象；选项 = 可展示的数据节点；可配置 = 「我有配置文档」这一句话。
//! 4e 之后三者的**行**住同一张表（[`Registry`]），但**通道**仍是独立的三条：
//! 广播面（`Plugin::traverse(TRAVERSE_AVAILABLE_TOOLS / _OPTIONS)`）与收集器实例
//! 各自一对——键没变，变的是行住哪张表。
//!
//! ## 键与值同住（D-095 根因一）
//!
//! 三条键的值类型都是 `Arc<dyn Registry>`（本域契约），故键住本域；`symbio_sdk::keys`
//! 只留机制与通用词汇键。键名（`tool_visitor` / `option_visitor` /
//! `configurable_visitor`）是 `ctx` 上的外部协议面，**不随搬迁而变**。
//!
//! 读侧不再设统一的「工具清单助手」：工具清单读点各自直走
//! `Registry::list(family::TOOL)`（LLM 工具面 / 请求级开销估算 / 闸门声明位 /
//! 线上名解析），口径即 `Registry` 本身，无需第二层包装（D-086 §7 的上移裁决
//! 在 D-440 终态下收敛为「读点直连」，helper 于 `60cedd50` 后零调用，正式删除）。
//!
//! 顺序 = 注册顺序（D-057），调用方不得重排。

use std::sync::Arc;

use symbio_kernel::keys::SymbioKey;

/// 能力收集器 Key —— 第一条收集**通道**（能力 = 可调用对象）
///
/// 写侧是全体插件（`traverse(TRAVERSE_AVAILABLE_TOOLS)` 广播时容器把收集器实例
/// 挂上 ctx）；读侧读点直走 `Registry::list(family::TOOL)`。
pub struct CapabilityVisitorKey;
impl SymbioKey for CapabilityVisitorKey {
    type Value = Arc<dyn symbio_kernel::capability::Registry>;
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

/// 选项收集器 Key —— 与 [`CAPABILITY_VISITOR`] 平行的第二条收集**通道**
/// （选项 = 可展示的数据节点，见 `symbio_sdk::capability::option`）。
///
/// 4e 之后值类型是统一的 [`symbio_kernel::capability::Registry`]：选项是同一张表里的行，
/// 通道（广播与收集器实例）仍是独立的一条——键没变，变的是行住哪张表。
pub struct OptionVisitorKey;
impl SymbioKey for OptionVisitorKey {
    type Value = Arc<dyn symbio_kernel::capability::Registry>;
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

/// 可配置声明收集器 Key —— 第三条收集**通道**
/// （可配置 = 「我有配置文档」这一句话，见 `symbio_sdk::capability::configurable`）。
/// 4e 之后值类型是统一的 [`symbio_kernel::capability::Registry`]。
pub struct ConfigurableVisitorKey;
impl SymbioKey for ConfigurableVisitorKey {
    type Value = Arc<dyn symbio_kernel::capability::Registry>;
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
pub const CONFIGURABLE_VISITOR: ConfigurableVisitorKey = ConfigurableVisitorKey;
