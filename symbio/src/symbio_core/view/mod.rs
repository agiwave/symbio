//! 视图与预算 —— 封闭语法（`Budget` / `View`）住 `symbio_kernel`，词汇与判据住这里。
//!
//! 召回视图与智能体命名空间登记表是**词汇**（谁的记忆、哪个 namespace），由本模块
//! owner 声明；`Budget` / `View` 是投影框架的形状，由内核提供（DE-023 D-188 C2a）。
//! 既有路径 `crate::symbio_core::view::…` 两类都解析得到。

pub use symbio_kernel::view::*;

// ── 召回视图（S5，[plan/01 §9](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")）────────────────
//
// 住 view 域的理由（[plan/05 §3.1](../../../../docs/003-design/018-模块架构.md "DE-018-模块架构")）：③ 的投影
// 产出它、② 的检索 Translator 消费它——跨包共享的**视图类型**必须住中性层，
// 谁也不持有谁的句柄。

use crate::symbio_core::event::Timestamp;

/// **智能体命名空间登记表**（[D-069 §决策 3](../../../../docs/003-design/005-协议规范.md#d-069-principal-三字段kind-与-id-分家id-是可路由地址 "DE-005-协议规范")）。
///
/// 主体 id 是可路由地址 `<namespace>/<name>`；kind 是元数据，与 id 分家。判定「这条 id
/// 是不是智能体一族」不再靠字符串前缀，而是查这张**登记表**——把 reflex 拆到独立 decider
/// 插件时，登记表加一条 `"decider"` 即可，判据一行不动。
///
/// 今天只有 `session/*` 承载本机智能体；person / tool / system 三类各自住在自己的 namespace 下，
/// 都**不在**这张表里（`visible_to` 的第 3 条「非 agent 一律可见」靠这张表分岔）。
pub const AGENT_NAMESPACES: &[&str] = &["session"];

/// id 的**归属段**（`<namespace>/<name>` 的第一段）——不是主体清单，是路由用的地址解析首段。
pub fn principal_namespace(id: &str) -> Option<&str> {
    id.split_once('/').map(|(ns, _)| ns)
}

/// 这条主体 id 属于**智能体一族**吗？（[D-069 §决策 3]）
///
/// 判据：`namespace` 落在 [`AGENT_NAMESPACES`] 登记表里 ⇒ 智能体；否则 ⇒ 不是。
/// **未识别的 namespace** 也返回 `false`——不是 agent，`visible_to` 第 3 条就会放行；
/// 但授权表 `production_matrix` 会照旧 fail-closed（未知主体 = 无 grants）。
pub fn is_agent_id(id: &str) -> bool {
    principal_namespace(id).is_some_and(|ns| AGENT_NAMESPACES.contains(&ns))
}

/// **可见域的平凡判据**（plan/11 批 1 ③，
/// S08 §4 的 `vis_scope` 平凡值）：`actor` 这条事实在 `viewer` 的视图里可见吗？
///
/// 三条规则，一条判定方（判据在 [] 后由前缀切到命名空间登记表）：
///
/// 1. 自己的发言可见（`actor == viewer`）；
/// 2. 会话内的另一方（人）与**未标身份**的事件可见——会话是一条双向线程，
///    线程里的话彼此都看得见；
/// 3. **别的智能体**的发言不可见——第三方私有，不进本主体的 prompt。
///    「是不是智能体」= [`is_agent_id`]；`person/local` / `vdfs/write` / `system/compactor` 都不是智能体，一律放行。
///
/// 平凡值保证：今天的会话是单主体的（所有智能体发言同一 id `session/main`）⇒ 这条
/// 判定对每一条事件都返回 `true`，视图与接线前**逐字一致**。多主体一出现
/// （子智能体、对等体），第 3 条才开始挡东西——那正是它存在的理由。
pub fn visible_to(actor: &str, viewer: &str) -> bool {
    actor == viewer || !is_agent_id(actor)
}

/// 一条被召回的记忆。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RecallEntry {
    /// 来源事件（`memory.encoded`）的 seq——溯源到事实。
    pub seq: u64,
    /// 编码时间（Unix 毫秒）。
    pub ts: Timestamp,
    /// 内容（编码时的正文）。
    pub content: String,
    /// 认知内容标签（七类，[plan/01 §9.1](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")；
    /// 「记忆类型」不是枚举，是过滤参数——加第六类只加参数）。
    pub tag: String,
}

/// 召回视图：as-of 可见、未被遗忘、按新近度排序的记忆条目。
///
/// ## 为什么视图自带 `degraded`
///
/// 投影产出的 [`View`] 外壳已经带 `degraded`（F2：可降级是类型义务），但召回视图
/// **会脱离外壳单独流动**——`memory_events::recall_view` 取 `.value` 后把它存进
/// `TurnState.recall_view`、再交给注入段（`prompt_section`）。外壳一丢，
/// 「这次只显示了一部分」就没人说得出来（静默的形态从「空且不标」变成「有部分结果
/// 且不标」）。故让视图**自描述**：取值点把外壳的 `degraded` **誊抄**进本字段
/// （[D-105](../../../../docs/003-design/001-核心架构.md#d-105-投影预算语义收敛到框架层闭包只返回-v窗口截断由-apply-统一执行 "DE-001-核心架构")：外壳是唯一的降级事实源，
/// 闭包不自置，字段由唯一丢外壳的取值点落笔），消费方不必回捞外壳。
///
/// `#[serde(default)]`：旧序列化形态（无此字段）仍读得出，缺省 `false`（不谎称降级）。
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct RecallView {
    pub entries: Vec<RecallEntry>,
    /// 本次召回是否因预算耗尽而**只给出部分结果**（S06 §6 验收 2）。
    #[serde(default)]
    pub degraded: bool,
}

impl RecallView {
    /// 是否包含某内容（跨主体隔离断言 C13 的观测面；写方拿它做**同文去重**）。
    pub fn contains_content(&self, content: &str) -> bool {
        self.entries.iter().any(|e| e.content == content)
    }
}

#[cfg(test)]
#[path = "mod.test.rs"]
mod tests;
