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
    /// 上一轮交互的**结局**（`completed` / `aborted` / `failed`）。
    ///
    /// 这是**状态**而不是事件：提示音 / 错误条 / 重试入口都据此判定，
    /// 不需要「Abort 与 Error 谁先到」这种顺序知识（见
    /// `session/docs/node-state-streaming.md` §3.3）。
    ///
    /// `None` = 本进程内还没有跑完过一轮（或刚被新一轮请求复位）。
    pub last_outcome: Option<String>,
    /// 上一轮的错误短消息（面向用户）。仅当 `last_outcome == failed` 时存在。
    ///
    /// 存在的意义：错误可能发生在**任何消息节点创建之前**（能力收集失败 /
    /// provider 解析失败 / transport 级失败），此时没有失败节点可承载它，
    /// 只能挂在会话节点上——原先是前端一个平行状态（`sessionErrors`），
    /// 现在它是节点的属性（`attributes.error`）。
    pub last_error: Option<String>,
    /// 连续自动压缩失败次数（仅自动路径计数；主动 `context_compact` 不计）。
    ///
    /// 熔断依据：LLM 压缩是**整段历史的完整请求**，失败一次就是数分钟的注定浪费。
    /// 连续失败达到阈值即"开闸"跳过自动压缩——历史已回滚、本轮仍能正常回复，
    /// 不必每轮再白等一次注定失败的请求（实测会话 `09d74431` 就是这种反复失败）。
    /// 任一次压缩**成功**清零；开闸后按冷却时长重试一次（半开），让瞬时错误自愈。
    pub auto_compress_failures: u32,
    /// 熔断开闸时刻（`None` = 未开闸）。用于冷却判断。
    pub auto_compress_circuit_opened_at: Option<std::time::Instant>,
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
    /// - VDFS 转写列表：`<根>/session/<id>/消息` 把这份缓冲叠加在落库转写之上。
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
                last_outcome: None,
                last_error: None,
                auto_compress_failures: 0,
                auto_compress_circuit_opened_at: None,
            }),
            live_messages: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// 自动压缩熔断阈值：连续失败达到此数即开闸跳过。
    pub const COMPRESS_CIRCUIT_LIMIT: u32 = 3;
    /// 开闸后的冷却时长：期间跳过自动压缩；冷却结束允许一次重试（半开）。
    pub const COMPRESS_CIRCUIT_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(300);

    /// 自动压缩熔断：是否应跳过本次自动压缩。
    ///
    /// 判定：连续失败达到阈值即认为"这次 LLM 压缩注定失败"——继续重试只是每轮
    /// 白等数分钟。开闸后冷却期内一律跳过；冷却结束放行一次（半开），由这次
    /// 成功与否决定是否复位或重开。
    pub async fn compression_should_skip(&self) -> bool {
        let inner = self.inner.read().await;
        if inner.auto_compress_failures < Self::COMPRESS_CIRCUIT_LIMIT {
            return false;
        }
        match inner.auto_compress_circuit_opened_at {
            // 已开闸且冷却未到：跳过
            Some(opened) => opened.elapsed() < Self::COMPRESS_CIRCUIT_COOLDOWN,
            // 计数达标却没记录开闸时刻（不应发生）：保守放行一次探查
            None => false,
        }
    }

    /// 记录一次自动压缩**成功**：清零失败计数与开闸时刻。
    /// 任何路径的压缩成功都调用——熔断只在"连续失败"时维持。
    pub async fn compression_record_success(&self) {
        let mut inner = self.inner.write().await;
        inner.auto_compress_failures = 0;
        inner.auto_compress_circuit_opened_at = None;
    }

    /// 记录一次自动压缩**失败**：计数 +1；首次达到阈值时记下开闸时刻。
    /// 冷却期内失败不刷新开闸时刻（避免「每次失败都重置冷却」导致永不重试）。
    pub async fn compression_record_failure(&self) {
        let mut inner = self.inner.write().await;
        inner.auto_compress_failures += 1;
        if inner.auto_compress_failures >= Self::COMPRESS_CIRCUIT_LIMIT
            && inner.auto_compress_circuit_opened_at.is_none()
        {
            inner.auto_compress_circuit_opened_at = Some(std::time::Instant::now());
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

#[cfg(test)]
#[path = "active.test.rs"]
mod tests;
