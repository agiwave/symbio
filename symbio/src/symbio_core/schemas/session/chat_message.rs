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
    /// 节点已被删除（**终态，不可重试**）。
    ///
    /// 协议里没有 `remove` 操作——删除就是一次状态迁移，与出现、增长、完成
    /// 同走一帧。落在此状态的帧在接收端**就地移除**该节点（从本地视图消失），
    /// 不产生任何「清空重读」。
    Removed,
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
            MessageStatus::Removed => "removed",
        }
    }

    /// [`Self::as_str`] 的逆：状态词 → 枚举。未知词返回 `None`。
    ///
    /// 存在的理由与 `as_str` 是同一条的另一半：VDFS 变更**不带载荷**（ADR-025），
    /// 消费端拿到 `<sid>/消息/<mid>` 的 `updated` 后只能回读**节点**
    /// （`stat`），而节点上承载状态的是那个**词**。把它解析回枚举若在每个消费端
    /// 各手写一次 match，词表就又有了第二份定义。
    pub fn of(word: &str) -> Option<Self> {
        match word {
            "pending" => Some(MessageStatus::Pending),
            "streaming" => Some(MessageStatus::Streaming),
            "waiting_user_action" => Some(MessageStatus::WaitingUserAction),
            "completed" => Some(MessageStatus::Completed),
            "aborted" => Some(MessageStatus::Aborted),
            "failed" => Some(MessageStatus::Failed),
            "removed" => Some(MessageStatus::Removed),
            _ => None,
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

    /// 流式**增量**：本帧新到达的那一段正文，追加到接收端该节点正文尾部。
    ///
    /// ## 与 `content` 互斥，语义由字段本身给出
    ///
    /// 一帧里两者**绝不同时出现**——同帧携带即协议违例，唯一写入点
    /// （[`crate::plugins::session::transcript::Transcript::apply`]）报错丢弃。
    /// 消费端因此永远不必从帧的形状里推断「该拼接还是该替换」：
    ///
    /// | 字段 | 语义 | 用在哪 |
    /// |---|---|---|
    /// | `delta` | 追加到正文尾部（O(delta) 窄帧） | 流式热路径：正文 / 推理 / 工具参数逐片 |
    /// | `content` | **整条替换**该节点正文（幂等） | 一次性节点的完整体、存储回执的权威副本、编辑后的新正文 |
    ///
    /// ## 为什么不落存储
    ///
    /// 增量是**不完整的片段**，不是消息的形态：它只在出方向的帧上存在，
    /// 存储里只有 `content`（图内累积的正文）。改名的同一份结构因此承载两件事：
    /// - 帧方向：`delta` 是「这一段」；
    /// - 存储方向：`content` 是「全部」。
    ///
    /// 「delta 永不落盘」不是发射方的自律，而是持久层的不变量——
    /// `chat_session::ensure_durable_states` 在写入点直接拒绝携带 `delta` 的消息。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delta: Option<String>,

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

    /// ToolCall 节点的 **wire id**（LLM provider 返回的 `tool_call_id`）。
    ///
    /// ## 为什么节点 id 不能直接用它
    ///
    /// 许多 OpenAI 兼容网关**跨轮复用** `call_0` / `call_xxx` 这类短 id。而消息
    /// id 在三处被当作**唯一键**：VDFS 地址（`<sid>/消息/<mid>`）、前端 store
    /// （按 id 的 map）、消费循环的在途合并。wire id 直接当节点 id 会让下一轮
    /// 的同 id 工具调用更新到上一轮的老节点（后端 `Vec` 存储不撞，刷新后"自愈"
    /// ——正是"后端正常、前端显示混乱"的根源）。
    ///
    /// 因此 ToolCall 节点 id 在**诞生时**分配（会话内唯一），provider 的原始 id
    /// 存放在本字段，仅在构建 LLM 请求包时使用（部分协议要求回传原值，如
    /// OpenAI Responses 的 `call_id` 链）。历史数据无此字段：请求构建回退节点 id。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

/// 取一批消息中已有的最大 `seq`，作为后续分配的起点（Lamport 计数器的当前水位）。
pub fn max_seq(messages: &[ChatMessage]) -> i64 {
    messages.iter().filter_map(|m| m.seq).max().unwrap_or(0)
}

/// 按切片顺序为消息补发 `seq`（只补缺号），返回分配后的新水位。
///
/// # 两条不变式（顺序与稳定，缺一不可）
///
/// 1. **顺序**：`seq` 沿数组严格递增——调用方（`replace_messages`）的契约是
///    「数组顺序即权威顺序」，`ordered()` 靠 `seq` 复现它；
/// 2. **稳定**：**已带 `seq` 的消息一律原样保留**，绝不因为一次整表重写而改号。
///
/// 第 2 条不是可有可无的洁癖：`seq` 是**消费者（前端）手里的顺序锚点**。
/// 前端在流式阶段就按本地游标给消息排好了序，整表重写若把既有消息改成新号，
/// 两边就各持一套互不相容的序号——同一条消息在「有后端序号」与「有本地序号」
/// 两种形态下会排到列表的两个位置，压缩后顺序看起来就乱了。
///
/// # 填号方向：无号项从**相邻的既有序号**向外让位
///
/// `base` 只在**新列表完全没有既有序号**时才用得上（整批新节点的场景，
/// 接在旧水位之后即可）。只要列表里已经有序号，无号项就按它在数组中的位置
/// 分三段处理——**每一段都朝"不越过既有序号"的方向让位**：
///
/// | 位置 | 取值 | 理由 |
/// |---|---|---|
/// | 开头（压缩快照接替被压掉的历史） | `首号 − k … 首号 − 1`（**往下**） | 往上找会越过首号，快照反而排到保留区**之后** |
/// | 末尾（本轮在途追加） | `末号 + 1 …`（往上） | 与既有历史接续，不抢前面的号 |
/// | 夹缝（两个既有序号之间） | `前驱 + 1 …`（往上） | 间隙足够时不会撞上后驱 |
///
/// **为什么开头那一段必须整体往下、且一次算好**：逐个"往上找空位"会先占用
/// 首号本身，把既有序号顶成下一个号——正是"seq 随压缩改变"的直接来源。
/// 由 `首号` 是最小序号可知 `首号 − k … 首号 − 1` 全部落在既有序号**之外**，
/// 因此这一段天然不可能撞号，也不需要逐个探测。
///
/// 旧实现从 `base` 起向上无条件填号，于是 L2 压缩这种「前缀重写」必然被改号：
/// 新列表是 `[快照, 保留区…]`，快照无 seq 拿到 `base+1`（成了**最大**），
/// 保留区沿用旧**低**序号却因 `existing > cursor` 不成立而被逐个改号——
/// 实测会话 `mtmae8j2wxam4dhrei` 的保留区因此从 `631..642` 被抬到 `870..881`。
/// 顺序虽然被"修"对了，代价却是整段历史的序号全部重排。
///
/// 真正的修法也在调用方：压缩时给快照显式指定**被压缩内容的槽位序号**
/// （`keep[0].seq - 1`，见 `chat_loop::compress`），使新列表**本来就单调**。
/// 于是本函数退化为"只填缺号"，既有序号一个不动——顺序与稳定同时成立。
///
/// 兜底：夹缝容不下无号项时（`前驱` 与 `后驱` 之间没有空号），按"前驱 + 1"会
/// 越过 `后驱`，破坏数组顺序。此时**顺序不变式优先**——整表按数组顺序重排
/// （与旧实现同口径）。该输入在真实路径上不可达（快照在开头、在途追加在末尾），
/// 但"静默破坏顺序"比"多一次改号"更糟，故显式兜底而不是放任。
pub fn assign_seq(messages: &mut [ChatMessage], base: i64) -> i64 {
    // 全列表无号（整批新节点）：接在调用方水位之后——`base` 的唯一用途
    let Some(first) = messages.iter().filter_map(|m| m.seq).min() else {
        let mut cursor = base;
        for m in messages.iter_mut() {
            cursor += 1;
            m.seq = Some(cursor);
        }
        return cursor;
    };

    // ① 开头连续的无号项：整体落在 `首号` 之前（`首号` 是最小号 ⇒ 必不撞号）
    let head = messages.iter().take_while(|m| m.seq.is_none()).count();
    for (cursor, m) in (first - head as i64..).zip(messages.iter_mut().take(head)) {
        m.seq = Some(cursor);
    }

    // ② 其余无号项：从「前驱 + 1」往上找空位。前驱初始取 `base`，但列表首项
    //    要么是刚填好的开头段、要么是既有序号，第一轮就会把水位拉到它上面。
    let mut taken: std::collections::HashSet<i64> = messages.iter().filter_map(|m| m.seq).collect();
    let mut cursor = base;
    for m in messages.iter_mut() {
        match m.seq {
            // 既有序号：原样保留（**绝不改号**），并作为新的前驱水位
            Some(existing) => cursor = existing,
            None => {
                let mut candidate = cursor + 1;
                while taken.contains(&candidate) {
                    candidate += 1;
                }
                m.seq = Some(candidate);
                taken.insert(candidate);
                cursor = candidate;
            }
        }
    }

    // ③ 夹缝容不下时的兜底：顺序不变式优先于稳定不变式（数组顺序是权威）
    if messages.windows(2).any(|w| w[0].seq >= w[1].seq) {
        let start = first - head as i64;
        let mut cursor = start - 1;
        for m in messages.iter_mut() {
            cursor += 1;
            m.seq = Some(cursor);
        }
        return cursor;
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
/// - `RetryCompaction`：压缩失败重试（删除 Failed 压缩节点，重新执行一次压缩）
///
/// 恢复语义：删除旧消息 → 重新执行 → 生成新的响应子节点。
/// 对于工具调用场景，ToolCall 父节点 id 保持不变（稳定锚点），仅状态更新。
///
/// 线格式为 `snake_case`（见 `#[serde(rename_all)]`），与前端
/// `composables/useChatConnection.ts::ResumePayload.action` 的字面量逐字相等——
/// 这条跨栈契约由 `resume_action_wire_words_are_snake_case` 钉住。
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
    /// 压缩失败重试：删除 Failed 压缩节点，重新执行一次上下文压缩。
    ///
    /// `target_id` 指向**压缩节点**（`msg_type = Compression`、`status = Failed`）。
    /// 语义上属"删除-重建"，与其余恢复动作同款；但**不改动历史**——压缩失败
    /// 本身从不丢消息（见 `CompressionFailure` 的说明），重试只是再试一次
    /// LLM 摘要，成功后由快照接替被压缩的那段。
    RetryCompaction,
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

    /// `ResumeAction` 的**线格式词**是跨栈契约：前端 `ResumePayload.action` 的
    /// 字面量必须与它逐字相等。
    ///
    /// 与 `message_status_word_matches_serde` 同一动机——两侧分叉时编译期与运行期
    /// 都不报错，只会在运行期表现为「点了重试没反应」（后端 serde 反序列化失败）。
    /// 因此把全部取值都钉住，而不只是本次新增的那个。
    #[test]
    fn resume_action_wire_words_are_snake_case() {
        let cases = [
            (ResumeAction::RetryTurn, "retry_turn"),
            (ResumeAction::Retry, "retry"),
            (ResumeAction::Approve, "approve"),
            (ResumeAction::Reject, "reject"),
            (ResumeAction::Supply, "supply"),
            (ResumeAction::Answer, "answer"),
            (ResumeAction::RetryCompaction, "retry_compaction"),
        ];
        for (action, word) in cases {
            assert_eq!(
                serde_json::to_value(&action).unwrap().as_str(),
                Some(word),
                "线格式词与前端字面量分叉：{action:?}"
            );
            // 反向：前端发来的字符串必须能解回同一个取值
            let back: ResumeAction = serde_json::from_value(serde_json::json!(word)).unwrap();
            assert_eq!(format!("{back:?}"), format!("{action:?}"));
        }
    }

    /// 顺序不变式优先：列表顺序已经错了（`existing <= cursor`）时，按数组顺序改号。
    ///
    /// 数组顺序是权威（`replace_messages` 的契约），所以这一支仍然保留；但注意它
    /// **只在顺序确实错了时**才触发——正常路径（顺序本就正确）走的是"只补缺号"。
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
        // base = 100：压缩前会话已有的水位（本列表已有序号，故不参与填号起点）
        assign_seq(&mut msgs, 100);
        assert!(
            msgs[0].seq.unwrap() < msgs[1].seq.unwrap(),
            "seq 必须沿数组递增，否则 ordered() 会重排数组顺序"
        );
        assert!(msgs[1].seq.unwrap() < msgs[2].seq.unwrap());
        assert_eq!(msgs[1].seq, Some(95), "顺序本就正确的序号不得被改写");
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

    /// **压缩契约**：`base` 高于既有序号时，既有序号依然一个都不许动。
    ///
    /// 这正是实测会话 `mtmae8j2wxam4dhrei` 的病态：`replace_messages` 传
    /// `base = max_seq(旧列表)`（868），而新列表是 `[快照(槽位 630), 保留区(631..642)]`。
    /// 旧实现从 868 起步，把保留区整段抬到 870..881；正确行为是原样保留。
    #[test]
    fn assign_seq_never_rewrites_existing_seqs_even_when_base_is_higher() {
        let mut msgs = vec![
            ChatMessage {
                id: "snapshot".into(),
                // 快照接替被压缩内容的槽位：keep[0].seq - 1
                seq: Some(630),
                ..Default::default()
            },
            ChatMessage {
                id: "keep1".into(),
                seq: Some(631),
                ..Default::default()
            },
            ChatMessage {
                id: "keep2".into(),
                seq: Some(632),
                ..Default::default()
            },
        ];
        assign_seq(&mut msgs, 868);
        assert_eq!(msgs[0].seq, Some(630), "快照的槽位序号必须原样保留");
        assert_eq!(
            msgs[1].seq,
            Some(631),
            "保留区序号必须原样保留（压缩不改号）"
        );
        assert_eq!(msgs[2].seq, Some(632));
    }

    /// 全新列表（无任何既有序号）才使用 `base`：整批新节点接在旧水位之后。
    #[test]
    fn assign_seq_uses_base_only_for_fresh_lists() {
        let mut msgs = vec![
            ChatMessage {
                id: "n1".into(),
                seq: None,
                ..Default::default()
            },
            ChatMessage {
                id: "n2".into(),
                seq: None,
                ..Default::default()
            },
        ];
        assign_seq(&mut msgs, 868);
        assert_eq!(msgs[0].seq, Some(869));
        assert_eq!(msgs[1].seq, Some(870));
    }

    /// 开头段有**多个**无号项时，整段落在首号之前且沿数组递增。
    ///
    /// 逐个"往上找空位"会先占用首号本身、把既有序号顶成下一个号；正确做法是
    /// 整段一次算好（`首号 − k … 首号 − 1`）。
    #[test]
    fn assign_seq_places_leading_run_below_first_existing() {
        let mut msgs = vec![
            ChatMessage {
                id: "s1".into(),
                seq: None,
                ..Default::default()
            },
            ChatMessage {
                id: "s2".into(),
                seq: None,
                ..Default::default()
            },
            ChatMessage {
                id: "keep".into(),
                seq: Some(95),
                ..Default::default()
            },
        ];
        assign_seq(&mut msgs, 100);
        assert_eq!(msgs[0].seq, Some(93));
        assert_eq!(msgs[1].seq, Some(94));
        assert_eq!(msgs[2].seq, Some(95), "既有序号不得被开头段顶走");
    }

    /// 末尾段接在末号之后——本轮在途追加的常规路径。
    #[test]
    fn assign_seq_places_trailing_run_after_last_existing() {
        let mut msgs = vec![
            ChatMessage {
                id: "a".into(),
                seq: Some(5),
                ..Default::default()
            },
            ChatMessage {
                id: "inflight".into(),
                seq: None,
                ..Default::default()
            },
        ];
        // base 高于既有序号（= 旧列表水位）：末尾段仍须接在**末号**之后，不得跳到 base
        assign_seq(&mut msgs, 868);
        assert_eq!(msgs[0].seq, Some(5));
        assert_eq!(msgs[1].seq, Some(6));
    }

    /// 夹缝容不下时**顺序不变式优先**：整表按数组顺序重排（显式兜底，不静默破坏顺序）。
    ///
    /// 该输入在真实路径上不可达（快照在开头、在途追加在末尾），但这条断言把
    /// "宁可多改一次号，也不留下顺序倒挂"这个取舍钉住。
    #[test]
    fn assign_seq_falls_back_to_array_order_when_gap_is_full() {
        let mut msgs = vec![
            ChatMessage {
                id: "a".into(),
                seq: Some(5),
                ..Default::default()
            },
            ChatMessage {
                id: "mid".into(),
                seq: None,
                ..Default::default()
            },
            ChatMessage {
                id: "b".into(),
                seq: Some(6),
                ..Default::default()
            },
        ];
        assign_seq(&mut msgs, 0);
        assert!(
            msgs.windows(2).all(|w| w[0].seq < w[1].seq),
            "数组顺序是权威：兜底后必须严格递增，实得 {:?}",
            msgs.iter().map(|m| m.seq).collect::<Vec<_>>()
        );
    }
}
