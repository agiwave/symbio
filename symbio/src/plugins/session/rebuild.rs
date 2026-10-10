//! 重建引擎 —— 事件网格 → `ChatMessage` 节点树（[ADR-049](../../../../docs/decisions/session.md#adr-049-wal-是会话唯一事实源sessionstore-降为缓存轨-s-的起点) 的读侧真源）。
//!
//! ## 为什么需要它
//!
//! 轨 S 的目标态是「`v2-events.wal` 是会话消息的唯一事实源，`messages.json` 降为
//! 可重建的缓存」。读侧要给出前端与轮次窗口看的那份**图语义**消息列表，就必须有人
//! 把**事实**（事件）折回**节点树**。本模块就是那个人。
//!
//! 它与 `symbio_core::projection::transcript` 是**两个出口、同一份事实**：
//!
//! | 出口 | 形态 | 消费者 |
//! |---|---|---|
//! | `transcript()` | 扁平三角色（`role` / `text` / `tool`） | 组 prompt（模型只要对话内容） |
//! | 本模块 | `ChatMessage` 节点树（`id` / `parent_id` / `type` / `status` / `seq`） | 前端列表、轮次窗口、会话叶子 |
//!
//! 两个出口**都不加工内容**——它们只是同一批事实的两种投影。任何「为了好看」的
//! 增删都会让模型或前端看到网格里没有的东西（`prompt_fidelity` 报的就是这个）。
//!
//! ## 形状来源：不在这里定义
//!
//! **写入侧怎么构造，重建就怎么构造**——同一个节点只有一份形状定义。生产侧已经在
//! 哪里定义了它，就复用那一个：
//!
//! | 节点 | 形状的单点定义 | 重建怎么拿到 |
//! |---|---|---|
//! | Turn / Reasoning / Text / ToolCall / 工具结果 | [`super::message_build`] 的 `llm_build_*` | 直接调用 |
//! | 轮边界汇报 | [`super::round_hook::ReportDraft::to_message`]（`dialog_node`） | 直接调用 |
//! | 用户发言 / 补充 | **写入侧内联构造**（CLI 请求体 / `v2_exec::supplement_into_request`） | 逐字复刻（无共享构造器） |
//!
//! 「逐字复刻」那两格是**已知的脆弱点**：没有共享构造器可调，只能靠判据钉住
//! （`rebuild.test.rs` 的形状复刻用例）。曾经在这里自作主张给用户消息补 `status`
//! ——落库那份没有，于是同一条发言在实时面与重建面是两个身份，前端的节点合并键
//! （把 `status` 算在内）当场失效。**重建不得"改进"形状**：改进属写侧，重建只负责
//! 把发生过的事原样折回来（ADR-049 的零行为变更）。
//!
//! **唯一允许的覆写是 `timestamp`**：构造器取「此刻」（`clock_now_ms`），而重建必须
//! 是**纯函数**（同一事件序列 ⇒ 逐字相同的输出，N1）。故本模块一律把时间戳覆写成
//! 事件自身的 `ts`——它才是「这件事发生在什么时候」的权威答案。
//!
//! ## 载荷里的节点结构（S2.6 铺的路）
//!
//! 收束 / 进展 / 兜底三种格子共用一份产出形状（`turn_runner::produced_payload`），
//! 载荷里带 `turn_id` / `text_id` / `reasoning_id` / `reasoning` / `tool_calls[]` /
//! `rounds[]`；`artifact.added` 带 `tool_call_id` / `wire_id` / `args` / `result_id` /
//! `success` / `turn_id`；`turn.supplemented` 带 `id` / `ids`。**节点身份因此是
//! 事实的一部分**，重建不必凭空造号——只有「载荷确实没记的」（旧事件的补充格、
//! 恢复路的结果节点）才由事件 id 稳定合成（合成必须**稳定**，见 [`synthetic_id`]）。
//!
//! ## `seq`：重建的顺序锚点
//!
//! 重建按**发射顺序**发号（单调递增），因此同一事件序列恒得同一组 `seq`。
//! 它与存储分配的号**不是同一套**——ADR-049 已写明切源后「`seq` 分配」属
//! 呈现细节，可不同。前端只需一个稳定且单调的锚点，不需要它是存储号。

use std::collections::HashSet;

use crate::symbio_core::chat_message::{
    ChatMessage, MessageContent, MessageRole, MessageStatus, MessageType,
};
use crate::symbio_core::{
    Entity, Event, Verb, EVENT_ARTIFACT_ADDED, EVENT_ASSISTANT_FALLBACK, EVENT_ASSISTANT_FINAL,
    EVENT_ASSISTANT_PROGRESS, EVENT_ASSISTANT_REPORTED, EVENT_TURN_SUPPLEMENTED,
    EVENT_USER_MESSAGE,
};

use super::message_build::{llm_build_text_node, llm_build_tool_call_node, llm_build_tool_message};

/// 从事件序列重建会话消息节点树（**纯函数**：同输入 ⇒ 逐字同输出）。
///
/// 顺序 = 事件的发射顺序；`seq` 按发射顺序单调发号。同 id 的节点只保留**首次**
/// 出现的那一份——收束载荷的 `tool_calls[]` 与 `artifact.added` 记的是同一批调用，
/// 只认先到的那条（它们形状相同，但结果正文只归 `artifact.added`，见模块文档）。
pub fn rebuild(events: &[Event]) -> Vec<ChatMessage> {
    let mut out: Vec<ChatMessage> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut seq: i64 = 0;

    for e in events {
        for mut m in nodes_of(e) {
            // 同 id 只认首次：同一批调用在两个载荷里各记一次（收束的 `tool_calls[]`
            // 是**引用**，结果正文归 `artifact.added`）。丢后者不丢信息——先到的那条
            // 已经带着结果节点 id。
            if !seen.insert(m.id.clone()) {
                continue;
            }
            seq += 1;
            m.seq = Some(seq);
            out.push(m);
        }
    }
    out
}

/// 一条事件 → 它承载的节点（顺序即树内顺序）。
fn nodes_of(e: &Event) -> Vec<ChatMessage> {
    match (e.entity, e.verb, e.kind.as_str()) {
        (Entity::Turn, Verb::Opened, EVENT_USER_MESSAGE) => vec![user_node(e)],
        (Entity::Turn, Verb::Asserted, EVENT_TURN_SUPPLEMENTED) => vec![supplement_node(e)],
        // 收束 / 进展 / 兜底：三种格子共用一份产出形状（`produced_payload`），
        // 差别只在 `status`——它们是**同一批产出事实**的三种状态。
        (
            Entity::Turn,
            Verb::Closed | Verb::Progressed,
            EVENT_ASSISTANT_FINAL | EVENT_ASSISTANT_FALLBACK | EVENT_ASSISTANT_PROGRESS,
        ) => produced_nodes(e),
        // 轮边界汇报：界面文本，进列表（用户看得见），但不是模型的对话内容。
        (Entity::Turn, Verb::Asserted, EVENT_ASSISTANT_REPORTED) => vec![reported_node(e)],
        // 工具结果：调用与结果两张卡都归这一格（一个事实一个 owner）。
        (Entity::Artifact, Verb::Asserted, EVENT_ARTIFACT_ADDED) => artifact_nodes(e),
        _ => Vec::new(),
    }
}

/// 用户发言（`turn × opened`）：载荷 `{ text, tier, id }`。
///
/// `id` 是**流式期已广播的节点 id**（`user_node_id` 入格）——前端按 id 合并，
/// 重建必须复用它，否则同一条发言在实时面与重建面是两个身份。
///
/// ## 为什么不设 `status`（与"用户发言是完成态"的直觉相反）
///
/// 形状必须**逐字复刻落库那一份**，而落库的用户消息由调用方给（CLI 的
/// `session/chat/send` 请求体，`..Default::default()`）——它**没有** `status`。
/// 重建若自作主张补一个 `Completed`，同一条发言在实时面（无 status）与重建面
/// （`completed`）就是两个身份：前端的节点合并键把 `status` 算在内
/// （`useChatConnection.ts` 的 `own` 键），合并当场失效、历史里冒出重复节点。
/// 「用户发言算不算完成态」是一个可以单独讨论的形状改进，但它属于**写侧**，
/// 不归重建——重建只负责把发生过的事原样折回来（ADR-049 的零行为变更）。
fn user_node(e: &Event) -> ChatMessage {
    ChatMessage {
        id: node_id(e, "id"),
        role: Some(MessageRole::User),
        msg_type: Some(MessageType::Text),
        content: Some(MessageContent::Text(text_of(e, "text"))),
        timestamp: Some(e.ts),
        ..Default::default()
    }
}

/// 轮边界折进的补充（`turn × asserted`，缺口 3）：载荷 `{ text, count, id, ids }`。
///
/// 形状**逐字复刻落库那一份**（`v2_exec::supplement_into_request` 的 `ChatMessage`）：
/// 它**没有** `msg_type`、**没有** `status`，但**有** `meta.supplement*` 三键
/// （用户消息的合并标记，前端据它认出「这是一次折进」）。重建必须三样都对上，
/// 否则同一条补充在实时面与重建面是两个节点。
///
/// `id` 与 `ids` 由**载荷**给出（S2.6 的原则：节点身份是事实的一部分）。载荷缺 `id`
/// （旧事件）时按事件 id 稳定合成——合成的后果已判定可接受：重开后这条的 id 与
/// 实时面那份不同，但**内容与角色一致**（ADR-049 的「呈现细节可能不同」）。
fn supplement_node(e: &Event) -> ChatMessage {
    use super::transcript::supplements::{
        META_SUPPLEMENT, META_SUPPLEMENT_COUNT, META_SUPPLEMENT_IDS,
    };
    let count = e.payload.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
    let ids = e
        .payload
        .get("ids")
        .cloned()
        .unwrap_or_else(|| serde_json::json!([]));
    ChatMessage {
        id: node_id(e, "id"),
        role: Some(MessageRole::User),
        content: Some(MessageContent::Text(text_of(e, "text"))),
        meta: Some(serde_json::json!({
            META_SUPPLEMENT: true,
            META_SUPPLEMENT_COUNT: count,
            META_SUPPLEMENT_IDS: ids,
        })),
        timestamp: Some(e.ts),
        ..Default::default()
    }
}

/// 轮边界汇报（`turn × asserted`，缺口 4）：界面文本。
///
/// 形状**复用写入侧的构造** [`ReportDraft::to_message`](super::round_hook::ReportDraft::to_message)
/// ——它就是落库镜像上那一格的定义（`dialog_node(text, "progress", true)`）。
/// 复抄一遍的代价是已知的：上一版落库侧抄漏 `msg_type`，节点落了库却不成形状；
/// 重建这边再抄一遍，迟早各错各的。`meta.exclude_from_context` 因此随构造一并得到
/// ——它是**前端与轮次窗口**读的判别位，缺了轮次窗口会把「给人看的汇报」当成模型的
/// 对话内容发进请求包。
///
/// `id` 由载荷给（缺则按事件 id 合成，同 [`supplement_node`]）；`timestamp` 覆写成
/// 事件自身的 `ts`（构造器取「此刻」会破坏纯函数性，见模块头）。
fn reported_node(e: &Event) -> ChatMessage {
    let draft = super::round_hook::ReportDraft {
        text: text_of(e, "text"),
        tool_rounds: e
            .payload
            .get("tool_rounds")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize,
        quiet_ms: e
            .payload
            .get("quiet_ms")
            .and_then(|v| v.as_i64())
            .unwrap_or(0),
    };
    let mut m = draft.to_message();
    m.id = node_id(e, "id");
    m.timestamp = Some(e.ts);
    m
}

/// 收束 / 进展 / 兜底格 → `[Turn 根, Reasoning 子?, Text 子?, rounds 的中途产出…]`。
///
/// 形状与 [`super::message_build::llm_build_assistant_messages`] 同源（同一判据：
/// reasoning-only 时那段内容归 Text 子节点，不另立 Reasoning 子，否则同一段内容
/// 在存储层出现两份）。
fn produced_nodes(e: &Event) -> Vec<ChatMessage> {
    let Some(turn_id) = str_of(e, "turn_id") else {
        return Vec::new();
    };
    let status = status_of(e);
    let text = text_of(e, "text");
    let reasoning = text_of(e, "reasoning");
    let mut nodes = Vec::new();

    // Turn 是组合节点（仅分组、无正文）：与 `llm_build_assistant_messages` 同形。
    nodes.push(ChatMessage {
        id: turn_id.clone(),
        role: Some(MessageRole::Assistant),
        msg_type: Some(MessageType::Turn),
        status: Some(status.clone()),
        timestamp: Some(e.ts),
        ..Default::default()
    });

    // reasoning-only 的判据与落库侧**逐字相同**（`message_build::llm_build_assistant_messages`）：
    // 思考与正文是同一段内容且无独立文本回复时，内容归 Text 子节点。
    let reasoning_only = !reasoning.trim().is_empty() && reasoning.trim() == text.trim();

    if !reasoning.trim().is_empty() && !reasoning_only {
        nodes.push(ChatMessage {
            id: str_of(e, "reasoning_id").unwrap_or_else(|| format!("{turn_id}-reasoning")),
            parent_id: Some(turn_id.clone()),
            role: Some(MessageRole::Assistant),
            msg_type: Some(MessageType::Reasoning),
            content: Some(MessageContent::Text(reasoning)),
            status: Some(status.clone()),
            timestamp: Some(e.ts),
            ..Default::default()
        });
    }

    if !text.trim().is_empty() {
        let child_id = if reasoning_only {
            str_of(e, "reasoning_id").or_else(|| str_of(e, "text_id"))
        } else {
            str_of(e, "text_id")
        }
        .unwrap_or_else(|| format!("{turn_id}-text"));
        // 形状经 `llm_build_text_node`（单点定义），只覆写「此刻」为事件时刻。
        let mut m = llm_build_text_node(&turn_id, &text, None, child_id);
        m.timestamp = Some(e.ts);
        m.status = Some(status.clone());
        nodes.push(m);
    }

    // 工具轮的**中途产出**（每轮正文 / 思考）：`rounds[]` 各带自己的节点 id。
    // 它们与末轮产出同为 Turn 根的子节点（`v2_tools` 把中途正文定格在同一个根下）。
    if let Some(rounds) = e.payload.get("rounds").and_then(|v| v.as_array()) {
        for (i, r) in rounds.iter().enumerate() {
            push_round_nodes(&mut nodes, &turn_id, i, r, &status, e.ts);
        }
    }

    nodes
}

/// 一轮中途产出（`rounds[i]`）→ 最多两个子节点（Reasoning / Text）。
fn push_round_nodes(
    nodes: &mut Vec<ChatMessage>,
    turn_id: &str,
    i: usize,
    r: &serde_json::Value,
    status: &MessageStatus,
    ts: i64,
) {
    let get = |k: &str| {
        r.get(k)
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(str::to_string)
    };
    let text = get("text").unwrap_or_default();
    let reasoning = get("reasoning").unwrap_or_default();
    let reasoning_only = !reasoning.trim().is_empty() && reasoning.trim() == text.trim();

    if !reasoning.trim().is_empty() && !reasoning_only {
        nodes.push(ChatMessage {
            id: get("reasoning_id").unwrap_or_else(|| format!("{turn_id}-r{i}")),
            parent_id: Some(turn_id.to_string()),
            role: Some(MessageRole::Assistant),
            msg_type: Some(MessageType::Reasoning),
            content: Some(MessageContent::Text(reasoning)),
            status: Some(status.clone()),
            timestamp: Some(ts),
            ..Default::default()
        });
    }
    if !text.trim().is_empty() {
        let id = if reasoning_only {
            get("reasoning_id").or_else(|| get("text_id"))
        } else {
            get("text_id")
        }
        .unwrap_or_else(|| format!("{turn_id}-t{i}"));
        let mut m = llm_build_text_node(turn_id, &text, None, id);
        m.timestamp = Some(ts);
        m.status = Some(status.clone());
        nodes.push(m);
    }
}

/// 工具格（`artifact × asserted`）→ `[ToolCall 子, 结果子]`。
///
/// 载荷自带两张卡的全部身份：`tool_call_id`（ToolCall 节点 id）/ `wire_id`
/// （provider 的调用 id）/ `args`（**已定稿的参数文本**）/ `result_id`（结果节点 id）/
/// `success`。参数走 `llm_build_tool_call_node` 的文本入口——**不经 `Value` 重序列化**
/// （键序一变，同一次调用在重建前后就是两个内容，见该函数的文档）。
fn artifact_nodes(e: &Event) -> Vec<ChatMessage> {
    let Some(turn_id) = str_of(e, "turn_id") else {
        return Vec::new();
    };
    let call_id = str_of(e, "tool_call_id").unwrap_or_else(|| format!("{turn_id}-call"));
    let success = e
        .payload
        .get("success")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let mut call = llm_build_tool_call_node(
        &turn_id,
        call_id.clone(),
        Some(text_of(e, "tool")),
        text_of(e, "args"),
        str_of(e, "wire_id"),
    );
    call.timestamp = Some(e.ts);
    call.status = Some(MessageStatus::Completed);

    // 结果节点 id：载荷有就用；恢复路记 `null` ⇒ 按事件 id 稳定合成（同一条事件
    // 每次重建得同一个 id）。合成用的**不是** call_id——结果与调用是两张卡，
    // 共用 id 会让前端把它们合成一条。
    let result_id = str_of(e, "result_id").unwrap_or_else(|| format!("{}-result", e.event_id));
    let mut result = llm_build_tool_message(
        &call_id,
        &text_of(e, "text"),
        Some(success),
        Some(result_id),
    );
    result.timestamp = Some(e.ts);

    vec![call, result]
}

// ── 载荷取值（缺失一律静默给默认值：重建必须对**任意**历史事件容错）────────────

/// 取非空字符串字段。
fn str_of(e: &Event, key: &str) -> Option<String> {
    e.payload
        .get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// 取字符串字段（允许空串——正文可以是空串，那是「答了空话」的事实）。
fn text_of(e: &Event, key: &str) -> String {
    e.payload
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string()
}

/// 节点 id：载荷字段优先；缺失（该格不记节点身份）⇒ 按事件 id 稳定合成。
///
/// 「稳定」是硬要求：同一条事件每次重建必须得同一个 id，否则每次重开会话都会
/// 多出一批「新」节点（前端按 id 合并）。
fn node_id(e: &Event, key: &str) -> String {
    str_of(e, key).unwrap_or_else(|| synthetic_id(e))
}

/// 该格不记节点身份时的合成 id：**事件 id 本身就是最稳的那个键**——它由写方按
/// 「一件事一个键」的规则给出（`s-{turn}-{n}` / `a-{turn}-{n}`），同一条事件恒得同值。
fn synthetic_id(e: &Event) -> String {
    e.event_id.clone()
}

/// 载荷 `status` → 消息状态（未知 / 缺失按 `Completed`——网格里能落格的产出，
/// 默认语义就是「发生过且结束了」）。
fn status_of(e: &Event) -> MessageStatus {
    match e.payload.get("status").and_then(|v| v.as_str()) {
        Some("aborted") => MessageStatus::Aborted,
        Some("failed") => MessageStatus::Failed,
        Some("pending") => MessageStatus::WaitingUserAction,
        _ => MessageStatus::Completed,
    }
}

#[cfg(test)]
#[path = "rebuild.test.rs"]
mod tests;
