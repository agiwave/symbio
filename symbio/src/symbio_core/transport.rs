// Corresponding Frontend: tauri/src/services/plugin.ts
//! 插件双向分形路由协议
//!
//! 定义了统一的消息载荷模型 (PluginPayload) 和对称的消息容器 (PluginMessage)。
//! 支持 JSON 数据与长连接会话。

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::any::Any;
use std::fmt;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

// 1. 统一交互帧
//
// ## 载荷为什么是 `Arc<Value>` 而不是 `Value`
//
// 本帧在**扇出**时逐订阅者克隆：`event_bus::try_publish` 与
// 已退役的转写流的 `publish_frame` 也对订阅表里的每个 `tx` 做一次 `frame.clone()`。
// 载荷是 `Value` 时，这个克隆是**整棵 JSON 树的深拷贝**——订阅者越多、消息越长，
// 出帧路径上的纯拷贝开销越大（一次回复可达上百帧 × 每个订阅者一份）。
//
// 换成 `Arc<Value>` 后：克隆 = 一次引用计数自增；载荷只有一份，所有订阅者共享。
// 语义完全不变（`Arc<Value>` 通过 `Deref` 就是 `&Value`，序列化结果与 `Value` 逐字节相同），
// 变的是**谁付拷贝的钱**——从「每个订阅者各付一次」变成「一次都不付」。
//
// 代价：`Arc<T>: Deserialize` 需要 serde 的 `rc` feature（已在 `Cargo.toml` 开启）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PluginFrame {
    /// 业务数据负载
    Data(Arc<Value>),
    /// 异常报告 (错误信息, 详细数据)
    Error(String, Option<Value>),
}

impl PluginFrame {
    /// 用 `Value` 构造一个 `Data` 帧。
    ///
    /// 这是 `Data` 帧**唯一推荐的构造入口**：`Data` 的载荷类型是 `Arc<Value>`，
    /// 直接写变体就得在每个调用点各写一遍 `Arc::new`（且容易漏）。
    pub fn data(value: Value) -> Self {
        Self::Data(Arc::new(value))
    }

    /// 取出 `Data` 帧的载荷引用（非 `Data` 帧返回 `None`）。
    pub fn value(&self) -> Option<&Value> {
        match self {
            PluginFrame::Data(v) => Some(v),
            PluginFrame::Error(_, _) => None,
        }
    }

    pub fn into_value(self) -> Value {
        match self {
            // 独占时直接解包（零拷贝）；仍有其它持有者才退化为深拷贝。
            PluginFrame::Data(v) => Arc::try_unwrap(v).unwrap_or_else(|shared| (*shared).clone()),
            _ => serde_json::json!({}),
        }
    }

    /// 尝试将 Data 帧解析为指定的业务事件模型。
    ///
    /// **不克隆载荷**：用 `&Value` 自身的 `Deserializer` 实现（与
    /// `serde_json::from_value` 走同一条路径，只是不要求所有权）。
    pub fn try_into_event<T: DeserializeOwned>(&self) -> Result<T, String> {
        match self {
            PluginFrame::Data(v) => T::deserialize(v.as_ref())
                .map_err(|e| format!("Failed to deserialize frame data: {e}")),
            PluginFrame::Error(msg, _) => Err(format!("Cannot deserialize Error frame: {msg}")),
        }
    }

    /// 读取 `Error` 帧携带的机器可读错误码。
    ///
    /// 消费侧据此分派（中止 / 上下文丢失重试 / 普通失败），**不得**对
    /// `meta["code"]` 做字符串字面量比较。
    /// 非 `Error` 帧、无 meta、或 code 不可识别均返回 `None`。
    pub fn error_code(&self) -> Option<crate::symbio_core::ErrorCode> {
        let PluginFrame::Error(_, meta) = self else {
            return None;
        };
        let raw = meta.as_ref()?.get("code")?.as_str()?;
        let code = crate::symbio_core::ErrorCode::from_code(raw);
        match code {
            crate::symbio_core::ErrorCode::Unknown => None,
            other => Some(other),
        }
    }

    /// 是否为"用户主动中止"的 `Error` 帧（等价于 `error_code() == Some(Aborted)`）。
    pub fn is_abort(&self) -> bool {
        self.error_code() == Some(crate::symbio_core::ErrorCode::Aborted)
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

/// 统一有效载荷
///
/// **3 态穷尽枚举**，按「怎么交付给对端」分成两类——分类点唯一
/// （`plugins::gateway::server::classify_payload`）：
/// - **一次性**：`Data` / `Empty` —— 可直接转成线上载荷；
/// - **长连接**：`Session` —— 返回通道，后续用帧通信。
///
/// ## 为什么没有「原生对象」变体
///
/// 历史上这里有 `Native(Arc<dyn Any + Send + Sync>)`，自述「不可序列化的原生
/// 接口，仅限进程内透传」。它**从未有过构造点**：唯二的匹配点都在 `gateway`，
/// 且都是「拒绝跨传输」——因为 `Arc<dyn Any>` 恰恰是协议层表达不了的东西，
/// 于是那两处只能写 `Err`。
///
/// 进程内对象的透传走的是**上下文扩展桶**（`InvokeRequest::set_raw`，见
/// `SimpleRequest::extensions` 的注释），与载荷枚举无关。两者职责不同：
/// 载荷枚举描述「这次调用**返回**什么」，扩展桶描述「这次调用**带着**什么」。
/// `Native` 是后者误入前者的产物，故删除——协议层不再假装存在这条路径。
#[derive(Default)]
pub enum PluginPayload {
    /// 空载荷
    #[default]
    Empty,
    /// 可序列化的原生数据（延迟序列化，进程内零拷贝）
    Data(SerializeData),
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
            }
            Self::Empty => Err("Cannot deserialize Empty payload".to_string()),
            Self::Session(_) => Err("Cannot deserialize Session payload".to_string()),
        }
    }

    /// 序列化数据（用于跨进程通信场景）
    pub fn serialize(&self) -> Result<Value, String> {
        match self {
            Self::Data(obj) => obj.serialize(),
            Self::Empty => Ok(Value::Null),
            Self::Session(_) => Err("Cannot serialize Session payload".to_string()),
        }
    }
}

impl fmt::Debug for PluginPayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "Empty"),
            Self::Data(_) => write!(f, "Data(SerializeData)"),
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

// ==================== 线路层消息容器 ====================
//
// 下面两个类型是线路层唯一的请求/响应容器定义，供 Tauri IPC 与 HTTP/WebSocket
// 两种传输共用同一份线上格式（不得另立结构体）；前端 `services/plugin.ts`
// 也按这个结构收发。

/// 传输层的统一请求结构
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginMessageWire {
    pub metadata: Value,
    pub payload: Value,
}

/// 传输层的统一响应载荷结构
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum PluginPayloadWire {
    /// 立即响应数据
    Data(Value),
    /// 连接已建立，返回 `connection_id`
    Connection(String),
}
