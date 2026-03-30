//! 核心类型定义

use futures::Stream;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::pin::Pin;

/// 插件元数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginMeta {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub version: String,
    /// 输入 Schema（JSON 格式），为空表示不需要输入
    #[serde(default, rename = "input_schema")]
    pub input: Option<Value>,
    /// 输出 Schema（JSON 格式），用于结果展示
    #[serde(default, rename = "output_schema")]
    pub output: Option<Value>,
    #[serde(default)]
    pub author: Option<String>,
}

/// 插件调用结果
pub type PluginResult<T> = Result<T, PluginError>;

/// 插件错误类型
#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    #[error("插件未找到：{0}")]
    NotFound(String),
    #[error("插件调用未实现")]
    NotImplemented,
    #[error("输入验证失败：{0}")]
    ValidationError(String),
    #[error("内部错误：{0}")]
    InternalError(String),
    #[error("解析错误：{0}")]
    ParseError(String),
}

impl From<PluginError> for String {
    fn from(err: PluginError) -> Self {
        err.to_string()
    }
}

/// 流式数据块
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamChunk {
    pub data: Value,
    #[serde(default)]
    pub done: bool,
    /// 错误信息（流中可报告错误，不中断流）
    #[serde(default)]
    pub error: Option<String>,
}

/// Boxed Stream 类型别名
pub type BoxStream<T> = Pin<Box<dyn Stream<Item = T> + Send>>;

/// 插件调用返回类型
/// 
/// - `Single`: 单次返回，同步场景，零堆分配
/// - `Stream`: 流式返回，按需拉取
pub enum InvokeStream {
    Single(StreamChunk),
    Stream(BoxStream<StreamChunk>),
}

impl InvokeStream {
    /// 创建单次返回（同步场景）
    pub fn single(data: Value) -> Self {
        InvokeStream::Single(StreamChunk { 
            data, 
            done: true,
            error: None,
        })
    }


    /// 创建流式返回
    pub fn stream<S>(stream: S) -> Self
    where
        S: Stream<Item = StreamChunk> + Send + 'static,
    {
        InvokeStream::Stream(Box::pin(stream))
    }

    /// 收集所有 chunk 为 Vec
    pub async fn collect(self) -> Vec<StreamChunk> {
        match self {
            InvokeStream::Single(chunk) => vec![chunk],
            InvokeStream::Stream(mut stream) => {
                use futures::StreamExt;
                let mut chunks = Vec::new();
                while let Some(chunk) = stream.next().await {
                    chunks.push(chunk);
                }
                chunks
            }
        }
    }
}
