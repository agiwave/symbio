//! CLI 对后端协议的**唯一读法**。
//!
//! 同一份协议有两个前端（Tauri / CLI），机制纪律必须同判据：凡是「后端字段叫什么、
//! 某个形状是什么意思」的知识，只能在这里读一次，别处一律调用本模块。
//! `scripts/mechanism-audit.mjs` 的 M-009 / M-010 守这条边界——散回调用点不会
//! 报错，只会在后端改字段名时**静默错读**，而两个前端从此各有一种读法。
//!
//! ## 地址段名为什么由 CLI 自持
//!
//! `SESSION_SEG` / `MESSAGES_SEG` 是 session 插件的 `pub(crate)` 内部词汇、不在
//! `symbio_core` 的可见面上（C-002：插件侧不深引 core 域目录）。CLI 按**地址契约**
//! 自持一份：拼错表现为「帧收不到」，不会静默错乱。自持不等于可以复制——三处常量
//! 因此只在本文件定义一次（M-010）。

use serde_json::Value;
use symbio::symbio_core::chat_node::{ChatNode, MessageContent, NodeStatus};

/// 会话域的**挂载段名**（ASCII）。
pub(crate) const SESSION_SEG: &str = "session";

/// 消息集合的**路径段**（ASCII）。
pub(crate) const MESSAGES_SEG: &str = "message";

/// 会话运行态中「本轮被中止」的 `outcome` 取值。
pub(crate) const OUTCOME_ABORTED: &str = "aborted";

/// 无人值守运行模式（会话 metadata 的 `mode` 取值）。
///
/// 默认值由 `args.rs` 引用本常量，而不是各写一份字面量。
pub(crate) const MODE_AUTO: &str = "auto";

/// 汇报节点的元数据键：该节点只给界面看，不进模型上下文。
///
/// 与产者 `model/message_builder::META_EXCLUDE_FROM_CONTEXT` 是同一个键的两端；
/// 跨插件常量不互引，前端按协议读。
const META_EXCLUDE_FROM_CONTEXT: &str = "exclude_from_context";

/// 交互卡子形态：`meta.prompt.kind`。
const PROMPT_KIND: &str = "kind";

/// 节点元数据的**唯一读取口**。
///
/// 调用方传节点、拿引用；字段解释留在本模块（M-009）。
pub(crate) fn meta_of(node: &ChatNode) -> Option<&Value> {
    node.meta.as_ref()
}

/// 节点是不是**对话面汇报节点**（开场白 / 进度播报）——它给用户看，不进模型对话内容。
///
/// 判据取 `meta.exclude_from_context` 而不是 `meta.surface`：答话节点同样带
/// `surface = "reply"`（见 `chat_loop/compose.rs`），两者只有这个标记不同。
pub(crate) fn is_progress_surface(meta: Option<&Value>) -> bool {
    meta.and_then(|v| v.get(META_EXCLUDE_FROM_CONTEXT))
        .and_then(Value::as_bool)
        == Some(true)
}

/// 一张**等待用户动作**的交互卡（`Interaction` + `WaitingUserAction`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WaitingCard {
    /// 卡片正文（审批要确认什么 / 提问问什么）
    pub text: String,
    pub kind: WaitKind,
    /// 审批卡：要批准的工具名
    pub tool: Option<String>,
    /// 审批卡：该工具的风险等级
    pub risk: Option<String>,
    /// 提问卡：问题个数
    pub questions: usize,
}

/// 交互卡的两种业务形态 + 未标注兜底。
///
/// 取值来自 `meta.prompt.kind`（`confirm` / `question`），与 `ChatNode::derive_actions`
/// 映射「通过/拒绝」还是「回答」的是同一份词表。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WaitKind {
    Confirm,
    Question,
    Other,
}

impl WaitingCard {
    /// 这张卡缺的是哪种答复——播报里必须说清，否则用户只看到「卡住了」。
    pub(crate) fn missing_answer(&self) -> String {
        match self.kind {
            WaitKind::Confirm => "通过 / 拒绝".to_string(),
            WaitKind::Question if self.questions > 1 => format!("回答 {} 个问题", self.questions),
            WaitKind::Question => "回答".to_string(),
            WaitKind::Other => "用户答复（终端无法推断答复形式）".to_string(),
        }
    }

    /// 卡片的中文名（播报头部）。
    pub(crate) fn label(&self) -> &'static str {
        match self.kind {
            WaitKind::Confirm => "审批卡",
            WaitKind::Question => "提问卡",
            WaitKind::Other => "交互卡",
        }
    }

    /// 卡片正文；空正文给一句可辨认的兜底，而不是让播报出现空白行。
    pub(crate) fn title(&self) -> &str {
        let text = self.text.trim();
        if text.is_empty() {
            "（卡片未带说明）"
        } else {
            text
        }
    }
}

/// 一条节点变更是不是「等待用户动作」的交互卡；是则给出播报所需的事实。
///
/// 判据只用协议谓词：`is_interaction()`（形状）+ `WaitingUserAction`（状态）。
/// **不用 `failure_kind` 猜**——控制流不能建立在启发式上（与 `pending_prompt_from`
/// 同一纪律），而 `meta.prompt.kind` 只决定卡片子形态。
pub(crate) fn waiting_card(node: &ChatNode) -> Option<WaitingCard> {
    if !node.is_interaction() || node.status != Some(NodeStatus::WaitingUserAction) {
        return None;
    }
    let prompt = meta_of(node).and_then(|m| m.get("prompt"));
    let kind = match prompt
        .and_then(|p| p.get(PROMPT_KIND))
        .and_then(Value::as_str)
    {
        Some("confirm") => WaitKind::Confirm,
        Some("question") => WaitKind::Question,
        _ => WaitKind::Other,
    };
    Some(WaitingCard {
        text: node
            .content
            .as_ref()
            .map(MessageContent::to_text)
            .unwrap_or_default(),
        kind,
        tool: prompt
            .and_then(|p| p.get("tool_name"))
            .and_then(Value::as_str)
            .map(str::to_string),
        risk: prompt
            .and_then(|p| p.get("risk_level"))
            .and_then(Value::as_str)
            .map(str::to_string),
        questions: prompt
            .and_then(|p| p.get("questions"))
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0),
    })
}

#[cfg(test)]
#[path = "protocol.test.rs"]
mod tests;
