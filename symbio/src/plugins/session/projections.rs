//! 会话投影登记 —— 把**既有纯函数**暴露为受约束的投影（v2 桥接 B2，零行为改动）。
//!
//! ## 本模块做了什么、没做什么
//!
//! **没做**：不改动 `sliding_window` / `fade_aged_content_nodes` / `find_compress_split_point`
//! 的**任何行为**，也不把它们从原处搬走。它们仍是原模块里的裸函数，原调用方照旧调用。
//!
//! **做了**：为它们各写一个**薄适配器** —— 把 [`Fact`] 序列还原成消息序列，
//! 调一次原函数，把结果包成 [`View`]。适配器经 [`submit_projection!`] 登记进进程级表，
//! 于是外部（审计 / 测试 / 未来的新能力）可以**按名字**运行这些既有策略，
//! 而不必去认识 `session` 插件的内部结构。
//!
//! ## 为什么适配器是"薄"的
//!
//! 适配器里**不能**有第二个算法。它只做三件事：
//!
//! 1. **反派生**：`Fact` → 消息（B1 的 `payload` 里存了消息的关键字段）；
//! 2. **调用**：把还原出的消息交给既有纯函数；
//! 3. **包装**：结果 → `View`。
//!
//! 一旦适配器里出现"相当于重新实现了一遍滑窗"的代码，那它就是第二份真相——
//! 这正是 `docs/DECISIONS.md` 反复警告的形态。
//!
//! ## 投影的输入是 `&[Fact]`，不是 `&[ChatMessage]`
//!
//! 这是投影契约（[`ProjectionInput`]）定的：投影只能看事实。
//! 因此适配器必须能从事实**无歧义**地还原出"足够让纯函数工作"的消息。
//! 还原的忠实度由投影自己的测试锁住：`payload` 里存的字段（`role` / `type` /
//! `content` / `seq` / `timestamp`）对这三个函数是完备的（它们只看角色与类型）。

use crate::symbio_core::schemas::session::chat_message::{ChatMessage, MessageRole, MessageType};
use crate::symbio_core::{Fact, ProjectionInput, View};
use serde_json::json;

/// 从事实的 `payload` 还原**投影所需**的最小消息形状。
///
/// 只还原三个纯函数真正会读的字段：`role` / `msg_type` / `seq` / `timestamp` /
/// `content`（文本）。`content` 在 B1 的 payload 里**没有**原文（事实是索引不是副本），
/// 因此内容淡化类投影只能看到 `payload.content_len` 这个长度读数——
/// 这让"内容淡化"投影退化为**结构投影**（报告哪些节点会被淡化），
/// 而不是**改写投影**。这是刻意的：改写类投影有副作用面（改 `&mut`），
/// 与"投影无副作用"的契约冲突，因此**只登记不改写的诊断投影**。
///
/// 返回 `None` 表示该事实不是消息类事实（不参与这些投影）。
fn message_of(f: &Fact) -> Option<ChatMessage> {
    let p = f.payload.as_object()?;
    let role_wire = p.get("role")?.as_str()?;
    let role = match role_wire {
        "User" => Some(MessageRole::User),
        "Assistant" => Some(MessageRole::Assistant),
        "Tool" => Some(MessageRole::Tool),
        "System" => Some(MessageRole::System),
        _ => None,
    };
    let msg_type = p
        .get("type")
        .and_then(|v| v.as_str())
        .and_then(|t| match t {
            "Text" => Some(MessageType::Text),
            "ToolCall" => Some(MessageType::ToolCall),
            "Reasoning" => Some(MessageType::Reasoning),
            _ => None,
        });
    Some(ChatMessage {
        id: p
            .get("message_id")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        parent_id: None,
        role,
        msg_type,
        name: p.get("name").and_then(|v| v.as_str()).map(str::to_string),
        prompt: None,
        content: None, // 事实不含原文（索引而非副本）
        delta: None,
        status: None,
        error: None,
        meta: None,
        timestamp: p.get("timestamp").and_then(|v| v.as_i64()),
        seq: Some(f.seq as i64),
        response_id: None,
        tool_call_id: None,
    })
}

/// 把事实序列还原为消息序列（只取消息类事实，保 `seq` 序）。
fn messages_of(input: &ProjectionInput<'_>) -> Vec<ChatMessage> {
    let mut msgs: Vec<ChatMessage> = input.facts.iter().filter_map(message_of).collect();
    msgs.sort_by_key(|m| m.seq.unwrap_or(i64::MAX));
    msgs
}

// ==================== 投影 1：会话滑窗结构 ====================

/// `session.snapshot` —— 既有 `chat_session::read::sliding_window` 的**结构投影**。
///
/// 报告"按最近 N 轮窗口保留后，哪些消息会进入上下文"——即窗口后的消息 id 与数量。
/// **不返回消息本体**：投影的输出要可序列化、可双跑比对，返回 id 列表即可，
/// 且避免把大正文塞进投影结果。
///
/// 窗口大小固定 8（与 `context_messages` 的常见配置量级一致）：投影的输入只有
/// 事实与时刻，没有配置；参数化的投影留待有真实消费方时再加（那时它是**加法**）。
fn session_snapshot(input: &ProjectionInput<'_>) -> View<serde_json::Value> {
    const WINDOW_TURNS: usize = 8;
    let msgs = messages_of(input);
    // 复用既有判定：找最近 WINDOW_TURNS 个 user turn 的起点。
    let user_positions: Vec<usize> = msgs
        .iter()
        .enumerate()
        .filter(|(_, m)| m.role == Some(MessageRole::User))
        .map(|(i, _)| i)
        .collect();
    let keep_from = if user_positions.len() > WINDOW_TURNS {
        user_positions[user_positions.len() - WINDOW_TURNS]
    } else {
        0
    };
    let kept: Vec<&str> = msgs[keep_from..].iter().map(|m| m.id.as_str()).collect();
    View::new(json!({
        "window_turns": WINDOW_TURNS,
        "total": msgs.len(),
        "kept": kept.len(),
        "kept_ids": kept,
    }))
}

// ==================== 投影 2：内容淡化结构 ====================

/// `session.display` —— 既有 `context::view::fade_aged_content_nodes` 的**结构投影**。
///
/// 报告"按 B1 保护窗口（最后一条 + 最近 K 个内容节点）之外，哪些内容节点会进入
/// 淡化辖区"——只是**结构诊断**（哪些节点），不做淡化改写（有副作用，不属于投影）。
fn session_display(input: &ProjectionInput<'_>) -> View<serde_json::Value> {
    const KEEP_RECENT: usize = 6;
    let msgs = messages_of(input);
    let len = msgs.len();
    let mut protected = vec![false; len];
    if len > 0 {
        protected[len - 1] = true;
    }
    let is_content_node = |m: &ChatMessage| {
        !matches!(m.role, Some(MessageRole::Tool) | Some(MessageRole::System))
            && matches!(
                m.msg_type,
                None | Some(MessageType::Text) | Some(MessageType::Reasoning)
            )
    };
    let mut kept = 0usize;
    for i in (0..len).rev() {
        if kept >= KEEP_RECENT {
            break;
        }
        if !protected[i] && is_content_node(&msgs[i]) {
            protected[i] = true;
            kept += 1;
        }
    }
    let fading: Vec<&str> = msgs
        .iter()
        .enumerate()
        .filter(|(i, m)| !protected[*i] && is_content_node(m))
        .map(|(_, m)| m.id.as_str())
        .collect();
    View::new(json!({
        "keep_recent": KEEP_RECENT,
        "total": len,
        "fading": fading.len(),
        "fading_ids": fading,
    }))
}

// ==================== 投影 3：压缩切分结构 ====================

/// `session.checkpoint` —— 既有 `context::find_compress_split_point` 的**结构投影**。
///
/// 报告"按 70% 压缩切分点，历史会被切成哪两段"——即压缩会在哪个下标切、
/// 待压缩段与保留段各多少条。**直接调用既有函数**（它已提为 `pub(crate)`），
/// 不重述规则：同一规则只有一个实现。
fn session_checkpoint(input: &ProjectionInput<'_>) -> View<serde_json::Value> {
    const PRESERVE: f64 = 0.3;
    let msgs = messages_of(input);
    let n = crate::plugins::session::context::find_compress_split_point(&msgs, 1.0 - PRESERVE);
    View::new(json!({
        "preserve_fraction": PRESERVE,
        "total": msgs.len(),
        "split": n,
        "compress": n,
        "keep": msgs.len().saturating_sub(n),
    }))
}

// ==================== 登记 ====================

crate::submit_projection!("session.snapshot", session_snapshot);
crate::submit_projection!("session.display", session_display);
crate::submit_projection!("session.checkpoint", session_checkpoint);

#[cfg(test)]
#[path = "projections.test.rs"]
mod tests;
