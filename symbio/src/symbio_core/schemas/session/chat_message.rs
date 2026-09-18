use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 消息角色
///
/// 请求 / 响应由角色区分（分型结构）：
/// - 顶层会话：`User`(请求) → `Turn`(`Assistant`，响应)
/// - 工具调用：`ToolCall`(`Assistant`，请求) → `Turn`(`Tool`，响应)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum MessageRole {
    #[default]
    User,
    Assistant,
    Tool,
    System,
}

/// 消息类型
///
/// `Turn` 与 `ToolCall` 是组合节点（不含实际内容，仅分组子节点）；
/// `Text` / `Reasoning` 是内容节点（携带可显示内容）。
/// 工具调用是分型结构：`ToolCall`(请求) 的子节点包含一个 `Turn`(`Tool`，响应)，
/// 该响应 `Turn` 与顶层助手响应 `Turn` 结构完全一致（可再嵌套 `ToolCall`）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum MessageType {
    #[default]
    Text,
    Reasoning,
    /// 组合节点：一轮响应（顶层为 `Assistant`，工具响应为 `Tool`），仅分组子节点
    Turn,
    /// 组合节点：一次工具调用（请求），携带 `name`，其子节点含请求 `Text` 与响应 `Text/Turn`
    ToolCall,
    /// 待用户响应节点：承载 `ask_user` 提问或工具执行前确认（confirm）。
    /// 状态为 `WaitingUserAction` 时表示等待用户输入，答案以一条普通 `user`
    /// 消息（`meta.responds_to` 指向本节点 id）回填，回填后本节点标记 `Completed`。
    /// 结构化载荷存放于 `meta.prompt`，详见会话激活/恢复状态机设计文档。
    UserPrompt,
    /// 上下文压缩节点：**不是对话内容**，是系统对历史的一次整理动作。
    ///
    /// 以消息节点而非「会话级横幅」呈现，是因为压缩本就是会话里发生的一步：
    /// 它有自己的位置（当前时刻）、自己的状态（进行中 / 已完成），被压掉的历史
    /// 就发生在它之前。横幅没有位置概念——用户滚到消息流中间时看不见它，
    /// 事后也无法回溯「上次压缩发生在哪里、压掉了多少」。
    Compression,
}

/// 消息状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum MessageStatus {
    #[default]
    Pending,
    Streaming,
    WaitingUserAction,
    Completed,
    /// 用户主动终止。
    ///
    /// 它既不是 `Failed`（没有出错，是用户按了停止）也不是 `Completed`（没有跑完，
    /// 内容是半截的）。此前中止只能二选一：冒泡路径标 `Failed`（给一个 ⚠ 错误角标，
    /// 把用户自己的操作渲染成故障），其余路径标 `Completed`（说谎——那一轮根本没跑完）。
    ///
    /// 语义上它**可重试**：`RetryTurn` 本就不看状态（只校验节点是 Turn），
    /// 缺的只是前端一个「这个终态可以重试」的判据。
    Aborted,
    Failed,
}

impl MessageStatus {
    /// 状态词——**与 `serde` 的序列化名逐字一致**（由单测锁死）。
    ///
    /// ## 为什么需要它
    ///
    /// VDFS 节点投影（`plugins/session/plugin/nodes.rs::message_status`）要把状态
    /// 写进 `VdfsNode.status`，而前端按同一套词读回。若那里手写字符串字面量，
    /// 枚举改名时就会出现「存储写 `streaming`、节点报 `streaming` 之外的词」
    /// 这种**编译期看不见**的分叉。
    ///
    /// 于是状态词只有一份定义（本函数），投影与序列化都从它派生。
    pub fn as_str(&self) -> &'static str {
        match self {
            MessageStatus::Pending => "pending",
            MessageStatus::Streaming => "streaming",
            MessageStatus::WaitingUserAction => "waiting_user_action",
            MessageStatus::Completed => "completed",
            MessageStatus::Aborted => "aborted",
            MessageStatus::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageUrl {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    Text { text: String },
    ImageUrl { image_url: ImageUrl },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Parts(Vec<ContentPart>),
}

impl MessageContent {
    pub fn is_empty(&self) -> bool {
        match self {
            MessageContent::Text(s) => s.is_empty(),
            MessageContent::Parts(p) => p.is_empty(),
        }
    }

    pub fn to_text(&self) -> String {
        match self {
            MessageContent::Text(s) => s.clone(),
            MessageContent::Parts(parts) => parts
                .iter()
                .filter_map(|p| match p {
                    ContentPart::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(" "),
        }
    }

    pub fn len(&self) -> usize {
        self.to_text().len()
    }

    pub fn truncate(&mut self, max_len: usize) {
        match self {
            MessageContent::Text(s) => {
                if s.len() > max_len {
                    *s = s.chars().take(max_len).collect();
                }
            }
            MessageContent::Parts(_) => {
                let text = self.to_text();
                if text.len() > max_len {
                    let truncated: String = text.chars().take(max_len).collect();
                    *self = MessageContent::Text(truncated);
                }
            }
        }
    }
}

impl Default for MessageContent {
    fn default() -> Self {
        MessageContent::Text(String::new())
    }
}

/// 聊天消息定义 (所有字段增量可选，通过 id 归并)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChatMessage {
    pub id: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<MessageRole>,

    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub msg_type: Option<MessageType>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    #[serde(skip_serializing)]
    pub prompt: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<MessageContent>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<MessageStatus>,

    /// 失败原因（面向用户的可读短消息）。仅当 `status == Failed` 时存在。
    /// 用于把"会话异常中断/失败"的终态持久化到历史，使切换会话后仍能看到上次错误。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<i64>,

    /// 会话内**单调自增序号**：消息先后顺序的权威锚点。
    ///
    /// ## 为什么不用 `timestamp` 排序
    ///
    /// `timestamp` 是"业务时刻"，不是"顺序"。拿它当排序键会有三个硬伤：
    /// - **并列**：一轮里的 Turn + Reasoning + Text + 多个 ToolCall 常常在同一毫秒内
    ///   批量落库，`timestamp` 完全相同，排序只能退化成"看数组当前顺序"，
    ///   而数组顺序本身又可能被上一次错误排序打乱，形成自我强化的错乱；
    /// - **逆序**：时钟回拨、多进程/多会话写入会让"晚产生"的消息拿到更小的时间戳；
    /// - **缺失**：流式补丁路径不写 `timestamp`，需要哨兵值（如 `i64::MAX`）兜底，
    ///   本质是在用魔法值掩盖语义缺失。
    ///
    /// `seq` 由会话存储在**写入时**分配（Lamport 计数器：从已有最大 seq 续上），
    /// 单调递增且永不并列，因此能无损恢复插入顺序，且跨调用、跨进程重启都不回退。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>,
}

impl ChatMessage {
    /// 把 `patch` 合并进 `self`（**增量语义**，与前端 `handleChatEvent` 对齐）。
    ///
    /// # 为什么必须合并而不是整条覆盖
    ///
    /// 调用方常常只构造"局部补丁"（例如 `ChatMessage { id, meta, ..Default::default() }`，
    /// 只为给一个 ToolCall 打 `recoverable` 标记）。若存储层用 `*existing = patch` 整条覆盖，
    /// 该消息的 `role` / `msg_type` / `content` / `name` / `timestamp` 会被一并抹成 `None`：
    ///   · 前端渲染丢失类型与内容（节点退化成空壳）；
    ///   · 下一轮 `flatten_chat_messages` 把它转成一条 `role` 缺失、`content` 缺失的
    ///     native message，请求体里出现 `"content": null`，Provider 侧 untagged enum
    ///     `MessageContent` 反序列化失败 → 整个会话被一条 400 卡死。
    /// 因此这里统一规定：`None` 表示"不修改该字段"，`Some` 才覆盖。
    ///
    /// # 合并规则
    ///
    /// - `content`：`Text` / `Reasoning` 走 **增量追加**（SSE delta 语义）；
    ///   `ToolCall` 与 `Parts` 走 **全量替换**（每帧都是完整参数/内容）。
    /// - `meta`：浅合并（同键以 patch 为准）。
    /// - 其余字段：`Some` 覆盖，`None` 保留原值。
    /// - `error`：显式区分"清空"——只有 patch 携带 `error` 字段时才生效，
    ///   由于 `Option<String>` 无法区分"不修改"和"清空"，约定 `Some(String::new())` 表示清空。
    pub fn apply_patch(&mut self, patch: &ChatMessage) {
        if let Some(role) = &patch.role {
            self.role = Some(role.clone());
        }
        if let Some(t) = &patch.msg_type {
            self.msg_type = Some(t.clone());
        }
        if let Some(n) = &patch.name {
            self.name = Some(n.clone());
        }
        if let Some(p) = &patch.parent_id {
            self.parent_id = Some(p.clone());
        }
        if let Some(s) = &patch.status {
            self.status = Some(s.clone());
        }
        if let Some(ts) = patch.timestamp {
            self.timestamp = Some(ts);
        }
        // seq 是顺序锚点，只能由存储层在写入时分配；补丁里不带就表示"保持不变"。
        if let Some(s) = patch.seq {
            self.seq = Some(s);
        }
        if let Some(rid) = &patch.response_id {
            self.response_id = Some(rid.clone());
        }
        if patch.prompt.is_some() {
            self.prompt = patch.prompt.clone();
        }
        if let Some(e) = &patch.error {
            // Some("") 语义为"清空错误原因"
            self.error = if e.is_empty() { None } else { Some(e.clone()) };
        }

        if let Some(new_content) = &patch.content {
            match self.msg_type {
                Some(MessageType::ToolCall) => {
                    // 工具调用参数是每帧全量的 JSON
                    self.content = Some(new_content.clone());
                }
                Some(MessageType::Text) | Some(MessageType::Reasoning) => {
                    match (&mut self.content, new_content) {
                        (Some(existing), MessageContent::Text(new_text)) => match existing {
                            MessageContent::Text(buf) => buf.push_str(new_text),
                            other => *other = MessageContent::Text(new_text.clone()),
                        },
                        (None, MessageContent::Text(new_text)) => {
                            self.content = Some(MessageContent::Text(new_text.clone()));
                        }
                        _ => self.content = Some(new_content.clone()),
                    }
                }
                _ => self.content = Some(new_content.clone()),
            }
        }

        if let Some(new_meta) = &patch.meta {
            match &mut self.meta {
                Some(existing) => {
                    if let (Some(dst), Some(src)) = (existing.as_object_mut(), new_meta.as_object())
                    {
                        for (k, v) in src {
                            dst.insert(k.clone(), v.clone());
                        }
                    } else {
                        self.meta = Some(new_meta.clone());
                    }
                }
                None => self.meta = Some(new_meta.clone()),
            }
        }
    }
}

/// 取一批消息中已有的最大 `seq`，作为后续分配的起点（Lamport 计数器的当前水位）。
pub fn max_seq(messages: &[ChatMessage]) -> i64 {
    messages.iter().filter_map(|m| m.seq).max().unwrap_or(0)
}

/// 按切片顺序为消息补发 / **纠正** `seq`，返回分配后的新水位。
///
/// # 不变式：`seq` 必须沿数组单调不减
///
/// 调用方（`replace_messages`）的契约是「**数组顺序即权威顺序**」。因此 `seq`
/// 一旦与数组顺序不一致，它就是**错的顺序**，而不是"值得保留的历史值"。
///
/// 典型反例（L2 压缩）：新列表是 `[快照, 保留区…]`——快照是新建的（无 seq，
/// 拿到 `base+1`，成了**最大**），而保留区沿用旧的**低**序号。于是 `ordered()`
/// 会把快照排到最后：压缩后的历史记忆跑到整段转写末尾，前端表现为消息顺序错乱
/// （用户消息插进了"更早"的历史里）。这也是"保留原值"这条旧规则的字面后果。
///
/// 因此已带 `seq` 的消息也要检查：一旦它不大于已分配的水位（即会破坏单调性），
/// 就按数组顺序重新发号。
///
/// # 语义（Lamport 计数器）
///
/// - 起点是 `base`（通常是当前会话已有的最大 seq），新值严格 `base+1, base+2, …`；
/// - **正常路径不会触发任何重排**：数组顺序本就等于 seq 顺序时，既有序号原样保留
///   （由 `assign_seq_leaves_already_ordered_seqs_untouched` 锁定）——
///   本函数只在顺序**已经错了**的时候才改写既有序号；
/// - 由于起点取自"已有最大值"，跨调用、跨进程重启都不会回退或撞号。
///
/// `base = max_seq(existing)` 需在调用前算好：本批次内已带 seq 的消息不参与递增，
/// 但它们的 seq 可能大于 base，因此返回"实际达到的最大 seq"作为新水位。
pub fn assign_seq(messages: &mut [ChatMessage], base: i64) -> i64 {
    let mut cursor = base;
    for m in messages.iter_mut() {
        match m.seq {
            // 已带序号且仍大于水位：沿用，并把水位抬到它（正常路径走的都是这支）
            Some(existing) if existing > cursor => cursor = existing,
            // 缺号，**或**已带但已破坏单调性：按数组顺序重新发号
            _ => {
                cursor += 1;
                m.seq = Some(cursor);
            }
        }
    }
    cursor
}

/// 会话恢复动作。
///
/// 统一覆盖所有"非普通消息"的用户主动操作（与 `session_chat::Request.message` 互斥）：
/// - `RetryTurn`：LLM 失败重试（删除 Failed Turn 及其所有子节点，重新走 LLM 请求）
/// - `Retry`：工具执行失败后重试（用原 args）
/// - `Approve` / `Reject`：confirm 审批的批准/拒绝
/// - `Supply`：工具执行失败后补充参数重试（合并 args）
/// - `Answer`：ask_user 提问的答案回填
///
/// 恢复语义：删除旧消息 → 重新执行 → 生成新的响应子节点。
/// 对于工具调用场景，ToolCall 父节点 id 保持不变（稳定锚点），仅状态更新。
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "snake_case")]
pub enum ResumeAction {
    /// LLM 失败重试：删除 Failed Turn 及其所有子节点，重新走 LLM 请求
    RetryTurn,
    /// 工具失败重试：删除 Failed 工具结果子节点，重新执行工具（用原 args）
    Retry,
    /// 工具审批通过：删除 user_prompt 子节点，重新执行工具（带 approved=true）
    Approve,
    /// 工具审批拒绝：删除 user_prompt 子节点，生成拒绝结果子节点
    Reject,
    /// 工具补充参数：删除 Failed 工具结果子节点，合并参数后重新执行工具
    Supply,
    /// 提问回答：删除 user_prompt 子节点，生成答案结果子节点
    Answer,
}

/// 会话恢复请求（与 `session_chat::Request.message` 互斥；存在时走 resume 分支）。
///
/// 由 session 编排层随请求透传，最终在 session 插件会话循环
/// （`session/chat_loop.rs:run_chat_loop`）的 turn 循环前由 `session/resume.rs:process_resume` 处理。
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ResumeRequest {
    /// 目标消息 ID（Failed Turn 或 ToolCall，稳定标识，覆盖式更新锚点）
    pub target_id: String,
    pub action: ResumeAction,
    /// supply 时的补充参数（与原 args 浅合并）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Value>,
    /// reject 时的拒绝原因
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// answer（ask_user）时的答案对象
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `MessageStatus::as_str()` 必须与 `serde` 的序列化名逐字一致。
    ///
    /// 这条断言是**唯一**能拦住「VDFS 节点状态词与存储状态词分叉」的地方：
    /// 两者一旦不同，前端就会按节点状态渲染出与存储不一致的角标，
    /// 且编译期与运行期都不会报错（只会静默显示错的状态）。
    #[test]
    fn message_status_word_matches_serde() {
        for st in [
            MessageStatus::Pending,
            MessageStatus::Streaming,
            MessageStatus::WaitingUserAction,
            MessageStatus::Completed,
            MessageStatus::Aborted,
            MessageStatus::Failed,
        ] {
            let wire = serde_json::to_value(&st).unwrap();
            assert_eq!(
                wire.as_str(),
                Some(st.as_str()),
                "状态词与序列化名分叉：{st:?}"
            );
        }
    }

    /// `Completed` 不得与「未标注」被折叠成同一个节点状态词。
    ///
    /// 折叠曾让消费端必须把 `active` **猜回** `completed`（一次信息丢失 + 一次还原）；
    /// 这条断言把"两者可区分"钉住。
    #[test]
    fn completed_is_distinct_from_unset_sentinel() {
        assert_eq!(MessageStatus::Completed.as_str(), "completed");
        assert_ne!(MessageStatus::Completed.as_str(), "active");
    }

    /// `assign_seq` 必须让 `seq` 沿数组**单调不减**——数组顺序即权威顺序。
    ///
    /// 反例（L2 压缩）：新列表是 `[快照(新建、无 seq), 保留区(沿用旧低序号)]`。
    /// 「已带 seq 的一律保持原值」会让快照拿到 `base+1`（**最大**），而保留区仍是
    /// 更小的旧序号 ⇒ `ordered()` 把快照排到**最后**：压缩后的历史记忆跑到整段
    /// 转写末尾，前端表现为消息顺序错乱（用户消息插进了"更早"的历史里）。
    #[test]
    fn assign_seq_keeps_array_order_authoritative() {
        let mut msgs = vec![
            ChatMessage {
                id: "snapshot".into(),
                seq: None,
                ..Default::default()
            },
            ChatMessage {
                id: "keep1".into(),
                seq: Some(95),
                ..Default::default()
            },
            ChatMessage {
                id: "keep2".into(),
                seq: Some(96),
                ..Default::default()
            },
        ];
        // base = 100：压缩前会话已有的水位
        assign_seq(&mut msgs, 100);
        assert_eq!(msgs[0].seq, Some(101), "快照是数组首元素，必须先于保留区");
        assert!(
            msgs[0].seq.unwrap() < msgs[1].seq.unwrap(),
            "seq 必须沿数组递增，否则 ordered() 会重排数组顺序"
        );
        assert!(msgs[1].seq.unwrap() < msgs[2].seq.unwrap());
    }

    /// 正常路径**不得**触发重排：数组顺序本就等于 seq 顺序时，既有序号原样保留。
    ///
    /// 这条是上面那条的反面保险——修单调性不能以"每次落库都重排历史"为代价。
    #[test]
    fn assign_seq_leaves_already_ordered_seqs_untouched() {
        let mut msgs = vec![
            ChatMessage {
                id: "a".into(),
                seq: Some(5),
                ..Default::default()
            },
            ChatMessage {
                id: "b".into(),
                seq: Some(6),
                ..Default::default()
            },
            ChatMessage {
                id: "c".into(),
                seq: None,
                ..Default::default()
            },
        ];
        assign_seq(&mut msgs, 0);
        assert_eq!(msgs[0].seq, Some(5), "已有序的历史不得被改写");
        assert_eq!(msgs[1].seq, Some(6));
        assert_eq!(msgs[2].seq, Some(7), "缺号者续接水位");
    }
}
