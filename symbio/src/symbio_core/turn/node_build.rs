//! ChatNode 的**写侧构造与建树整形**词表（后端机制，不是显示定义）。
//!
//! ## 为什么住 core、为什么不在 `chat_node.rs`
//!
//! 这批函数曾在 [D-023](../../../../docs/003-design/001-核心架构.md#d-023-symbio_core-的准入规则--依赖方数量 "DE-001-核心架构") 按「依赖方数量」
//! 判据从 core 下沉到 `session/message_build`——那个判据今天仍然成立（消费方没有变多），
//! 变的是**位置**：[plan/14 步 6 · 批次 6b](../../../../docs/003-design/014-单环一表.md "DE-014-单环一表")
//! 把驱动器切成独立插件 `loop`（裁决一）后，写侧（驱动器）与
//! 读侧投影（留守 `rebuild`）隔着 E-009 的插件墙共用同一份形状源，同一条判据在两个插件
//! 之间不再有用 ⇒ 按 D-082 裁决三上移回 core。**同一条判据的第二个形态**：单插件时代
//! 它确实只有 session 一个消费方。
//!
//! 落点是**本模块**而非 [`crate::symbio_core::chat_node`]：`chat_node.rs` 是前端机制化显示节点的
//! **形状定义**（角色 / 形状五值 / style_tag 词表 / 线格式），不收留写侧机制；
//! 构造与折叠是「按该形状产节点」的手艺，与形状源同目录紧邻、各住各的文件。
//!
//! ## 消费方（D-082 裁决三 2/3/6/7 组）
//!
//! | 符号 | 消费方 |
//! |---|---|
//! | [`dialog_node`] | [`crate::symbio_core::facts::ReportDraft::to_message`]（汇报落格；调用口在 loop 侧 `report_exec`）· `chat_loop/compose` 的判决落地 |
//! | [`llm_build_text_node`] / [`llm_build_tool_call_nodes`] / [`llm_build_tool_call_node`] / [`tool_call_args_text`] / [`llm_build_tool_message`] | 留守 `message_build::llm_build_assistant_messages` · `tool_dispatch` · `tool_executor` · `rebuild` |
//! | [`PresentationTree`] / [`detach_if_flat`] / [`flatten_whole_tree`] / [`turn_root_of`] | 留守 `config`（解析）· `chat_session/read` · `plugin/nodes` · 驱动器 `chat_loop/io` · `orchestrator/failure` · `resume` |
//!
//! core 侧**没有任何东西反过来调本模块**——方向永远是消费方 → core。

use crate::symbio_core::{
    chat_node::{ChatNode, MessageContent, NodeKind, NodeStatus, CHAT_STYLE_TAG_TOOL},
    clock_now_ms, llm_short_id, TurnToolCallInfo,
};

use crate::symbio_core::principal::Principal;

// ==================== 节点构造族 ====================

/// 对话面文本节点的 `meta.surface` 取值（唯一字面量落点）。
pub const SURFACE_REPLY: &str = "reply";

/// 构造对话面文本节点（**纯函数**，可单测）。
///
/// `exclude_from_context`：`Answered` 的答话为 `false`（它进请求包——它就是这一轮的
/// 答复），`Escalate` 的首响为 `true`（界面开场白，不进请求包）。判据见
/// `compose` 侧的判决文档。
///
/// `principal` 是**这条发言是谁说的**（D-147：身份只写一份）。对话面文本恒由
/// 本会话主体产出，调用方从请求上下文取 [`crate::symbio_core::request_principal`]。
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
/// `session::message_build::TurnStreamChildIds`），否则同一段正文在存储层与流式层
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

/// 助手**工具调用**子节点组（骨架 `Skeleton` + tag `tool`，`parent_id` 指向本轮骨架）。
///
/// 工具调用骨架（`Skeleton` + tag `tool`）自身携带请求参数（`content` = JSON 文本），
/// 不设独立的请求子节点。`id` 是节点 id（与流式帧一致），provider 的 wire id 存
/// `tool_call_id`。
///
/// 单点定义：落库视图（`session::message_build::llm_build_assistant_messages`）与
/// 工具分发方（`tool_dispatch`）都经此构造——两处各写一份，工具卡片的形状迟早会
/// 分叉（前端读的是同一套字段）。
pub fn llm_build_tool_call_nodes(
    parent_id: &str,
    tool_calls: &[TurnToolCallInfo],
    principal: &Principal,
) -> Vec<ChatNode> {
    tool_calls
        .iter()
        .map(|tc| {
            let tc_id = tc.id.clone().unwrap_or_else(llm_short_id);
            llm_build_tool_call_node(
                parent_id,
                tc_id,
                tc.name.clone(),
                tool_call_args_text(tc),
                tc.wire_id.clone(),
                principal,
            )
        })
        .collect()
}

/// ToolCall 节点的**定稿参数文本**（JSON 文本；解析失败 ⇒ 残破原文而非占位 `{}`：
/// 存储层保真，事后能看出模型究竟发了什么，而不是留下一个看似合法的假空参数）。
///
/// 落库构造（[`llm_build_tool_call_nodes`]）与事件载荷（`turn_runner` 的
/// `artifact.added`）共用这一个取法——两处各写一份必然漂移。
pub fn tool_call_args_text(tc: &TurnToolCallInfo) -> String {
    match &tc.parse_error {
        Some(raw) => raw.clone(),
        None => tc.arguments.to_string(),
    }
}

/// 助手**单个工具调用**子节点（骨架 `Skeleton` + tag `tool`，`parent_id` 指向本轮骨架）。
///
/// `args_text` 是**已定稿的参数文本**（JSON 文本，或解析失败时的残破原文）——
/// 重建引擎从事件载荷直接回放这段文本，不再经 `Value` 重序列化（键序一变，
/// 同一次调用在重建前后就是两个内容）。形状与 [`llm_build_tool_call_nodes`] 同源。
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

// ==================== 建树折叠（`presentation_tree` 的节点侧） ====================
//
// 建树策略的 flat 折叠（[]）。`grouped` 是今天行为：本轮 reasoning/text/tool_call
// 挂 `Skeleton{run}` 根下；`flat` 把本轮**响应骨架的直接子节点**升到会话根
// （`parent_id: None`），工具结果仍挂自己的 tool_call 骨架下（那是「工具调用内部
// 有自己的骨架」，与建树策略正交）。前端只看 `parent_id`、**不知道这个开关**——
// 切档零代码改动是本任务的判据核心。
//
// 脱父子节点带 `meta.turn_root = <骨架 id>`：后端两个「本轮工作」子树遍历器
// （`resume::process_retry_subtree` 的 BFS、`failure::subtree_of`）按 `parent_id`
// 收集后代，脱父子节点够不着。记号让「本轮工作 = 骨架 ∪ {turn_root==骨架}」与
// grouped 的「骨架子树」逐字等价，遍历器多一条并集即可。`grouped` 从不写这个键，
// 存量数据零字节改动，两档各自可独立回退。

/// 建树策略 —— 后端组树时新节点 `parent_id` 的挂法（[]）。
///
/// 这是一个**纯后端旋钮**：前端只按 `parent_id` 插树，不知道本开关存在，切档
/// 前端零代码改动（这是本任务判据的核心）。档间差异只在「本轮节点挂会话根，
/// 还是挂本轮的 `Skeleton{run}` 根」这一件事上——工具结果始终挂在自己的
/// `tool_call` 骨架下（那是「工具调用内部有自己的骨架」，与建树策略正交）。
///
/// `deep`（子智能体递归嵌套）随子会话转播桥在 149-ii 接线：子会话的节点早已由
/// `relay_bridge` 无条件挂到发起它的父工具卡下，本档只是让读侧投影**保留**那棵嵌套树
/// （与 `grouped` 同样不升根），语义差异全在前端如何展开。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentationTree {
    /// 本轮所有节点挂会话根（与用户发言互为兄弟）；只有工具调用内部保留自己的骨架。
    Flat,
    /// 每条用户消息开一个 `Skeleton{run}` 根，本轮其余节点挂它下（**出厂默认 = 今天行为**）。
    Grouped,
    /// 子会话递归挂到发起它的那张工具卡内（多智能体嵌套）。存储与 `grouped` 逐字相同，
    /// 区别只在读侧投影：本档**不做**任何升根折叠，保留转播桥已建好的嵌套树。
    Deep,
}

impl PresentationTree {
    /// `PLUGIN.yml` / wire 里的字符串值。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Flat => "flat",
            Self::Grouped => "grouped",
            Self::Deep => "deep",
        }
    }

    /// 建树策略候选（表单与说明同源，不写第二份字面量）。
    pub(crate) const OPTIONS: [(&'static str, &'static str); 3] = [
        ("grouped", "本轮归到响应骨架下（默认）"),
        ("flat", "本轮全部铺平到会话根"),
        ("deep", "子会话挂到发起它的工具卡内（递归嵌套）"),
    ];
}

/// 单个「本轮响应骨架直接子节点」在 flat 档下升根（并打 turn_root 记号）。
///
/// `run_root_id` 是本轮响应骨架的 id：只有 `parent_id` 恰指它的节点才脱——这样
/// 工具结果（parent = tool_call）天然不受影响。站 B 逐帧落点用它（那一帧的父就是
/// 本轮 root_id），站 A 落库 Vec 里逐节点用它（root_id 同为骨架 id）。
pub fn detach_if_flat(node: &mut ChatNode, run_root_id: &str, mode: PresentationTree) {
    if mode == PresentationTree::Flat && node.parent_id.as_deref() == Some(run_root_id) {
        node.parent_id = None;
        node.meta = merge_turn_root(node.meta.take(), run_root_id);
    }
}

/// 对刚落库 / 重建的一整棵树套用 flat 折叠（rebuild 读层用：rebuild 核心 config-free、
/// 自行重derive了 grouped 父，这里据 `is_run()` 认出各轮骨架再把它们的孩子升根）。
pub fn flatten_whole_tree(nodes: &mut [ChatNode], mode: PresentationTree) {
    if mode != PresentationTree::Flat {
        return;
    }
    // 工具卡 id 集合：夹在工具卡内的 `Skeleton{run}` 是**子会话**转播来的（`relay_bridge`
    // 无条件把子会话顶层节点挂到父工具卡下），它虽仍 `is_run()`，但绝不能被折叠——
    // 否则子会话的孙节点会被错误升到会话根，拆散嵌套树。只有**会话根级**的 run 骨架才折叠。
    let tool_cards: std::collections::HashSet<String> = nodes
        .iter()
        .filter(|n| n.is_tool_call())
        .map(|n| n.id.clone())
        .collect();
    let run_roots: std::collections::HashSet<String> = nodes
        .iter()
        .filter(|n| n.is_run())
        .filter(|n| {
            n.parent_id
                .as_deref()
                .map(|p| !tool_cards.contains(p))
                .unwrap_or(true)
        })
        .map(|n| n.id.clone())
        .collect();
    for n in nodes.iter_mut() {
        let p = match n.parent_id.as_deref() {
            Some(p) if run_roots.contains(p) => p.to_string(),
            _ => continue,
        };
        n.parent_id = None;
        n.meta = merge_turn_root(n.meta.take(), &p);
    }
}

/// 读 `meta.turn_root`（遍历器并集用）。
pub fn turn_root_of(node: &ChatNode) -> Option<&str> {
    node.meta
        .as_ref()
        .and_then(|m| m.get("turn_root"))
        .and_then(|v| v.as_str())
}

/// 把 `turn_root` 合并进已有 meta（不覆盖其它键）。
fn merge_turn_root(meta: Option<serde_json::Value>, root_id: &str) -> Option<serde_json::Value> {
    let mut m = meta.unwrap_or_else(|| serde_json::json!({}));
    if let Some(obj) = m.as_object_mut() {
        obj.insert("turn_root".to_string(), serde_json::json!(root_id));
    }
    Some(m)
}

#[cfg(test)]
#[path = "node_build.test.rs"]
mod tests;
