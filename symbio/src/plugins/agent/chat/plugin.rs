//! AI 对话插件实现
//!
//! 职责：
//! - 前端对话接口（支持连接模式）
//! - 通过 @llm 能力路由调用实际的 LLM 插件 (openai)
//!
//! 连接模式：
//! - { type: "send", messages, session_id }: 发送消息（新请求自动中止旧请求）
//! - { type: "abort" }: 中止当前请求
//! - { type: "get_status" }: 查询 session 工作状态
//!
//! Session 状态管理：
//! - 每个 session 维护工作状态（is_working, current_content, request_id）
//! - 前端连接/重连时可查询状态
//! - 正在工作时前端显示停止按钮

use crate::symbio_core::traits::{Plugin, CAPABILITY_LLM};
use crate::symbio_core::types::{PluginMeta, PluginResult, InvokeStream, StreamChunk, Connection};
use serde_json::{Value, json};
use std::sync::{Arc, Weak, atomic::{AtomicU64, Ordering}};
use dashmap::DashMap;

/// 全局请求 ID 生成器
static REQUEST_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Session 工作状态
#[derive(Clone, Debug)]
struct SessionState {
    /// 是否正在工作
    is_working: bool,
    /// 当前累积的内容
    current_content: String,
    /// 当前请求 ID
    request_id: u64,
}

impl Default for SessionState {
    fn default() -> Self {
        Self {
            is_working: false,
            current_content: String::new(),
            request_id: 0,
        }
    }
}

/// 全局 session 状态存储
lazy_static::lazy_static! {
    static ref SESSION_STATES: DashMap<String, SessionState> = DashMap::new();
}

/// AI 对话插件
pub struct ChatPlugin {
    meta: PluginMeta,
    parent: Option<Weak<dyn Plugin>>,
}

impl ChatPlugin {
    fn create_meta() -> PluginMeta {
        PluginMeta {
            name: "chat".to_string(),
            description: "AI 对话插件 - 支持连接模式和状态查询".to_string(),
            version: "0.3.0".to_string(),
            input: Some(json!({
                "type": "object",
                "properties": {
                    "type": {
                        "type": "string",
                        "enum": ["send", "abort", "get_status"],
                        "description": "消息类型"
                    },
                    "messages": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "role": { "type": "string" },
                                "content": { "type": "string" }
                            }
                        }
                    },
                    "session_id": { "type": "string", "description": "会话 ID" }
                }
            })),
            output: Some(json!({
                "type": "object",
                "properties": {
                    "content": { "type": "string" },
                    "error": { "type": "string" },
                    "is_working": { "type": "boolean" }
                }
            })),
            author: Some("Symbio Team".to_string()),
        }
    }

    pub fn new(parent: Option<Weak<dyn Plugin>>) -> Self {
        Self { meta: Self::create_meta(), parent }
    }

    fn get_parent(&self) -> Option<Arc<dyn Plugin>> {
        self.parent.as_ref().and_then(|w| w.upgrade())
    }

    /// 获取 session 状态
    fn get_session_state(session_id: &str) -> SessionState {
        match SESSION_STATES.get(session_id) {
            Some(entry) => entry.value().clone(),
            None => SessionState::default(),
        }
    }

    /// 更新 session 状态
    fn update_session_state<F>(session_id: &str, f: F)
    where
        F: FnOnce(&mut SessionState),
    {
        let mut state = SESSION_STATES.entry(session_id.to_string()).or_default();
        f(&mut state);
    }

    /// 设置工作状态
    fn set_working(session_id: &str, request_id: u64) {
        Self::update_session_state(session_id, |s| {
            s.is_working = true;
            s.request_id = request_id;
        });
    }

    /// 更新内容
    fn update_content(session_id: &str, content: &str, request_id: u64) {
        Self::update_session_state(session_id, |s| {
            if s.request_id == request_id {
                s.current_content = content.to_string();
            }
        });
    }

    /// 完成工作
    fn complete_work(session_id: &str, request_id: u64) {
        Self::update_session_state(session_id, |s| {
            if s.request_id == request_id {
                s.is_working = false;
            }
        });
    }

    /// 中止工作
    fn abort_work(session_id: &str) -> (bool, u64, String) {
        let state = Self::get_session_state(session_id);
        if state.is_working {
            Self::update_session_state(session_id, |s| {
                s.is_working = false;
            });
            (true, state.request_id, state.current_content)
        } else {
            (false, 0, String::new())
        }
    }
}

impl Default for ChatPlugin {
    fn default() -> Self { Self::new(None) }
}

#[async_trait::async_trait]
impl Plugin for ChatPlugin {
    fn meta(&self, _path: &str) -> PluginResult<PluginMeta> {
        Ok(self.meta.clone())
    }

    fn invoke(&self, _path: &str, input: Value) -> PluginResult<InvokeStream> {
        let parent = self.get_parent();
        let stream = async_stream::stream! {
            let session_id = input.get("session_id").and_then(|v| v.as_str()).unwrap_or("default");
            let message = input.get("messages")
                .and_then(|msgs| msgs.as_array())
                .and_then(|arr| arr.last())
                .and_then(|msg| msg.get("content"))
                .and_then(|c| c.as_str());

            match message {
                Some(msg) => {
                    if let Some(ref p) = parent {
                        let llm_input = json!({"action": "chat", "message": msg, "session_id": session_id});
                        match p.invoke(&format!("@{}", CAPABILITY_LLM), llm_input) {
                            Ok(llm_stream) => {
                                use futures::StreamExt;
                                let mut stream = match llm_stream {
                                    InvokeStream::Single(chunk) => { yield chunk; return; }
                                    InvokeStream::Stream(s) => s,
                                };
                                while let Some(chunk) = stream.next().await { yield chunk; }
                            }
                            Err(e) => {
                                yield StreamChunk { data: json!({}), done: true, error: Some(format!("调用 LLM 失败: {}", e)) };
                            }
                        }
                    } else {
                        yield StreamChunk { data: json!({}), done: true, error: Some("父插件未设置".to_string()) };
                    }
                }
                None => {
                    yield StreamChunk { data: json!({}), done: true, error: Some("缺少消息内容".to_string()) };
                }
            }
        };
        Ok(InvokeStream::Stream(Box::pin(stream)))
    }

    /// 连接模式 - 支持请求中止和状态查询
    async fn connect(&self, _path: &str, input: Value, conn: Connection) -> PluginResult<()> {
        let session_id = input.get("session_id").and_then(|v| v.as_str()).unwrap_or("default").to_string();
        let current_request_id = Arc::new(AtomicU64::new(0));

        // 检查 session 当前状态并发送
        // 注意：如果 session 不在活跃工作中（request_id 为 0 或默认值），确保 is_working 为 false
        let mut state = Self::get_session_state(&session_id);
        // 如果 request_id 为 0（默认值），说明没有活跃请求，强制设置 is_working 为 false
        if state.request_id == 0 {
            state.is_working = false;
        }
        let _ = conn.send(json!({
            "type": "connected",
            "session_id": session_id.clone(),
            "is_working": state.is_working,
            "current_content": state.current_content,
            "request_id": state.request_id
        }));

        // 获取 Arc 引用
        let parent = self.get_parent();
        let sid = session_id.clone();
        let curr_id = current_request_id.clone();
        let c = conn.clone();
        
        // 记录当前活动的 session（用于 abort）
        let active_session: Arc<std::sync::Mutex<String>> = Arc::new(std::sync::Mutex::new(sid.clone()));
        let active_session_for_send = active_session.clone();

        conn.on_message(move |message: Value| {
            let msg_type = message.get("type").and_then(|t| t.as_str()).unwrap_or("");

            match msg_type {
                "send" => {
                    let request_id = REQUEST_ID_COUNTER.fetch_add(1, Ordering::SeqCst);
                    curr_id.store(request_id, Ordering::SeqCst);

                    let messages = message.get("messages").cloned();
                    let session = message.get("session_id").and_then(|s| s.as_str()).unwrap_or(&sid).to_string();
                    
                    // 更新活动 session
                    *active_session_for_send.lock().unwrap() = session.clone();

                    // 如果正在工作，先中止
                    let (was_working, old_id, old_content) = Self::abort_work(&session);
                    if was_working {
                        let _ = c.send(json!({
                            "type": "aborted",
                            "request_id": old_id,
                            "content": old_content,
                            "reason": "new_request"
                        }));
                    }

                    // 设置工作状态
                    Self::set_working(&session, request_id);

                    // 请求开始
                    let _ = c.send(json!({"type": "request_start", "request_id": request_id, "session_id": session}));

                    // 提取用户消息字符串，避免借用问题
                    let user_msg = messages.as_ref()
                        .and_then(|msgs| msgs.as_array())
                        .and_then(|arr| arr.last())
                        .and_then(|m| m.get("content"))
                        .and_then(|c| c.as_str())
                        .map(|s| s.to_string());

                    if let Some(user_msg) = user_msg {
                        // spawn 异步任务处理 LLM 请求，不阻塞消息处理器
                        let parent = parent.clone();
                        let sid_clone = session.clone();
                        let curr_id_clone = curr_id.clone();
                        let c_clone = c.clone();
                        let c_clone_for_error = c.clone();
                        let sid_for_monitor = session.clone();
                        let curr_id_for_monitor = curr_id.clone();

                        let handle = tokio::spawn(async move {
                            eprintln!("[chat] Spawning LLM request for session {}", sid_clone);
                            let llm_input = json!({"action": "chat", "message": user_msg, "session_id": sid_clone.clone()});

                            match parent.as_ref().and_then(|p| p.invoke(&format!("@{}", CAPABILITY_LLM), llm_input).ok()) {
                                Some(InvokeStream::Stream(mut stream)) => {
                                    eprintln!("[chat] LLM stream received, processing");
                                    use futures::StreamExt;
                                    let mut content = String::new();

                                    while let Some(chunk) = stream.next().await {
                                        // 检查是否被中止或连接关闭
                                        if curr_id_clone.load(Ordering::SeqCst) != request_id || c_clone.is_closed() {
                                            return;
                                        }

                                        // 检查错误
                                        if let Some(ref err) = chunk.error {
                                            Self::complete_work(&sid_clone, request_id);
                                            let _ = c_clone.send(json!({
                                                "type": "error",
                                                "request_id": request_id,
                                                "error": err
                                            }));
                                            return;
                                        }

                                        if let Some(text) = chunk.data.get("content").and_then(|c| c.as_str()) {
                                            content = text.to_string();
                                            Self::update_content(&sid_clone, &content, request_id);
                                        }

                                        let _ = c_clone.send(json!({
                                            "type": "chunk",
                                            "request_id": request_id,
                                            "data": chunk.data,
                                            "done": chunk.done
                                        }));

                                        if chunk.done { break; }
                                    }

                                    // 完成请求
                                    if curr_id_clone.load(Ordering::SeqCst) == request_id {
                                        Self::complete_work(&sid_clone, request_id);
                                        let _ = c_clone.send(json!({
                                            "type": "request_complete",
                                            "request_id": request_id,
                                            "content": content
                                        }));
                                    }
                                }
                                Some(InvokeStream::Single(chunk)) => {
                                    Self::complete_work(&sid_clone, request_id);
                                    if let Some(ref err) = chunk.error {
                                        let _ = c_clone.send(json!({"type": "error", "request_id": request_id, "error": err}));
                                    } else {
                                        let _ = c_clone.send(json!({"type": "chunk", "request_id": request_id, "data": chunk.data, "done": true}));
                                        let _ = c_clone.send(json!({"type": "request_complete", "request_id": request_id}));
                                    }
                                }
                                None => {
                                    Self::complete_work(&sid_clone, request_id);
                                    let _ = c_clone.send(json!({"type": "error", "request_id": request_id, "error": "LLM 调用失败: 无父插件"}));
                                }
                            }
                        });

                        // spawn 一个监控任务，检测 LLM 任务是否 panic
                        tokio::spawn(async move {
                            if let Err(e) = handle.await {
                                eprintln!("[chat] LLM request failed: {:?}", e);
                                Self::complete_work(&sid_for_monitor, request_id);
                                let _ = c_clone_for_error.send(json!({
                                    "type": "error",
                                    "request_id": request_id,
                                    "error": format!("LLM 请求失败: {}", e)
                                }));
                            }
                        });
                    }
                }
                "abort" => {
                    let session = active_session.lock().unwrap().clone();
                    eprintln!("[chat] Abort requested for session {}", session);
                    let state = Self::get_session_state(&session);
                    eprintln!("[chat] Session state: is_working={}, request_id={}", state.is_working, state.request_id);
                    
                    let (was_working, old_id, old_content) = Self::abort_work(&session);
                    if was_working {
                        // 重置 curr_id 以触发正在进行的流处理检查失败
                        curr_id.store(0, Ordering::SeqCst);
                        eprintln!("[chat] Aborted, curr_id reset to 0, old_id={}", old_id);
                        let _ = c.send(json!({
                            "type": "aborted",
                            "request_id": old_id,
                            "content": old_content
                        }));
                    } else {
                        eprintln!("[chat] Abort ignored - not working");
                    }
                }
                "get_status" => {
                    let state = Self::get_session_state(&sid);
                    let _ = c.send(json!({
                        "type": "status",
                        "session_id": sid,
                        "is_working": state.is_working,
                        "current_content": state.current_content,
                        "request_id": state.request_id
                    }));
                }
                _ => {
                    let _ = c.send(json!({"type": "error", "error": format!("未知消息类型: {}", msg_type)}));
                }
            }
        });

        // 等待连接关闭
        while !conn.is_closed() {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }

        // 连接关闭时重置工作状态，避免下次连接时状态错误
        // 使用 update_session_state 完全重置状态，而不仅仅是设置 is_working = false
        Self::update_session_state(&session_id, |s| {
            s.is_working = false;
            s.current_content = String::new();
            // 保留 request_id 用于调试，但确保下次连接时能正确判断
        });
        eprintln!("[chat] Connection closed: {}, state reset", session_id);
        Ok(())
    }
}