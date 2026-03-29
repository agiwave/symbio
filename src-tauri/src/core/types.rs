//! 核心类型定义

use serde::{Deserialize, Serialize};
use serde_json::Value;

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
}
