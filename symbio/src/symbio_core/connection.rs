//! 连接管理模块
//!
//! 提供平台无关的双向通信连接机制。
//! 插件通过 `Connection` 句柄与客户端进行持久化双向通信。

use crate::symbio_core::event::EventSender;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

/// 连接句柄
///
/// 插件通过此句柄与客户端进行双向通信：
/// - `send()` / `emit()`: 向客户端发送消息
/// - `on_message()`: 注册客户端消息处理器
/// - `state()`: 连接级状态存储
/// - `close()`: 主动关闭连接
#[derive(Clone)]
pub struct Connection {
    pub id: String,
    inner: Arc<ConnectionInner>,
}

struct ConnectionInner {
    event_sender: Arc<dyn EventSender>,
    state: std::sync::Mutex<HashMap<String, Value>>,
    closed: AtomicBool,
    message_handler: std::sync::Mutex<Option<Box<dyn Fn(Value) + Send + Sync>>>,
}

impl Connection {
    /// 创建新连接
    pub fn new(id: String, event_sender: Arc<dyn EventSender>) -> Self {
        Self {
            id,
            inner: Arc::new(ConnectionInner {
                event_sender,
                state: std::sync::Mutex::new(HashMap::new()),
                closed: AtomicBool::new(false),
                message_handler: std::sync::Mutex::new(None),
            }),
        }
    }

    /// 向客户端发送消息（类型为 "message"）
    pub fn send(&self, data: Value) -> Result<(), String> {
        self.emit("message", data)
    }

    /// 向客户端发送特定类型事件
    pub fn emit(&self, event_type: &str, data: Value) -> Result<(), String> {
        if self.inner.closed.load(Ordering::Relaxed) {
            return Err("Connection closed".to_string());
        }

        let event_name = format!("connect/{}", self.id);
        let payload = serde_json::json!({
            "type": event_type,
            "data": data
        });

        self.inner.event_sender.emit(&event_name, payload)
    }

    /// 注册客户端消息处理器
    ///
    /// 当客户端通过 `connect.send` 发送消息时，此处理器会被调用。
    /// 插件应在此处理器中处理客户端消息并通过 `conn.send()` 回复。
    pub fn on_message<F>(&self, handler: F)
    where
        F: Fn(Value) + Send + Sync + 'static,
    {
        let mut guard = self.inner.message_handler.lock().unwrap();
        *guard = Some(Box::new(handler));
    }

    /// 触发消息处理器（由运行时框架调用）
    pub fn handle_message(&self, message: Value) {
        if let Some(handler) = self.inner.message_handler.lock().unwrap().as_ref() {
            handler(message);
        }
    }

    /// 检查连接是否已关闭
    pub fn is_closed(&self) -> bool {
        self.inner.closed.load(Ordering::Relaxed)
    }

    /// 关闭连接
    pub fn close(&self, reason: &str) -> Result<(), String> {
        self.inner.closed.store(true, Ordering::Relaxed);
        self.emit("disconnected", serde_json::json!({ "reason": reason }))
    }

    /// 获取连接级状态存储
    ///
    /// 用于在连接生命周期内存储状态数据，
    /// 例如会话 ID、用户偏好等。
    pub fn state(&self) -> &std::sync::Mutex<HashMap<String, Value>> {
        &self.inner.state
    }
}

/// 连接管理器
///
/// 管理所有活动连接的生命周期，包括：
/// - 创建和销毁连接
/// - 超时自动清理
/// - 连接状态查询
pub struct ConnectionManager {
    connections: dashmap::DashMap<String, ConnectionInfo>,
    timeout_secs: u64,
    next_id: AtomicU64,
    cleanup_started: AtomicBool,
}

#[derive(Clone)]
struct ConnectionInfo {
    conn: Connection,
    created_at: std::time::Instant,
    last_active: Arc<std::sync::Mutex<std::time::Instant>>,
}

impl ConnectionManager {
    /// 创建新的连接管理器
    ///
    /// `timeout_secs`: 连接超时时间（秒），超时后自动清理
    pub fn new(_event_sender: Arc<dyn EventSender>, timeout_secs: u64) -> Self {
        Self {
            connections: dashmap::DashMap::new(),
            timeout_secs,
            next_id: AtomicU64::new(1),
            cleanup_started: AtomicBool::new(false),
        }
    }

    /// 启动后台清理任务（延迟初始化，确保在 Tokio runtime 中调用）
    fn ensure_cleanup_started(&self, event_sender: Arc<dyn EventSender>) {
        if !self.cleanup_started.load(Ordering::Relaxed) {
            self.cleanup_started.store(true, Ordering::Relaxed);
            self.start_cleanup_task(event_sender);
        }
    }

    /// 生成唯一连接 ID
    fn generate_id(&self) -> String {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        format!("conn_{}", id)
    }

    /// 创建新连接
    pub fn create(&self, event_sender: Arc<dyn EventSender>) -> Connection {
        // 延迟启动清理任务
        self.ensure_cleanup_started(event_sender.clone());

        let id = self.generate_id();
        let conn = Connection::new(id.clone(), event_sender);

        self.connections.insert(id.clone(), ConnectionInfo {
            conn: conn.clone(),
            created_at: std::time::Instant::now(),
            last_active: Arc::new(std::sync::Mutex::new(std::time::Instant::now())),
        });

        conn
    }

    /// 获取连接
    pub fn get(&self, id: &str) -> Option<Connection> {
        self.connections.get(id).map(|entry| {
            // 更新最后活跃时间
            *entry.last_active.lock().unwrap() = std::time::Instant::now();
            entry.conn.clone()
        })
    }

    /// 移除并关闭连接
    pub fn remove(&self, id: &str) -> Option<Connection> {
        self.connections.remove(id).map(|(_, info)| {
            info.conn.close("removed").ok();
            info.conn
        })
    }

    /// 检查连接是否存活
    pub fn is_alive(&self, id: &str) -> bool {
        self.connections.get(id).map_or(false, |entry| {
            entry.last_active.lock().unwrap().elapsed().as_secs() < self.timeout_secs
        })
    }

    /// 获取连接数量
    pub fn len(&self) -> usize {
        self.connections.len()
    }

    /// 检查是否为空
    pub fn is_empty(&self) -> bool {
        self.connections.is_empty()
    }

    /// 启动后台清理任务
    fn start_cleanup_task(&self, _event_sender: Arc<dyn EventSender>) {
        let manager = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                let timeout = manager.timeout_secs;
                manager.connections.retain(|_, info| {
                    info.last_active.lock().unwrap().elapsed().as_secs() < timeout
                });
            }
        });
    }
}

// 为 ConnectionManager 实现 Clone
impl Clone for ConnectionManager {
    fn clone(&self) -> Self {
        Self {
            connections: self.connections.clone(),
            timeout_secs: self.timeout_secs,
            next_id: AtomicU64::new(self.next_id.load(Ordering::Relaxed)),
            cleanup_started: AtomicBool::new(self.cleanup_started.load(Ordering::Relaxed)),
        }
    }
}
