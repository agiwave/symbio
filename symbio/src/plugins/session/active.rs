use crate::symbio_core::vdfs::ChangeSubscriptions;
use crate::symbio_core::AbortSignal;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{atomic::AtomicU64, atomic::Ordering, Arc};
use tokio::sync::{Mutex, RwLock};

/// 全局请求 ID 生成器
pub static REQUEST_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

/// 会话内部状态 (封装为单个锁定对象以保证原子性)
pub struct ActiveSessionStateInner {
    pub is_working: bool,
    /// 在途 Turn 的中止信号（登记后 `handle_abort` 直接置位，无需投帧）。
    ///
    /// 收口前这里是 `Option<mpsc::Sender<PluginFrame>>`——中止只能靠「往执行期
    /// 通道投一帧 `ControlSignal::Abort`」，执行方在 `select!` 里收帧再置位标志。
    /// 现在登记的就是那个信号本身：置位即生效、置位即唤醒（无轮询、无帧）。
    ///
    /// `None` 的双重含义与收口前一致：既表示「当前没有在途 Turn」，也是
    /// `handle_abort` 判断 chat_loop 是否已收敛的判据。
    pub abort_signal: Option<AbortSignal>,
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
    /// 会话级告警（可恢复，面向用户）：持久化失败 / 长度截断 / 工具轮次上限。
    ///
    /// 它是**状态**不是事件：由出口的告警通道（`EventSink::warn`）写入、随会话节点
    /// `attributes.warning` 下发，前端按状态渲染；新一轮请求开始（`Working`）时清除。
    /// 与 `last_error`（失败终态）不同：告警不改变运行态，会话照常运行。
    pub last_warning: Option<String>,
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
    /// 会话**转写**（在途图 + 单调 seq + 实时发布）。
    ///
    /// ## 一份数据，两个视图
    ///
    /// 这是消费循环里唯一写入的那个 Transcript（见 `transcript.rs`），不是第二份拷贝：
    ///
    /// - 实时面：每条变更（`created` / `updated` + `delta` / `deleted`）经
    ///   `apply` 投到 VDFS 变更订阅表——一张表、一条 `vdfs/watch`；
    /// - VDFS 转写列表：`<根>/session/<id>/消息` 把在途图叠加在落库转写之上。
    ///
    /// 之所以必须共享：**流式期间消息还没落库**（`persist_messages` 只在每轮结束时
    /// 写盘）。若 VDFS 只读存储，列表在流式期间就是空的。
    ///
    /// 生命周期：每轮开始时清空（防上一轮残留），本轮落库后清空（防与落库版本重复）。
    pub transcript: Arc<Mutex<super::transcript::Transcript>>,
}

impl Default for ActiveSessionState {
    fn default() -> Self {
        Self::new(ChangeSubscriptions::default())
    }
}

impl ActiveSessionState {
    /// `changes` 是**本 provider 自持的那张 VDFS 变更表**——转写的实时面投给它。
    /// 不收它、让 `Transcript` 自己去取全局表，会投到没有订阅者的那张表上
    /// （见 `Transcript::new` 的说明）。
    pub fn new(changes: ChangeSubscriptions) -> Self {
        Self::with_session_id(String::new(), changes)
    }

    /// 带 session_id 的构造函数
    pub fn with_session_id(session_id: String, changes: ChangeSubscriptions) -> Self {
        Self {
            request_id: AtomicU64::new(0),
            session_id: session_id.clone(),
            inner: RwLock::new(ActiveSessionStateInner {
                is_working: false,
                abort_signal: None,
                last_content: String::new(),
                last_tool_calls: Vec::new(),
                last_outcome: None,
                last_error: None,
                last_warning: None,
                auto_compress_failures: 0,
                auto_compress_circuit_opened_at: None,
            }),
            transcript: Arc::new(Mutex::new(super::transcript::Transcript::new(
                session_id,
                Arc::new(changes),
            ))),
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
    /// VDFS 变更表句柄：**每一份**新建的 `ActiveSessionState` 都把自己的
    /// `Transcript` 接到这张表上。共享句柄而非各建一张——`vdfs/watch` 登记的是
    /// 插件那一张（`SessionPlugin::change_subs`），别的表上没有订阅者。
    changes: ChangeSubscriptions,
}

impl ActiveSessionManager {
    pub fn new(changes: ChangeSubscriptions) -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            changes,
        }
    }

    pub async fn get_or_create(&self, session_id: &str) -> Arc<ActiveSessionState> {
        let mut sessions = self.sessions.write().await;
        if let Some(state) = sessions.get(session_id) {
            return state.clone();
        }
        let state = Arc::new(ActiveSessionState::with_session_id(
            session_id.to_string(),
            self.changes.clone(),
        ));
        sessions.insert(session_id.to_string(), state.clone());
        state
    }
}

#[cfg(test)]
#[path = "active.test.rs"]
mod tests;
