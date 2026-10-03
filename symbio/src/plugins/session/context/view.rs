//! 请求视图层 —— 只影响**单次请求、不落库**的全部裁剪。
//!
//! 唯一入口 [`build_request_view`]：内容节点淡化 → 轮次淡化 → 分层滑窗骨架化 →
//! 水位提醒，按固定顺序执行。视图每轮从存储重建，四步天然幂等。

use super::*;
use crate::symbio_core::chat_message::{MessageContent, MessageStatus};

/// 轮次淡化（请求视图级）：当对话轮次过多时，把较早的工具结果（`role=Tool`、`msg_type=Text`）
/// 做 head/tail 摘要，保留最近 `keep_recent_turns` 个 user turn 起的原文，以及**全部**
/// assistant 文本 / 推理节点（绝不改动 assistant 消息，最大限度保护思维链）。
///
/// 只作用于传入的视图副本（由 [`build_request_view`] 每轮从存储重建），存储层保留全文，
/// 因此**不写存档文件**、天然幂等（无重复存档问题）。被淡化的结果标记 `tool_result_faded`，
/// 模型如需完整输出可重新运行对应工具。
pub fn fade_aged_tool_results(messages: &mut [ChatMessage], keep_recent_turns: usize) {
    use super::super::tokenizer::{default_tokenizer, Tokenizer};

    // 以 user 消息为边界，定位"最近 keep_recent_turns 个 user turn"的起始下标；
    // 该下标之前的消息视为"历史"，对其中的工具结果做淡化。
    let user_positions: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, m)| m.role == Some(MessageRole::User))
        .map(|(i, _)| i)
        .collect();
    let keep_from = if user_positions.len() > keep_recent_turns {
        user_positions[user_positions.len() - keep_recent_turns]
    } else {
        0
    };

    const FADE_BUDGET: usize = 2048; // 老化工具结果压到约 2k token
    let tok = default_tokenizer();
    for m in messages.iter_mut().take(keep_from) {
        if m.role == Some(MessageRole::Tool) && m.msg_type == Some(MessageType::Text) {
            if let Some(MessageContent::Text(t)) = &m.content {
                if tok.count(t) > FADE_BUDGET {
                    m.content = Some(MessageContent::Text(
                        super::super::tools::summarize_tool_result(t, FADE_BUDGET),
                    ));
                    let mut meta = m.meta.clone().unwrap_or_else(|| serde_json::json!({}));
                    meta["tool_result_faded"] = serde_json::json!(true);
                    m.meta = Some(meta);
                }
            }
        }
    }
}

/// 内容节点淡化（请求视图级）：B1 保护窗口（最后一条消息 + 最近 `keep_recent`
/// 个内容节点）之外的超大内容节点做 head/tail 摘要。
///
/// 内容节点 = User/Assistant 的正文与思考（`msg_type` 为 Text/Reasoning/None，
/// 与旧 `is_content_node` 判定一致）；role=Tool 不在此列（L0 守卫与
/// [`fade_aged_tool_results`] 的辖区），System（系统提示）永不淡化。
/// **L2 快照消息（`meta.compacted`）同样豁免**：快照本身已是 token 最小化的
/// 压缩产物，对其再切中段等于双重压缩，且切掉的恰是模型的唯一深历史记忆。
///
/// 触发条件（与旧存档压缩门控同构）：行数超 `line_threshold` **或** token 超
/// [`MESSAGE_TOKEN_CAP`]。行数触发走按行 head/tail 切分（[`line_head_tail`]），
/// token 触发走 [`super::super::tools::summarize_head_tail`]（与工具淡化同一机制本体，仅占位文案
/// 不同：内容节点无法"重新运行"，指向会话存储中的完整原文）。
///
/// 只作用于传入的视图副本（由 [`build_request_view`] 每轮从存储重建），存储层
/// 保留完整原文，因此**不写存档文件**、天然幂等。被淡化的节点标记 `content_faded`。
pub fn fade_aged_content_nodes(
    messages: &mut [ChatMessage],
    keep_recent: usize,
    line_threshold: usize,
) {
    use super::super::tokenizer::{default_tokenizer, Tokenizer};

    // 内容节点判定：正文与思考（含 msg_type 缺省的历史消息）。
    // L2 快照（meta.compacted）豁免：它已是压缩产物，再切 = 双重压缩。
    fn is_content_node(m: &ChatMessage) -> bool {
        if m.meta
            .as_ref()
            .and_then(|meta| meta.get("compacted"))
            .and_then(|v| v.as_bool())
            == Some(true)
        {
            return false;
        }
        !matches!(m.role, Some(MessageRole::Tool) | Some(MessageRole::System))
            && matches!(
                m.msg_type,
                None | Some(MessageType::Text) | Some(MessageType::Reasoning)
            )
    }

    // B1 保护窗口：最后一条消息恒保护 + 最近 keep_recent 个内容节点保持原样
    // （思维连续性：近期推理/正文必须完整，否则模型"失忆"刚说的话）。
    let len = messages.len();
    let mut protected = vec![false; len];
    if len > 0 {
        protected[len - 1] = true;
    }
    let mut kept = 0usize;
    for i in (0..len).rev() {
        if kept >= keep_recent {
            break;
        }
        if !protected[i] && is_content_node(&messages[i]) {
            protected[i] = true;
            kept += 1;
        }
    }

    let tok = default_tokenizer();
    for (i, m) in messages.iter_mut().enumerate() {
        if protected[i] || !is_content_node(m) {
            continue;
        }
        if let Some(MessageContent::Text(t)) = &m.content {
            let over_tokens = tok.count(t) > MESSAGE_TOKEN_CAP;
            let over_lines = t.lines().count() > line_threshold;
            if !(over_tokens || over_lines) {
                continue;
            }
            let faded = if over_tokens {
                super::super::tools::summarize_head_tail(
                    t,
                    MESSAGE_TOKEN_CAP,
                    "该早期内容已在请求视图中淡化以控制上下文长度，完整原文保留于会话存储。",
                )
            } else {
                line_head_tail(t, line_threshold)
            };
            m.content = Some(MessageContent::Text(faded));
            let mut meta = m.meta.clone().unwrap_or_else(|| serde_json::json!({}));
            meta["content_faded"] = serde_json::json!(true);
            m.meta = Some(meta);
        }
    }
}

/// 行数超限的按行 head/tail 切分（短行密集、token 未超预算的内容）：
/// 保留头 `line_threshold/4` 行与尾同量行，中段以省略标记占位。
/// 与 token 路径（[`super::super::tools::summarize_head_tail`]）同构：头少尾多（结论在尾部）。
fn line_head_tail(text: &str, line_threshold: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let total = lines.len();
    let side = (line_threshold / 4).max(1);
    // 行数不足以省略任何中段时不动原文（淡化必须有净收益）
    if total <= side * 2 + 2 {
        return text.to_string();
    }
    let omitted = total - side * 2;
    let mut out = lines[..side].join("\n");
    out.push_str(&format!("\n[... 已省略 {omitted} 行中段内容 ...]\n"));
    out.push_str(&lines[total - side..].join("\n"));
    out
}

/// 水位提醒文案（请求级注入，不落库）。模型不应直接回应此提示。
const CONTEXT_NUDGE_TEXT: &str =
    "[system note] Context usage is approaching the limit. If you are \
     at a natural stage boundary, call the context_compact tool now to distill older history and \
     continue seamlessly; otherwise keep working and it will be compacted automatically. Do not \
     respond to this note directly.";

/// 构建本次 LLM 请求的视图（请求视图层唯一入口）。
///
/// 存储视图（`get_context_messages`）只负责过滤与轮次窗口；一切**只影响单次请求、
/// 不落库**的裁剪都在这里按固定顺序执行：
///
/// 1. 内容节点淡化：B1 保护窗口外的超大正文/思考做 head/tail 摘要（存储保留
///    全文，阈值取会话配置 line_threshold / token 上限 2048）；
/// 2. 轮次淡化（fade）：`fade_active`（轮次超过激活阈值）时，对较早的工具结果做
///    head/tail 摘要（无存档，存储保留全文）；
/// 3. 工具级骨架化：`window > 0` 时按分层滑窗把过期调用的参数与结果替换为占位
///    文案（`retention` 为工具自声明的保留策略，可为空——为空时仍执行全局窗口，
///    ToolCall↔Tool 配对与 parent_id 传播完整保留，不会造成大模型逻辑断联）；
/// 4. 水位提醒（nudge）：`inject_nudge` 时在视图末尾追加一条一次性系统提示——
///    请求级注入、不写会话存储，因此不占用轮次窗口的 User 计数，也不会在前端
///    以用户消息的形式出现。
///
/// 视图每轮从存储重建，四个步骤天然幂等，不存在重复存档 / 重复注入问题。
/// 全部压缩由此统一收敛于"发给大模型之前"（写入时压缩已废除，落库恒为原文）。
// 8 个参数均为单一调用点（chat_loop）传入的独立语义旋钮，强行打包成
// config struct 只会多一层间接而无行为收益，故显式豁免 clippy 参数数上限。
#[allow(clippy::too_many_arguments)]
pub fn build_request_view(
    messages: &[ChatMessage],
    window: usize,
    retention: &std::collections::HashMap<
        String,
        crate::symbio_core::CapabilityToolContextRetention,
    >,
    fade_active: bool,
    fade_keep_turns: usize,
    content_keep_recent: usize,
    line_threshold: usize,
    inject_nudge: bool,
) -> Vec<ChatMessage> {
    let mut view = messages.to_vec();
    fade_aged_content_nodes(&mut view, content_keep_recent, line_threshold);
    if fade_active {
        fade_aged_tool_results(&mut view, fade_keep_turns);
    }
    if window > 0 {
        view = super::window::apply_layered_sliding_window(&view, window, retention);
    }
    if inject_nudge {
        view.push(ChatMessage {
            id: uuid::Uuid::new_v4().to_string(),
            role: Some(MessageRole::User),
            msg_type: Some(MessageType::Text),
            content: Some(MessageContent::Text(CONTEXT_NUDGE_TEXT.to_string())),
            status: Some(MessageStatus::Completed),
            meta: Some(serde_json::json!({ "kind": "context_nudge" })),
            ..Default::default()
        });
    }
    view
}

#[cfg(test)]
#[path = "view.test.rs"]
mod tests;
