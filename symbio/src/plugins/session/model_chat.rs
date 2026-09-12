// Corresponding Frontend: tauri/src/protocols/model_chat.ts
use crate::symbio_core::schemas::session::chat_message::{ChatMessage, ResumeRequest};
use serde::{Deserialize, Serialize};

/// Model 推理请求 (由 Session 插件或 Agent 发起)
///
/// 注意：工作区路径 (workdir) 由 PluginMessage.workdir 路由层统一传递，
/// 不在此业务结构体中重复定义。
///
/// 核心协议设计说明：
/// - 本协议采用"单消息输入"模式，不再携带完整会话历史
/// - 具体协议实现层（如 OpenAI Chat/Responses、Anthropic、Gemini）决定是否需要获取历史
/// - 有状态协议（如 OpenAI Responses）通过 previous_response_id 关联上下文
/// - 无状态协议（如标准 OpenAI Chat）需主动从会话服务获取历史消息
///
/// 关于 `Option` 与序列化（契约，2026-09-14 归一）：
/// - 字段保持 `Option` 是**契约要求**：`None`（调用方未表态）与"零值"语义不同
///   （如 `auto_compress: None` → chat_loop 取默认 `true`；`max_tool_rounds: None` → 无限轮次）。
///   因此**不得**改成带 serde default 的裸类型，否则缺省会翻转为零值语义。
/// - 输出**不使用** `skip_serializing_if`：`None` 序列化为 `null`，使键集合恒定存在，
///   不随字段取值增删（历史上 `stream` 无该属性、其余字段有，导致同一协议在不同取值下
///   键集不同，消费方难以稳定判别"未设置"）。
/// - 输入侧 `Option` 字段 serde 天然接受"缺键"与"`null`"，二者等价，无需逐字段 `default`。
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct Request {
    /// 系统提示词
    pub system_prompt: Option<String>,
    /// 当前需要发送的单条消息
    pub single_message: Option<ChatMessage>,
    /// 是否使用流式输出
    pub stream: Option<bool>,

    /// 最大工具轮数
    pub max_tool_rounds: Option<usize>,
    /// 工具上下文窗口（最近 N 轮工具调用保留明细）
    pub tool_context_window: Option<usize>,
    /// 是否开启自动语义压缩
    pub auto_compress: Option<bool>,
    /// 是否启用工具压缩（context_compact 暴露给模型 + 水位提醒；独立于 auto_compress）
    pub enable_compact_tool: Option<bool>,

    /// 指定本次会话使用的 Model Provider ID（来自 `ModelProvidersConfig.providers`）
    /// 由 session 编排层解析（精确 id → is_default → 首个注册），为空时同样走回退链取默认 Provider
    pub provider_id: Option<String>,
    /// 是否加载历史会话消息。
    /// - `None` / `Some(true)`：从会话存储加载历史（默认行为）
    /// - `Some(false)`：仅使用本次 `single_message`，不携带任何历史会话信息
    ///   （用于心跳任务等"无上下文"场景）
    pub load_history: Option<bool>,
    /// 会话恢复操作（与 `single_message` 互斥）。
    ///
    /// 存在时由 session 插件会话循环（`session/chat_loop.rs:run_chat_loop`）在 turn 循环前处理：
    /// - `RetryTurn`：删除 Failed Turn 及其所有子节点，重新走 LLM 请求
    /// - `Retry`/`Approve`/`Reject`/`Supply`/`Answer`：删除旧工具响应子节点 →
    ///   重新执行工具（approve/retry/supply）或直接生成结果（reject/answer）→
    ///   创建新响应子节点 → 成功则继续 turn 循环，失败则退出等下次 resume。
    ///
    /// CAPABILITY_VISITOR 已由 agent chat handler 设置，`execute_tool_async` 直接复用。
    pub resume: Option<ResumeRequest>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbio_core::schemas::session::chat_message::ResumeAction;

    /// 全字段显式赋值（`Some`）的样本，供键集合契约使用。
    ///
    /// **刻意不使用 `..Default::default()`**：这样一旦 `Request` 新增字段，
    /// 本函数直接编译失败，强制作者把它加进契约测试——避免"新字段带着
    /// `skip_serializing_if` 悄悄加入、键集合契约被无声破坏"。
    fn fully_populated() -> Request {
        Request {
            system_prompt: Some("p".into()),
            single_message: Some(ChatMessage::default()),
            stream: Some(false),
            max_tool_rounds: Some(3),
            tool_context_window: Some(4),
            auto_compress: Some(false),
            enable_compact_tool: Some(true),
            provider_id: Some("pid".into()),
            load_history: Some(false),
            resume: Some(ResumeRequest {
                target_id: "t1".into(),
                action: ResumeAction::RetryTurn,
                args: None,
                reason: None,
                answer: None,
            }),
        }
    }

    fn sorted_keys(v: &serde_json::Value) -> Vec<String> {
        let mut k: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
        k.sort();
        k
    }

    /// 跨语言契约：序列化输出的**键集合恒定**——`None` 一律输出 `null`，
    /// 不随字段取值增删。历史上仅 `stream` 无 `skip_serializing_if`，其余字段有，
    /// 同一协议在不同取值下键集不同，消费方无法稳定判别"未设置"。
    #[test]
    fn request_key_set_is_value_independent() {
        let empty = serde_json::to_value(Request::default()).unwrap();
        let full = serde_json::to_value(fully_populated()).unwrap();

        assert_eq!(sorted_keys(&empty), sorted_keys(&full), "键集合必须与取值无关");
        // 且 None 字段以显式 null 存在（而非缺键）
        assert_eq!(empty["load_history"], serde_json::Value::Null);
    }

    /// 契约另一侧：`None`（未表态）**不得**与零值混淆——故字段保持 `Option` 且
    /// 缺键/`null` 等价可反序列化（调用方省略字段时不翻转为 `false` / `0`）。
    #[test]
    fn absent_fields_deserialize_as_none_not_zero() {
        let r: Request = serde_json::from_str("{}").unwrap();
        assert_eq!(r.auto_compress, None, "缺键必须是 None（chat_loop 据此取默认 true）");
        assert_eq!(r.max_tool_rounds, None, "缺键必须是 None（0 会被解释成 0 轮熔断）");
        assert_eq!(r.load_history, None, "缺键必须是 None（默认加载历史）");

        let r2: Request =
            serde_json::from_str(r#"{"auto_compress":null,"max_tool_rounds":null,"load_history":null}"#)
                .unwrap();
        // `null` 与缺键等价（Request 未派生 PartialEq，逐字段比对）
        assert_eq!(r2.auto_compress, r.auto_compress);
        assert_eq!(r2.max_tool_rounds, r.max_tool_rounds);
        assert_eq!(r2.load_history, r.load_history);
    }

    /// 旧调用方仍可能带已删除的 `thinking` 死字段：serde 默认忽略未知键，须保持兼容。
    #[test]
    fn unknown_fields_are_tolerated() {
        let r: Request =
            serde_json::from_str(r#"{"stream":true,"thinking":{"enabled":true}}"#).unwrap();
        assert_eq!(r.stream, Some(true));
    }
}
