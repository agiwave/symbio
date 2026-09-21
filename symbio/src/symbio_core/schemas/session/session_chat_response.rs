use super::chat_message::ChatMessage;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub message: ChatMessage,
}

/// 会话转写节点操作（进程内通道的线协议：chat_loop / 工具执行 / 恢复重写 /
/// 子会话转播 → 编排器 [`crate::plugins::session::transcript::Transcript`]，唯一写入点）。
///
/// ## 它不是前端协议
///
/// 前端可见的一切经 `Transcript` 分配 seq 后以 `NodeEvent` 下发
/// （一条流、单调 seq、缺口即整份重同步）。每个变元都是**一种显式操作**，
/// 不承载「部分字段 + 逐字段合并」的补丁语义：
///
/// - [`NodeOp::Upsert`]：**完整消息快照**——按 id 整条替换（不存在则创建）。
///   Start / Update / End 的区分由 Transcript 按图内状态机械判别（仅用于核心日志），
///   线上只有一种全量帧，没有第二种解释。
/// - [`NodeOp::Append`]：往已存在消息的 Text 内容尾部**追加**增量
///   （流式 token 的窄载荷，O(delta)；对未知 id 追加是协议违例，接收端报错丢弃）。
/// - [`NodeOp::Remove`]：删除一条消息（工具恢复清理旧子节点、压缩清理、
///   重试清除半截流——"作废"本身就是可持久化的状态变更）。
/// - [`NodeOp::Reset`]：转写被截断 / 压缩重写——消费端清空本地转写并从存储整份重读。
/// - [`NodeOp::Warn`]：会话级可恢复告警（持久化失败、长度截断、工具轮次上限），
///   写入会话节点 `attributes.warning`；新一轮请求开始时清除。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum NodeOp {
    /// 完整消息快照：接收端按 id 整条替换（不存在则创建）。
    ///
    /// 载荷装箱：`ChatMessage` 内联约 272 字节，直接铺在变体里会让**整个枚举**的
    /// 尺寸由它决定，而本枚举沿通道走的多是流式热路径上的窄帧（`Append`）。
    /// 装箱后枚举尺寸落在窄帧量级，代价是一次分配——相对于每个构造点都必然
    /// 执行的 `serde_json::to_value`（它本就要建整棵 JSON 树），可以忽略。
    Upsert { message: Box<ChatMessage> },
    /// 流式追加：`delta` 追加到 `message_id` 消息的 Text 内容尾部。
    Append { message_id: String, delta: String },
    /// 删除指定 id 的消息。
    Remove { message_id: String },
    /// 转写被截断 / 压缩重写：消费端清空本地转写并从存储整份重读。
    Reset,
    /// 会话级告警（可恢复，UI 应显示给用户；`Some` 设置 / `None` 清除）。
    ///
    /// `warning` 字段是面向用户的本地化短消息。它是**状态**不是事件：
    /// 落在会话节点属性上，前端按节点状态渲染，不依赖"恰好收到这一帧"。
    Warn { warning: Option<String> },
}
