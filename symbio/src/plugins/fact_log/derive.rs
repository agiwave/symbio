//! 从磁盘派生事实 —— 本插件的**全部机制**，全是纯函数。
//!
//! ## 为什么是纯函数
//!
//! 事实必须**可双跑比对**（断言 A4）：同一份磁盘 → 同一串事实，逐字节相同。
//! 纯函数是这条性质的最简实现——没有时钟、没有随机、没有全局状态。
//! `at_ms` 取自消息自己的 `timestamp`（磁盘上已有），**不调 `clock_now_ms`**。
//!
//! ## seq 分配：确定性、全局单调、可重入
//!
//! `seq` 必须跨会话全局唯一且单调。做法是把 `(会话序号, 消息 seq)` 编码进一个
//! `u64`：
//!
//! ```text
//! seq = session_ordinal * SESSION_STRIDE + local_seq
//! ```
//!
//! - `session_ordinal`：会话按 id 字典序排序后的位次（**确定性**，不依赖目录
//!   枚举顺序）；
//! - `local_seq`：消息在会话内的 `seq`（存储写入时分配，单调且不并列）；
//! - `SESSION_STRIDE`：留给单会话的最大消息数，取 `2^40`——远超任何真实会话，
//!   因此**同一会话内不会溢出到别人的区间**。
//!
//! 于是"seq 严格递增"在**派生结果内**成立（按 `(session_ordinal, local_seq)` 输出），
//! 且**同一份输入永远得到同一个 seq**（可双跑）。
//!
//! ## 溯源（I2）
//!
//! - 助手消息 → 指向**同一会话内最近一条用户消息**（它是对该轮的应答）；
//! - 工具调用/结果 → 指向**同一会话内最近一条助手消息**（它是被调用的起因）；
//! - 用户消息 / 轮次状态 → 无前驱。
//!
//! 断言类事实（本阶不产出，留给记忆/审查阶）由 [`FactKind::is_assertion`] 标注。

use crate::symbio_core::schemas::session::chat_message::{ChatMessage, MessageRole, MessageType};
use crate::symbio_core::{Fact, FactKind, FactPrincipal};
use serde_json::json;

/// 单会话在全局 seq 空间里的步长（`2^40` ≈ 1.1 万亿条消息 / 会话）。
pub const SESSION_STRIDE: u64 = 1 << 40;

/// 一个会话的原始输入：id + 消息列表（**按 seq 升序**，调用方保证）。
pub struct SessionFactsInput<'a> {
    pub session_id: &'a str,
    /// 全局会话位次（决定 seq 高位；由 [`derive_facts`] 按 id 排序算出）
    pub session_ordinal: u64,
    pub messages: &'a [ChatMessage],
}

/// 把若干会话的消息**纯函数式**派生为事实序列。
///
/// 输出按 `(session_ordinal, local_seq)` 升序，`seq` 严格递增。
/// `max_facts` 为 0 表示不限制；超限则**截断**（调用方据 `facts.len()` 判是否截断）。
///
/// **确定性保证**：输入相同 → 输出逐字节相同（`session_ordinal` 由 `inputs` 的
/// 排列决定，调用方应先按 `session_id` 排序，见 [`sort_inputs`]）。
pub fn derive_facts(inputs: &[SessionFactsInput<'_>], max_facts: usize) -> Vec<Fact> {
    let mut out: Vec<Fact> = Vec::new();
    for input in inputs {
        let base = input.session_ordinal.saturating_mul(SESSION_STRIDE);

        // 会话内两个"最近锚点"：用于给后续事实填溯源（I2）。
        let mut last_user_seq: Option<u64> = None;
        let mut last_assistant_seq: Option<u64> = None;

        for msg in input.messages {
            let local = match msg.seq {
                Some(s) if s > 0 => s as u64,
                // 无 seq 的消息（理论上不该出现在存储里）跳过，不猜号——
                // 猜号会破坏"确定性"，而确定性是事实的全部价值。
                _ => continue,
            };
            let seq = base.saturating_add(local);

            let Some(kind) = kind_of(msg) else {
                continue;
            };

            let caused_by = match kind {
                // 助手发言是对"最近一条用户消息"的应答
                FactKind::TurnAssistantFinal | FactKind::TurnAssistantFallback => last_user_seq,
                // 工具调用/结果是对"最近一条助手消息"的应答
                FactKind::ArtifactAdded => last_assistant_seq,
                _ => None,
            };

            out.push(Fact {
                seq,
                kind,
                principal: FactPrincipal::new(input.session_id),
                caused_by,
                at_ms: msg.timestamp.unwrap_or(0),
                payload: payload_of(msg),
            });

            // 更新锚点（供后续事实溯源）
            match kind {
                FactKind::TurnUserMessage => last_user_seq = Some(seq),
                FactKind::TurnAssistantFinal | FactKind::TurnAssistantFallback => {
                    last_assistant_seq = Some(seq)
                }
                _ => {}
            }
        }
    }

    // 排序保证 seq 严格递增（确定性）：按 seq 升序，seq 相同则按 kind 序稳定排列
    out.sort_by_key(|f| (f.seq, f.kind.wire()));

    if max_facts > 0 && out.len() > max_facts {
        out.truncate(max_facts);
    }
    out
}

/// 按 `session_id` 字典序排序输入，并就地分配 `session_ordinal`。
///
/// **调用方必须先用它**，否则 `seq` 会随目录枚举顺序漂移，破坏可双跑性。
pub fn sort_inputs<'a>(inputs: &mut [SessionFactsInput<'a>]) {
    inputs.sort_by(|a, b| a.session_id.cmp(b.session_id));
    for (i, input) in inputs.iter_mut().enumerate() {
        input.session_ordinal = i as u64 + 1; // 从 1 起：0 保留给"无前驱"
    }
}

/// 消息 → 事实类型（`None` = 该消息不产生事实）。
///
/// 映射是**保守**的：只把"确实发生了某个可命名事实"的消息转成事实，
/// 其余（如占位节点、reasoning 片段）跳过——宁可少，不可错。
fn kind_of(msg: &ChatMessage) -> Option<FactKind> {
    // `role` / `msg_type` / `status` 都只实现了 `Clone`（非 `Copy`），
    // 且 `msg` 是**借用**——因此一律用 `as_ref()` 取引用后再 `match`，
    // 绝不把它们移出。这个约束恰是事实层"只读观察"边界的技术体现。
    let ty = msg.msg_type.as_ref();
    let role = msg.role.as_ref();

    match (role, ty) {
        (Some(MessageRole::User), _) => Some(FactKind::TurnUserMessage),
        (Some(MessageRole::Assistant), Some(MessageType::ToolCall)) => {
            Some(FactKind::ArtifactAdded)
        }
        (Some(MessageRole::Tool), _) => Some(FactKind::ArtifactAdded),
        (Some(MessageRole::Assistant), _) => {
            // 助手终态：失败 = fallback（到点必答的兜底），其余 = final
            match msg.status.as_ref() {
                Some(crate::symbio_core::schemas::session::chat_message::MessageStatus::Failed) => {
                    Some(FactKind::TurnAssistantFallback)
                }
                _ => Some(FactKind::TurnAssistantFinal),
            }
        }
        _ => None,
    }
}

/// 消息 → 载荷（**沿用既有 schema**，不新建第二套结构）。
///
/// 只取事实需要的字段，避免把整条消息（含大正文）塞进事实——事实是"索引"，
/// 不是"存储的副本"。
fn payload_of(msg: &ChatMessage) -> serde_json::Value {
    // 全部字段**借用**后取值（`msg` 只读）：`as_ref()` 用于枚举、`&` 用于 String。
    // `json!` 会对传入的引用自动序列化，因此 `&String` / `Option<&_>` 都可直接给。
    let status = msg.status.as_ref().map(|s| format!("{s:?}"));
    json!({
        "message_id": &msg.id,
        "role": msg.role.as_ref().map(|r| format!("{r:?}")),
        "type": msg.msg_type.as_ref().map(|t| format!("{t:?}")),
        "name": msg.name.as_deref(),
        "status": status,
        "timestamp": msg.timestamp,
    })
}

#[cfg(test)]
#[path = "derive.test.rs"]
mod tests;
