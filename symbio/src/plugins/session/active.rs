use crate::symbio_core::schemas::session::chat_message as cm;
use crate::symbio_core::PluginFrame;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{atomic::AtomicU64, atomic::Ordering, Arc};
use tokio::sync::{mpsc, Mutex, RwLock};

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
    /// 本轮**在途**（尚未落库）的消息缓冲。
    ///
    /// ## 一份数据，两个视图
    ///
    /// 这与消费循环里收集流式补丁的那个缓冲是**同一个 `Arc`**（见
    /// `orchestrator::run_chat_loop_task`），不是第二份拷贝：
    ///
    /// - 前端实时流：补丁逐帧经 `StreamEvent::Update` 下发；
    /// - VDFS 转写列表：`.vdfs/session/<id>/消息` 把这份缓冲叠加在落库转写之上。
    ///
    /// 之所以必须共享：**流式期间消息还没落库**（`persist_messages` 只在每轮结束时
    /// 写盘）。若 VDFS 只读存储，列表在流式期间就是空的，`created` / `appended`
    /// 事件到达时消费者去 `list` 会一无所获——「转写即列表」当场失效。
    ///
    /// 生命周期：每轮开始时清空（防上一轮残留），本轮落库后清空（防与落库版本重复）。
    pub live_messages: Arc<Mutex<Vec<cm::ChatMessage>>>,
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
            live_messages: Arc::new(Mutex::new(Vec::new())),
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
