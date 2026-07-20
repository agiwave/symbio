// Corresponding Frontend: tauri/src/services/plugin.ts
//! 插件双向分形路由协议 (V2.7 Unified)
//!
//! 定义了统一的消息载荷模型 (PluginPayload) 和对称的消息容器 (PluginMessage)。
//! 支持 JSON 数据、原生接口 (Native Interface) 和长连接会话。

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::any::Any;
use std::fmt;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

// 1. 统一交互帧
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PluginFrame {
    /// 业务数据负载
    Data(Value),
    /// 异常报告 (错误信息, 详细数据)
    Error(String, Option<Value>),
}

impl PluginFrame {
    pub fn into_value(self) -> Value {
        match self {
            PluginFrame::Data(v) => v,
            _ => serde_json::json!({}),
        }
    }

    /// 尝试将 Data 帧解析为指定的业务事件模型
    pub fn try_into_event<T: serde::de::DeserializeOwned>(&self) -> Result<T, String> {
        match self {
            PluginFrame::Data(v) => serde_json::from_value::<T>(v.clone())
                .map_err(|e| format!("Failed to deserialize frame data: {e}")),
            PluginFrame::Error(msg, _) => Err(format!("Cannot deserialize Error frame: {msg}")),
        }
    }
}

type NativeSerializer = fn(&Arc<Box<dyn Any + Send + Sync + 'static>>) -> Result<Value, String>;

/// 可序列化的原生数据（延迟序列化，进程内零拷贝）
#[derive(Debug)]
pub struct SerializeData {
    data: Arc<Box<dyn Any + Send + Sync + 'static>>,
    serializer: NativeSerializer,
}

impl SerializeData {
    pub fn new<T: Serialize + Clone + Send + Sync + 'static>(data: &T) -> Self {
        let cloned = data.clone();
        Self {
            data: Arc::new(Box::new(cloned)),
            serializer: |obj| {
                if let Some(val) = obj.downcast_ref::<T>() {
                    serde_json::to_value(val).map_err(|e| format!("Serialization failed: {}", e))
                } else {
                    Err("Type mismatch in serializer".to_string())
                }
            },
        }
    }

    pub fn serialize(&self) -> Result<Value, String> {
        (self.serializer)(&self.data)
    }

    pub fn downcast_ref<T: 'static>(&self) -> Option<&T> {
        self.data.downcast_ref::<T>()
    }
}

impl Clone for SerializeData {
    fn clone(&self) -> Self {
        Self {
            data: self.data.clone(),
            serializer: self.serializer,
        }
    }
}

/// 统一有效载荷 (V2.7)
#[derive(Default)]
pub enum PluginPayload {
    /// 空载荷
    #[default]
    Empty,
    /// 可序列化的原生数据（延迟序列化，进程内零拷贝）
    Data(SerializeData),
    /// 不可序列化的原生接口，仅限进程内透传
    Native(Arc<dyn Any + Send + Sync>),
    /// 异步通道 (用于 Session 模式)
    Session(PluginChannel),
}

impl PluginPayload {
    /// 从可序列化数据构建 PluginPayload（直接存储原生对象，延迟序列化）
    pub fn new<T: Serialize + Clone + Send + Sync + 'static>(data: &T) -> Self {
        Self::Data(SerializeData::new(data))
    }

    /// 从 PluginPayload 中提取并反序列化为指定类型（非 Option，返回 Result）
    /// 优先尝试直接类型转换（零拷贝），失败时尝试序列化后反序列化
    pub fn get<T: DeserializeOwned + Serialize + Clone + Send + Sync + 'static>(
        self,
    ) -> Result<T, String> {
        match self {
            Self::Data(obj) => {
                if let Some(val) = obj.downcast_ref::<T>() {
                    return Ok(val.clone());
                }
                serde_json::from_value(obj.serialize()?)
                    .map_err(|e| format!("Failed to deserialize data: {}", e))
            },
            Self::Empty => Err("Cannot deserialize Empty payload".to_string()),
            Self::Native(_) => Err("Cannot deserialize Native payload".to_string()),
            Self::Session(_) => Err("Cannot deserialize Session payload".to_string()),
        }
    }

    /// 序列化数据（用于跨进程通信场景）
    pub fn serialize(&self) -> Result<Value, String> {
        match self {
            Self::Data(obj) => obj.serialize(),
            Self::Empty => Ok(Value::Null),
            Self::Native(_) => Err("Cannot serialize Native payload".to_string()),
            Self::Session(_) => Err("Cannot serialize Session payload".to_string()),
        }
    }
}

impl fmt::Debug for PluginPayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "Empty"),
            Self::Data(_) => write!(f, "Data(SerializeData)"),
            Self::Native(_) => write!(f, "Interface(dyn Any)"),
            Self::Session(_) => write!(f, "Session(PluginChannel)"),
        }
    }
}

/// 双向交互通道
#[derive(Debug)]
pub struct PluginChannel {
    pub tx: mpsc::Sender<PluginFrame>,
    pub rx: mpsc::Receiver<PluginFrame>,
    pub cancel_token: CancellationToken,
}

impl PluginChannel {
    pub fn pair(buffer: usize) -> (Self, Self) {
        let (tx1, rx1) = mpsc::channel(buffer);
        let (tx2, rx2) = mpsc::channel(buffer);
        let token = CancellationToken::new();
        (
            Self {
                tx: tx1,
                rx: rx2,
                cancel_token: token.clone(),
            },
            Self {
                tx: tx2,
                rx: rx1,
                cancel_token: token,
            },
        )
    }
}
