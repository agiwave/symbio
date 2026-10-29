//! 会话上下文治理域（`context`）
//!
//! 「发给大模型之前」的全部上下文处理：何时压、从哪切、压完怎么落，以及请求级的
//! 注入段（段由 provider 贡献，宿主只按序拼接）。
//!
//! ## 模块分工
//!
//! | 文件 | 职责 |
//! |---|---|
//! | 本文件 | 触发与度量：阈值 / 切分点 / token 估算 / 收益护栏（「何时压、从哪切」） |
//! | [`providers`] | 本插件贡献的段：长期记忆召回 / 任务调度候选集（`ContextProvider` 实现） |
//! | [`view`]     | 段 → 请求节点（`build_request_sections` 唯一入口；不决定有哪几段） |
//! | [`conversation_view`] | 对话线投影：user 消息 + assistant 文本节点（对话面唯一分界） |
//! | [`prompt`]   | 压缩提示词、`<state_snapshot>` 快照协议 |
//! | [`pipeline`] | 压缩执行流水线：阈值触发的自动压缩与用户重试共用的唯一内核 |
//!
//! **对话体不在这里加工**（[D-078](../../symbio-plugin-session/README.md#d-078-请求视图三层淡化退役请求体是事实网格的纯投影注入段只追加不改写 "crates/symbio-plugin-session/src/README")）：请求里的历史是
//! 事实网格的纯投影（[D-048a](../../symbio-plugin-session/README.md#d-048a-v2-的-prompt-形状必须回到结构化消息不把工具结果拍平成纯文本 "crates/symbio-plugin-session/src/README")），请求视图级的
//! 内容淡化 / 轮次淡化 / 分层滑窗骨架化三层已退役。本域对历史的唯一动作是
//! **换基**（语义压缩写成 `session.consolidated` 一格事实），不是改写视图。
//!
//! 分层：门面与 `view` / `conversation_view` / `prompt` 是**纯策略**（无 I/O），
//! `pipeline` 是**执行**（LLM 请求 / 落库 / 广播），是本域唯一的副作用出口。

use std::sync::Arc;

use symbio_core::chat_node::{ChatNode, NodeKind, NodeRole};
use symbio_core::PluginInvokeRequest;

pub(crate) use super::message_build::TurnOutputExt;

mod conversation_view;
mod pipeline;
mod prompt;
mod providers;
mod view;

// 门面 = 域内唯一对外出口：调用方（`chat_loop` / `resume`）只经 `context::X` 取用，
// 不感知域内文件划分；域内子模块则经 `use super::*` 取用门面（含下面的 re-export）。
pub use self::conversation_view::{conversation_view, CONVERSATION_VIEW_LIMIT};
pub(crate) use self::pipeline::{auto_compress_process, retry_compaction};
pub use self::prompt::{
    build_compression_request, compression_prompt_fingerprint, extract_snapshot,
    get_compression_prompt, render_snapshot_for_history, COMPRESSION_PROTOCOL_VERSION,
};
pub use self::view::build_request_sections;

const COMPRESSION_TOKEN_THRESHOLD: f64 = 0.7;
const COMPRESSION_PRESERVE_THRESHOLD: f64 = 0.3;
const MIN_COMPRESSION_FRACTION: f64 = 0.05;

/// 迟滞系数：快照后估算再增长不足 (post_tokens × 1.15) 时跳过被动压缩。
pub const COMPACT_HYSTERESIS_FACTOR: f64 = 1.15;
/// 自动压缩最小收益：待压缩历史低于该 token 数时不值得一次 LLM 调用。
pub const MIN_COMPACT_TOKENS: usize = 4000;

/// 压缩收益护栏：待压缩历史是否够大，值得花一次 LLM 摘要调用。
///
/// 门槛语义只有这一个出处（自动与用户重试共用——重试走
/// `auto_compress_process(force)`，判定链同一条）。
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
pub fn snapshot_slot_seq(compressed: &[ChatNode], keep: &[ChatNode]) -> Option<i64> {
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
fn find_compress_split_point(messages: &[ChatNode], fraction: f64) -> usize {
    if fraction <= 0.0 || fraction >= 1.0 || messages.is_empty() {
        return 0;
    }

    // 尾部未配对 ToolCall 检查：从末尾往前扫，遇到 Tool 结果即配对完成；
    // 若先遇到 Assistant 的 ToolCall 节点（其结果不在其后），说明有悬空调用。
    let mut pending_tool_call = false;
    for m in messages.iter().rev() {
        match m.node_role() {
            Some(NodeRole::Tool) => {
                pending_tool_call = false;
                break;
            }
            Some(NodeRole::Assistant) if m.is_tool_call() => {
                pending_tool_call = true;
                break;
            }
            Some(NodeRole::Assistant) => {
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
        if msg.node_role() == Some(NodeRole::User) {
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

/// 估算单条消息的 token 数。
///
/// 统一使用 `CalibratedTokenizer`（启发式 × provider 用量反馈校准），
/// **禁止**裸 `text.len()/4`：UTF-8 下中文每字 3 字节，`len()/4` 会低估约 2 倍，
/// 导致压缩触发过晚、请求撞上 provider 的 context-length 400。
///
/// 计入：正文内容 + LLM prompt 前缀（时间/工作区上下文，`to_api_value` 会拼进
/// 实际请求文本并由 provider 计费）+ ToolCall 节点参数（content 即 JSON 文本）
/// + 每消息固定结构开销。
pub fn estimate_message_tokens(m: &ChatNode) -> usize {
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
    if let Some(name) = m.tool_name() {
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
pub fn estimate_context_tokens(messages: &[ChatNode], overhead_tokens: usize) -> usize {
    messages.iter().map(estimate_message_tokens).sum::<usize>() + overhead_tokens
}

/// 上下文段的预算（token）：上限减去**已经被占用的那一体**（历史消息 + 请求级固定开销）。
///
/// 与 [`context_watermark`] 的 `request_tokens` 同一把尺（`estimate_context_tokens`）：
/// 水位是给出口看的读数，本函数是给裁剪的上界——两个数若各算各的，就会出现
/// 「水位说已满、段却以为自己还有空间」。
pub fn section_budget(
    messages: &[ChatNode],
    overhead_tokens: usize,
    context_limit: usize,
) -> usize {
    context_limit.saturating_sub(estimate_context_tokens(messages, overhead_tokens))
}

/// 估算请求级固定开销：system prompt + **给定**工具定义。
///
/// 与 [`estimate_request_overhead`] 算法完全一致，区别只在于**不自己取工具**——
/// 由调用方传入已收集好的工具清单。会话主循环每轮只收集一次工具
/// （见 `chat_loop::collect_reads`），应优先使用本函数复用该清单，
/// 避免同一轮内对 visitor 注册表重复查询。
pub fn estimate_overhead_with_tools(
    system_prompt: &str,
    tools: &[symbio_core::CapabilityMeta],
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
    let tools = symbio_core::tool_metas(ctx).await;
    estimate_overhead_with_tools(system_prompt, &tools)
}

/// 自动压缩阈值（token）——**唯一出处**。
///
/// 触发判定（[`should_start_compression`]）与交给前端的读数都从这里取：系数只在
/// 本模块出现一次，下游「自己乘一遍 0.7」就是第二套口径，而它会在改系数那天
/// 静默失真。
fn compression_threshold_tokens(context_limit: usize) -> usize {
    (context_limit as f64 * COMPRESSION_TOKEN_THRESHOLD) as usize
}

/// 本轮请求开头的**上下文水位**——四个绝对 token 数，进 Turn 骨架节点的
/// `meta.context`（载荷键的读方是 session 的 `stats_reading::context_reading_of`）。
///
/// ## 为什么是「本轮请求开头」而不是「此刻」
///
/// 水位是**请求级事实**：它是「这一体发出去时有多大」的一次测量，只在轮开头算
/// （`prepare_request_sections` 的 ③″④⑥）。轮内工具结果不断长进来，中途没有一个时刻
/// 存在「当前水位」这个数——造一个每几秒重估一次的数会把估算成本请进读路径，
/// 而它自报的精度并不会更高。
///
/// ## 四个数都是**绝对值**，一个比率都不带
///
/// `request / limit` 这种除法留给出口：出口只除一次（`format_tokens` 同一个函数里），
/// 而前端拿到比率又会再印一遍文案，两处各自排版必然漂移。系数也不写死在前端——
/// 它跟着 `compress_tokens` 这一个数走，改 0.7 不需要动前端。
///
/// ⚠️ **估算，不是实测**：`CalibratedTokenizer` 的启发式 × provider 用量反馈校准，
/// 与真实计费 token 有偏差。所以读数该说的是「够不够、离自动整理还差多少」，
/// 不是「花了多少」——后者归成本台账（`cost_ledger`）。
pub fn context_watermark(
    messages: &[ChatNode],
    overhead_tokens: usize,
    context_limit: usize,
) -> serde_json::Value {
    serde_json::json!({
        // 内容水位：会话消息本身（不含请求级开销）。与压缩快照的 `post_tokens`
        // 同一个口径，迟滞判定用的就是它。
        "content_tokens": estimate_context_tokens(messages, 0) as u64,
        // 请求水位：内容 + system prompt + 全部工具 schema——真正发给模型的那一体积，
        // 与阈值相比的是它（见 `should_start_compression`）。
        "request_tokens": estimate_context_tokens(messages, overhead_tokens) as u64,
        "limit_tokens": context_limit as u64,
        "compress_tokens": compression_threshold_tokens(context_limit) as u64,
    })
}

/// 检查是否需要压缩，返回是否应该开始压缩。
///
/// `overhead_tokens`：system prompt + 工具定义等请求级固定开销，必须计入。
/// `force`：跳过阈值判断（用户 /compact 或主动压缩工具）。
pub fn should_start_compression(
    messages: &[ChatNode],
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
    let threshold = compression_threshold_tokens(context_limit);

    if current < threshold {
        return false;
    }

    // 迟滞：若最近一次快照后已增长不足 15%，说明上一轮压缩收益被挥霍，
    // 再次压缩大概率是"快照过小"导致的循环调用，跳过并依赖确定性层消化。
    // 例外：force（用户主动重试）不受此限制。
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

/// 准备压缩：将要压缩的历史提取出来，生成压缩请求
/// 返回 (压缩请求, 要压缩的历史, 要保留的历史)
pub fn prepare_compression(
    messages: &[ChatNode],
) -> Option<(Vec<ChatNode>, Vec<ChatNode>, Vec<ChatNode>)> {
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

    // 生成压缩请求：历史对话原样 + 末尾指令（唯一构造点）。
    // 压缩模板由调用方经 system role 注入，避免格式指令在对话中出现两次
    // （system 一次 + user 一次）诱导模型模仿输出。
    let compression_request = build_compression_request(&history_to_compress);

    Some((compression_request, history_to_compress, history_to_keep))
}

#[cfg(test)]
#[path = "context.test.rs"]
mod tests;
