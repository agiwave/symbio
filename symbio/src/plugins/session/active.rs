use crate::symbio_core::PluginFrame;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{atomic::AtomicU64, atomic::Ordering, Arc};
use tokio::sync::{mpsc, RwLock};

/// 全局请求 ID 生成器
pub static REQUEST_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

/// 会话内部状态 (封装为单个锁定对象以保证原子性)
pub struct ActiveSessionStateInner {
    pub is_working: bool,
    /// 用于向 MODEL 任务发送控制信号 (Abort)
    pub ai_control_tx: Option<mpsc::Sender<PluginFrame>>,
    /// 允许多个前端订阅同一个会话
    pub frontends: Vec<mpsc::Sender<PluginFrame>>,
    pub last_content: String,
    pub last_tool_calls: Vec<Value>,
}

/// 会话状态锚点
pub struct ActiveSessionState {
    pub request_id: AtomicU64,
    /// 会话 ID 字符串（用于 EventBus 标签）
    pub session_id: String,
    pub inner: RwLock<ActiveSessionStateInner>,
}

impl Default for ActiveSessionState {
    fn default() -> Self {
        Self::new()
    }
}

impl ActiveSessionState {
    pub fn new() -> Self {
        Self::with_session_id(String::new())
    }

    /// 带 session_id 的构造函数
    pub fn with_session_id(session_id: String) -> Self {
        Self {
            request_id: AtomicU64::new(0),
            session_id,
            inner: RwLock::new(ActiveSessionStateInner {
                is_working: false,
                ai_control_tx: None,
                frontends: Vec::new(),
                last_content: String::new(),
                last_tool_calls: Vec::new(),
            }),
        }
    }

    /// 返回 session_id 字符串的便捷方法
    pub fn request_id_str(&self) -> String {
        if !self.session_id.is_empty() {
            return self.session_id.clone();
        }
        // 兜底：使用内部 request_id
        self.request_id.load(Ordering::SeqCst).to_string()
    }
}

/// 活跃会话管理器
pub struct ActiveSessionManager {
    pub sessions: Arc<RwLock<HashMap<String, Arc<ActiveSessionState>>>>,
}

impl Default for ActiveSessionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ActiveSessionManager {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn get_or_create(&self, session_id: &str) -> Arc<ActiveSessionState> {
        let mut sessions = self.sessions.write().await;
        if let Some(state) = sessions.get(session_id) {
            return state.clone();
        }
        let state = Arc::new(ActiveSessionState::with_session_id(session_id.to_string()));
        sessions.insert(session_id.to_string(), state.clone());
        state
    }
}
