//! 工具名的**线上形态**（wire form）——内部端口身份与 function-calling 协议字符集
//! 之间唯一的一次换算处。
//!
//! ## 为什么只留下出方向这一半
//!
//! 能力名是**框架内的地址**（可以带 `.`，MCP 工具是 `mcp.<server>.<tool>`）；
//! function-calling 协议对工具名的字符集有硬约束（OpenAI / Anthropic 的交集是
//! `[A-Za-z0-9_-]`）。两者之间需要一次换算，但换算**只在装配期发生一次**：
//! `session` 在装配本轮暴露集合时调用 [`capability_to_wire`] 生成
//! `tool_aliases`（内部身份 → 模型侧别名），写进 `session.config`；四个协议的下行
//! 序列化与模型返回调用的解析都读**那一份别名数据**（D-444 ①）。
//!
//! 旧形状在这里还有一半 `capability_resolve`：在「已知名字集合」里按投影反查，
//! 用「精确匹配优先」在两个候选里挑一个。它已随 D-444 ② 退场——**碰撞必须在请求
//! 成形前被拒绝**，而不是返回时靠优先级掩盖。装配期的 `exposed_tools` 逐 wire 分组，
//! 一个 wire 对应两个身份就整组不暴露；解析侧（`symbio-plugin-tool`）只按别名表
//! value 命中，遇到歧义同样拒绝。字符串反演与「字面名优先」都不再是契约的一部分。
//!
//! ## 为什么放在本 crate
//!
//! 本 crate 的准入规则是**依赖方数量**：只被一个模块依赖的内容一律下沉回该模块。
//! 眼下唯一的装配期消费方是 `session`；之所以还在共享层，是因为它是**协议字符集的
//! 契约参数**（不是任何世界的策略），且别名表要跨 session / model / tool 三个插件
//! 保持同源。字符集规则本身不放进插件：放进去就等于四份协议各写一次。
//!
//! ## 不保证单射（这是装配期的事）
//!
//! 选 `__` 而不是转义序列（如 `_2f`）：工具名是给模型看的，`vdfs__read` 比
//! `vdfs_2fread` 可读得多，而模型的工具选择质量直接受名字可读性影响。代价是不可逆，
//! 且 `a.b` 与 `a__b` 都会得到 `a__b`。**本函数不负责消歧**：碰撞由装配期拒绝
//! （见 `symbio-plugin-session/src/plugin/turn.rs::exposed_tools` 与其反例单测）。

/// 线上形态里代替非法字符的序列。
///
/// 不对外导出：它是 [`capability_to_wire`] 的实现细节，唯一的出方向使用点就在本文件。
const TOOL_NAME_WIRE_SEPARATOR: &str = "__";

/// function-calling 协议允许的字符集（OpenAI / Anthropic 的交集）。
///
/// 取交集而非并集：一个名字要同时能送给所有协议，落在交集里才是安全的。
/// 注意 `.` 与 `:` **不在**其中——Gemini 允许，OpenAI / Anthropic 不允许。
fn is_wire_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// 内部身份 → 模型侧别名：字符集之外的字符全部变 `TOOL_NAME_WIRE_SEPARATOR`。
///
/// 幂等（名字已落在字符集内时原样返回），所以对存量会话无害。
/// **不保证单射**：`a.b` 与 `a__b` 都得到 `a__b`；同轮暴露集合里出现这种碰撞，
/// 由装配期拒绝两方，不把歧义的清单送给模型（D-444 ②）。
pub fn capability_to_wire(canonical: &str) -> String {
    if canonical.chars().all(is_wire_char) {
        return canonical.to_string();
    }
    let mut out = String::with_capacity(canonical.len() + 8);
    for c in canonical.chars() {
        if is_wire_char(c) {
            out.push(c);
        } else {
            out.push_str(TOOL_NAME_WIRE_SEPARATOR);
        }
    }
    out
}

#[cfg(test)]
#[path = "tool_name.test.rs"]
mod tests;
