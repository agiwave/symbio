//! 嵌入服务抽象（symbio_core 层）
//!
//! 所有插件通过 `dyn EmbeddingService` 访问嵌入能力，
//! **不**直接引用 `crate::providers::embedding::FastEmbedService` 等具体实现。
//!
//! 这是 `providers/`（具体实现层）之上的**抽象接口层**：trait 在此定义，
//! 具体实现（FastEmbed / Noop 等）放在 `src/providers/embedding`，
//! 并通过通用对象创建机制自注册到 `fastembed` / `noop` id，
//! 业务模块用 `create_object::<dyn EmbeddingService>("fastembed", ctx)` 获取实例。

use async_trait::async_trait;
use thiserror::Error;

/// 嵌入服务错误
#[derive(Debug, Error)]
pub enum EmbeddingError {
    #[error("嵌入模型初始化失败: {0}")]
    Init(String),
    #[error("嵌入失败: {0}")]
    Embed(String),
}

/// 嵌入服务：把一段文本映射为稠密向量，用于语义检索 / 记忆查找。
///
/// 是否真正支持语义检索由实现决定——不支持时（如 `NoopEmbeddingService`）
/// 上层应降级到精确名称匹配（例如 `codebase_search` 退化为 ripgrep 关键词检索）。
#[async_trait]
pub trait EmbeddingService: Send + Sync {
    /// 把一段文本编码为稠密向量
    async fn embed(&self, text: &str) -> Option<Vec<f32>>;
}
