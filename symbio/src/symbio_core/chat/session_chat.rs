//
// 注意：工作区路径 (workdir) 由 PluginMessage.workdir 路由层统一传递，
// 不在此业务结构体中重复定义。
use super::chat_node::{ChatNode, ResumeRequest};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct Request {
    pub session_id: Option<String>,
    pub agent_id: Option<String>,
    pub message: Option<ChatNode>,
    /// 选定的 Model Provider ID。
    ///
    /// 随每次发送携带；为空时回退会话 `metadata.provider_id`（默认 `none` = 用默认 Provider）。
    /// 与 `mode` / `risk_level` 是**完全对称**的三条链：`req > session.metadata > 默认`
    /// （`plugins/session/orchestrator/entry.rs::resolve_session_params`）。
    /// 这条回退是 [D-066](../../../../docs/003-design/012-会话输入与地址.md "D-066") 的前提——
    /// 「地址写入不承载运行选项」正靠它成立，故它同时是一条不变量。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    /// 心跳任务专用：本次发送是否携带历史会话信息。
    /// - `None` / `Some(true)`：携带历史（默认行为）
    /// - `Some(false)`：本次发送不加载历史会话信息
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_history: Option<bool>,
    /// 会话运行模式：auto（无人值守，需交互工具返回友好错误不弹框）/ interactive（默认，会话流中渲染交互卡）。
    /// 随每次发送携带；为空时回退会话 metadata.mode（默认 auto
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// 会话执行风险等级阈值：low / medium / high（与 agent_id/provider_id/mode 同级别）。
    /// 随每次发送携带；为空时回退会话 metadata.risk_level（默认 medium）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub risk_level: Option<String>,
    /// 入站请求的**来源主体**（[DE-001 §12.2](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")
    /// 跨体承诺的「向谁宣告」，[D-067](../../../../docs/003-design/011-Agent目录规范.md "D-067")）。
    ///
    /// 完整主体名（如 `agent:main`）——**谁把这条消息交进来**。它不是「以谁的身份
    /// 跑轮」（那是 `agent_id` ⇒ 会话主体），而是「这条入站消息的出处」：轮末
    /// `record_derived` 把它用作 `commitment.*` 的 `to`（本主体向**它**立约）。
    /// `None` = 无来源声明（同机直呼 / 心跳等本机路径）⇒ 落 `PRINCIPAL_USER`，
    /// 与接线前写死的常量逐字一致。
    ///
    /// 线上入口 = 请求 `metadata.source_principal`（网关 `build_ctx` 落 ctx）；
    /// 收件箱入队口（`vdfs/write <会话>/inbox`）把它随条目寄存进 `params`，
    /// 消费时经 `run_inbox_turn` 挂回执行 ctx。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_principal: Option<String>,
    /// 会话恢复操作（与 `message` 互斥）。
    /// 存在时走 resume 分支：删除旧消息 → 重新执行 → 续写 chat_loop。
    /// 支持场景：
    /// - `RetrySubtree`：LLM 失败重试（删除靶子节点及其子节点，重新走 LLM 请求）
    /// - `Retry`/`Approve`/`Reject`/`Supply`/`Answer`：工具调用恢复
    ///   为 None 且 `message` 存在时走正常 send 分支。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume: Option<ResumeRequest>,
}

/// `session/chat` 的响应：本轮定稿后的消息副本。
///
/// ## 消息变更只有一种线协议形状
///
/// 消息变更的线协议一度是独立的一层「显式操作」枚举（`upsert` / `append` /
/// `remove` / `reset` / `warn`），后来收敛成只剩一个变元的 `Change`。两者都已
/// 删除：**帧就是一条 [`ChatNode`]**（线协议上曾叫 `NodeEvent` =
/// `{ session_id, seq, message }`，那层包装与承载它的转写流已于 2026-09-23
/// 随 D-025 一并退役——消息变更改走 `VdfsChange`）。
///
/// 原来那层枚举只是在重复 `ChatNode` 已经有的字段：
///
/// | 原枚举里的字段 | 现在的落点 |
/// |---|---|
/// | `Change.message_id` | `ChatNode.id` |
/// | `Change.delta` | `ChatNode.delta`（增量；与 `content` 互斥） |
/// | `Change.status` / `error` / `meta` / 身份字段 | `ChatNode` 的同名字段 |
/// | `Remove { message_id }` | `status = removed`（一次状态迁移） |
/// | `Reset` | 不存在——协议里没有「清空重读」的消息形态，删除逐条下发 |
/// | `Warn { warning }` | 会话节点状态（`TranscriptWriter::warn`），不是消息帧 |
///
/// 于是「消息变更」只剩一种形状：消费端不必再维护第二套解析与第二套合并规则，
/// 「这条消息现在是什么样」与「这帧说了什么」也不再是两份需要对齐的数据。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub message: ChatNode,
}
