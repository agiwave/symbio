//! Session 插件实现
//!
//! 提供会话历史和上下文管理

use super::types::{ChatMessage, ContextEntry, Session, SessionContext, LlmContext};
use crate::core::traits::{Plugin, CAPABILITY_SESSION};
use crate::core::types::{PluginMeta, PluginResult, PluginError, InvokeStream, StreamChunk};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::{Arc, Weak};
use tokio::sync::RwLock;

/// Session 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionConfig {
    /// 存储目录
    #[serde(default = "default_storage_dir")]
    pub storage_dir: String,
    /// 最大消息数
    #[serde(default = "default_max_messages")]
    pub max_messages: usize,
    /// 自动压缩
    #[serde(default = "default_auto_compress")]
    pub auto_compress: bool,
    /// 压缩阈值（消息数）
    #[serde(default = "default_compress_threshold")]
    pub compress_threshold: usize,
}

fn default_storage_dir() -> String { 
    dirs::data_local_dir()
        .map(|p| p.join("symbio").join("sessions").to_string_lossy().to_string())
        .unwrap_or_else(|| "~/.local/share/symbio/sessions".to_string())
}
fn default_max_messages() -> usize { 100 }
fn default_auto_compress() -> bool { true }
fn default_compress_threshold() -> usize { 50 }

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            storage_dir: default_storage_dir(),
            max_messages: default_max_messages(),
            auto_compress: default_auto_compress(),
            compress_threshold: default_compress_threshold(),
        }
    }
}

/// Session 插件
pub struct SessionPlugin {
    meta: PluginMeta,
    config: Arc<RwLock<SessionConfig>>,
    storage_dir: Arc<RwLock<PathBuf>>,
    /// 父插件引用（用于保存配置）
    parent: Option<Weak<dyn Plugin>>,
}

impl SessionPlugin {
    fn create_meta() -> PluginMeta {
        PluginMeta {
            name: "session".to_string(),
            description: "会话历史和上下文管理".to_string(),
            version: "0.1.0".to_string(),
            input: Some(json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["get", "append", "clear", "list", "get_context", "add_context", "clear_context"]
                    },
                    "session_id": { "type": "string" },
                    "messages": { "type": "array" },
                    "context_path": { "type": "string" }
                },
                "required": ["action"]
            })),
            output: None,
            author: Some("Symbio Team".to_string()),
        }
    }

    /// 主构造函数（Factory 机制使用）
    pub fn new(parent: Option<Weak<dyn Plugin>>, config: SessionConfig) -> Self {
        let storage_dir = PathBuf::from(&config.storage_dir);
        Self {
            meta: Self::create_meta(),
            config: Arc::new(RwLock::new(config)),
            storage_dir: Arc::new(RwLock::new(storage_dir)),
            parent,
        }
    }

    /// 获取父插件引用
    fn get_parent(&self) -> Option<Arc<dyn Plugin>> {
        self.parent.as_ref().and_then(|w| w.upgrade())
    }

    /// 获取配置 Schema
    fn config_schema() -> Value {
        json!({
            "storage_dir": {
                "type": "string",
                "title": "存储目录",
                "description": "会话数据存储目录",
                "default": default_storage_dir()
            },
            "max_messages": {
                "type": "integer",
                "title": "最大消息数",
                "description": "每个会话保存的最大消息数量",
                "minimum": 10,
                "maximum": 1000,
                "default": 100
            },
            "auto_compress": {
                "type": "boolean",
                "title": "自动压缩",
                "description": "当消息数超过阈值时自动压缩历史",
                "default": true
            },
            "compress_threshold": {
                "type": "integer",
                "title": "压缩阈值",
                "description": "触发自动压缩的消息数量",
                "minimum": 10,
                "maximum": 500,
                "default": 50
            }
        })
    }

    fn session_path(dir: &PathBuf, session_id: &str) -> PathBuf {
        let safe_id = session_id.replace(['/', '\\', ':'], "_");
        dir.join(format!("{}.json", safe_id))
    }

    fn context_path(dir: &PathBuf) -> PathBuf {
        dir.join("context.json")
    }

    async fn get_or_create_session(&self, session_id: &str) -> Result<Session, PluginError> {
        let dir = self.storage_dir.read().await.clone();
        let path = Self::session_path(&dir, session_id);
        
        if path.exists() {
            let content = tokio::fs::read_to_string(&path)
                .await
                .map_err(|e| PluginError::InternalError(format!("读取会话失败: {}", e)))?;
            serde_json::from_str(&content)
                .map_err(|e| PluginError::ParseError(format!("解析会话失败: {}", e)))
        } else {
            Ok(Session::new(session_id))
        }
    }

    async fn save_session(&self, session: &Session) -> Result<(), PluginError> {
        let dir = self.storage_dir.read().await.clone();
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| PluginError::InternalError(format!("创建目录失败: {}", e)))?;
        
        let path = Self::session_path(&dir, &session.id);
        let content = serde_json::to_string_pretty(session)
            .map_err(|e| PluginError::InternalError(format!("序列化失败: {}", e)))?;
        
        tokio::fs::write(&path, content)
            .await
            .map_err(|e| PluginError::InternalError(format!("写入会话失败: {}", e)))
    }

    async fn get_context_data(&self) -> SessionContext {
        let dir = self.storage_dir.read().await.clone();
        let path = Self::context_path(&dir);
        
        if path.exists() {
            tokio::fs::read_to_string(&path)
                .await
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default()
        } else {
            SessionContext::default()
        }
    }

    async fn save_context(&self, context: &SessionContext) -> Result<(), PluginError> {
        let dir = self.storage_dir.read().await.clone();
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| PluginError::InternalError(format!("创建目录失败: {}", e)))?;
        
        let path = Self::context_path(&dir);
        let content = serde_json::to_string_pretty(context)
            .map_err(|e| PluginError::InternalError(format!("序列化失败: {}", e)))?;
        
        tokio::fs::write(&path, content)
            .await
            .map_err(|e| PluginError::InternalError(format!("写入上下文失败: {}", e)))
    }

    async fn list_sessions(&self) -> Result<Vec<Session>, PluginError> {
        let dir = self.storage_dir.read().await.clone();
        let mut sessions = Vec::new();
        
        if !dir.exists() {
            return Ok(sessions);
        }

        let mut entries = tokio::fs::read_dir(&dir)
            .await
            .map_err(|e| PluginError::InternalError(format!("读取目录失败: {}", e)))?;

        while let Some(entry) = entries.next_entry().await.map_err(|e| PluginError::InternalError(e.to_string()))? {
            let path = entry.path();
            if path.extension().map(|e| e == "json").unwrap_or(false) {
                let name = path.file_stem().unwrap().to_string_lossy();
                if name == "context" {
                    continue;
                }
                if let Ok(content) = tokio::fs::read_to_string(&path).await {
                    if let Ok(session) = serde_json::from_str::<Session>(&content) {
                        sessions.push(session);
                    }
                }
            }
        }

        sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(sessions)
    }

    async fn handle_get(&self, input: &Value) -> StreamChunk {
        let session_id = input.get("session_id").and_then(|v| v.as_str());
        match session_id {
            Some(sid) => {
                match self.get_or_create_session(sid).await {
                    Ok(session) => StreamChunk {
                        data: json!({
                            "success": true,
                            "session": session
                        }),
                        done: true,
                        error: None,
                    },
                    Err(e) => StreamChunk {
                        data: json!({}),
                        done: true,
                        error: Some(e.to_string()),
                    },
                }
            }
            None => StreamChunk {
                data: json!({}),
                done: true,
                error: Some("缺少 session_id 参数".to_string()),
            },
        }
    }

    async fn handle_append(&self, input: &Value) -> StreamChunk {
        let session_id = input.get("session_id").and_then(|v| v.as_str());
        let messages_val = input.get("messages").and_then(|v| v.as_array());
        
        match (session_id, messages_val) {
            (Some(sid), Some(msgs)) => {
                let mut session = match self.get_or_create_session(sid).await {
                    Ok(s) => s,
                    Err(e) => {
                        return StreamChunk {
                            data: json!({}),
                            done: true,
                            error: Some(e.to_string()),
                        };
                    }
                };
                
                let now = chrono::Utc::now().timestamp();
                for msg in msgs {
                    if let Some(chat_msg) = parse_message(msg, now) {
                        session.messages.push(chat_msg);
                    }
                }
                
                // 截断消息历史
                let max_msgs = self.config.read().await.max_messages;
                while session.messages.len() > max_msgs {
                    session.messages.remove(0);
                }
                
                session.updated_at = now;
                
                if let Err(e) = self.save_session(&session).await {
                    return StreamChunk {
                        data: json!({}),
                        done: true,
                        error: Some(e.to_string()),
                    };
                }
                
                StreamChunk {
                    data: json!({
                        "success": true,
                        "message_count": session.messages.len()
                    }),
                    done: true,
                    error: None,
                }
            }
            _ => StreamChunk {
                data: json!({}),
                done: true,
                error: Some("缺少 session_id 或 messages 参数".to_string()),
            },
        }
    }

    async fn handle_clear(&self, input: &Value) -> StreamChunk {
        let session_id = input.get("session_id").and_then(|v| v.as_str());
        match session_id {
            Some(sid) => {
                let dir = self.storage_dir.read().await.clone();
                let path = Self::session_path(&dir, sid);
                
                if path.exists() {
                    if let Err(e) = tokio::fs::remove_file(&path).await {
                        return StreamChunk {
                            data: json!({}),
                            done: true,
                            error: Some(format!("删除会话失败: {}", e)),
                        };
                    }
                }
                
                StreamChunk {
                    data: json!({ "success": true, "message": "会话已删除" }),
                    done: true,
                    error: None,
                }
            }
            None => StreamChunk {
                data: json!({}),
                done: true,
                error: Some("缺少 session_id 参数".to_string()),
            },
        }
    }

    async fn handle_list(&self) -> StreamChunk {
        match self.list_sessions().await {
            Ok(sessions) => StreamChunk {
                data: json!({
                    "success": true,
                    "sessions": sessions.iter().map(|s| json!({
                        "id": s.id,
                        "message_count": s.messages.len(),
                        "updated_at": s.updated_at
                    })).collect::<Vec<_>>()
                }),
                done: true,
                error: None,
            },
            Err(e) => StreamChunk {
                data: json!({}),
                done: true,
                error: Some(e.to_string()),
            },
        }
    }

    async fn handle_get_context(&self, input: &Value) -> StreamChunk {
        let session_id = input.get("session_id").and_then(|v| v.as_str()).unwrap_or("default");
        let include_history = input.get("history").and_then(|v| v.as_bool()).unwrap_or(true);
        
        let session = self.get_or_create_session(session_id).await.unwrap_or_else(|_| Session::new(session_id));
        
        // 获取工具列表
        let tools = self.fetch_tools().await;
        
        let llm_context = LlmContext {
            system_prompt: "You are a helpful AI assistant.".to_string(),
            tools,
            history: if include_history { session.messages } else { vec![] },
        };
        
        StreamChunk {
            data: json!(llm_context),
            done: true,
            error: None,
        }
    }

    /// 从 tools 插件获取工具列表
    async fn fetch_tools(&self) -> Vec<Value> {
        if let Some(parent) = self.get_parent() {
            // 调用 tools 插件的 list 工具
            match parent.invoke("tools", json!({"tool": "list"})) {
                Ok(InvokeStream::Single(chunk)) if chunk.error.is_none() => {
                    // 解析返回的工具列表
                    if let Some(tools) = chunk.data.get("tools").and_then(|t| t.as_array()) {
                        return tools.clone();
                    }
                }
                _ => {}
            }
        }
        vec![]
    }

    async fn handle_add_context(&self, input: &Value) -> StreamChunk {
        let path = input.get("path").and_then(|v| v.as_str());
        match path {
            Some(p) => {
                let mut context = self.get_context_data().await;
                
                if !context.entries.iter().any(|e| e.path == p) {
                    context.entries.push(ContextEntry {
                        path: p.to_string(),
                        entry_type: "file".to_string(),
                        added_at: chrono::Utc::now().timestamp(),
                    });
                    
                    if let Err(e) = self.save_context(&context).await {
                        return StreamChunk {
                            data: json!({}),
                            done: true,
                            error: Some(e.to_string()),
                        };
                    }
                }
                
                StreamChunk {
                    data: json!({ "success": true, "message": "上下文已添加" }),
                    done: true,
                    error: None,
                }
            }
            None => StreamChunk {
                data: json!({}),
                done: true,
                error: Some("缺少 path 参数".to_string()),
            },
        }
    }

    async fn handle_clear_context(&self) -> StreamChunk {
        let dir = self.storage_dir.read().await.clone();
        let path = Self::context_path(&dir);
        
        if path.exists() {
            if let Err(e) = tokio::fs::remove_file(&path).await {
                return StreamChunk {
                    data: json!({}),
                    done: true,
                    error: Some(format!("清除上下文失败: {}", e)),
                };
            }
        }
        
        StreamChunk {
            data: json!({ "success": true, "message": "上下文已清除" }),
            done: true,
            error: None,
        }
    }
}

impl Default for SessionPlugin {
    fn default() -> Self {
        Self::new(None, SessionConfig::default())
    }
}

#[async_trait]
impl Plugin for SessionPlugin {
    fn meta(&self, path: &str) -> PluginResult<PluginMeta> {
        if path == "config" {
            return Ok(PluginMeta {
                name: "config".to_string(),
                description: "Session 配置管理".to_string(),
                version: "0.1.0".to_string(),
                input: Some(json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["get", "set", "schema"],
                            "description": "操作类型"
                        },
                        "config": {
                            "type": "object",
                            "description": "配置数据（set 操作时使用）"
                        }
                    },
                    "required": ["action"]
                })),
                output: Some(json!({
                    "type": "object",
                    "properties": {
                        "success": { "type": "boolean" },
                        "config": { "type": "object" },
                        "schema": { "type": "object" }
                    }
                })),
                author: Some("Symbio Team".to_string()),
            });
        }
        Ok(self.meta.clone())
    }

    fn capabilities(&self) -> Vec<&'static str> {
        vec![CAPABILITY_SESSION]
    }

    fn invoke(&self, path: &str, input: Value) -> PluginResult<InvokeStream> {
        // 处理 config path
        if path == "config" {
            let action = input.get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("get");
            
            let config = Arc::clone(&self.config);
            let storage_dir = Arc::clone(&self.storage_dir);
            let parent = self.get_parent();

            let result = tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    match action {
                        "get" => {
                            let cfg = config.read().await;
                            StreamChunk {
                                data: json!({
                                    "storage_dir": cfg.storage_dir,
                                    "max_messages": cfg.max_messages,
                                    "auto_compress": cfg.auto_compress,
                                    "compress_threshold": cfg.compress_threshold
                                }),
                                done: true,
                                error: None,
                            }
                        }
                        "set" => {
                            if let Some(new_config) = input.get("config") {
                                let mut cfg = config.write().await;
                                if let Some(v) = new_config.get("storage_dir").and_then(|v| v.as_str()) {
                                    cfg.storage_dir = v.to_string();
                                    // 更新实际存储目录
                                    let mut dir = storage_dir.write().await;
                                    *dir = PathBuf::from(v);
                                }
                                if let Some(v) = new_config.get("max_messages").and_then(|v| v.as_u64()) {
                                    cfg.max_messages = v as usize;
                                }
                                if let Some(v) = new_config.get("auto_compress").and_then(|v| v.as_bool()) {
                                    cfg.auto_compress = v;
                                }
                                if let Some(v) = new_config.get("compress_threshold").and_then(|v| v.as_u64()) {
                                    cfg.compress_threshold = v as usize;
                                }
                            }
                            // 通知父插件保存配置
                            if let Some(p) = parent {
                                let _ = p.invoke("save_config", json!({}));
                            }
                            StreamChunk {
                                data: json!({ "success": true }),
                                done: true,
                                error: None,
                            }
                        }
                        "schema" => {
                            StreamChunk {
                                data: json!({
                                    "success": true,
                                    "schema": Self::config_schema()
                                }),
                                done: true,
                                error: None,
                            }
                        }
                        _ => StreamChunk {
                            data: json!({}),
                            done: true,
                            error: Some(format!("未知操作: {}", action)),
                        },
                    }
                })
            });

            return Ok(InvokeStream::Single(result));
        }

        let action = input.get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PluginError::ValidationError("缺少 action 参数".to_string()))?
            .to_string();

        // 使用 tokio runtime 执行异步操作并返回结果
        let result = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                match action.as_str() {
                    "get" => self.handle_get(&input).await,
                    "append" => self.handle_append(&input).await,
                    "clear" => self.handle_clear(&input).await,
                    "list" => self.handle_list().await,
                    "get_context" => self.handle_get_context(&input).await,
                    "add_context" => self.handle_add_context(&input).await,
                    "clear_context" => self.handle_clear_context().await,
                    _ => StreamChunk {
                        data: json!({}),
                        done: true,
                        error: Some(format!("未知操作: {}", action)),
                    },
                }
            })
        });

        Ok(InvokeStream::Single(result))
    }
}

fn parse_message(val: &Value, timestamp: i64) -> Option<ChatMessage> {
    let role = val.get("role")?.as_str()?.to_string();
    let content = val.get("content").and_then(|c| c.as_str()).unwrap_or("").to_string();
    
    Some(ChatMessage {
        role,
        content,
        timestamp,
        tool_calls: None,
        tool_call_id: None,
    })
}