//! 会话上下文压缩服务
//!
//! 职责：
//! - 检测何时需要压缩会话历史
//! - 准备压缩指令（在当前会话内处理，不递归）
//! - 触发 PreCompact Hook 事件

use std::sync::Arc;

use crate::symbio_core::schemas::session::chat_message::{
    ChatMessage, MessageContent, MessageRole, MessageStatus, MessageType,
};
use crate::symbio_core::{InvokeRequest, InvokeRequestExt};

const COMPRESSION_TOKEN_THRESHOLD: f64 = 0.7;
const COMPRESSION_PRESERVE_THRESHOLD: f64 = 0.3;
const MIN_COMPRESSION_FRACTION: f64 = 0.05;

/// 单个内容节点的 token 预算：行数阈值之外的第二触发条件，也是 token 路径的
/// 淡化预算。单行长 JSON/URL/base64（1 行但数万 token）必须触发，否则绕过防线。
pub const MESSAGE_TOKEN_CAP: usize = 2048;

/// 主动压缩工具：水位提醒阈值（相对有效上限）。
pub const CONTEXT_NUDGE_THRESHOLD: f64 = 0.55;
/// 迟滞系数：快照后估算再增长不足 (post_tokens × 1.15) 时跳过被动压缩。
pub const COMPACT_HYSTERESIS_FACTOR: f64 = 1.15;
/// 主动压缩最小收益：待压缩历史低于该 token 数时不值得一次 LLM 调用。
pub const MIN_COMPACT_TOKENS: usize = 4000;

/// 压缩协议版本（P2-3）：快照 meta 记录此版本，用于从产物侧验证协议演进是否生效。
/// 语义：v2 = L0 会话目录存档 + 三层统一取回协议 + JSON 语义摘要 + 快照指纹。
pub const COMPRESSION_PROTOCOL_VERSION: &str = "v2";

/// 当前压缩提示词的指纹（FNV-1a 64，取高 32 位十六进制）。
///
/// 提示词文本变更 → 指纹变更 → 新快照 meta 可观测；用于回答
/// "提示词强化是否生效"——对比历史快照 meta 即可确认该轮压缩用的提示词版本。
pub fn compression_prompt_fingerprint() -> String {
    fn fnv1a(data: &[u8]) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in data {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        h
    }
    let prompt = get_compression_prompt();
    format!("{:08x}", (fnv1a(prompt.as_bytes()) >> 32) as u32)
}

/// 获取压缩提示词
pub fn get_compression_prompt() -> String {
    r#"You are the component that summarizes internal chat history into a given structure.

When the conversation history grows too large, you will be invoked to distill the entire history into a concise, structured XML snapshot. This snapshot is CRITICAL, as it will become the agent's *only* memory of the past. The agent will resume its work based solely on this snapshot. All crucial details, plans, errors, and user directives MUST be preserved.

First, you will think through the entire history in a private <scratchpad>. Review the user's overall goal, the agent's actions, tool outputs, file modifications, and any unresolved questions. Identify every piece of information that is essential for future actions.

After your reasoning is complete, generate the final <state_snapshot> XML object. Be incredibly dense with information. Omit any irrelevant conversational filler.

Signal-to-noise rules (apply while writing the snapshot):
- Reconcile before output: a previous snapshot found in the history is unverified INPUT, not ground truth. When newer findings contradict an inherited item, correct or drop it; never copy old <key_knowledge> forward unchecked — an error that enters a snapshot survives every later generation.
- Each fact appears exactly once across ALL sections. If the same fact fits multiple sections, place it in the most relevant one and do not repeat it.
- Record conclusions and outcomes, not process metrics. Drop line numbers, byte counts, read ranges, raw dumps, and step-by-step command transcripts; keep the final state and what it implies for future work.
- <key_knowledge> holds only facts and constraints that still bind FUTURE work. Drop test-writing trivia, closed-issue aftermath notes, and volatile numbers (test counts, file sizes); state invariants instead ("full suite + clippy clean", not a passing count).
- <completed_items>: one line per item — the outcome and where it landed. If an item leaves a lasting constraint, that constraint belongs in <key_knowledge>; the how-it-was-done narrative stays in the transcript.
- Compress each error to ONE line: the conclusion plus its root cause. Keep errors ONLY if they still constrain future actions (an unresolved failure, a known pitfall); drop errors that were already fixed and whose fix is recorded in <completed_items>.
- Before listing a question in <open_questions>, if it can be verified with a single cheap tool call (reading a file, running a quick command), perform that verification during this compaction and record the confirmed answer instead.
- If a todo list exists in the conversation, reference its item IDs/titles in <in_progress_items> instead of restating full descriptions; the agent retains live access to the list.

The structure MUST be as follows:

<state_snapshot>
    <overall_goal>
        A single, concise sentence describing the user's high-level objective.
    </overall_goal>

    <key_knowledge>
        Crucial facts, conventions, and constraints the agent must remember based on the conversation history and interaction with the user. Use bullet points.
    </key_knowledge>

    <completed_items>
        Items that have been completed, including:
        - Files created, modified, or deleted
        - Commands executed and their results
        - User approvals or confirmations received
        - Problems solved or resolved
    </completed_items>

    <in_progress_items>
        Items currently in progress or pending completion.
    </in_progress_items>

    <open_questions>
        Questions that remain unanswered or issues that need to be addressed.
    </open_questions>
</state_snapshot>"#.to_string()
}

/// 找到压缩分割点（仅在 User 边界切分，保证保留区从一轮对话的开头开始）。
///
/// 返回值语义：
/// - `> 0`：`messages[..n]` 为待压缩历史，`messages[n..]` 为保留区；
/// - `0`：找不到安全切分点，本轮放弃压缩。
///
/// 安全规则：
/// 1. **禁止全量压缩**：旧版"末尾是 Assistant 就压缩全部"分支已移除——它违背
///    30% 保留语义，且会把进行中的 tool_calls 压成孤儿（结果子节点失去父节点）。
/// 2. 尾部若存在未配对的 ToolCall（结果子节点尚未落库，典型于 continuation 场景），
///    任何切分都可能拆散配对，返回 0 放弃本轮压缩。
fn find_compress_split_point(messages: &[ChatMessage], fraction: f64) -> usize {
    if fraction <= 0.0 || fraction >= 1.0 || messages.is_empty() {
        return 0;
    }

    // 尾部未配对 ToolCall 检查：从末尾往前扫，遇到 Tool 结果即配对完成；
    // 若先遇到 Assistant 的 ToolCall 节点（其结果不在其后），说明有悬空调用。
    let mut pending_tool_call = false;
    for m in messages.iter().rev() {
        match m.role {
            Some(MessageRole::Tool) => {
                pending_tool_call = false;
                break;
            }
            Some(MessageRole::Assistant) if m.msg_type == Some(MessageType::ToolCall) => {
                pending_tool_call = true;
                break;
            }
            Some(MessageRole::Assistant) => {
                // 纯文本/推理结尾：无悬空调用
                break;
            }
            _ => continue,
        }
    }
    if pending_tool_call {
        return 0;
    }

    let char_counts: Vec<usize> = messages
        .iter()
        .map(|m| serde_json::to_string(m).map(|s| s.len()).unwrap_or(0))
        .collect();
    let total_char_count: usize = char_counts.iter().sum();
    let target_char_count = total_char_count as f64 * fraction;

    let mut last_split_point = 0;
    let mut cumulative_char_count = 0;

    for (i, msg) in messages.iter().enumerate() {
        if msg.role == Some(MessageRole::User) {
            let has_content = msg
                .content
                .as_ref()
                .map(|c| !c.to_text().is_empty())
                .unwrap_or(false);
            if has_content {
                if cumulative_char_count >= target_char_count as usize {
                    return i;
                }
                last_split_point = i;
            }
        }
        cumulative_char_count += char_counts[i];
    }

    last_split_point
}

/// 找到当前 Turn（root_id）的切分下标：当前用户指令（Turn 根之前最近的根级 User）。
///
/// 供 run_chat_loop 计算 run_context_compact 的切分点。切分点必须覆盖完整
/// Turn 子树的起点，保留区 = [用户指令, Turn, 子节点...]，parent 链不断裂。
/// 历史缺陷对照：
/// - 旧版切在本 Turn 首个 ToolCall：用户指令与 Turn 根被压进快照，保留区只剩
///   parent 悬空的 ToolCall 子树 → 孤儿剔除 → provider 400；
/// - 更早版本全历史正向扫描会命中会话最早的 ToolCall，切分点落在会话开头，
///   旧内容几乎全部留在保留区，压缩后水位不降 → 高频反复触发。
///
/// 回退：Turn 之前无根级 User 时切在 Turn 根下标（子树仍完整）；Turn 不存在
/// 返回 0（run_context_compact 以 split==0 视为中止）。水位提醒 nudge 为请求级
/// 注入不落库，向前回扫不会误命中（见 chat_loop nudge 注释）。
pub fn find_turn_user_split_idx(messages: &[ChatMessage], root_id: &str) -> usize {
    let turn_idx = match messages.iter().position(|m| m.id == root_id) {
        Some(i) => i,
        None => return 0,
    };
    messages[..turn_idx]
        .iter()
        .rposition(|m| m.role == Some(MessageRole::User) && m.parent_id.is_none())
        .unwrap_or(turn_idx)
}

/// 估算单条消息的 token 数。
///
/// 统一使用 `CalibratedTokenizer`（启发式 × provider 用量反馈校准），
/// **禁止**裸 `text.len()/4`：UTF-8 下中文每字 3 字节，`len()/4` 会低估约 2 倍，
/// 导致压缩触发过晚、请求撞上 provider 的 context-length 400。
///
/// 计入：正文内容 + LLM prompt 前缀（时间/工作区上下文，`to_api_value` 会拼进
/// 实际请求文本并由 provider 计费）+ ToolCall 节点参数（content 即 JSON 文本）
/// + 每消息固定结构开销。
pub fn estimate_message_tokens(m: &ChatMessage) -> usize {
    use super::tokenizer::{default_tokenizer, Tokenizer};

    const PER_MESSAGE_OVERHEAD: usize = 8; // role / 框架 / 分隔符等固定开销
    let tok = default_tokenizer();

    let mut total = PER_MESSAGE_OVERHEAD;
    if let Some(c) = &m.content {
        let text = c.to_text();
        if !text.is_empty() {
            total += tok.count(&text);
        }
    }
    if let Some(name) = &m.name {
        if !name.is_empty() {
            total += tok.count(name);
        }
    }
    // prompt 不持久化但每轮随请求发送，漏算会系统性低估水位。
    if let Some(p) = &m.prompt {
        if !p.is_empty() {
            total += tok.count(p);
        }
    }
    total
}

/// 估算消息列表的总 token 数。
///
/// `overhead_tokens` 为请求级固定开销（system prompt + 工具定义），由调用方传入；
/// 压缩触发判断必须计入这部分，否则阈值虚高、触发过晚。
pub fn estimate_context_tokens(messages: &[ChatMessage], overhead_tokens: usize) -> usize {
    messages.iter().map(estimate_message_tokens).sum::<usize>() + overhead_tokens
}

/// 估算请求级固定开销：system prompt + 全部工具定义。
pub async fn estimate_request_overhead(system_prompt: &str, ctx: &Arc<dyn InvokeRequest>) -> usize {
    use super::tokenizer::{default_tokenizer, Tokenizer};
    use crate::symbio_core::CAPABILITY_VISITOR;

    let tok = default_tokenizer();
    let mut total = tok.count(system_prompt);

    if let Some(tool_visitor) = ctx.get(CAPABILITY_VISITOR) {
        for cap in tool_visitor.list_capability().await {
            let schema_json = cap.input_schema.to_string();
            total += tok.count(&format!("{} {} {}", cap.name, cap.description, schema_json));
        }
    }
    total
}

/// 检查是否需要压缩，返回是否应该开始压缩。
///
/// `overhead_tokens`：system prompt + 工具定义等请求级固定开销，必须计入。
/// `force`：跳过阈值判断（用户 /compact 或主动压缩工具）。
pub fn should_start_compression(
    messages: &[ChatMessage],
    context_limit: usize,
    force: bool,
    overhead_tokens: usize,
) -> bool {
    if messages.is_empty() {
        return false;
    }
    if force {
        return true;
    }
    let current = estimate_context_tokens(messages, overhead_tokens);
    let threshold = (context_limit as f64 * COMPRESSION_TOKEN_THRESHOLD) as usize;

    if current < threshold {
        return false;
    }

    // 迟滞：若最近一次快照后已增长不足 15%，说明上一轮压缩收益被挥霍，
    // 再次压缩大概率是"快照过小"导致的循环调用，跳过并依赖确定性层消化。
    // 例外：force（主动压缩 / 用户指令）不受此限制。
    // 口径对齐：post_tokens 记录的是内容水位（不含请求级 overhead），
    // 这里同样用扣除 overhead 后的内容侧读数比较；若直接用含 overhead 的
    // current，overhead 越大越容易"虚高"越过地板，迟滞保护失效。
    if let Some(last) = messages.iter().find(|m| {
        m.meta
            .as_ref()
            .map(|meta| meta.get("compacted") == Some(&serde_json::json!(true)))
            .unwrap_or(false)
    }) {
        if let Some(post) = last
            .meta
            .as_ref()
            .and_then(|meta| meta.get("post_tokens"))
            .and_then(|v| v.as_u64())
        {
            let content_tokens = current.saturating_sub(overhead_tokens);
            let hysteresis_floor = (post as f64 * COMPACT_HYSTERESIS_FACTOR) as usize;
            if content_tokens < hysteresis_floor {
                return false;
            }
        }
    }

    true
}

/// 水位提醒判断：估算用量是否达到提醒阈值（55%）。
/// 主动压缩工具（context_compact）配合此机制，让模型在安全时机自行压缩。
pub fn should_emit_context_nudge(
    messages: &[ChatMessage],
    context_limit: usize,
    overhead_tokens: usize,
) -> bool {
    if messages.is_empty() {
        return false;
    }
    let current = estimate_context_tokens(messages, overhead_tokens);
    current >= (context_limit as f64 * CONTEXT_NUDGE_THRESHOLD) as usize
}

/// 准备压缩：将要压缩的历史提取出来，生成压缩指令
/// 返回 (压缩指令, 要压缩的历史, 要保留的历史)
pub fn prepare_compression(
    messages: &[ChatMessage],
) -> Option<(ChatMessage, Vec<ChatMessage>, Vec<ChatMessage>)> {
    let split_point = find_compress_split_point(messages, 1.0 - COMPRESSION_PRESERVE_THRESHOLD);
    if split_point == 0 {
        return None;
    }

    let history_to_compress = messages[..split_point].to_vec();
    let history_to_keep = messages[split_point..].to_vec();

    // 检查压缩比例
    let compress_char_count: usize = history_to_compress
        .iter()
        .map(|m| serde_json::to_string(m).map(|s| s.len()).unwrap_or(0))
        .sum();
    let total_char_count: usize = messages
        .iter()
        .map(|m| serde_json::to_string(m).map(|s| s.len()).unwrap_or(0))
        .sum();

    if total_char_count > 0
        && (compress_char_count as f64) / (total_char_count as f64) < MIN_COMPRESSION_FRACTION
    {
        return None;
    }

    // 生成压缩指令
    let history_json = serde_json::to_string(&history_to_compress).unwrap_or_default();
    // 诉求3：user 消息只携带数据；压缩模板由调用方经 system role 注入，
    // 避免格式指令在对话中出现两次（system 一次 + user 一次）诱导模型模仿输出
    let prompt = format!("## Chat History to Summarize:\n{history_json}");

    let compression_msg = ChatMessage {
        id: uuid::Uuid::new_v4().to_string(),
        role: Some(MessageRole::User),
        msg_type: Some(MessageType::Text),
        content: Some(MessageContent::Text(prompt)),
        ..Default::default()
    };

    Some((compression_msg, history_to_compress, history_to_keep))
}

/// 轮次淡化（请求视图级）：当对话轮次过多时，把较早的工具结果（`role=Tool`、`msg_type=Text`）
/// 做 head/tail 摘要，保留最近 `keep_recent_turns` 个 user turn 起的原文，以及**全部**
/// assistant 文本 / 推理节点（绝不改动 assistant 消息，最大限度保护思维链）。
///
/// 只作用于传入的视图副本（由 [`build_request_view`] 每轮从存储重建），存储层保留全文，
/// 因此**不写存档文件**、天然幂等（无重复存档问题）。被淡化的结果标记 `tool_result_faded`，
/// 模型如需完整输出可重新运行对应工具。
pub fn fade_aged_tool_results(messages: &mut [ChatMessage], keep_recent_turns: usize) {
    use super::tokenizer::{default_tokenizer, Tokenizer};

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
                        super::tool_result_guard::summarize_tool_result(t, FADE_BUDGET),
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
/// token 触发走 [`summarize_head_tail`]（与工具淡化同一机制本体，仅占位文案
/// 不同：内容节点无法"重新运行"，指向会话存储中的完整原文）。
///
/// 只作用于传入的视图副本（由 [`build_request_view`] 每轮从存储重建），存储层
/// 保留完整原文，因此**不写存档文件**、天然幂等。被淡化的节点标记 `content_faded`。
pub fn fade_aged_content_nodes(
    messages: &mut [ChatMessage],
    keep_recent: usize,
    line_threshold: usize,
) {
    use super::tokenizer::{default_tokenizer, Tokenizer};

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
                super::tool_result_guard::summarize_head_tail(
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
/// 与 token 路径（[`summarize_head_tail`]）同构：头少尾多（结论在尾部）。
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
/// 3. 工具级骨架化：`window > 0` 且存在保留策略声明时，按分层滑窗把过期调用的
///    参数与结果替换为占位文案（ToolCall↔Tool 配对与 parent_id 传播完整保留，
///    不会造成大模型逻辑断联）；
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
    retention: &std::collections::HashMap<String, crate::symbio_core::ToolContextRetention>,
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
    if window > 0 && !retention.is_empty() {
        view = super::context_window::apply_layered_sliding_window(&view, window, retention);
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

/// 从模型输出中提取 `<state_snapshot>` XML 块（容错空白与转义）。
/// 模型偶发会把 scratchpad 也吐出来，或被 max_tokens 截断——必须解析校验，
/// 残缺内容不能原样成为唯一记忆。
pub fn extract_snapshot(text: &str) -> Option<String> {
    const OPEN: &str = "<state_snapshot>";
    const CLOSE: &str = "</state_snapshot>";
    let start = text.find(OPEN)?;
    let after_open = start + OPEN.len();
    let end = text[after_open..].find(CLOSE)? + after_open;
    let inner = text[after_open..end].trim();
    if inner.is_empty() {
        return None;
    }
    Some(format!("{OPEN}\n{inner}\n{CLOSE}"))
}

/// 快照校验失败时的兜底：把纯文本输出包裹为极简快照。
/// 有总比无好——但明确标注这是降级产物，下一轮压缩时模型可据此补全结构。
pub fn fallback_snapshot(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(format!(
        "<state_snapshot>\n[降级快照：上次压缩时模型未按结构输出，以下为原始摘要，内容可能不完整]\n\n{trimmed}\n</state_snapshot>"
    ))
}

/// 输入超限死锁的本地机械兜底（**不依赖 LLM**）。
///
/// 场景（日志实证）：上下文估算已超 Provider 有效输入上限时，LLM 摘要请求与
/// 上下文**同源超限**（请求体就携带完整待压缩历史），必然 400（"Input token
/// exceed the limit"）——每轮重发注定失败的巨型请求，压缩永远无法收敛。
///
/// 处理：从尾部按 token 预算机械保留最近上下文；保留起点回退到最近一条 user
/// 消息（轮边界）——保证保留区 Turn 子树 parent 链完整、且新上下文以 user 开头
///（多数 provider 的硬要求）。头部插入一条 user 角色的本地说明（告知模型历史
/// 被截断及转存路径），模型可据此意识到上下文不完整。
///
/// 返回 `(新消息列表, 被截断条数)`；若无需截断（本就在预算内）原样返回 `(原列表, 0)`。
pub fn emergency_tail_compression(
    messages: &[ChatMessage],
    target_tokens: usize,
    transcript_hint: Option<&str>,
) -> (Vec<ChatMessage>, usize) {
    // 从尾部反向累计；最后一条无条件保留（当前用户指令 / 生成中回复）
    let mut acc = 0usize;
    let mut start = messages.len();
    while start > 0 {
        let t = estimate_message_tokens(&messages[start - 1]);
        if acc + t > target_tokens && start < messages.len() {
            break;
        }
        acc += t;
        start -= 1;
    }
    // 保留起点回退到最近一条 user 消息（轮边界）
    while start < messages.len() && messages[start].role != Some(MessageRole::User) {
        start += 1;
    }
    if start == 0 {
        return (messages.to_vec(), 0);
    }
    let mut note = format!(
        "[CONTEXT TRUNCATED — 本地兜底压缩]\n早期 {start} 条消息因超出 Provider 输入上限被本地截断（LLM 摘要请求与上下文同源超限，无法执行）。近期上下文如下，请基于其继续任务。"
    );
    if let Some(p) = transcript_hint {
        note.push_str(&format!("\n完整历史转存：{p}"));
    }
    let header = ChatMessage {
        id: uuid::Uuid::new_v4().to_string(),
        role: Some(MessageRole::User),
        msg_type: Some(MessageType::Text),
        content: Some(MessageContent::Text(note)),
        status: Some(MessageStatus::Completed),
        meta: Some(serde_json::json!({
            "compacted": true,
            "compaction": "emergency_tail",
            "removed_messages": start,
        })),
        ..Default::default()
    };
    let mut out = Vec::with_capacity(messages.len() - start + 1);
    out.push(header);
    out.extend_from_slice(&messages[start..]);
    (out, start)
}

/// `context_compact` 工具名（chat_loop 拦截分发用）。
pub const CONTEXT_COMPACT_TOOL_NAME: &str = "context_compact";

/// 把模型输出的 XML 快照渲染为纯文本分节（用于落库，诉求3）。
///
/// 快照会以 assistant 消息长期驻留上下文——若原样保留 `<state_snapshot>`/
/// `<key_knowledge>` 等 XML 标签，模型会把历史里的这条消息当作"期望输出格式"
/// 来模仿，在正常对话中频繁输出同类标签总结。落库前把标签替换为可读分节标记：
/// 信息不丢，但切断"XML 标签 → 格式模仿"的泄漏链。
/// 压缩子系统内部（提取/校验/纠正重试）仍统一使用 XML。
///
/// 头部注入时点声明：快照是"截至某时刻"的状态切片，其【进行中】/【待确认】
/// 分节会随后续轮次自然过期。没有时点标记，读者（模型或人）无法区分
/// "快照说未完成"与"实际早已完成"——只能靠最新消息反推，易误判旧状态为现役。
pub fn render_snapshot_for_history(snapshot_text: &str) -> String {
    let rendered = snapshot_text
        .replace("<state_snapshot>", "")
        .replace("</state_snapshot>", "")
        .replace("<overall_goal>", "【目标】")
        .replace("</overall_goal>", "")
        .replace("<key_knowledge>", "【关键知识】")
        .replace("</key_knowledge>", "")
        .replace("<completed_items>", "【已完成】")
        .replace("</completed_items>", "")
        .replace("<in_progress_items>", "【进行中】")
        .replace("</in_progress_items>", "")
        .replace("<open_questions>", "【待确认问题】")
        .replace("</open_questions>", "")
        .trim()
        .to_string();
    let snapshot_at = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default();
    format!("[快照时点：{snapshot_at}，其后消息未纳入本快照，【进行中】/【待确认问题】以最新消息为准]\n{rendered}")
}

/// 构建主动压缩请求（`context_compact` 工具执行体）。
///
/// 与被动压缩（`prepare_compression`）的差异：显式指定待压缩历史，
/// 并把模型的 `hints`（必须保留的关键信息）追加进提示词——
/// 让模型"亲手"决定快照里必须留下什么。
pub fn build_compression_request(history: &[ChatMessage], hints: Option<&str>) -> ChatMessage {
    let history_json = serde_json::to_string(history).unwrap_or_else(|_| "[]".to_string());
    let hints_section = match hints {
        Some(h) if !h.trim().is_empty() => {
            format!(
                "\n\n## Must-Preserve Hints (from the agent, MUST be kept in the snapshot):\n{h}"
            )
        }
        _ => String::new(),
    };
    // 诉求3：user 消息只携带数据；压缩模板由调用方经 system role 注入，
    // 避免格式指令在对话中出现两次（system 一次 + user 一次）诱导模仿
    let prompt = format!("## Chat History to Summarize:\n{history_json}{hints_section}");
    ChatMessage {
        id: uuid::Uuid::new_v4().to_string(),
        role: Some(MessageRole::User),
        msg_type: Some(MessageType::Text),
        content: Some(MessageContent::Text(prompt)),
        status: Some(MessageStatus::Completed),
        ..Default::default()
    }
}

/// `context_compact` 工具定义：暴露给模型，由模型在任务阶段间隙主动调用。
///
/// 不进 CapabilityVisitor 分发——由 chat_loop 拦截执行（需要编排器内部的
/// 压缩链路：LLM 摘要 + 上下文替换 + 会话持久化）。
pub fn context_compact_tool_meta() -> crate::symbio_core::CapabilityMeta {
    crate::symbio_core::CapabilityMeta {
        name: CONTEXT_COMPACT_TOOL_NAME.to_string(),
        context_retention: None,
        description: "Compact the conversation history: distill older messages into a \
            structured state snapshot and keep only recent context in the session. \
            Call this when you have just finished a major subtask, when the context is \
            filled with intermediate outputs you no longer need, or when a system note \
            warns that context usage is high. The session continues seamlessly from the \
            snapshot. Pass `hints` describing information that MUST be preserved \
            (file paths, plans, constraints, open questions)."
            .to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "hints": {
                    "type": "string",
                    "description": "Key facts/plans/constraints that must be preserved in the snapshot"
                }
            }
        }),
        keywords: vec![
            "compact".to_string(),
            "压缩".to_string(),
            "上下文".to_string(),
        ],
        category: Some(crate::symbio_core::CapabilityCategory::Core),
        examples: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user_msg(text: &str) -> ChatMessage {
        ChatMessage {
            id: uuid::Uuid::new_v4().to_string(),
            role: Some(MessageRole::User),
            msg_type: Some(MessageType::Text),
            content: Some(MessageContent::Text(text.to_string())),
            ..Default::default()
        }
    }

    fn assistant_msg(text: &str) -> ChatMessage {
        ChatMessage {
            id: uuid::Uuid::new_v4().to_string(),
            role: Some(MessageRole::Assistant),
            msg_type: Some(MessageType::Text),
            content: Some(MessageContent::Text(text.to_string())),
            ..Default::default()
        }
    }

    fn tool_call_msg() -> ChatMessage {
        ChatMessage {
            id: uuid::Uuid::new_v4().to_string(),
            role: Some(MessageRole::Assistant),
            msg_type: Some(MessageType::ToolCall),
            name: Some("local/file_read".to_string()),
            content: Some(MessageContent::Text(r#"{"path":"a.rs"}"#.to_string())),
            ..Default::default()
        }
    }

    fn tool_result_msg() -> ChatMessage {
        ChatMessage {
            id: uuid::Uuid::new_v4().to_string(),
            role: Some(MessageRole::Tool),
            msg_type: Some(MessageType::Text),
            content: Some(MessageContent::Text("file content".to_string())),
            ..Default::default()
        }
    }

    #[test]
    fn test_estimate_tokens_counts_tool_calls_and_chinese() {
        // 中文 3 字节/字：旧的 len()/4 会严重低估；tokenizer 应给出合理值
        let m = user_msg(&"你好世界。".repeat(100)); // 500 字 ≈ 500~1000 tok
        let est = estimate_message_tokens(&m);
        assert!(est >= 400, "中文估算不应低估: {est}");

        // ToolCall 参数（content 即 JSON）必须被计入
        let tc = tool_call_msg();
        assert!(estimate_message_tokens(&tc) > 8);
    }

    #[test]
    fn test_estimate_context_tokens_includes_overhead() {
        let msgs = vec![user_msg("hi")];
        let est = estimate_context_tokens(&msgs, 5000);
        assert!(est >= 5000, "请求级固定开销必须计入");
    }

    #[test]
    fn test_split_point_never_full_compress_on_assistant_tail() {
        // U A U A：末尾是 Assistant（旧版会全量压缩）。保留 30% ⇒ 应在早期 User 边界切分。
        let msgs = vec![
            user_msg(&"x".repeat(1000)),
            assistant_msg(&"y".repeat(1000)),
            user_msg(&"z".repeat(1000)),
            assistant_msg(&"w".repeat(1000)),
        ];
        let split = find_compress_split_point(&msgs, 0.7);
        assert!(
            split > 0 && split < msgs.len(),
            "不得全量压缩, split={split}"
        );
        assert_eq!(msgs[split].role, Some(MessageRole::User));
    }

    #[test]
    fn test_split_point_zero_on_pending_tool_call() {
        // 尾部 ToolCall 无结果子节点（continuation 场景）⇒ 必须放弃压缩
        let msgs = vec![
            user_msg(&"x".repeat(1000)),
            assistant_msg(&"y".repeat(1000)),
            tool_call_msg(),
        ];
        assert_eq!(find_compress_split_point(&msgs, 0.7), 0);
    }

    #[test]
    fn test_split_point_ok_when_tool_call_paired() {
        // 配对完整的 ToolCall/Tool 不阻塞压缩
        let msgs = vec![
            user_msg(&"x".repeat(1000)),
            assistant_msg(&"y".repeat(1000)),
            tool_call_msg(),
            tool_result_msg(),
            user_msg(&"z".repeat(1000)),
        ];
        let split = find_compress_split_point(&msgs, 0.7);
        assert!(split > 0, "配对完整时不应放弃压缩");
    }

    fn turn_root_msg(id: &str) -> ChatMessage {
        ChatMessage {
            id: id.to_string(),
            parent_id: None,
            role: Some(MessageRole::Assistant),
            msg_type: Some(MessageType::Turn),
            content: Some(MessageContent::Text("turn".to_string())),
            ..Default::default()
        }
    }

    /// 切分点回归：必须前移到当前用户指令（保留区 = 用户指令 + 完整 Turn 子树）。
    /// 旧实现切在本 Turn 首个 ToolCall，用户指令与 Turn 根被压进快照，保留区
    /// 只剩 parent 悬空的 ToolCall 子树 → provider 400。
    #[test]
    fn test_turn_split_idx_starts_at_user_instruction() {
        let cur_call = ChatMessage {
            parent_id: Some("cur-root".to_string()),
            ..tool_call_msg()
        };
        let old_call = ChatMessage {
            parent_id: Some("old-root".to_string()),
            ..tool_call_msg()
        };
        let msgs = vec![
            user_msg("旧问题"),
            turn_root_msg("old-root"),
            old_call,
            tool_result_msg(),
            user_msg("新问题"),
            turn_root_msg("cur-root"),
            cur_call,
            tool_result_msg(),
        ];
        // 当前用户指令在下标 4：保留区从"新问题"起，含完整 Turn 子树
        assert_eq!(find_turn_user_split_idx(&msgs, "cur-root"), 4);
        // 旧 Turn：其用户指令即会话首条 ⇒ 0（run_context_compact 以 split==0 中止）
        assert_eq!(find_turn_user_split_idx(&msgs, "old-root"), 0);
        // Turn 不存在 ⇒ 0（中止）
        assert_eq!(find_turn_user_split_idx(&msgs, "no-such-turn"), 0);
    }

    /// 回退与中止语义：Turn 前无根级 User ⇒ 切在 Turn 根（子树仍完整）；
    /// 压缩后快照(User)即历史 ⇒ 切点 0 → 中止，避免把快照自身当作待压缩历史。
    #[test]
    fn test_turn_split_idx_fallback_and_abort() {
        let turn = turn_root_msg("cur-root");
        let msgs = vec![assistant_msg("开场白"), turn.clone(), tool_call_msg()];
        assert_eq!(find_turn_user_split_idx(&msgs, "cur-root"), 1);

        let snapshot = user_msg("[CONTEXT SNAPSHOT]");
        let msgs2 = vec![snapshot, turn];
        assert_eq!(find_turn_user_split_idx(&msgs2, "cur-root"), 0);
    }

    /// 滞后口径回归：迟滞比较必须用扣除请求级 overhead 后的内容侧读数。
    /// 旧实现直接比较含 overhead 的 current：overhead 越大越容易虚高越过
    /// post_tokens×1.15 地板，迟滞保护失效 → 压缩高频触发。
    #[test]
    fn test_hysteresis_compares_content_tokens_excluding_overhead() {
        let make_snapshot = || {
            let mut m = assistant_msg("snapshot");
            m.meta = Some(serde_json::json!({
                "compacted": true,
                "post_tokens": 1000
            }));
            m
        };
        let floor = (1000.0 * COMPACT_HYSTERESIS_FACTOR) as usize;

        // 内容水位低于地板，但 current = 内容 + overhead 虚高越过地板：
        // 旧口径（直接比较 current）会触发，新口径（扣除 overhead）必须拦截。
        let msgs = vec![make_snapshot(), user_msg(&"中文字符填充".repeat(50))];
        let content_tokens = estimate_context_tokens(&msgs, 0);
        assert!(
            content_tokens < floor,
            "前提：内容水位 {content_tokens} 应低于地板 {floor}"
        );
        let overhead = floor - content_tokens + 10;
        let current = content_tokens + overhead;
        assert!(current >= floor, "前提：current 应虚高越过地板");
        let threshold_limit = (current as f64 / 0.7 * 0.99) as usize;
        assert!(!should_start_compression(
            &msgs,
            threshold_limit,
            false,
            overhead
        ));

        // 对照：内容真实增长越过地板后，迟滞放行（overhead 不应造成过度抑制）。
        let grown = vec![make_snapshot(), user_msg(&"中文字符填充".repeat(400))];
        let grown_tokens = estimate_context_tokens(&grown, 0);
        assert!(
            grown_tokens >= floor,
            "前提：{grown_tokens} 应不低于地板 {floor}"
        );
        let grown_overhead = 500;
        let grown_current = grown_tokens + grown_overhead;
        let grown_limit = (grown_current as f64 / 0.7 * 0.99) as usize;
        assert!(should_start_compression(
            &grown,
            grown_limit,
            false,
            grown_overhead
        ));
    }

    #[test]
    fn test_should_start_compression_hysteresis() {
        // 压缩后 post_tokens=1000，当前估算 1100（< 1150）⇒ 即使超 70% 阈值也跳过
        let mut snapshot = assistant_msg("snapshot");
        snapshot.meta = Some(serde_json::json!({
            "compacted": true,
            "post_tokens": 1000
        }));
        // 构造一条足够大的历史让 current 估算落在 (1000, 1150) 区间
        let msgs = vec![snapshot, user_msg(&"中文字符填充".repeat(50))];
        let small_limit = estimate_context_tokens(&msgs, 0); // ≈ current
                                                             // 阈值设为远小于 current ⇒ 无迟滞时会触发
        let threshold_limit = (small_limit as f64 / 0.7 * 0.99) as usize;
        // current/threshold ≈ 0.7/0.99 > 1 ⇒ 超阈值，但 current < 1150 ⇒ 迟滞应拦截
        assert!(!should_start_compression(&msgs, threshold_limit, false, 0));
        // force 不受迟滞限制
        assert!(should_start_compression(&msgs, threshold_limit, true, 0));
    }

    #[test]
    fn test_extract_snapshot() {
        let out = "thinking... <state_snapshot>\n  <overall_goal>done</overall_goal>\n</state_snapshot> trailing";
        let s = extract_snapshot(out).unwrap();
        assert!(s.starts_with("<state_snapshot>"));
        assert!(s.contains("done"));
        assert!(extract_snapshot("no xml here").is_none());
        assert!(extract_snapshot("<state_snapshot></state_snapshot>").is_none());
    }

    #[test]
    fn test_fallback_snapshot() {
        let s = fallback_snapshot("plain summary").unwrap();
        assert!(s.contains("state_snapshot"));
        assert!(s.contains("plain summary"));
        assert!(fallback_snapshot("  \n ").is_none());
    }

    /// 诉求3：降级快照不得引入 `<key_knowledge>` 标签——
    /// 它会持久化到历史中，诱导模型在正常对话里复述该格式。
    #[test]
    fn test_fallback_snapshot_has_no_key_knowledge_tag() {
        let s = fallback_snapshot("plain summary").unwrap();
        assert!(
            !s.contains("<key_knowledge>"),
            "降级快照不得包含 key_knowledge 结构标签"
        );
    }

    /// 模板回归锚点：Signal-to-noise 规则必须包含（1）旧快照对账规则——
    /// 防错误知识跨快照遗传（真实事故：一条错误论断存活三个快照周期，与
    /// 纠错证据并存于同一快照的两个分节）；（2）key_knowledge 卫生与
    /// completed_items 瘦身规则——防过程性知识与易变数字常驻快照。
    /// 注：模板文本变化会改变 compression_prompt_fingerprint，属预期
    /// （指纹仅写入快照 meta 作溯源，无硬编码依赖）。
    #[test]
    fn test_compression_prompt_has_reconciliation_and_hygiene_rules() {
        let prompt = get_compression_prompt();
        assert!(
            prompt.contains("unverified INPUT, not ground truth"),
            "模板缺少旧快照对账规则（防错误跨代遗传）"
        );
        assert!(
            prompt.contains("never copy old <key_knowledge> forward unchecked"),
            "模板缺少禁止盲目继承 key_knowledge 的规则"
        );
        assert!(
            prompt.contains("state invariants instead"),
            "模板缺少 key_knowledge 卫生规则（不变量替代易变数字）"
        );
        assert!(
            prompt.contains("one line per item"),
            "模板缺少 completed_items 瘦身规则"
        );
    }

    /// 诉求3：压缩模板只走 system role，user 消息只携带待压缩数据
    /// （prepare_compression / build_compression_request 均不得内嵌模板）。
    #[test]
    fn test_compression_request_user_message_carries_data_only() {
        let msgs = vec![
            user_msg(&"x".repeat(1000)),
            assistant_msg(&"y".repeat(1000)),
            user_msg(&"z".repeat(1000)),
        ];
        let (msg, _, _) = prepare_compression(&msgs).unwrap();
        let text = msg
            .content
            .as_ref()
            .map(|c| c.to_text())
            .unwrap_or_default();
        assert!(text.contains("Chat History to Summarize"));
        assert!(
            !text.contains("<state_snapshot>"),
            "压缩模板不得随 user 消息下发"
        );

        let req = build_compression_request(&msgs, Some("keep file paths"));
        let req_text = req
            .content
            .as_ref()
            .map(|c| c.to_text())
            .unwrap_or_default();
        assert!(req_text.contains("keep file paths"));
        assert!(!req_text.contains("<state_snapshot>"));
    }

    /// 诉求3：落库快照渲染后不残留 XML 标签——
    /// 历史中的 assistant 消息不再示范 state_snapshot/key_knowledge 结构。
    #[test]
    fn test_render_snapshot_for_history_strips_xml_tags() {
        let xml = extract_snapshot(
            "<state_snapshot>\n<key_knowledge>\n- A\n- B\n</key_knowledge>\n</state_snapshot>",
        )
        .unwrap();
        let rendered = render_snapshot_for_history(&xml);
        assert!(!rendered.contains("<state_snapshot>"));
        assert!(!rendered.contains("<key_knowledge>"));
        assert!(rendered.contains("【关键知识】"));
        assert!(rendered.contains("- A"));
    }

    #[test]
    fn test_prepare_compression_rejects_when_no_split() {
        // 单条 User 消息（巨型单轮场景）：无切分点 ⇒ prepare_compression 返回 None
        let msgs = vec![user_msg(&"x".repeat(100_000))];
        assert!(prepare_compression(&msgs).is_none());
    }

    // ==================== build_request_view（请求视图层） ====================

    /// 带显式 id 的 Assistant/ToolCall 消息（与 session/context.rs 骨架化测试同构）
    fn view_tc(id: &str, name: &str, args: &str) -> ChatMessage {
        ChatMessage {
            id: id.to_string(),
            role: Some(MessageRole::Assistant),
            msg_type: Some(MessageType::ToolCall),
            name: Some(name.to_string()),
            content: Some(MessageContent::Text(args.to_string())),
            status: Some(MessageStatus::Completed),
            ..Default::default()
        }
    }

    /// 工具结果消息，parent_id 指向所属 ToolCall（id 形如 "{parent}-result"）
    fn view_result(parent: &str, text: &str) -> ChatMessage {
        ChatMessage {
            id: format!("{parent}-result"),
            parent_id: Some(parent.to_string()),
            role: Some(MessageRole::Tool),
            msg_type: Some(MessageType::Text),
            content: Some(MessageContent::Text(text.to_string())),
            status: Some(MessageStatus::Completed),
            ..Default::default()
        }
    }

    /// 构建短工具名 → 保留策略映射（模拟 chat_loop 运行时从 CapabilityVisitor 动态解析）
    fn view_retention(
        entries: &[(&str, crate::symbio_core::ToolContextRetention)],
    ) -> std::collections::HashMap<String, crate::symbio_core::ToolContextRetention> {
        entries.iter().map(|(n, r)| (n.to_string(), *r)).collect()
    }

    fn view_text(m: &ChatMessage) -> String {
        m.content.as_ref().map(|c| c.to_text()).unwrap_or_default()
    }

    /// 透传回归：无 fade、无骨架化、无 nudge 时，视图与输入逐条等价，且输入不被修改。
    #[test]
    fn test_build_request_view_passthrough() {
        let msgs = vec![
            user_msg("hello"),
            assistant_msg("hi"),
            view_tc("t1", "local/file_read", r#"{"path":"a.rs"}"#),
            view_result("t1", "file content"),
        ];
        let retention = std::collections::HashMap::new();
        let view = build_request_view(&msgs, 15, &retention, false, 12, 3, 200, false);

        assert_eq!(view.len(), msgs.len());
        for (v, m) in view.iter().zip(msgs.iter()) {
            assert_eq!(v.id, m.id);
            assert_eq!(v.role, m.role);
            assert_eq!(view_text(v), view_text(m));
        }
        // 存储侧不受请求视图污染
        assert!(msgs.iter().all(|m| m.meta.is_none()));
    }

    /// nudge 请求级注入：追加到视图末尾，不落库（原输入不变），meta 可识别。
    #[test]
    fn test_build_request_view_injects_nudge_at_tail() {
        let msgs = vec![user_msg("u1"), assistant_msg("a1"), user_msg("u2")];
        let retention = std::collections::HashMap::new();

        let view = build_request_view(&msgs, 15, &retention, false, 12, 3, 200, true);
        assert_eq!(view.len(), msgs.len() + 1);
        let nudge = view.last().unwrap();
        assert_eq!(nudge.role, Some(MessageRole::User));
        assert_eq!(
            nudge
                .meta
                .as_ref()
                .and_then(|m| m.get("kind"))
                .and_then(|k| k.as_str()),
            Some("context_nudge")
        );
        assert!(view_text(nudge).contains("context_compact"));

        // 原输入不被修改：nudge 不写入会话存储
        assert_eq!(msgs.len(), 3);
        assert!(msgs.iter().all(|m| m.meta.is_none()));

        // inject_nudge=false 时不注入
        let plain = build_request_view(&msgs, 15, &retention, false, 12, 3, 200, false);
        assert_eq!(plain.len(), msgs.len());
    }

    /// fade 激活：老化工具结果被 head/tail 摘要并标记 `tool_result_faded`，
    /// 不写存档（无 archive_path），assistant 消息与最近轮次不受影响，存储保留全文。
    #[test]
    fn test_build_request_view_fades_aged_tool_results() {
        // 3 个 user turn；fade_keep_turns=2 ⇒ 第一个 turn 的工具结果应被淡化
        let long_text = "line\n".repeat(12_000); // 远超 2048 token
        let msgs = vec![
            user_msg("turn-1"),
            view_tc("t1", "local/file_read", r#"{"path":"big.txt"}"#),
            view_result("t1", &long_text),
            assistant_msg("analysis"),
            user_msg("turn-2"),
            view_tc("t2", "local/file_read", r#"{"path":"small.txt"}"#),
            view_result("t2", "tiny"),
            assistant_msg("done"),
            user_msg("turn-3"),
        ];
        let retention = std::collections::HashMap::new();
        let view = build_request_view(&msgs, 0, &retention, true, 2, 3, 200, false);

        let faded = &view[2];
        let faded_text = view_text(faded);
        assert!(faded_text.contains("已省略"), "老化工具结果应被摘要化");
        assert!(faded_text.len() < long_text.len(), "摘要应显著短于原文");
        assert_eq!(
            faded
                .meta
                .as_ref()
                .and_then(|m| m.get("tool_result_faded"))
                .and_then(|v| v.as_bool()),
            Some(true)
        );
        assert!(
            faded
                .meta
                .as_ref()
                .and_then(|m| m.get("archive_path"))
                .is_none(),
            "请求视图级淡化不得写存档"
        );
        // assistant 消息绝不被淡化
        assert_eq!(view_text(&view[3]), "analysis");
        // 最近轮次内的小结果保持原文、无标记
        assert_eq!(view_text(&view[6]), "tiny");
        assert!(view[6].meta.is_none());
        // 存储中的原文不受影响
        assert_eq!(view_text(&msgs[2]), long_text);
    }

    /// fade 天然幂等：视图每轮从存储重建，对同一视图重复淡化不产生二次改写。
    #[test]
    fn test_fade_snapshot_message_is_exempt() {
        // L2 快照（meta.compacted=true）已是压缩产物，内容淡化必须豁免：
        // 再切中段 = 双重压缩，切掉的恰是模型唯一的深历史记忆（真实事故回归）。
        let mut snapshot = user_msg(&"x".repeat(30_000));
        snapshot.meta = Some(serde_json::json!({"compacted": true}));
        let mut msgs = vec![snapshot, user_msg("窗口内内容节点"), user_msg("末条消息")];
        fade_aged_content_nodes(&mut msgs, 1, 200);
        assert_eq!(view_text(&msgs[0]), "x".repeat(30_000));
        assert!(msgs[0]
            .meta
            .as_ref()
            .and_then(|m| m.get("content_faded"))
            .is_none());
    }

    #[test]
    fn test_render_snapshot_for_history_prepends_timestamp() {
        // 快照是"截至某时刻"的状态切片：【进行中】/【待确认】会随后续轮次自然
        // 过期。头部必须自带时点声明，否则读者无法区分"快照说未完成"与
        // "实际早已完成"（真实事故：快照称"u6/u7 编译阻塞"，实际早已全绿）。
        let xml = extract_snapshot(
            "<state_snapshot>\n<in_progress_items>\n- 旧任务\n</in_progress_items>\n</state_snapshot>",
        )
        .unwrap();
        let rendered = render_snapshot_for_history(&xml);
        assert!(rendered.starts_with("[快照时点："));
        assert!(rendered.contains("以最新消息为准]"));
        assert!(rendered.contains("【进行中】"));
    }

    #[test]
    fn test_fade_is_idempotent() {
        let long_text = "line\n".repeat(12_000);
        let mut msgs = vec![
            user_msg("t1"),
            view_result("t1", &long_text),
            user_msg("t2"),
        ];
        fade_aged_tool_results(&mut msgs, 1);
        let once = view_text(&msgs[1]);
        let once_meta = msgs[1].meta.clone();
        fade_aged_tool_results(&mut msgs, 1);
        assert_eq!(view_text(&msgs[1]), once);
        assert_eq!(msgs[1].meta, once_meta);
    }

    /// 内容节点淡化：B1 保护窗口外的超大 User/Assistant 正文与思考做 head/tail
    /// 摘要并标记 `content_faded`；保护窗口内（最近 keep_recent 个内容节点 +
    /// 最后一条消息）保持原样；ToolCall 不属内容节点，绝不淡化。
    #[test]
    fn test_fade_aged_content_nodes_line_path() {
        // 40 行 > threshold=16：触发行数路径（头尾各 4 行，中段省略 32 行）
        let big = (0..40)
            .map(|i| format!("line-{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let msgs = vec![
            user_msg(&big),      // idx0：窗口外，应淡化
            assistant_msg(&big), // idx1：窗口外，应淡化
            assistant_msg(&big), // idx2：最近 1 个内容节点（配额）⇒ B1 保护
            user_msg("tail"),    // idx3：最后一条，恒保护
        ];
        let mut view = msgs.clone();
        fade_aged_content_nodes(&mut view, 1, 16);

        // idx0/idx1 被淡化：含中段省略标记，且显著短于原文
        for i in [0usize, 1] {
            let t = view_text(&view[i]);
            assert!(t.contains("已省略 32 行中段内容"), "idx{i} 应走行数路径");
            assert!(t.len() < big.len());
            assert_eq!(
                view[i]
                    .meta
                    .as_ref()
                    .and_then(|m| m.get("content_faded"))
                    .and_then(|v| v.as_bool()),
                Some(true),
                "idx{i} 应带 content_faded 标记"
            );
        }
        // idx2 虽超大但在保护窗口内（keep_recent=1）⇒ 原文保留
        assert_eq!(view_text(&view[2]), big, "B1 窗口内内容节点不淡化");
        assert!(view[2].meta.is_none());
        // idx3 最后一条恒保护
        assert_eq!(view_text(&view[3]), "tail");
        // 存储原文不受视图污染
        assert_eq!(view_text(&msgs[0]), big);
    }

    /// token 路径：token 超预算（行数未超）的内容节点走 summarize_head_tail，
    /// 占位文案指向会话存储（内容节点无法“重新运行”）。
    #[test]
    fn test_fade_aged_content_nodes_token_path() {
        // 单行 2 万字符：行数=1 远低于阈值，但 token 远超 2048
        let huge = "x".repeat(20_000);
        // keep_recent=1 ⇒ 窗口 = 末条 + idx1；idx0 落在窗口外
        let msgs = vec![user_msg(&huge), user_msg("mid"), user_msg("tail")];
        let mut view = msgs.clone();
        fade_aged_content_nodes(&mut view, 1, 200);

        let t = view_text(&view[0]);
        assert!(
            t.contains("该早期内容已在请求视图中淡化"),
            "token 路径占位文案"
        );
        assert!(t.len() < huge.len());
        assert_eq!(
            view[0]
                .meta
                .as_ref()
                .and_then(|m| m.get("content_faded"))
                .and_then(|v| v.as_bool()),
            Some(true)
        );
        // 存储原文不动
        assert_eq!(view_text(&msgs[0]), huge);
    }

    /// 幂等：淡化后的文本（含省略标记）再次进入 fade 流程，结果逐字节不变。
    #[test]
    fn test_fade_aged_content_nodes_idempotent() {
        let big = (0..40)
            .map(|i| format!("line-{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut msgs = vec![user_msg(&big), user_msg("tail")];
        fade_aged_content_nodes(&mut msgs, 1, 16);
        let once = view_text(&msgs[0]);
        let once_meta = msgs[0].meta.clone();
        fade_aged_content_nodes(&mut msgs, 1, 16);
        assert_eq!(view_text(&msgs[0]), once);
        assert_eq!(msgs[0].meta, once_meta);
    }

    /// ToolCall 豁免：工具调用参数不属于内容节点（msg_type=ToolCall），
    /// 即便超长也不淡化——调用参数是骨架化层的辖区。
    #[test]
    fn test_fade_aged_content_nodes_skips_tool_call() {
        let big_args = "x".repeat(20_000);
        let mut msgs = vec![
            view_tc("t1", "local/file_read", &big_args),
            user_msg("tail"),
        ];
        fade_aged_content_nodes(&mut msgs, 1, 200);
        assert_eq!(view_text(&msgs[0]), big_args, "ToolCall 参数不淡化");
        assert!(msgs[0].meta.is_none());
    }

    /// 骨架化：window + 保留策略声明时，过期调用的参数与结果被占位文案替换，
    /// 最新调用保留原文；ToolCall↔Tool 配对与 parent_id 完整保留（逻辑不断联）。
    #[test]
    fn test_build_request_view_skeletonizes_with_retention() {
        let msgs = vec![
            user_msg("u1"),
            view_tc("t1", "local/file_read", r#"{"path":"old.txt"}"#),
            view_result("t1", "old content"),
            user_msg("u2"),
            view_tc("t2", "local/file_read", r#"{"path":"new.txt"}"#),
            view_result("t2", "new content"),
        ];
        // LastOnly：同工具仅保留最近一次调用（t1 应骨架化）
        let retention = view_retention(&[(
            "file_read",
            crate::symbio_core::ToolContextRetention::LastOnly,
        )]);
        let view = build_request_view(&msgs, 15, &retention, false, 12, 3, 200, false);

        assert!(
            view_text(&view[1]).contains("skeletonized"),
            "过期调用参数应骨架化"
        );
        assert!(
            view_text(&view[2]).contains("skeletonized"),
            "过期调用结果应骨架化"
        );
        assert_eq!(view_text(&view[4]), r#"{"path":"new.txt"}"#);
        assert_eq!(view_text(&view[5]), "new content");
        // 配对与 parent_id 保留
        assert_eq!(view[2].parent_id.as_deref(), Some("t1"));
        assert_eq!(view[5].parent_id.as_deref(), Some("t2"));
        // 存储视图不受影响
        assert_eq!(view_text(&msgs[1]), r#"{"path":"old.txt"}"#);
    }

    /// 顺序保证：nudge 在骨架化之后追加，始终位于视图末尾；
    /// nudge 不占用轮次窗口计数、不参与骨架化。
    #[test]
    fn test_build_request_view_nudge_comes_after_skeletonization() {
        let msgs = vec![
            user_msg("u1"),
            view_tc("t1", "local/file_read", r#"{"path":"old.txt"}"#),
            view_result("t1", "old content"),
            user_msg("u2"),
            view_tc("t2", "local/file_read", r#"{"path":"new.txt"}"#),
            view_result("t2", "new content"),
        ];
        let retention = view_retention(&[(
            "file_read",
            crate::symbio_core::ToolContextRetention::LastOnly,
        )]);
        let view = build_request_view(&msgs, 15, &retention, false, 12, 3, 200, true);

        assert_eq!(view.len(), msgs.len() + 1);
        // 末尾是 nudge；倒数第二条仍是保留原文的最新工具结果
        assert!(view_text(view.last().unwrap()).contains("system note"));
        assert_eq!(view_text(&view[view.len() - 2]), "new content");
    }
}
