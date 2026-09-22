//! 工具名的**线上形态**（wire form）——能力名与 function-calling 协议字符集之间
//! 唯一的换算处。
//!
//! ## 为什么需要换算
//!
//! 能力名是**框架内的地址**，可以带任意分隔符（今天 MCP 工具是
//! `mcp.<server>.<tool>`，将来还可能有别的形状）；而 function-calling 协议
//! 对工具名的字符集有硬约束——OpenAI / Anthropic 只接受 `[A-Za-z0-9_-]`。
//! 两者之间必须有一次换算，且**出方向与入方向必须成对**。
//!
//! ## 为什么放在 `symbio_core`
//!
//! core 的准入规则是**依赖方数量**，不是「够不够底层」：只被一个模块依赖的内容
//! 一律下沉回该模块（core 是跨模块的系统架构，不是通用工具箱）。按这条规则核一遍
//! 本模块的两个函数：
//!
//! | 函数 | 依赖方 | 位置 |
//! |---|---|---|
//! | [`to_wire`] | `model` 插件（4 个协议的请求序列化）+ 本模块的 [`resolve`] | core |
//! | [`resolve`] | `session` 插件（`tool_executor` 把模型给的名字认回来） | core |
//!
//! 两者是**同一份契约的两半**（出方向与入方向必须成对，改一个不改另一个就是静默
//! 错位），因此放在一起。而它们能落在 core，是因为**跨模块**：名字由 `model` 发出、
//! 由 `session` 认回，两个插件互相不可见，只有 core 是共同可见处。
//!
//! 对照：[`crate::symbio_core::sse`] 的 `SseLineParser` 是同样的形状——**契约**在
//! core、**字段名与实现**在拥有它的层。本模块里没有任何协议字段名，只有字符集与
//! 分隔符这两个「契约本身」的参数。
//!
//! 反面例子（**故意没进 core**）：工具结果的字段名读取器
//! （`session/tool_executor.rs::extract_result`）只有一个消费方，留在原地。
//! 「工具结果字段名」这条跨插件约定因此只能以文档形式存在——这是自觉保留的张力，
//! 不是疏漏，详见该函数的文档。
//!
//! ## 收口前的问题
//!
//! 换算曾以 `name.replace("/", "__")` 的形式散在 10 处（4 个协议各 1~2 处、
//! `model/types.rs` 2 处、`session/tool_executor.rs` 1 处），入方向则是
//! `tool_name.replace("__", "/")`。三个后果：
//!
//! 1. **只处理了一个字符**：今天**没有任何**能力名含 `/`（实有名字是
//!    `vdfs_read` / `shell` / `mcp.<server>.<tool>` / `agent_<id>_<tool>`），
//!    于是这条替换是空转；而真正非法的 `.`（MCP 名字带点）它不管——协议
//!    字符集之外的名字就这样送给了模型。
//! 2. **入方向靠字符串反演**：`replace("__", "/")` 假定「名字里的 `__` 一定
//!    是 `/` 变的」。名字里本来就有 `__` 时这条假定直接错，且**错得没有声音**
//!    ——解析到一个不存在的工具，报错信息还指着另一个名字。
//! 3. **换算是隐式的**：没人知道「线上名」是一个需要成对维护的投影，于是
//!    加协议时只记得抄 `replace`，不记得它是契约的一半。
//!
//! ## 现在
//!
//! - 出方向：[`to_wire`] —— 字符集之外的字符一律变 `__`，**一个具名函数**；
//! - 入方向：[`resolve`] —— 在**已知名字集合**里找出谁映射到这个名字，
//!   不做字符串反演。
//!
//! 入方向为什么必须查表：`a__b` 既可能是 `a.b` 的线上形态，也可能本身就是
//! `a__b`。字符串分不出这两种，注册表可以——[`resolve`] **精确匹配优先**，
//! 于是「字面名」永远赢过「投影名」，歧义有一个确定的答案而不是一个猜测。

/// 线上形态里代替非法字符的序列。
///
/// 选 `__` 而不是转义序列（如 `_2f`）：工具名是**给模型看的**，
/// `vdfs__read` 比 `vdfs_2fread` 可读得多，而模型的工具选择质量直接受名字可读性
/// 影响。代价是不可逆——所以入方向查表（见模块文档）。
pub const WIRE_SEPARATOR: &str = "__";

/// function-calling 协议允许的字符集（OpenAI / Anthropic 的交集）。
///
/// 取交集而非并集：一个名字要同时能送给所有协议，落在交集里才是安全的。
/// 注意 `.` 与 `:` **不在**其中——Gemini 允许，OpenAI / Anthropic 不允许。
fn is_wire_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// 能力名 → 线上名：字符集之外的字符全部变 [`WIRE_SEPARATOR`]。
///
/// 幂等：名字已落在字符集内时原样返回（今天绝大多数名字如此，
/// `vdfs_read` → `vdfs_read`）。因此这个函数对**存量会话无害**——
/// 名字没变，落库的 `ToolCall.name` 也就没变。
///
/// 不保证单射（`a.b` 与 `a__b` 都得到 `a__b`）——歧义由 [`resolve`] 在
/// 注册表里消解，不由本函数承担。
pub fn to_wire(canonical: &str) -> String {
    if canonical.chars().all(is_wire_char) {
        // 快路径：绝大多数名字无需分配新串（`to_string` 仍是一次拷贝，
        // 但省掉了逐字符判断与拼接）
        return canonical.to_string();
    }
    let mut out = String::with_capacity(canonical.len() + 8);
    for c in canonical.chars() {
        if is_wire_char(c) {
            out.push(c);
        } else {
            out.push_str(WIRE_SEPARATOR);
        }
    }
    out
}

/// 线上名 → 能力名：在 `known`（注册表里的全部能力名）中找出它对应的那一个。
///
/// 语义：
/// 1. `known` 里有**字面相等**的名字 → 返回它（精确匹配优先，歧义到此为止）；
/// 2. 否则返回第一个 `to_wire(name) == wire` 的名字；
/// 3. 都没有 → `None`（**不猜**）。
///
/// 返回 `None` 的调用方应当**报错**而不是回退到「把名字反演一下试试」：
/// 解析失败意味着这个名字不在注册表里，任何反演都只能解析到另一个名字，
/// 而「调起另一个工具」比「报错」坏得多。
pub fn resolve<'a>(wire: &str, known: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let mut projected = None;
    for name in known {
        if name == wire {
            return Some(name);
        }
        if projected.is_none() && to_wire(name) == wire {
            projected = Some(name);
        }
    }
    projected
}

#[cfg(test)]
#[path = "tool_name.test.rs"]
mod tests;
