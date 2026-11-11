//! 端口身份 → 模型侧别名（wire form）的**唯一实现**，以及本轮暴露投影（`D-469`）。
//!
//! 能力名是框架内的地址（可以带 `.`，MCP 工具是 `mcp.<server>.<tool>`）；
//! function-calling 协议对工具名的字符集有硬约束（OpenAI / Anthropic 的交集是
//! `[A-Za-z0-9_-]`）。**协议字符集是模型侧的知识**：谁把工具清单交给模型，谁就负责
//! 命名。本模块因此是唯一的换算处——会话装配只交候选集合，经
//! `model/project_tools` 取回这里算出的别名表与碰撞，再原样写进 `session.config`；
//! 共享层不再持有一份同名规则（`D-469`）。
//!
//! ## 一次投影回答三件事
//!
//! - `exposed`：本轮真正能见的身份（按注册序）。两个身份映到同一 wire ⇒ **整组退出**，
//!   请求成形前就少掉它们（`D-444` ②）。
//! - `aliases`：内部身份 → 模型侧别名（正向表）。协议下行序列化按 canonical 查它，
//!   模型返回的调用按 value 反查它，两端读同一份数据。
//! - `collisions`：整组退出的原因（`{ wire, names }`），供 `session.config` 报因。
//!
//! ## 不保证单射
//!
//! 选 `__` 而不是转义序列（如 `_2f`）：工具名是给模型看的，`vdfs__read` 比
//! `vdfs_2fread` 可读得多，而模型的工具选择质量直接受名字可读性影响。代价是不可逆，
//! 且 `a.b` 与 `a__b` 都会得到 `a__b`——所以碰撞必须在装配期被拒绝，不能留到返回时
//! 靠「精确匹配优先」掩盖。

use std::collections::{BTreeMap, HashSet};

use serde::Serialize;

/// 线上形态里代替非法字符的序列（[`capability_to_wire`] 的实现细节）。
const TOOL_NAME_WIRE_SEPARATOR: &str = "__";

/// function-calling 协议允许的字符集（OpenAI / Anthropic 的交集）。
///
/// 取交集而非并集：一个名字要同时能送给所有协议，落在交集里才是安全的。
/// 注意 `.` 与 `:` **不在**其中——Gemini 允许，OpenAI / Anthropic 不允许。
fn is_wire_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// 内部身份 → 模型侧别名：字符集之外的字符全部变 [`TOOL_NAME_WIRE_SEPARATOR`]。
///
/// 幂等（名字已落在字符集内时原样返回），所以对存量会话无害。
/// **不保证单射**：`a.b` 与 `a__b` 都得到 `a__b`；同轮暴露集合里出现这种碰撞，
/// 由 [`project`] 拒绝两方，不把歧义的清单送给模型（`D-444` ②）。
fn capability_to_wire(canonical: &str) -> String {
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

/// 一个被拒收的 wire 名：哪些身份撞在了它上面（`names` 有序，便于比对与报因）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ToolAliasCollision {
    pub wire: String,
    pub names: Vec<String>,
}

/// 一次投影的完整回答（route 载荷的形状，`session.config` 照抄前两项）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub(crate) struct ToolProjection {
    /// 本轮可暴露的身份（输入顺序，已剔除碰撞整组）。
    pub exposed: Vec<String>,
    /// 正向表：内部身份 → 模型侧别名。
    pub aliases: BTreeMap<String, String>,
    /// 整组退出的原因；空 = 无碰撞。
    pub collisions: Vec<ToolAliasCollision>,
}

/// 把候选暴露集合投影成「谁能见 + 各自叫什么 + 谁因碰撞退出」。
///
/// 输入应是会话已经交集好的候选（显式暴露 ∩ 作用域 ∩ 有效绑定，按注册序）；
/// 本函数只做协议侧命名与碰撞裁决，不碰任何世界状态。
pub(crate) fn project(names: &[String]) -> ToolProjection {
    let mut by_wire: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for name in names {
        by_wire
            .entry(capability_to_wire(name))
            .or_default()
            .push(name.clone());
    }

    let mut rejected: HashSet<String> = HashSet::new();
    let mut projection = ToolProjection::default();
    for (wire, mut group) in by_wire {
        group.sort();
        group.dedup();
        if group.len() > 1 {
            // 同名绑定（同一 wire 下多个身份）：整组退出本轮暴露集合。
            projection.collisions.push(ToolAliasCollision {
                wire,
                names: group.clone(),
            });
            rejected.extend(group);
        } else if let Some(name) = group.pop() {
            projection.aliases.insert(name, wire);
        }
    }
    projection.exposed = names
        .iter()
        .filter(|name| !rejected.contains(*name))
        .cloned()
        .collect();
    projection
}

#[cfg(test)]
#[path = "tool_projection.test.rs"]
mod tests;
