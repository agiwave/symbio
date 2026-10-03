// Corresponding Frontend: tauri/src/schemas/conversation_line.ts
//! 对话线投影 —— 从**一份存储**里切出「用户与助手说过的话」。
//!
//! ## 一条存储，两条线，一个分界
//!
//! 存储只有一份（ADR-020：转写只有一个写入者），但有两种读者：
//!
//! | | **对话线**（本文件） | **工作线**（[`super::view::build_request_view`]） |
//! |---|---|---|
//! | 内容 | user 消息 + assistant **文本节点** | 推理 / 工具调用 / 工具结果 / Turn 容器 |
//! | 谁读 | `classify` / `compose` 的上下文 + 前端「对话」面板 | worker 的请求视图 + 前端「工作」面板 |
//!
//! **分界是一条纯函数，不是两处判断**：三处共用同一份规则（两个插件 + 前端），
//! 否则会出现「界面看得到、插件看不到」的错位——那类 bug 没有错误信号，
//! 只表现为「助手答得不对」，排查方向会被带偏到提示词上。
//!
//! ## 判据只有两条
//!
//! 1. `role = user` 且是**文本节点**；
//! 2. `role = assistant` 且是**文本节点**（`msg_type` 缺省或 `Text`）。
//!
//! 「是文本节点」挡掉 `ToolCall` / `Reasoning` / `Turn` / `Compression` /
//! `UserPrompt` 五类；「角色」挡掉 `role = tool`（工具结果——可能含几万 token 的
//! 源码或命令输出）与 `role = system`（注入的框架文本）。这正是 C-D4 的两条边界。
//!
//! ## 助手说的话有两种位置，**都算对话线**
//!
//! 根级的 assistant 文本是「单独说的一句话」（首响 / 汇报）；`Turn` 的**子**
//! `Text` 节点是**回复正文**——用户真正读到的那段回答。两者都是"助手说过的话"。
//!
//! 判据因此**不看 `parent_id`**。曾经按"根级"切（只收根级 assistant 文本），
//! 理由是"子 Text 属于工作线的一轮"。那是把**过程**与**说过的话**混成了一个判据，
//! 两处代价都是真的：
//!
//! - **插件失明**：`classify` / `compose` 的上下文里没有助手上一轮的回答。用户追问
//!   「那 LICENSE 呢？」时，判决只看得到用户问了什么、看不到自己答过什么——
//!   而"能不能直接答"恰恰取决于"已经答过什么"；`compose` 的措辞也会因此重复或断裂。
//! - **界面错位**：前端「对话」面板按同一条规则过滤时，用户会看到自己说的话
//!   与一句开场白，却**看不到回答本身**（回答在 `Turn` 组里）——那就不成其为对话。
//!
//! 工作线的边界因此是**排除法**（凡不在对话线上的节点），不是"位置法"。
//!
//! ## 它**不**看 `meta.exclude_from_context`
//!
//! 两个过滤器看着矛盾，其实各管一条线：
//!
//! - `exclude_from_context` 管的是**模型请求包**（工作线）：首响那句话不进
//!   provider 的消息数组，否则会出现连续两条 `assistant`（部分协议直接 400）；
//! - 本函数管的是**对话线**：首响是用户**真的看到过**的一句话，它当然属于对话——
//!   下一轮 `compose` 若不知道上一轮说过「好的，我去看看」，措辞就会重复或断裂。
//!
//! 把两者混成一个开关，会让「用户看得到但模型看不到」与「模型看得到但用户看不到」
//! 这两种**都错**的状态变成同一件事。

use crate::symbio_core::chat_message::{ChatMessage, MessageRole, MessageType};

/// 投影窗口（条数）——**默认值**，两个调用方（`classify` / `compose`）共用。
///
/// ## 为什么这个数住在投影这里，而不是各自的调用点
///
/// 它是「一次投影带多少上下文」这个问题的答案，属于**投影**；调用方只是在用它。
/// 放在调用点会让两个数各演化一次——而它们答的是同一个问题，没有理由不同。
/// 真的出现"两个插件该看不同窗口"的需求时，那是**在调用点**给出另一个数（函数本来就
/// 收 `limit`），不是在这里再定义一份。
///
/// ## 为什么是 12
///
/// 一次投影 ≈ 6 轮来回，足够判「用户在问刚才说过的事」。窗口放大直接放大**每次**
/// 分类 / 生成请求的开销，而这类问题问的几乎都是最近几句。
///
/// ## 为什么是常量而不是配置项
///
/// 没有一条平凡值能把它关掉（关掉它 = 没有上下文 = 功能缺失），因此它不符合
/// 「每个配置项都必须有平凡值」的准入（J2）——**一个无法被关掉的旋钮不是参数，是装饰**。
pub const CONVERSATION_VIEW_LIMIT: usize = 12;

/// 投影出对话线：按存储顺序保留最后 `limit` 条（`limit = 0` 表示不限制）。
///
/// 返回的是**新 `Vec`**（克隆节点），调用方可以自由改写而不影响存储。
pub fn conversation_view(messages: &[ChatMessage], limit: usize) -> Vec<ChatMessage> {
    let mut out: Vec<ChatMessage> = messages
        .iter()
        .filter(|m| is_conversation_node(m))
        .cloned()
        .collect();
    if limit > 0 && out.len() > limit {
        out.drain(..out.len() - limit);
    }
    out
}

/// 一条节点是否属于对话线（判据见模块文档，只有两条）。
///
/// **位置（`parent_id`）不参与判定**：`Turn` 的子 `Text` 是回复正文，它和根级的
/// 首响一样是"助手说过的话"。
fn is_conversation_node(m: &ChatMessage) -> bool {
    let is_text = matches!(m.msg_type, None | Some(MessageType::Text));
    is_text
        && matches!(
            m.role,
            Some(MessageRole::User) | Some(MessageRole::Assistant)
        )
}

#[cfg(test)]
#[path = "conversation_view.test.rs"]
mod tests;
