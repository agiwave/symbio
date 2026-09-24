//! 消息构造与会话持久化
//!
//! 负责：
//! - 构造 ChatMessage（assistant 消息、tool result 消息）
//! - 将对话轮次产生的新消息保存到 session

use std::collections::{HashMap, HashSet};

use crate::symbio_core::schemas::session::chat_message::{
    ChatMessage, MessageContent, MessageRole, MessageStatus, MessageType,
};

use super::types::*;

// Flatten ChatMessage to NativeMessage

/// LLM 请求包中保留的最近思考条数。
///
/// 思考链的价值随距离衰减：模型只需最近几条推理来"接上思路"，
/// 全量回传历史思考既浪费 token，也可能诱导模型复述旧结论。
/// 存储层 Reasoning 节点完整保留，此处只裁剪请求视图
/// （Anthropic 协议本就不回传历史思考——缺官方签名，此裁剪对它无副作用）。
const RETAINED_RECENT_REASONING: usize = 2;

/// 将存储的细粒度 ChatMessage 树扁平化，转换为 API 所需的 NativeMessage 列表。
///
/// 分型树结构（请求/响应由 MessageRole 区分，组合节点可选）：
/// - 根级交替：`User`(请求, `Text`) 与 `Turn`(响应, `Assistant` 组合) 互为兄弟
/// - `Turn` 子节点：`Reasoning` / `Text`(回复) / `ToolCall`(`Assistant` 组合)
/// - `ToolCall`：`Assistant` 组合节点，自身 `content` 携带请求参数（JSON 文本）；
///   子节点仅剩 `Text`(响应结果, `Tool`)
///   （响应结果也可包在 `Turn`(`Tool`) 内，视复杂度而定；组合节点可选）
///
/// 扁平化规则：
/// - `Turn` → 父 `assistant` native message（聚合 Reasoning / Text / tool_calls）
/// - `ToolCall` → 从其自身 `content` 读 args 拼入父 `assistant.tool_calls`
/// - 响应结果 `Text`(`Tool`) → 独立的 `role=tool` native message（tool_call_id = ToolCall 的 wire id）
/// - `User` / `System` 等 → 原样输出
/// - 失败 `Turn`（status=Failed）：半截输出照常聚合，并附加中断说明段落；
///   无结果的 `ToolCall` 合成占位 tool 结果、失败的工具结果推导 `success=false`
///   （"继续会话"中断可见性）
pub fn flatten_chat_messages(messages: &[ChatMessage]) -> Vec<NativeMessage> {
    let by_id: HashMap<&str, &ChatMessage> = messages.iter().map(|m| (m.id.as_str(), m)).collect();
    let mut children: HashMap<&str, Vec<&ChatMessage>> = HashMap::new();
    for m in messages {
        if let Some(pid) = m.parent_id.as_deref() {
            children.entry(pid).or_default().push(m);
        }
    }

    // 节点 id → wire id（provider 原始 tool_call_id）映射。
    //
    // ToolCall 节点 id 与 wire id 分离后（节点 id 会话内唯一，wire id 可能被
    // 网关跨轮复用），请求包必须回传 provider 认识的 wire id——部分协议要求
    // `tool_call_id` 与其签发值逐字一致（如 OpenAI Responses 的 call_id 链）。
    // 历史数据无 `tool_call_id` 字段 → 回退节点 id（旧数据里两者本就是同一个值）。
    let wire_id_of = |m: &ChatMessage| -> String {
        m.tool_call_id
            .clone()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| m.id.clone())
    };
    let wire_ids: HashMap<&str, String> = messages
        .iter()
        .filter(|m| m.msg_type == Some(MessageType::ToolCall))
        .map(|m| (m.id.as_str(), wire_id_of(m)))
        .collect();

    let is_root = |m: &ChatMessage| -> bool {
        m.parent_id
            .as_deref()
            .map(|p| by_id.contains_key(p))
            .unwrap_or(true)
    };

    let mut result: Vec<NativeMessage> = Vec::new();

    // 已聚合消费的节点 id 集合：每个根级 Turn 在聚合时会把自身及全部子孙合并进
    // 一条 assistant 消息。此处仅把「子孙」id 收集进来（不含 Turn 自身），这样：
    //  - 若同一节点在列表中以「裸根」形式（parent_id 为 None 或指向缺失节点）重复出现，
    //    会被跳过，避免 reasoning / tool_call 参数 / tool_result 在 LLM 请求中重复；
    //  - Turn 自身不会被误标记，保证它能被主循环正常聚合（把 Turn 自身 id 也收进
    //    consumed 会让根级 Turn 被跳过、永不聚合）。
    let mut consumed: HashSet<&str> = HashSet::new();
    /// 递归收集某 Turn 的全部子孙 id（不含 Turn 自身）到 `acc`
    fn collect_descendants<'a>(
        children: &HashMap<&'a str, Vec<&'a ChatMessage>>,
        acc: &mut HashSet<&'a str>,
        id: &'a str,
    ) {
        // 从 Turn 的直接子节点开始递归收集全部子孙 id（不含 Turn 自身）
        let mut stack: Vec<&'a str> = Vec::new();
        if let Some(kids) = children.get(id) {
            for kid in kids {
                stack.push(kid.id.as_str());
            }
        }
        while let Some(cur) = stack.pop() {
            acc.insert(cur);
            if let Some(kids) = children.get(cur) {
                for kid in kids {
                    stack.push(kid.id.as_str());
                }
            }
        }
    }
    for m in messages {
        if is_root(m) && m.msg_type == Some(MessageType::Turn) {
            collect_descendants(&children, &mut consumed, m.id.as_str());
        }
    }

    // ── 请求视图思考裁剪 ─────────────────────────────────────────────────
    // 按列表序（时间序）收集携带 Reasoning 子节点的根级 Turn，
    // 仅最近 RETAINED_RECENT_REASONING 条允许回传 reasoning_content：
    // 完全清空思考会让模型"失忆"后重复思考；全量回传历史思考既浪费
    // token，也可能诱导模型复述旧结论。只裁请求视图，存储层完整保留。
    let turns_with_reasoning: Vec<&str> = messages
        .iter()
        .filter(|m| {
            is_root(m)
                && m.msg_type == Some(MessageType::Turn)
                && children
                    .get(m.id.as_str())
                    .map(|kids| {
                        kids.iter()
                            .any(|c| c.msg_type == Some(MessageType::Reasoning))
                    })
                    .unwrap_or(false)
        })
        .map(|m| m.id.as_str())
        .collect();
    let retained_reasoning_turns: HashSet<&str> = turns_with_reasoning
        .iter()
        .rev()
        .take(RETAINED_RECENT_REASONING)
        .copied()
        .collect();

    for m in messages {
        // 已被聚合消费（作为某 Turn 的子节点）的节点不单独发出
        if consumed.contains(m.id.as_str()) {
            continue;
        }
        if !is_root(m) {
            continue; // 非根节点会在其父节点处理时被合并
        }

        match m.msg_type {
            Some(MessageType::Turn) => {
                let mut native: NativeMessage = m.clone().into();
                native.role = MessageRole::Assistant;
                native.content = None;
                native.reasoning_content = None;
                native.tool_calls = None;

                let mut tool_results: Vec<NativeMessage> = Vec::new();

                if let Some(kids) = children.get(m.id.as_str()) {
                    for child in kids {
                        match child.msg_type {
                            Some(MessageType::Reasoning) => {
                                // 仅最近 N 条思考进入请求视图
                                if retained_reasoning_turns.contains(m.id.as_str()) {
                                    native.reasoning_content =
                                        child.content.as_ref().map(|c| c.to_text());
                                }
                            }
                            Some(MessageType::Text) => {
                                native.content = child.content.clone();
                            }
                            Some(MessageType::ToolCall) => {
                                // 请求参数直接来自 ToolCall 节点自身的 content（JSON 文本）。
                                // 该文本在落库时（turn.rs:645）已由 `tc.arguments.to_string()`
                                // 规范化——即 serde_json 输出，故 re-parse 再 re-serialize 幂等。
                                // 因此**以字符串原样透传**：下游 types.rs 的 `is_string()` 快路径
                                // 会直接发出这段文本，省掉一次 Value 物化 + re-serialize（评审 §3.2）。
                                // 仅当文本非法时才回退 `{}`，与原 `from_str().unwrap_or({})` 行为完全一致
                                // （落库时 broken 原文会带 parse_error，正常路径不会走到这里）。
                                let args_text = child
                                    .content
                                    .as_ref()
                                    .map(|c| c.to_text())
                                    .unwrap_or_default();
                                let args: serde_json::Value =
                                    match serde_json::from_str::<serde_json::Value>(&args_text) {
                                        Ok(_) => serde_json::Value::String(args_text),
                                        Err(_) => serde_json::json!({}),
                                    };
                                let tc = ToolCall {
                                    id: Some(
                                        wire_ids
                                            .get(child.id.as_str())
                                            .cloned()
                                            .unwrap_or_else(|| child.id.clone()),
                                    ),
                                    kind: Some("function".to_string()),
                                    name: child.name.clone().unwrap_or_default(),
                                    arguments: args,
                                };
                                match &mut native.tool_calls {
                                    Some(calls) => calls.push(tc),
                                    None => native.tool_calls = Some(vec![tc]),
                                }

                                // 响应结果：ToolCall 下的 Text(Tool) 直接子节点，或 Turn(Tool) 内 Text
                                match find_tool_result(&children, child.id.as_str()) {
                                    Some(res) => {
                                        let mut tool_native: NativeMessage = res.clone().into();
                                        tool_native.role = MessageRole::Tool;
                                        tool_native.tool_call_id =
                                            wire_ids.get(child.id.as_str()).cloned();
                                        // 失败的工具结果推导 success=false：触发 Anthropic
                                        // tool_result 的 is_error=true，工具失败信息才能被
                                        // 模型感知；跨轮工具失败必须可见，不得被过滤丢失
                                        if res.status == Some(MessageStatus::Failed) {
                                            tool_native.success = Some(false);
                                        }
                                        tool_results.push(tool_native);
                                    }
                                    None => {
                                        // ToolCall 无对应结果（执行被打断，未产生结果）：
                                        // 合成占位 tool 结果，避免请求包出现"有 tool_call
                                        // 无 tool_result"触发 provider 400。必须在此处
                                        // 进入 tool_results（先于尾部孤儿剔除），否则会被
                                        // 当作孤儿 tool 结果误删。
                                        let tool_name = child.name.clone().unwrap_or_default();
                                        tool_results.push(NativeMessage {
                                            role: MessageRole::Tool,
                                            tool_call_id: wire_ids.get(child.id.as_str()).cloned(),
                                            content: Some(MessageContent::Text(format!(
                                                "[工具 {tool_name} 的执行被中断，未产生结果。]"
                                            ))),
                                            ..Default::default()
                                        });
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }

                // 注：本 Turn 的全部子孙 id 已在预处理遍（consumed 预扫描）中收集，
                // 主循环开头的 `if consumed.contains(m.id) { continue; }` 会跳过它们，
                // 因此此处无需再单独标记，避免重复代码与潜在误标记。

                // **中断说明**（"继续会话"中断可见性）：
                // 失败 Turn 的半截输出原样聚合后，附加中断说明段落——模型在上下文中
                // 看到"上次输出 → 中断说明 → 用户新消息"的连贯序列，避免思维链断裂
                // （等价于用户打断了模型说话，然后继续）。
                if m.status == Some(MessageStatus::Failed) {
                    let note = match m.error.as_deref() {
                        Some(err) if !err.trim().is_empty() => format!(
                            "[本轮回复被中断（原因：{err}）。以上是中断前的部分输出，仅供参考；请结合用户接下来的消息继续。]"
                        ),
                        _ => "[本轮回复被中断。以上是中断前的部分输出，仅供参考；请结合用户接下来的消息继续。]"
                            .to_string(),
                    };
                    native.content = Some(match native.content {
                        Some(MessageContent::Text(text)) if !text.is_empty() => {
                            MessageContent::Text(format!("{text}\n\n{note}"))
                        }
                        _ => MessageContent::Text(note),
                    });
                }

                result.push(native);
                result.extend(tool_results);
            }
            // 上下文压缩节点**不是对话内容**：它是系统对历史的一次整理动作，
            // 只服务于前端呈现与事后审计。发给模型纯属噪音——既占上下文，
            // 又会把"系统整理过上下文"当成一条事实陈述摆进对话序列。
            Some(MessageType::Compression) => {}
            _ => {
                // User / System 等根级内容节点
                let mut native: NativeMessage = m.clone().into();
                if native.role == MessageRole::Tool {
                    // tool_call_id 用父 ToolCall 的 wire id（历史数据回退节点 id）
                    native.tool_call_id = m
                        .parent_id
                        .as_deref()
                        .and_then(|pid| wire_ids.get(pid).cloned())
                        .or_else(|| m.parent_id.clone());
                }
                result.push(native);
            }
        }
    }

    // ── 请求包清洗（避免把脏消息发到 provider 触发反序列化失败）────────────
    // 1. 收集本批所有 tool_call id（用于识别孤儿 tool 结果）。
    let mut tool_call_ids: HashSet<String> = HashSet::new();
    for nm in &result {
        if let Some(calls) = &nm.tool_calls {
            for tc in calls {
                if let Some(id) = &tc.id {
                    tool_call_ids.insert(id.clone());
                }
            }
        }
    }
    let mut cleaned: Vec<NativeMessage> = Vec::with_capacity(result.len());
    for nm in result {
        // 2. 丢弃"无对应 tool_call 的 tool 结果"：provider 要求 tool 角色的
        //    content 必须关联到前文某个 assistant 的 tool_calls，否则直接 400。
        if nm.role == MessageRole::Tool {
            if let Some(tid) = &nm.tool_call_id {
                if !tool_call_ids.contains(tid) {
                    continue;
                }
            } else {
                continue;
            }
        }
        // 3. 丢弃"空 assistant 回合"：既无文本/推理内容、也无工具调用，
        //    发到 provider 是无意义且可能触发校验错误的占位消息。
        if nm.role == MessageRole::Assistant
            && nm.content.as_ref().map(|c| c.is_empty()).unwrap_or(true)
            && nm.reasoning_content.is_none()
            && nm.tool_calls.is_none()
        {
            continue;
        }
        cleaned.push(nm);
    }

    cleaned
}

/// 在 ToolCall 下查找响应结果节点：
/// 1. 直接的 `Text`(role=Tool) 子节点（非空内容优先）
/// 2. 或被 `Turn`(role=Tool) 包装的 `Text` 子节点（组合节点可选时的兼容形式，
///    子 agent 会话即以此种方式返回完整会话）
/// 3. 兜底：任意直接的 `Text`(role=Tool) 子节点
fn find_tool_result<'a>(
    children: &HashMap<&str, Vec<&'a ChatMessage>>,
    tool_call_id: &str,
) -> Option<&'a ChatMessage> {
    let kids = children.get(tool_call_id)?;
    // 1. 优先直接 Text(Tool) 且内容非空
    if let Some(direct) = kids.iter().find(|c| {
        c.role == Some(MessageRole::Tool)
            && c.msg_type == Some(MessageType::Text)
            && c.content.as_ref().map(|x| !x.is_empty()).unwrap_or(false)
    }) {
        return Some(direct);
    }
    // 2. 被 Turn(Tool) 包装（子 agent 会话）
    if let Some(turn) = kids
        .iter()
        .find(|c| c.msg_type == Some(MessageType::Turn) && c.role == Some(MessageRole::Tool))
    {
        if let Some(tkids) = children.get(turn.id.as_str()) {
            return tkids
                .iter()
                .find(|c| c.msg_type == Some(MessageType::Text))
                .copied();
        }
    }
    // 3. 兜底：任意直接 Text(Tool)
    kids.iter()
        .find(|c| c.role == Some(MessageRole::Tool) && c.msg_type == Some(MessageType::Text))
        .copied()
}

// ChatMessage 构造（build_assistant_messages / build_tool_message / short_id /
// StreamChildIds）实现在 symbio_core::turn：本文件仅测试消费，由 tests 模块
// 直接引用 core（避免 lib 侧 unused import 警告）

#[cfg(test)]
#[path = "message_builder.test.rs"]
mod tests;
