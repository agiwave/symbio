//! 会话协议里的**主体地址**（[D-069 §决策 1]）。
//!
//! 三个字段各司其职、互相**正交**：
//! - `kind`：**一族**（Person / Agent / Tool / System），编译期闭集，运行期判据读它
//! - `id`：可路由地址 `<namespace>/<name>`——找到主体、调用它、查看它的信息都靠这个地址；
//!   第一段是归属（哪个插件或概念），后续段是归属内的名字（[D-069 §决策 2]）
//! - `title`：**显示名**，渲染层用（工具响应节点常带、系统主体常不带）
//!
//! kind 与 id 分家是这条类型的**核心纪律**——过去把 kind 编进 id（`agent:main`）
//! 意味着 id 不再可路由（"谁的 main"没人说得出），加一族就要动判据；kind 独立之后
//! `is_agent_id` 用命名空间登记表分派（`AGENT_NAMESPACES`），加一族只是登记多一条。

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

/// **主体族**（[D-069 §决策 1]）——四值闭集。
///
/// `role` 已退出协议顶层，`kind` 是主体族的唯一承载点。
/// 因此 [] 里 `NodeRole::User` ↔ `PrincipalKind::Person`，`Assistant` ↔ `Agent`，
/// `Tool` ↔ `Tool`，`System` ↔ `System`；读侧一次性别名靠这套映射把旧消息里的 role 补进 kind。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum PrincipalKind {
    /// 人类主体（本机用户、将来接入的多用户）——今天的 `role = user` 归这一族
    Person,
    /// 智能体一族（本机主智能体、子智能体、对等体、角色行 worker/planner/verifier …）
    Agent,
    /// 工具一族——工具的**响应**节点归属它自己（不是调用它的 agent）
    Tool,
    /// 系统一族（compaction / heartbeat / scheduler 等无 agent 参与的自发事件）
    System,
}

impl PrincipalKind {
    /// 状态词——**与 `serde` 的序列化名逐字一致**（判据在 [`super::chat_node::NodeStatus::as_str`] 同款）。
    pub fn as_str(&self) -> &'static str {
        match self {
            PrincipalKind::Person => "person",
            PrincipalKind::Agent => "agent",
            PrincipalKind::Tool => "tool",
            PrincipalKind::System => "system",
        }
    }

    /// 是否智能体一族——`visible_to` 的运行时判据（[D-069 §决策 3]）。
    ///
    /// 注意与 `symbio_core::view::is_agent_id` 的分工：那条**从 id 反推 kind**（用于
    /// 只有字符串的授权表判据），本条**从结构体字段直接读**。[] 之后协议携带 `Principal`
    /// 结构体，`visible_to` 会切到 `kind.is_agent()`——`is_agent_id` 只服务 id 层的
    /// 可见域判据（`view::visible_to`）。
    pub fn is_agent(&self) -> bool {
        matches!(self, PrincipalKind::Agent)
    }
}

/// 会话协议携带的**主体**（[D-069 §决策 1]）。
///
/// 三个字段都是**协议字段**（进 wire）：`kind` 是族、`id` 是可路由地址、`title` 是显示名。
/// 序列化形态：
/// ```json
/// "principal": { "kind": "tool", "id": "vdfs/write", "title": "VDFS 写文件" }
/// ```
/// `title` 缺省不出现（`skip_serializing_if = "Option::is_none"`）；`kind` 与 `id` 都在
/// 编译期保证有值（`id` 是非空 `String`，`kind` 是枚举）。
///
/// **读侧兼容**：wire 上 `principal` 值既接受本结构体对象形态，也接受**裸字符串**
/// （历史 `messages.json` 里的 `"agent:main"` / `"user"`，以及未升级的调用方）。
/// 字符串走 [`Principal::from_legacy`]——[D-069 §决策 6] 的一次性读侧别名。
/// 序列化始终出对象形态；写侧构造 `Principal` 后不再有第二份表达。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Principal {
    /// 一族（Person / Agent / Tool / System），编译期闭集。
    pub kind: PrincipalKind,
    /// 可路由地址 `<namespace>/<name>`——本结构体唯一的主键（[D-069 §决策 5]）。
    pub id: String,
    /// 显示名（渲染层用；与 kind 无关，工具与角色常带，用户与主智能体常不带）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

impl<'de> Deserialize<'de> for Principal {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de::Error;
        let value = Value::deserialize(deserializer)?;
        if let Some(s) = value.as_str() {
            return Ok(Principal::from_legacy(s));
        }
        serde_json::from_value::<PrincipalRep>(value)
            .map(Principal::from)
            .map_err(D::Error::custom)
    }
}

/// Deserialize 内部用的**结构体形态**中转（避免自递归）。
#[derive(Deserialize)]
struct PrincipalRep {
    kind: PrincipalKind,
    id: String,
    #[serde(default)]
    title: Option<String>,
}

impl From<PrincipalRep> for Principal {
    fn from(r: PrincipalRep) -> Self {
        Principal {
            kind: r.kind,
            id: r.id,
            title: r.title,
        }
    }
}

impl Principal {
    pub fn new(kind: PrincipalKind, id: impl Into<String>) -> Self {
        Principal {
            kind,
            id: id.into(),
            title: None,
        }
    }

    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// 由 **LLM 可见工具名**（wire 形态）构造工具一族的主体。
    ///
    /// 工具的结果节点归**工具自己**（[D-069 §决策 1]：`Tool` 族「工具的响应节点
    /// 归属它自己，不是调用它的 agent」）。地址仍按 `<namespace>/<name>` 规范合成：
    /// - 已带 `/` 段的限定名（`local/todo_write`、`mcp/github/create_issue`）原样；
    /// - MCP 双下划线拼接名（`mcp__server__tool`）⇒ `mcp/server/tool`；
    /// - 首段是归属插件（vdfs / web / local / agent）⇒ `ns/其余`
    ///   （`vdfs_write` → `vdfs/write`，与 `symbio_core::authz` 登记的主体行逐一对齐）；
    /// - 其余只名工具 ⇒ `legacy/<name>`——地址是合成的，族不变。
    ///
    /// 产出对 [`Principal::from_legacy`] **幂等**（这些 namespace 都读回 Tool 族）。
    pub fn tool_principal(name: &str) -> Principal {
        let name = name.trim();
        if name.is_empty() {
            return Principal::new(PrincipalKind::Tool, "legacy/unknown");
        }
        let id = if name.contains('/') {
            name.to_string()
        } else if let Some(rest) = name.strip_prefix("mcp__") {
            format!("mcp/{}", rest.replace("__", "/"))
        } else if let Some((head, rest)) = name.split_once('_') {
            match head {
                "vdfs" | "web" | "local" | "agent" if !rest.is_empty() => {
                    format!("{head}/{rest}")
                }
                _ => format!("legacy/{name}"),
            }
        } else {
            format!("legacy/{name}")
        };
        Principal::new(PrincipalKind::Tool, id)
    }

    /// 从历史 `principal: String` 一次性映射——[D-069 §决策 6] 读侧别名的实现点。
    ///
    /// 旧形态（`agent:xxx` / `user` 裸字面量）与新形态（`<ns>/<name>`）都吃得下：
    /// - 新形态：第一段是 namespace，`session/*` ⇒ Agent，`person/*` ⇒ Person，
    ///   `vdfs/*` / `web/*` / `local/*` / `mcp/*` / `agent/*` / `legacy/*` ⇒ Tool，
    ///   `system/*` ⇒ System；
    ///   未识别 namespace 一律按 Agent 处理（授权表 fail-closed 不因此放松）。
    /// - 旧形态（`kind:name`）：冒号前的 kind 决定族，冒号后的 name 挂到 `session/` 下
    ///   （`agent:main` ⇒ `{Agent, session/main}`）。
    /// - 裸 `user` ⇒ `{Person, person/local}`（今天的会话属主）。
    ///
    /// 只在 [] 的一次性读侧别名里用；新写入点直接构造 `Principal` 结构体，
    /// 不走这条映射（构造即定型）。
    pub fn from_legacy(s: &str) -> Principal {
        // 新形态：`<ns>/<name>` 已带 namespace。
        if let Some((ns, _)) = s.split_once('/') {
            let kind = match ns {
                "person" => PrincipalKind::Person,
                "system" => PrincipalKind::System,
                // 工具命名空间：vdfs / web / local 是插件归属，mcp / agent 是
                // [`Principal::tool_principal`] 合成的地址形态，legacy 是旧 `tool:x`
                // 的兜底段——都读回 Tool 族，保证构造⇄字符串往返幂等。
                "vdfs" | "web" | "local" | "mcp" | "agent" | "legacy" => PrincipalKind::Tool,
                // session 与未识别 namespace 都按 Agent——授权表按 id 精确判，
                // 未登记 ⇒ fail-closed，kind 猜宽不放大权限面。
                _ => PrincipalKind::Agent,
            };
            return Principal::new(kind, s);
        }
        // 旧形态：`agent:xxx` / `person:xxx` / `tool:xxx` / `system:xxx`（D-069 v1）。
        if let Some((kind_word, name)) = s.split_once(':') {
            let (kind, id) = match kind_word {
                "person" => (PrincipalKind::Person, s.to_string()),
                "tool" => (PrincipalKind::Tool, format!("legacy/{name}")),
                "system" => (PrincipalKind::System, s.to_string()),
                // agent:xxx → session/xxx（本机部署的 agent 都住在 session 命名空间）
                _ => (PrincipalKind::Agent, format!("session/{name}")),
            };
            return Principal::new(kind, id);
        }
        // 裸字面量 `user` / 其他未标身份的历史主体名 → Person。
        if s == "user" {
            return Principal::new(PrincipalKind::Person, "person/local");
        }
        // 兜底：当作 agent 一族挂在 session 下，判据仍可路由（不静默丢）。
        Principal::new(PrincipalKind::Agent, format!("session/{s}"))
    }
}

// **读侧兼容反序列化**（[D-069 §决策 6]，）：`Option<Principal>` 字段上的
// 显式标记，语义已由 [`Principal::deserialize`] 的实现给出——`null` / 缺席 ⇒ `None`，
// 字符串 ⇒ `Principal::from_legacy`，对象 ⇒ 正常解构。字段用 `#[serde(default)]` 即可。
//
// 序列化只出对象形态（`skip_serializing_if = "Option::is_none"` 由字段自己带）。
// 本段是**用法规约**（没有对应的本地条目可挂），故用普通注释而非文档注释。

#[cfg(test)]
#[path = "principal.test.rs"]
mod tests;
