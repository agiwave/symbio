//! ChatNode 的**写侧构造与建树整形**词表（后端机制，不是显示定义）。
//!
//! ## 为什么住 core、为什么不在 `chat_node.rs`
//!
//! 这批函数曾在 [D-023](../../../../../docs/003-design/001-核心架构.md#d-023-内核的准入规则--依赖方数量 "DE-001-核心架构") 按「依赖方数量」
//! 判据从 core 下沉到 `session/message_build`——那个判据今天仍然成立（消费方没有变多），
//! 变的是**位置**：[D-082](../../../../../docs/003-design/014-单环一表.md "DE-014-单环一表")
//! 把驱动器切成独立插件 `loop`（裁决一）后，写侧（驱动器）与
//! 读侧投影（留守 `rebuild`）隔着 E-009 的插件墙共用同一份形状源，同一条判据在两个插件
//! 之间不再有用 ⇒ 按 D-082 裁决三上移回 core。**同一条判据的第二个形态**：单插件时代
//! 它确实只有 session 一个消费方。建树策略后来只剩 session 消费，已按同一判据回到
//! session；本模块保留共享的节点构造与轮范围收集。
//!
//! 落点是**本模块**而非 [`crate::sdk::chat::chat_node`]：`chat_node.rs` 是前端机制化显示节点的
//! **形状定义**（角色 / 形状五值 / style_tag 词表 / 线格式），不收留写侧机制；
//! 构造与折叠是「按该形状产节点」的手艺，与形状源同目录紧邻、各住各的文件。
//!
//! ## 消费方（D-082 裁决三 2/3/6/7 组）
//!
//! | 符号 | 消费方 |
//! |---|---|
//! | [`dialog_node`] | [`crate::sdk::round::facts::ReportDraft::to_message`]（汇报落格；调用口在 loop 侧 `report_exec`） |
//! | [`llm_build_text_node`] / [`llm_build_tool_call_node`] / [`tool_call_args_text`] / [`llm_build_tool_message`] | 落库构造 · 工具分发 · 重建引擎 |
//! | [`scope_root_of`] / [`subtree_of`] | 跨插件读侧投影（按轮范围收集节点） |
//!
//! core 侧**没有任何东西反过来调本模块**——方向永远是消费方 → core。

use crate::kernel::chat::chat_node::{
    ChatNode, MessageContent, NodeKind, NodeStatus, CHAT_STYLE_TAG_TOOL,
};
use crate::kernel::chat::principal::Principal;
use crate::sdk::clock::clock_now_ms;
use crate::sdk::llm::llm_short_id;

/// 结果节点身份的唯一派生：`{call_id}-result`（内核规则的转发，见
/// [`crate::kernel::event::result_node_id`]）。
///
/// 执行体（流式增量帧的落点）经门面的 `node_build` 模块取它——写侧 / 读侧 / 执行体
/// 共用一份派生，三处各算一遍必然漂移（DE-048 D-312）。
pub use crate::kernel::event::result_node_id;

// ==================== 节点构造族 ====================

/// 对话面文本节点的 `meta.surface` 取值（唯一字面量落点）。
pub const SURFACE_REPLY: &str = "reply";

/// 构造对话面文本节点（**纯函数**，可单测）。
///
/// 两个用户，都是「界面上的话，不是模型的对话内容」：
/// - 进度汇报：`ReportDraft::to_message`，`exclude_from_context = true`；它站在
///   **轮边界**上，不属于某一轮 ⇒ 根级；
/// - 首响（`Escalate`）：重建侧 `opening_node` 用它复刻落库那一格，再把父改写成
///   轮根（`D-426`）——形状一份，位置由事实给。
///
/// 答话（`Answered`）**不走这里**（`D-435`）：它与工具轮的正文是同一条通道
/// （轮根下的正文子 + 载荷 `meta` 带 `surface` / `reason`），不是第三种节点。
///
/// `principal` 是**这条发言是谁说的**（D-147：身份只写一份）。对话面文本恒由
/// 本会话主体产出，调用方从请求上下文取 `request_principal`（`authz` 域，控制面）。
pub fn dialog_node(
    text: &str,
    reason: &str,
    exclude_from_context: bool,
    principal: &Principal,
) -> ChatNode {
    let mut meta = serde_json::json!({
        "surface": SURFACE_REPLY,
        "reason": reason,
    });
    if exclude_from_context {
        meta["exclude_from_context"] = serde_json::json!(true);
    }
    ChatNode {
        id: llm_short_id(),
        // 根级：与用户消息互为兄弟（对话线），不是某个 Turn 的子节点
        parent_id: None,
        principal: Some(principal.clone()),
        msg_type: Some(NodeKind::Body),
        content: Some(MessageContent::Text(text.to_string())),
        status: Some(NodeStatus::Completed),
        timestamp: Some(clock_now_ms()),
        meta: Some(meta),
        ..Default::default()
    }
}

/// 助手**正文**子节点（`msg_type = Body`，`parent_id` 指向本轮骨架）。
///
/// `id` 由调用方给定：流式期已广播的节点 id **必须复用**（见
/// `session::message_build::RoundStreamChildIds`），否则同一段正文在存储层与流式层
/// 是两个身份。工具轮的中途正文也走本函数——分发方拿流式节点的 id 把这一轮
/// **定格**成一条完整消息（`tool_dispatch`）。
pub fn llm_build_text_node(
    parent_id: &str,
    content: &str,
    rid: Option<String>,
    id: String,
    principal: &Principal,
) -> ChatNode {
    ChatNode {
        id,
        parent_id: Some(parent_id.to_string()),
        principal: Some(principal.clone()),
        msg_type: Some(NodeKind::Body),
        content: Some(MessageContent::Text(content.into())),
        status: Some(NodeStatus::Completed),
        timestamp: Some(clock_now_ms()),
        response_id: rid,
        ..Default::default()
    }
}

/// 助手**单个工具调用**子节点（骨架 `Skeleton` + tag `tool`，`parent_id` 指向本轮骨架）。
///
/// `args_text` 是**已定稿的参数文本**（JSON 文本，或解析失败时的残破原文）——
/// 重建引擎从事件载荷直接回放这段文本，不再经 `Value` 重序列化（键序一变，
/// 同一次调用在重建前后就是两个内容）。骨架与 tag 约定与同组其它构造同源。
pub fn llm_build_tool_call_node(
    parent_id: &str,
    id: String,
    name: Option<String>,
    args_text: String,
    wire_id: Option<String>,
    principal: &Principal,
) -> ChatNode {
    ChatNode {
        id,
        parent_id: Some(parent_id.to_string()),
        principal: Some(principal.clone()),
        msg_type: Some(NodeKind::Skeleton),
        content: Some(MessageContent::Text(args_text)),
        status: Some(NodeStatus::Completed),
        timestamp: Some(clock_now_ms()),
        tool_call_id: wire_id,
        ..Default::default()
    }
    .with_style_tag(CHAT_STYLE_TAG_TOOL)
    .with_tool_name(name)
}

/// 构造工具执行结果消息（`msg_type = Body`，parent_id 指向 tool_call）。
/// 响应结果作为工具调用骨架的直接正文(`Tool`) 子节点（组合节点可选，故不包骨架）。
///
/// `tool_name` 是 LLM 可见的 wire 工具名（LLM 视角）：同时写进 `meta.tool_name`
/// 与 `principal`（——结果节点归**工具自己**一族，`role` 删除后
/// `principal.kind` 是唯一承载点）。名字未知（协议级失败）传 `None`：不往 meta
/// 里塞 null，主体交给落库收口点 `attributed` 兜底。
///
/// **重要**：结果子节点的 `status` 必须与实际执行结果一致——
/// 成功 `Completed`、失败 `Failed`。若失败结果被标成 `Completed`，
/// 当下一轮 `get_context_messages` 过滤掉 `Failed`
/// 的 `ToolCall` 父节点时，这个"孤儿"结果子节点（其 `tool_call_id`
/// 指向已被删除的 tool_call）会被保留下来，使新一轮 LLM 请求携带非法
/// `tool_call_id` → 请求包出错（"发给大语言模型的数据包会出错"）。
pub fn llm_build_tool_message(
    tool_call_id: &str,
    content: &str,
    success: Option<bool>,
    msg_id: Option<String>,
    tool_name: Option<&str>,
) -> ChatNode {
    let success = success.unwrap_or(true);
    let mut meta = serde_json::json!({ "success": success });
    if let Some(name) = tool_name {
        meta["tool_name"] = serde_json::json!(name);
    }
    ChatNode {
        id: msg_id.unwrap_or_else(llm_short_id),
        parent_id: Some(tool_call_id.into()),
        // 结果节点归**工具自己**一族：名字未知（协议级失败）时仍是「一条工具结果」，
        // 用 `legacy/unknown` 这个可路由的兜底地址承载族——不能退回会话主体，
        // 否则同一张卡片会按「助手说的话」呈现（D-147：身份只写一份）。
        principal: Some(
            tool_name
                .map(Principal::tool_principal)
                .unwrap_or_else(|| Principal::tool_principal("unknown")),
        ),
        msg_type: Some(NodeKind::Body),
        content: Some(MessageContent::Text(content.into())),
        status: Some(if success {
            NodeStatus::Completed
        } else {
            NodeStatus::Failed
        }),
        meta: Some(meta),
        timestamp: Some(clock_now_ms()),
        ..Default::default()
    }
}

/// 读 `meta.scope_root`（遍历器并集用）。
pub fn scope_root_of(node: &ChatNode) -> Option<&str> {
    node.meta
        .as_ref()
        .and_then(|m| m.get("scope_root"))
        .and_then(|v| v.as_str())
}

/// `root` 在 `messages` 里的**传递闭包子树**（含 `root` 自身）。
///
/// 「本轮的工作 = 骨架 ∪ 它的后代」这句话有**三份**消费方：失败降级
/// （`orchestrator::failure::persist_failure`）、重试（`chat_loop::wake`）、分支隐藏
/// （`projection::branch::branch_hide_unselected`）。各写一遍的后果是其中一份漏掉
/// `flat` 档：那一档下本轮响应骨架的直接子节点升了根（`parent_id: None`），只按
/// `parent_id` 收集够不着它们，于是「删不干净 / 藏不干净」在某一档静默发生。
/// 记号 `meta.scope_root` 因此与 `parent_id` 一起构成**同一条并集**，这里一次收口。
pub fn subtree_of(messages: &[ChatNode], root: &str) -> std::collections::HashSet<String> {
    let mut ids = std::collections::HashSet::new();
    ids.insert(root.to_string());
    let mut queue = vec![root.to_string()];
    for m in messages {
        if scope_root_of(m) == Some(root) && ids.insert(m.id.clone()) {
            queue.push(m.id.clone());
        }
    }
    while let Some(parent_id) = queue.pop() {
        for m in messages {
            if m.parent_id.as_deref() == Some(&parent_id) && ids.insert(m.id.clone()) {
                queue.push(m.id.clone());
            }
        }
    }
    ids
}

#[cfg(test)]
#[path = "node_build.test.rs"]
mod tests;
