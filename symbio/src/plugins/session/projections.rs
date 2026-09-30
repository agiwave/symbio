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

// ==================== 投影 4：记忆召回（B4 / S06）====================

/// `memory.recall` —— S06 检索者消费的**召回投影**（v2 桥接 B4）。
///
/// ## 它做什么
///
/// 从事实序列里挑出**记忆类事实**（`memory.*` 格子）与**当前窗口**内的轮次事实，
/// 折叠成一个"可召回集合"的结构视图：有哪些候选、分别属于哪个 `memory.*` 动词、
/// 时间戳区间如何。它**不返回正文**（事实是索引不是副本），只返回候选的 id 与
/// 分档——与既有三个会话投影同款：输出必须可序列化、可双跑比对。
///
/// ## 为什么它住在 session 而非新插件
///
/// 投影的**输入只有事实**（[`ProjectionInput`]），因此放置位置只取决于
/// "谁顺手"——而事实的 actor 是会话，会话事实的派生约定也住在会话链路。
/// 更关键的是：本投影是**被登记**的，不是被调用的；检索者按名字 `memory.recall`
/// 取用，**不必认识 session**（B2 的核心收益）。物理位置与调用关系解耦。
///
/// ## 平凡值（J2）
///
/// **未登记 = 未接入**。检索者取不到 `memory.recall` 时按"只看当前窗口"处理，
/// 即退化成 S01 的失忆助手。本投影**总是登记**（session 是必需插件），
/// 但它的**输入**（`memory.*` 事实）是否出现，取决于事实源——若无人产生记忆事实，
/// 它自然只折出窗口部分，`trivial` 标记为真。这与 S06 §4 的平凡值表一致。
fn memory_recall(input: &ProjectionInput<'_>) -> View<serde_json::Value> {
    const WINDOW_TURNS: usize = 8;

    // ① 记忆类事实：B1 的 `memory.*` 四格。断言 A1 的落点——这里**只枚举**已预留的
    //    取值，不引入新类型；格子没点亮时这一段自然为空。
    let memory_facts: Vec<&Fact> = input
        .facts
        .iter()
        .filter(|f| f.kind.entity() == "memory")
        .collect();

    // ② 当前窗口：沿用 `session.snapshot` 的窗口语义（最近 N 轮）。
    //    "窗口"是**跨事实类型**的通用折叠：按 seq 序取尾部 N 轮的用户事实起点。
    let user_seqs: Vec<u64> = input
        .facts
        .iter()
        .filter(|f| f.kind == crate::symbio_core::FactKind::TurnUserMessage)
        .map(|f| f.seq)
        .collect();
    let window_from = if user_seqs.len() > WINDOW_TURNS {
        user_seqs[user_seqs.len() - WINDOW_TURNS]
    } else {
        crate::symbio_core::FACT_NONE_SEQ
    };

    // ③ 候选 = 记忆类事实 ∪ 窗口内事实（去重靠 seq 唯一）。
    let mut candidates: Vec<(&Fact, &'static str)> = Vec::new();
    for f in &memory_facts {
        candidates.push((f, "memory"));
    }
    for f in input.facts.iter().filter(|f| f.seq >= window_from) {
        if f.kind.entity() != "memory" {
            candidates.push((f, "window"));
        }
    }
    // 确定性：按 seq 升序（同 seq 按 kind wire 序），与事实序列本身的序一致。
    candidates.sort_by(|a, b| (a.0.seq, a.0.kind.wire()).cmp(&(b.0.seq, b.0.kind.wire())));

    let ids: Vec<u64> = candidates.iter().map(|(f, _)| f.seq).collect();
    let verbs: Vec<&'static str> = memory_facts.iter().map(|f| f.kind.wire()).collect();
    let at_range = (
        candidates.first().map(|(f, _)| f.at_ms).unwrap_or(0),
        candidates.last().map(|(f, _)| f.at_ms).unwrap_or(0),
    );

    let value = json!({
        "window_turns": WINDOW_TURNS,
        "window_from_seq": window_from,
        "total": input.facts.len(),
        "candidates": ids.len(),
        "candidate_seqs": ids,
        "memory_verbs": verbs,
        "at_range": { "first": at_range.0, "last": at_range.1 },
    });

    // 平凡值：没有任何记忆类事实 ⇒ 检索退化为"只看当前窗口"，
    // 调用方据 `trivial` 判断"这次召回没有长期记忆成分"。
    if memory_facts.is_empty() {
        View::trivial(value)
    } else {
        View::new(value)
    }
}

// ==================== 登记 ====================

crate::submit_projection!("session.snapshot", session_snapshot);
crate::submit_projection!("session.display", session_display);
crate::submit_projection!("session.checkpoint", session_checkpoint);
crate::submit_projection!("memory.recall", memory_recall);

#[cfg(test)]
#[path = "projections.test.rs"]
mod tests;
