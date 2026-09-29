//! 会话上下文治理域（`context`）
//!
//! 「发给大模型之前」的全部上下文处理：何时压、从哪切、怎么裁、压完怎么落。
//!
//! ## 模块分工
//!
//! | 文件 | 职责 |
//! |---|---|
//! | 本文件 | 触发与度量：阈值 / 切分点 / token 估算 / 收益护栏（「何时压、从哪切」） |
//! | [`view`]     | 请求视图层：淡化 / 骨架化 / 水位提醒（`build_request_view` 唯一入口） |
//! | [`conversation_view`] | 对话线投影：user 消息 + 根级 assistant 文本节点（对话面唯一分界） |
//! | [`prompt`]   | 压缩提示词、`<state_snapshot>` 快照协议、`context_compact` 工具元 |
//! | [`window`]   | 分层滑窗骨架化（纯函数，无副作用） |
//! | [`pipeline`] | 压缩执行流水线：被动 L2 与主动 `context_compact` 共用的唯一内核 |
//!
//! 分层：门面与 `view` / `conversation_view` / `prompt` 是**纯策略**（无 I/O），
//! `pipeline` 是**执行**（LLM 请求 / 落库 / 广播），是本域唯一的副作用出口。

use std::sync::Arc;

use crate::symbio_core::schemas::session::chat_message::{ChatMessage, MessageRole, MessageType};
use crate::symbio_core::{PluginInvokeRequest, PluginInvokeRequestExt};

mod conversation_view;
mod pipeline;
mod prompt;
mod view;
mod window;

// 门面 = 域内唯一对外出口：调用方（`chat_loop` / `resume`）只经 `context::X` 取用，
// 不感知域内文件划分；域内子模块则经 `use super::*` 取用门面（含下面的 re-export）。
pub use self::conversation_view::{conversation_view, CONVERSATION_VIEW_LIMIT};
pub(crate) use self::pipeline::{auto_compress_process, retry_compaction, run_context_compact};
pub use self::prompt::{
    build_compression_request, compression_prompt_fingerprint, context_compact_tool_meta,
    extract_snapshot, get_compression_prompt, render_snapshot_for_history,
    COMPRESSION_PROTOCOL_VERSION, CONTEXT_COMPACT_TOOL_NAME,
};
pub use self::view::build_request_view;

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

/// 压缩收益护栏：待压缩历史是否够大，值得花一次 LLM 摘要调用。
///
/// **被动（L1 自动，70% 触发）与主动（`context_compact` 工具）两条路径共用同一门槛**
/// ——原先两处各写一遍 `tokens < MIN_COMPACT_TOKENS`，门槛语义（含"为什么是 4000"）
/// 因此有两个出处。被动路径天然满足该条件（能到 70% 的历史必然很大），真正的用途
/// 是拦掉主动路径的无效触发。
pub fn has_compaction_payoff(history_tokens: usize) -> bool {
    history_tokens >= MIN_COMPACT_TOKENS
}

/// 快照接替被压缩内容的**槽位序号**：`保留区首条 seq − 1`。
///
/// 为什么不是"新号"（`max_seq + 1`）：快照语义上是**被压掉那段历史的替身**，
/// 它必须排在保留区**之前**。若给它一个全新的最大号，数组顺序与 `seq` 顺序就
/// 互相矛盾（快照在数组首、seq 最大），消费者按 `seq` 排序会把历史记忆甩到末尾。
///
/// 而给它"槽位号"则让新列表**天然单调**，于是 `assign_seq` 退化为"只补缺号"，
/// **保留区任何一条消息的 seq 都不会被改写**——这正是"压缩不改变既有序号"的
/// 落实点（见 `assign_seq` 的稳定性不变式）。
///
/// 回退链：保留区首条无 `seq`（存量数据）→ 被压缩区末条的 `seq` → `None`
/// （两者都没有时交给 `assign_seq` 补号）。
pub fn snapshot_slot_seq(compressed: &[ChatMessage], keep: &[ChatMessage]) -> Option<i64> {
    keep.first()
        .and_then(|m| m.seq)
        .map(|s| s - 1)
        .or_else(|| compressed.last().and_then(|m| m.seq))
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

/// 估算请求级固定开销：system prompt + **给定**工具定义。
///
/// 与 [`estimate_request_overhead`] 算法完全一致，区别只在于**不自己取工具**——
/// 由调用方传入已收集好的工具清单。会话主循环每轮只收集一次工具
/// （见 `chat_loop::prepare_turn_inputs`），应优先使用本函数复用该清单，
/// 避免同一轮内对 visitor 注册表重复查询。
pub fn estimate_overhead_with_tools(
    system_prompt: &str,
    tools: &[crate::symbio_core::CapabilityMeta],
) -> usize {
    use super::tokenizer::{default_tokenizer, Tokenizer};

    let tok = default_tokenizer();
    let mut total = tok.count(system_prompt);

    for cap in tools {
        let schema_json = cap.input_schema.to_string();
        total += tok.count(&format!("{} {} {}", cap.name, cap.description, schema_json));
    }
    total
}

/// 估算请求级固定开销：system prompt + 全部工具定义。
///
/// 便捷包装：自行取一次工具清单后转交 [`estimate_overhead_with_tools`]。
/// 调用方若已持有工具清单（如会话主循环），应直接调后者以免重复取。
pub async fn estimate_request_overhead(
    system_prompt: &str,
    ctx: &Arc<dyn PluginInvokeRequest>,
) -> usize {
    use crate::symbio_core::CAPABILITY_VISITOR;

    let tools = match ctx.get(CAPABILITY_VISITOR) {
        Some(tool_visitor) => tool_visitor.list_capability().await,
        None => Vec::new(),
    };
    estimate_overhead_with_tools(system_prompt, &tools)
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

/// 准备压缩：将要压缩的历史提取出来，生成压缩请求
/// 返回 (压缩请求, 要压缩的历史, 要保留的历史)
pub fn prepare_compression(
    messages: &[ChatMessage],
) -> Option<(Vec<ChatMessage>, Vec<ChatMessage>, Vec<ChatMessage>)> {
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

    // 生成压缩请求：历史对话原样 + 末尾指令（唯一构造点，主动路径亦复用）。
    // 压缩模板由调用方经 system role 注入，避免格式指令在对话中出现两次
    // （system 一次 + user 一次）诱导模型模仿输出。
    let compression_request = build_compression_request(&history_to_compress, None);

    Some((compression_request, history_to_compress, history_to_keep))
}

#[cfg(test)]
#[path = "context.test.rs"]
mod tests;
