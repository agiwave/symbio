//! 读路径：存储视图与进入 LLM 上下文的候选集。
//!
//! 三个读侧纯函数（滑动轮次窗口 / content 归一 / 孤儿剔除）与两个读方法
//! 同处一文件：它们共同回答「哪些消息该进这次请求」，与写侧 [`super::write`]
//! 的「哪些消息能留在存储里」相对。

use super::*;

fn sliding_window(messages: &[ChatMessage], max_turns: usize) -> Vec<ChatMessage> {
    if max_turns == 0 {
        return messages.to_vec();
    }

    let mut user_indices = Vec::new();
    for (idx, msg) in messages.iter().enumerate() {
        if msg.role == Some(MessageRole::User) {
            user_indices.push(idx);
        }
    }

    if user_indices.len() <= max_turns {
        return messages.to_vec();
    }

    let start_idx = user_indices[user_indices.len() - max_turns];
    messages[start_idx..].to_vec()
}

/// 返回给 LLM 前对单条消息做 content 归一兜底：
/// - 角色为 None 的占位消息不入上下文，直接跳过（避免 role 空污染 LLM）；
/// - content 为合法 `Text`/`Parts` 的保留原样；
/// - 缺失 content 的一律补成空串（**不再原样放行 `None`**）。
///
/// ## 为什么不能再放行 `content: None`
///
/// `NativeMessage::to_api_value` 对 `None` 输出 JSON `null`，而 Provider 侧
/// `MessageContent` 是 `String | ContentBlock[]` 的 untagged enum，`null`
/// 两个变体都不匹配，于是整包被 400 拒绝：
/// `messages[1]: data did not match any variant of untagged enum MessageContent`。
/// 一条脏消息就能让整个会话彻底卡死在 400，且用户无法自行恢复。
///
/// 仅作用于返回的构造结果，不修改存储。
fn normalize_message_content(msg: ChatMessage) -> Option<ChatMessage> {
    // 占位消息（无角色）直接丢弃；as_ref 避免部分移动（后续 ..msg 复用其余字段）
    msg.role.as_ref()?;
    let content_ok = matches!(
        &msg.content,
        Some(MessageContent::Parts(_)) | Some(MessageContent::Text(_))
    );
    if content_ok {
        return Some(msg);
    }
    let raw = msg
        .content
        .as_ref()
        .map(|c| c.to_text())
        .unwrap_or_default();
    let text = if raw.is_empty() {
        // 组合节点（Turn/ToolCall）与无内容消息统一落空串，保证 JSON 里是合法字符串
        String::new()
    } else {
        raw
    };
    Some(ChatMessage {
        content: Some(MessageContent::Text(text)),
        ..msg
    })
}

/// 丢弃"孤儿"消息：`parent_id` 指向本批次里不存在的节点。
///
/// 孤儿来源：
/// - Failed Turn 被 `resume::process_retry_turn` 删除时若子节点未一并清理；
/// - `persist_failure` 把仅存在于内存的流式子节点直接追加进存储；
/// - `resume::process_tool_resume_action` 只删子节点、父节点已被其它路径清理。
///
/// 这些节点会被 `flatten_chat_messages` 当成根节点单独发一条 native message，
/// 其中 tool 结果还会携带一个请求里根本不存在的 `tool_call_id`，Provider 直接报错。
/// 因此进上下文前统一剔除（根级的 Turn / User 无 parent，天然不受影响）。
///
/// 采用**不动点迭代**：一次截断可能同时打断多层父子链（Turn → ToolCall → Tool），
/// 单趟过滤只解一层，剩下的会以"父在集合内"的假象漏出。
pub(crate) fn drop_orphan_messages(mut messages: Vec<ChatMessage>) -> Vec<ChatMessage> {
    // 不动点迭代：父节点被剔除后，其子节点也随之成为孤儿（截断/清理场景下
    // 可能出现多层链，如 Turn → ToolCall → Tool）。单趟过滤只解一层。
    loop {
        let ids: std::collections::HashSet<String> =
            messages.iter().map(|m| m.id.clone()).collect();
        let before = messages.len();
        messages.retain(|m| match m.parent_id.as_deref() {
            None => true,
            Some(pid) => ids.contains(pid),
        });
        if messages.len() == before {
            return messages;
        }
    }
}

impl PersistentChatSession {
    /// 全量消息（按单调 `seq` 排序后的存储视图）。
    pub(crate) async fn get_messages(&self) -> Result<Vec<ChatMessage>, PluginError> {
        let session = self.load_session().await?;
        let mut messages: Vec<_> = session.messages.to_vec();
        // 按**单调序号** `seq` 排序（稳定排序，缺失 seq 的旧数据排最后并保持插入顺序）。
        //
        // 这里曾经用 `sort_by_key(|m| m.timestamp)`，有两个致命问题：
        //   1. `Option` 序是 `None < Some(_)`，缺失 timestamp 的消息被顶到最前面，
        //      会话顺序被打乱，父节点可能排到子节点之后；
        //   2. timestamp 是"时刻"不是"顺序"——同一毫秒批量落库的消息会并列，
        //      排序退化为依赖数组当前顺序，而数组顺序又会被上一次错误排序打乱。
        // 改用写入时分配的单调 seq 后，两者都被消除。
        messages.sort_by_key(|m| m.seq.unwrap_or(i64::MAX));

        Ok(messages)
    }

    /// 获取进入 LLM 上下文的候选消息（存储视图：过滤 + 滑动轮次窗口）。
    pub(crate) async fn get_context_messages(
        &self,
        max_turns: Option<usize>,
    ) -> Result<Vec<ChatMessage>, PluginError> {
        let messages = self.get_messages().await?;
        // **不过滤 Failed 消息**（"继续会话"中断可见性，docs/turn-tool-mechanisms.md 2.6）：
        // 用户选择不重试、直接继续对话时，模型必须看到上一轮的中断现场——
        // 失败 Turn 的半截输出 + 中断说明——否则思维链断裂。
        // persist_failure 只把根 Turn 标 Failed（半截子节点定稿 Completed），
        // 因此整树保留即可；中断说明与占位工具结果由请求视图层
        // （flatten_chat_messages / build_request_view）按 status 动态补齐。
        // 下方孤儿过滤退化为安全网（正常路径失败 Turn 整树保留，无孤儿产生）。
        // 剔除父节点缺失的孤儿节点（否则会带着不存在的 tool_call_id 进请求包）
        let messages: Vec<ChatMessage> = drop_orphan_messages(messages);
        // 对各消息做 content 归一兜底（并跳过 role 为 None 的占位消息），
        // 防止历史坏消息导致 provider 反序列化 MessageContent 失败。
        let messages: Vec<ChatMessage> = messages
            .into_iter()
            .filter_map(normalize_message_content)
            .collect();
        let turns = max_turns.unwrap_or_else(|| {
            self.config
                .try_read()
                .map(|c| c.context_messages)
                .unwrap_or_else(|_| default_context_messages())
        });
        let result = sliding_window(&messages, turns);
        // 会话层只做全局轮次窗口；工具级骨架化/保留策略由模型插件 run_chat_loop
        // 构建请求视图时统一解析（build_request_view），避免对压缩原料的提前污染。
        Ok(result)
    }
}
