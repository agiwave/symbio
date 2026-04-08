//! Session 插件实现
//!
//! 提供会话历史和上下文管理
//!
//! 存储路径: <workspace>/.symbio/agent/session/

use super::types::{ChatMessage, ContextEntry, Session, SessionContext, LlmContext};
use crate::symbio_core::traits::{Plugin};
use crate::symbio_core::types::{PluginMeta, PluginResult, PluginError, InvokeStream, StreamChunk};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::{Arc, Weak};
use tokio::sync::RwLock;

/// Session 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionConfig {
    /// 存储目录 (~ 表示当前工作区)
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

/// 默认存储路径: ~ 表示工作区，实际路径为 <workspace>/.symbio/agent/session
fn default_storage_dir() -> String { 
    "~/.symbio/agent/session".to_string()
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
    /// 父插件引用（用于获取工作区路径）
    parent: Option<Weak<dyn Plugin>>,
    /// 缓存的工作区路径
    cached_workspace: Arc<RwLock<Option<String>>>,
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
            cached_workspace: Arc::new(RwLock::new(None)),
        }
    }

    /// 获取父插件引用
    fn get_parent(&self) -> Option<Arc<dyn Plugin>> {
        self.parent.as_ref().and_then(|w| w.upgrade())
    }

    /// 获取工作区路径（通过父插件调用 work 插件）
    async fn get_workspace_path(&self) -> Result<PathBuf, PluginError> {
        // 尝试从缓存获取
        {
            let cached = self.cached_workspace.read().await;
            if let Some(ref path) = *cached {
                return Ok(PathBuf::from(path));
            }
        }

        // 通过父插件调用 /work/workspace_path 获取工作区路径
        let workspace_path = if let Some(parent) = self.get_parent() {
            match parent.invoke("/work/workspace_path", json!({})) {
                Ok(InvokeStream::Single(chunk)) if chunk.error.is_none() => {
                    chunk.data.get("expanded_path")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                }
                Ok(InvokeStream::Single(chunk)) => {
                    eprintln!("[session] /work/workspace_path error: {:?}", chunk.error);
                    None
                }
                Err(e) => {
                    eprintln!("[session] /work/workspace_path failed: {:?}", e);
                    None
                }
                _ => None,
            }
        } else {
            eprintln!("[session] no parent plugin available");
            None
        };

        // 如果获取到工作区路径，缓存并返回
        if let Some(path) = workspace_path {
            let mut cached = self.cached_workspace.write().await;
            *cached = Some(path.clone());
            return Ok(PathBuf::from(path));
        }

        // 获取失败，返回错误
        Err(PluginError::InternalError("无法获取工作区路径".to_string()))
    }

    /// 解析存储路径，将 ~ 替换为工作区路径
    async fn resolve_storage_dir(&self) -> Result<PathBuf, PluginError> {
        let cfg = self.config.read().await;
        let storage_dir = &cfg.storage_dir;
        
        if storage_dir.starts_with('~') {
            let workspace = self.get_workspace_path().await?;
            let relative = storage_dir.strip_prefix('~').unwrap_or(storage_dir);
            Ok(workspace.join(relative.trim_start_matches('/')))
        } else {
            // 展开标准的 ~ 路径（用户主目录）
            Ok(PathBuf::from(shellexpand::tilde(storage_dir).to_string()))
        }
    }

    /// 更新存储目录
    async fn update_storage_dir(&self) -> Result<(), PluginError> {
        let resolved = self.resolve_storage_dir().await?;
        let mut dir = self.storage_dir.write().await;
        *dir = resolved;
        Ok(())
    }

    /// 获取配置 Schema
    fn config_schema() -> Value {
        json!({
            "storage_dir": {
                "type": "string",
                "title": "存储目录",
                "description": "会话数据存储目录（~ 表示当前工作区，实际路径: <workspace>/.symbio/agent/session）",
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

    async fn handle_get_messages(&self, input: &Value) -> StreamChunk {
        let session_id = input.get("session_id").and_then(|v| v.as_str());
        let limit = input.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;
        let before = input.get("before").and_then(|v| v.as_i64()); // 获取此时间戳之前的消息

        match session_id {
            Some(sid) => {
                match self.get_or_create_session(sid).await {
                    Ok(session) => {
                        // 按时间戳排序（从旧到新）
                        let mut messages: Vec<_> = session.messages.iter().cloned().collect();
                        messages.sort_by_key(|m| m.timestamp);

                        // 如果指定了 before，只获取该时间戳之前的消息
                        let filtered: Vec<_> = if let Some(before_ts) = before {
                            messages.into_iter().filter(|m| m.timestamp < before_ts).collect()
                        } else {
                            messages
                        };

                        // 获取最后 limit 条消息
                        let start = filtered.len().saturating_sub(limit);
                        let recent_messages: Vec<_> = filtered.into_iter().skip(start).collect();

                        StreamChunk {
                            data: json!({
                                "success": true,
                                "messages": recent_messages,
                                "has_more": start > 0,
                                "total": session.messages.len()
                            }),
                            done: true,
                            error: None,
                        }
                    }
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

        // 构建系统提示词
        let system_prompt = self.build_system_prompt().await;

        let llm_context = LlmContext {
            system_prompt,
            tools,
            history: if include_history { session.messages } else { vec![] },
        };

        StreamChunk {
            data: json!(llm_context),
            done: true,
            error: None,
        }
    }

    /// 构建系统提示词（从文件加载）
    /// 
    /// 规则：
    /// 1. 如果存在 ~/.symbio/README.ai.md 或 <workspace>/.symbio/README.ai.md，优先工作区的
    /// 2. 如果存在 <workspace>/README.ai.md，则也包括这个文件
    /// 3. 将上述文件拼接作为系统提示词
    async fn build_system_prompt(&self) -> String {
        let mut parts = Vec::new();

        // 获取工作区路径
        let workspace = match self.get_workspace_path().await {
            Ok(p) => p,
            Err(e) => {
                eprintln!("[session] Failed to get workspace path: {}", e);
                return "You are a helpful AI assistant.".to_string();
            }
        };

        // 1. 加载 .symbio/README.ai.md（优先工作区的，其次全局的）
        let workspace_symbio_readme = workspace.join(".symbio/README.ai.md");
        let home_dir = dirs::home_dir().unwrap_or_default();
        let global_symbio_readme = home_dir.join(".symbio/README.ai.md");

        let symbio_readme_content = if workspace_symbio_readme.exists() {
            tokio::fs::read_to_string(&workspace_symbio_readme).await.ok()
        } else if global_symbio_readme.exists() {
            tokio::fs::read_to_string(&global_symbio_readme).await.ok()
        } else {
            None
        };

        if let Some(content) = symbio_readme_content {
            if !content.trim().is_empty() {
                parts.push(content);
            }
        }

        // 2. 加载工作区根目录的 README.ai.md
        let workspace_readme = workspace.join("README.ai.md");
        if workspace_readme.exists() {
            if let Ok(content) = tokio::fs::read_to_string(&workspace_readme).await {
                if !content.trim().is_empty() {
                    parts.push(content);
                }
            }
        }

        // 3. 拼接所有部分
        if parts.is_empty() {
            "You are a helpful AI assistant.".to_string()
        } else {
            parts.join("\n\n---\n\n")
        }
    }

    /// 从 agent 插件获取所有可用工具列表（包括 tools、memory 等所有子插件）
    async fn fetch_tools(&self) -> Vec<Value> {
        if let Some(parent) = self.get_parent() {
            // 调用父插件（agent）的 available_tools 路径获取所有工具
            match parent.invoke("available_tools", json!({})) {
                Ok(InvokeStream::Single(chunk)) if chunk.error.is_none() => {
                    // 解析返回的工具列表
                    if let Some(tools) = chunk.data.get("tools").and_then(|t| t.as_array()) {
                        eprintln!("[session] fetched {} tools from parent available_tools", tools.len());
                        // 转换为 OpenAI 格式
                        // OpenAI 要求工具名称匹配 ^[a-zA-Z0-9_-]+$，所以将 / 替换为 __
                        return tools.iter().map(|tool| {
                            let raw_name = tool.get("name").and_then(|n| n.as_str()).unwrap_or("");
                            let safe_name = raw_name.replace('/', "__");
                            json!({
                                "type": "function",
                                "function": {
                                    "name": safe_name,
                                    "description": tool.get("description").and_then(|d| d.as_str()).unwrap_or(""),
                                    "parameters": tool.get("input_schema").cloned().unwrap_or(json!({}))
                                }
                            })
                        }).collect();
                    }
                }
                Ok(InvokeStream::Single(chunk)) => {
                    eprintln!("[session] available_tools error: {:?}", chunk.error);
                }
                Err(e) => {
                    eprintln!("[session] failed to fetch tools: {:?}", e);
                }
                _ => {}
            }
        } else {
            eprintln!("[session] no parent for fetching tools");
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

    async fn handle_update(&self, input: &Value) -> StreamChunk {
        let session_id = input.get("session_id").and_then(|v| v.as_str());
        let metadata = input.get("metadata");
        
        match session_id {
            Some(sid) => {
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
                
                // 更新 metadata
                if let Some(meta) = metadata {
                    if let Some(obj) = meta.as_object() {
                        if let Some(meta_obj) = session.metadata.as_object_mut() {
                            for (k, v) in obj {
                                meta_obj.insert(k.clone(), v.clone());
                            }
                        }
                    }
                }
                
                session.updated_at = chrono::Utc::now().timestamp();
                
                if let Err(e) = self.save_session(&session).await {
                    return StreamChunk {
                        data: json!({}),
                        done: true,
                        error: Some(e.to_string()),
                    };
                }
                
                StreamChunk {
                    data: json!({ "success": true }),
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
}

impl Default for SessionPlugin {
    fn default() -> Self {
        Self::new(None, SessionConfig::default())
    }
}

impl Clone for SessionPlugin {
    fn clone(&self) -> Self {
        Self {
            meta: self.meta.clone(),
            config: Arc::clone(&self.config),
            storage_dir: Arc::clone(&self.storage_dir),
            parent: self.parent.clone(),
            cached_workspace: Arc::clone(&self.cached_workspace),
        }
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
        vec![]
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
        let self_ref = Arc::new(self.clone());
        let result = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                // 先更新存储目录（解析工作区路径）
                if let Err(e) = self_ref.update_storage_dir().await {
                    eprintln!("[session] update_storage_dir error: {:?}", e);
                    return StreamChunk {
                        data: json!({}),
                        done: true,
                        error: Some(format!("无法获取工作区路径: {}", e)),
                    };
                }
                
                match action.as_str() {
                    "get" => self_ref.handle_get(&input).await,
                    "get_messages" => self_ref.handle_get_messages(&input).await,
                    "append" => self_ref.handle_append(&input).await,
                    "clear" | "delete" => self_ref.handle_clear(&input).await,
                    "list" => self_ref.handle_list().await,
                    "get_context" => self_ref.handle_get_context(&input).await,
                    "add_context" => self_ref.handle_add_context(&input).await,
                    "clear_context" => self_ref.handle_clear_context().await,
                    "update" => self_ref.handle_update(&input).await,
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
    let tool_call_id = val.get("tool_call_id").and_then(|v| v.as_str()).map(|s| s.to_string());
    let tool_calls = val.get("tool_calls")
        .and_then(|tc| serde_json::from_value(tc.clone()).ok());
    
    Some(ChatMessage {
        role,
        content,
        timestamp,
        tool_calls,
        tool_call_id,
    })
}