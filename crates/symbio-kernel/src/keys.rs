//! `SymbioKey` —— 类型安全的上下文键（内核机制面）。
//!
//! 键与值同住的规则和全部键实例留在宿主 `symbio_core::keys`；内核只装**机制**
//! （trait 本身），因为 `PluginInvokeRequest` / `ExecEnv` 等签名类型要按它取上下文。

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
    /// 与 `parse → None` 是**声明与结果**：`WIRE = false` ⇒ `parse` 必返 `None`。
    /// 反向不成立——`parse → None` 不必然是 `WIRE = false`。
    const WIRE: bool = true;

    /// 键的唯一名称（用于在 Map 中查找）
    fn name(&self) -> &'static str;

    /// 从字符串解析值 (用于从请求头等字符串存储中恢复)
    fn parse(&self, s: &str) -> Option<Self::Value>;

    /// 将值格式化为字符串 (用于存储到请求头等字符串存储中)
    fn format(&self, v: &Self::Value) -> String;
}

// 初始配置 Key (Value)。插件上下文的 `config` 是内核契约的一部分（`PluginSimpleRequest`
// 与 `PluginInvokeRequestExt` 都要取它），故随 trait 住内核；其余键实例留宿主。

/// 初始配置键。
pub struct ConfigKey;
impl SymbioKey for ConfigKey {
    type Value = serde_json::Value;
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
/// 初始配置键的唯一实例。
pub const CONFIG: ConfigKey = ConfigKey;
