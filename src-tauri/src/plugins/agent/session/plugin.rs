//! Session 插件实现
//!
//! 提供会话历史和上下文管理

use super::types::{ChatMessage, ContextEntry, Session, SessionContext, LlmContext};
use crate::core::traits::Plugin;
use crate::core::types::{PluginMeta, PluginResult, PluginError, InvokeStream, StreamChunk};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use futures::stream::StreamExt;

const DEFAULT_MAX_MESSAGES: usize = 100;

/// Session 插件
pub struct SessionPlugin {
    meta: PluginMeta,
    storage_dir: Arc<RwLock<PathBuf>>,
}

impl SessionPlugin {
    pub fn new(storage_dir: PathBuf) -> Self {
        Self {
            meta: PluginMeta {
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
            },
            storage_dir: Arc::new(RwLock::new(storage_dir)),
        }
    }

    pub fn default_dir() -> Self {
        let dir = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("symbio")
            .join("sessions");
        Self::new(dir)
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
                while session.messages.len() > DEFAULT_MAX_MESSAGES {
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
        
        let llm_context = LlmContext {
            system_prompt: "You are a helpful AI assistant.".to_string(),
            tools: vec![],
            history: if include_history { session.messages } else { vec![] },
        };
        
        StreamChunk {
            data: json!(llm_context),
            done: true,
            error: None,
        }
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
        Self::default_dir()
    }
}

#[async_trait]
impl Plugin for SessionPlugin {
    fn meta(&self, _path: &str) -> PluginResult<PluginMeta> {
        Ok(self.meta.clone())
    }

    fn invoke(&self, _path: &str, input: Value) -> PluginResult<InvokeStream> {
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