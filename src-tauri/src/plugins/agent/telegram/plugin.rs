//! Telegram 插件实现
//!
//! 提供 Telegram Bot API 集成，支持 LLM 对话

use super::types::{TelegramConfig, TelegramMessage};
use crate::core::traits::Plugin;
use crate::core::types::{PluginMeta, PluginResult, PluginError, InvokeStream, StreamChunk};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

/// Telegram 消息最大长度
const TELEGRAM_MAX_MESSAGE_LENGTH: usize = 4096;

/// RAII guard for typing indicator - ensures typing is stopped when dropped
pub struct TypingGuard {
    cancel_token: Option<CancellationToken>,
    typing_task: Option<tokio::task::JoinHandle<()>>,
}

impl TypingGuard {
    /// Create a new typing guard that continuously sends typing indicator
    pub fn new(client: reqwest::Client, api_url: String, chat_id: String) -> Self {
        let cancel_token = CancellationToken::new();
        let cancel_token_clone = cancel_token.clone();

        let typing_task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(tokio::time::Duration::from_secs(4)) => {
                        let body = json!({
                            "chat_id": chat_id,
                            "action": "typing"
                        });
                        let _ = client
                            .post(format!("{}/sendChatAction", api_url))
                            .json(&body)
                            .send()
                            .await;
                    }
                    _ = cancel_token_clone.cancelled() => {
                        break;
                    }
                }
            }
        });

        Self {
            cancel_token: Some(cancel_token),
            typing_task: Some(typing_task),
        }
    }

    /// Stop the typing indicator immediately
    pub fn stop(&mut self) {
        if let Some(token) = self.cancel_token.take() {
            token.cancel();
        }
        self.typing_task.take();
    }
}

impl Drop for TypingGuard {
    fn drop(&mut self) {
        self.stop();
    }
}

/// 解析后的消息
struct ParsedMessage {
    chat_id: String,
    user_id: i64,
    text: String,
}

/// Telegram 插件
pub struct TelegramPlugin {
    meta: PluginMeta,
    config: Arc<RwLock<TelegramConfig>>,
    client: reqwest::Client,
    /// 更新偏移量
    update_offset: AtomicI64,
    /// 监听器取消令牌
    listener_token: RwLock<Option<CancellationToken>>,
    /// 监听器运行状态
    listener_running: AtomicBool,
    /// 最新消息版本
    latest_version: Arc<RwLock<HashMap<String, u64>>>,
    /// LLM 插件引用（用于调用 chat）
    llm_plugin: Arc<RwLock<Option<Arc<dyn Plugin>>>>,
}

impl TelegramPlugin {
    fn create_meta() -> PluginMeta {
        PluginMeta {
            name: "telegram".to_string(),
            description: "Telegram Bot API 集成".to_string(),
            version: "0.1.0".to_string(),
            input: Some(json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["send", "get_updates", "configure", "get_config", "set_chat_id", "start_listener", "stop_listener", "status"]
                    },
                    "chat_id": { "type": "string" },
                    "text": { "type": "string" },
                    "parse_mode": { "type": "string" },
                    "bot_token": { "type": "string" }
                },
                "required": ["action"]
            })),
            output: None,
            author: Some("Symbio Team".to_string()),
        }
    }

    /// 主构造函数（Factory 机制使用）
    pub fn new(config: TelegramConfig) -> Self {
        Self {
            meta: Self::create_meta(),
            config: Arc::new(RwLock::new(config)),
            client: reqwest::Client::new(),
            update_offset: AtomicI64::new(0),
            listener_token: RwLock::new(None),
            listener_running: AtomicBool::new(false),
            latest_version: Arc::new(RwLock::new(HashMap::new())),
            llm_plugin: Arc::new(RwLock::new(None)),
        }
    }

    fn api_url(&self) -> Option<String> {
        let config = self.config.try_read().ok()?;
        if config.bot_token.is_empty() {
            None
        } else {
            Some(format!("https://api.telegram.org/bot{}", config.bot_token))
        }
    }

    fn get_chat_id(&self) -> Option<String> {
        let config = self.config.try_read().ok()?;
        config.chat_id.clone()
    }

    /// 检查用户是否被允许
    fn is_user_allowed(&self, user_id: i64) -> bool {
        let config = self.config.try_read();
        match config {
            Ok(c) => c.allowed_users.is_empty() || c.allowed_users.contains(&user_id),
            Err(_) => true,
        }
    }

    /// 获取新消息版本
    fn new_message_version(&self, chat_id: &str) -> u64 {
        if let Ok(mut versions) = self.latest_version.try_write() {
            let version = versions.get(chat_id).copied().unwrap_or(0) + 1;
            versions.insert(chat_id.to_string(), version);
            version
        } else {
            1 // 如果获取锁失败，返回默认版本
        }
    }

    /// 检查是否是最新消息
    fn is_latest_message(&self, chat_id: &str, version: u64) -> bool {
        if let Ok(versions) = self.latest_version.try_read() {
            versions.get(chat_id).copied() == Some(version)
        } else {
            true // 如果获取锁失败，假设是最新消息
        }
    }

    /// 找到最佳分割点
    fn find_best_split_point(text: &str, max_len: usize) -> usize {
        if text.len() <= max_len {
            return text.len();
        }
        let mut end = max_len;
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        let search_region = &text[..end];
        if let Some(pos) = search_region.rfind('\n') {
            return pos + 1;
        }
        if let Some(pos) = search_region.rfind(' ') {
            return pos;
        }
        end
    }

    /// 发送消息到 Telegram
    async fn send_message(&self, chat_id: &str, text: &str, parse_mode: Option<&str>) -> Result<i32, String> {
        let api_url = self.api_url().ok_or("未配置 bot_token")?;
        
        let mut remaining = text;
        let mut sent = 0;

        while !remaining.is_empty() {
            let split_point = Self::find_best_split_point(remaining, TELEGRAM_MAX_MESSAGE_LENGTH);
            let chunk = &remaining[..split_point];
            remaining = &remaining[split_point..];

            let msg = TelegramMessage {
                chat_id: chat_id.to_string(),
                text: chunk.to_string(),
                parse_mode: parse_mode.map(|s| s.to_string()),
                reply_to_message_id: None,
            };

            let resp = self.client
                .post(format!("{}/sendMessage", api_url))
                .json(&msg)
                .send()
                .await
                .map_err(|e| format!("发送失败: {}", e))?;

            if !resp.status().is_success() {
                return Err(format!("Telegram API 错误: {}", resp.status()));
            }
            sent += 1;
        }

        Ok(sent)
    }

    /// 处理发送消息
    async fn handle_send(&self, input: &Value) -> StreamChunk {
        let text = match input.get("text").and_then(|v| v.as_str()) {
            Some(t) => t,
            None => {
                return StreamChunk {
                    data: json!({}),
                    done: true,
                    error: Some("缺少 text 参数".to_string()),
                };
            }
        };

        let chat_id = input
            .get("chat_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| self.get_chat_id());

        let chat_id = match chat_id {
            Some(id) => id,
            None => {
                return StreamChunk {
                    data: json!({}),
                    done: true,
                    error: Some("未配置 chat_id".to_string()),
                };
            }
        };

        let parse_mode = input.get("parse_mode").and_then(|v| v.as_str());

        match self.send_message(&chat_id, text, parse_mode).await {
            Ok(sent) => StreamChunk {
                data: json!({
                    "success": true,
                    "sent": sent,
                    "message": format!("已发送 {} 条消息", sent)
                }),
                done: true,
                error: None,
            },
            Err(e) => StreamChunk {
                data: json!({}),
                done: true,
                error: Some(e),
            },
        }
    }

    /// 获取更新
    async fn handle_get_updates(&self) -> StreamChunk {
        let api_url = match self.api_url() {
            Some(url) => url,
            None => {
                return StreamChunk {
                    data: json!({}),
                    done: true,
                    error: Some("未配置 bot_token".to_string()),
                };
            }
        };

        let offset = self.update_offset.load(Ordering::SeqCst);
        
        let body = json!({
            "offset": offset,
            "timeout": 0,
            "allowed_updates": ["message"]
        });

        let resp = match self.client
            .post(format!("{}/getUpdates", api_url))
            .json(&body)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                return StreamChunk {
                    data: json!({}),
                    done: true,
                    error: Some(format!("请求失败: {}", e)),
                };
            }
        };

        if resp.status().is_success() {
            let updates: Value = match resp.json().await {
                Ok(u) => u,
                Err(e) => {
                    return StreamChunk {
                        data: json!({}),
                        done: true,
                        error: Some(format!("解析响应失败: {}", e)),
                    };
                }
            };

            // 处理更新
            if let Some(result) = updates.get("result").and_then(|v| v.as_array()) {
                for update in result {
                    // 更新 offset
                    if let Some(uid) = update.get("update_id").and_then(|v| v.as_i64()) {
                        self.update_offset.store(uid + 1, Ordering::SeqCst);
                    }
                    
                    // 自动保存 chat_id
                    if let Some(chat_id) = update
                        .get("message")
                        .and_then(|m| m.get("chat"))
                        .and_then(|c| c.get("id"))
                        .and_then(|id| id.as_i64())
                    {
                        if self.get_chat_id().is_none() {
                            if let Ok(mut config) = self.config.try_write() {
                                config.chat_id = Some(chat_id.to_string());
                            }
                        }
                    }
                }
            }

            StreamChunk {
                data: json!({
                    "success": true,
                    "updates": updates.get("result")
                }),
                done: true,
                error: None,
            }
        } else {
            StreamChunk {
                data: json!({}),
                done: true,
                error: Some(format!("Telegram API 错误: {}", resp.status())),
            }
        }
    }

    /// 配置
    async fn handle_configure(&self, input: &Value) -> StreamChunk {
        if let Ok(mut config) = self.config.try_write() {
            if let Some(token) = input.get("bot_token").and_then(|v| v.as_str()) {
                config.bot_token = token.to_string();
            }
            if let Some(id) = input.get("chat_id").and_then(|v| v.as_str()) {
                config.chat_id = Some(id.to_string());
            }
            if let Some(users) = input.get("allowed_users").and_then(|v| v.as_array()) {
                config.allowed_users = users.iter()
                    .filter_map(|v| v.as_i64())
                    .collect();
            }
            if let Some(streaming) = input.get("streaming_enabled").and_then(|v| v.as_bool()) {
                config.streaming_enabled = streaming;
            }
        }

        StreamChunk {
            data: json!({
                "success": true,
                "message": "配置已更新"
            }),
            done: true,
            error: None,
        }
    }

    /// 获取配置
    async fn handle_get_config(&self) -> StreamChunk {
        let config = self.config.try_read();
        match config {
            Ok(c) => StreamChunk {
                data: json!({
                    "success": true,
                    "configured": !c.bot_token.is_empty(),
                    "has_chat_id": c.chat_id.is_some(),
                    "streaming_enabled": c.streaming_enabled,
                    "allowed_users_count": c.allowed_users.len()
                }),
                done: true,
                error: None,
            },
            Err(_) => StreamChunk {
                data: json!({}),
                done: true,
                error: Some("读取配置失败".to_string()),
            },
        }
    }

    /// 设置 chat_id
    async fn handle_set_chat_id(&self, input: &Value) -> StreamChunk {
        let chat_id = match input.get("chat_id").and_then(|v| v.as_str()) {
            Some(id) => id,
            None => {
                return StreamChunk {
                    data: json!({}),
                    done: true,
                    error: Some("缺少 chat_id 参数".to_string()),
                };
            }
        };

        if let Ok(mut config) = self.config.try_write() {
            config.chat_id = Some(chat_id.to_string());
        }

        StreamChunk {
            data: json!({
                "success": true,
                "message": "chat_id 已设置"
            }),
            done: true,
            error: None,
        }
    }

    /// 启动监听器
    async fn handle_start_listener(&self, llm_plugin: Option<Arc<dyn Plugin>>) -> StreamChunk {
        // 设置 LLM 插件引用
        if let Some(ref plugin) = llm_plugin {
            if let Ok(mut p) = self.llm_plugin.try_write() {
                *p = Some(plugin.clone());
            }
        }

        // 检查是否已运行
        if self.listener_running.load(Ordering::SeqCst) {
            return StreamChunk {
                data: json!({
                    "success": true,
                    "message": "监听器已在运行"
                }),
                done: true,
                error: None,
            };
        }

        let api_url = match self.api_url() {
            Some(url) => url,
            None => {
                return StreamChunk {
                    data: json!({}),
                    done: true,
                    error: Some("未配置 bot_token".to_string()),
                };
            }
        };

        let cancel_token = CancellationToken::new();
        if let Ok(mut token) = self.listener_token.try_write() {
            *token = Some(cancel_token.clone());
        }
        self.listener_running.store(true, Ordering::SeqCst);

        let config = Arc::clone(&self.config);
        let client = self.client.clone();
        let update_offset = Arc::new(AtomicI64::new(self.update_offset.load(Ordering::SeqCst)));
        let listener_running = Arc::new(AtomicBool::new(true));
        let latest_version = Arc::clone(&self.latest_version);
        let llm_plugin_arc = Arc::clone(&self.llm_plugin);

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = cancel_token.cancelled() => {
                        listener_running.store(false, Ordering::SeqCst);
                        break;
                    }
                    _ = tokio::time::sleep(tokio::time::Duration::from_secs(1)) => {
                        // 检查是否启用轮询
                        let poll_enabled = config.try_read()
                            .map(|c| c.poll_enabled)
                            .unwrap_or(true);
                        
                        if !poll_enabled {
                            continue;
                        }

                        // 获取更新
                        let offset = update_offset.load(Ordering::SeqCst);
                        let body = json!({
                            "offset": offset,
                            "timeout": 30,
                            "allowed_updates": ["message"]
                        });

                        let resp = client
                            .post(format!("{}/getUpdates", api_url))
                            .json(&body)
                            .send()
                            .await;

                        if let Ok(resp) = resp {
                            if resp.status().is_success() {
                                if let Ok(data) = resp.json::<Value>().await {
                                    if let Some(results) = data.get("result").and_then(|v| v.as_array()) {
                                        for update in results {
                                            // 处理每条消息
                                            if let Err(e) = Self::process_update(
                                                &client,
                                                &api_url,
                                                update,
                                                &config,
                                                &update_offset,
                                                &latest_version,
                                                &llm_plugin_arc,
                                            ).await {
                                                tracing::error!("[telegram] 处理消息错误: {}", e);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });

        StreamChunk {
            data: json!({
                "success": true,
                "message": "监听器已启动"
            }),
            done: true,
            error: None,
        }
    }

    /// 停止监听器
    async fn handle_stop_listener(&self) -> StreamChunk {
        if let Ok(mut token) = self.listener_token.try_write() {
            if let Some(t) = token.take() {
                t.cancel();
            }
        }
        self.listener_running.store(false, Ordering::SeqCst);

        StreamChunk {
            data: json!({
                "success": true,
                "message": "监听器已停止"
            }),
            done: true,
            error: None,
        }
    }

    /// 状态
    async fn handle_status(&self) -> StreamChunk {
        let config = self.config.try_read();
        let (has_token, has_chat, streaming, poll) = match config {
            Ok(c) => (!c.bot_token.is_empty(), c.chat_id.is_some(), c.streaming_enabled, c.poll_enabled),
            Err(_) => (false, false, true, true),
        };

        StreamChunk {
            data: json!({
                "success": true,
                "configured": has_token,
                "has_chat_id": has_chat,
                "streaming_enabled": streaming,
                "poll_enabled": poll,
                "listener_running": self.listener_running.load(Ordering::SeqCst),
                "update_offset": self.update_offset.load(Ordering::SeqCst)
            }),
            done: true,
            error: None,
        }
    }

    /// 处理单个更新
    async fn process_update(
        client: &reqwest::Client,
        api_url: &str,
        update: &Value,
        config: &Arc<RwLock<TelegramConfig>>,
        update_offset: &Arc<AtomicI64>,
        latest_version: &Arc<RwLock<HashMap<String, u64>>>,
        llm_plugin: &Arc<RwLock<Option<Arc<dyn Plugin>>>>,
    ) -> Result<(), String> {
        // 解析消息
        let message = update.get("message").ok_or("无消息")?;
        let chat_id = message.get("chat")
            .and_then(|c| c.get("id"))
            .and_then(|id| id.as_i64())
            .ok_or("无 chat_id")?
            .to_string();
        let user_id = message.get("from")
            .and_then(|f| f.get("id"))
            .and_then(|id| id.as_i64())
            .ok_or("无 user_id")?;
        let text = message.get("text")
            .and_then(|t| t.as_str())
            .unwrap_or("");

        // 更新 offset
        if let Some(uid) = update.get("update_id").and_then(|v| v.as_i64()) {
            update_offset.store(uid + 1, Ordering::SeqCst);
        }

        // 检查用户权限
        {
            let cfg = config.try_read().map_err(|e| e.to_string())?;
            if !cfg.allowed_users.is_empty() && !cfg.allowed_users.contains(&user_id) {
                return Ok(()); // 忽略未授权用户
            }
        }

        // 创建消息版本
        let version = {
            let mut versions = latest_version.try_write().map_err(|e| e.to_string())?;
            let v = versions.get(&chat_id).copied().unwrap_or(0) + 1;
            versions.insert(chat_id.clone(), v);
            v
        };

        tracing::info!("[telegram] 消息 v{} 来自 {}: {}", version, user_id, text.chars().take(50).collect::<String>());

        // 启动 typing 指示器
        let typing_guard = TypingGuard::new(client.clone(), api_url.to_string(), chat_id.clone());

        // 获取 LLM 响应
        let response = {
            let llm = llm_plugin.try_read().map_err(|e| e.to_string())?;
            if let Some(ref plugin) = *llm {
                let input = json!({
                    "action": "chat",
                    "message": text
                });
                match plugin.invoke("", input) {
                    Ok(stream) => {
                        // 收集响应
                        let chunks = stream.collect().await;
                        if let Some(chunk) = chunks.first() {
                            if chunk.error.is_none() {
                                chunk.data.get("content")
                                    .and_then(|c| c.as_str())
                                    .map(|s| s.to_string())
                                    .unwrap_or_default()
                            } else {
                                chunk.error.clone().unwrap_or_else(|| "LLM 错误".to_string())
                            }
                        } else {
                            "无响应".to_string()
                        }
                    }
                    Err(e) => format!("LLM 调用失败: {}", e),
                }
            } else {
                "LLM 插件未配置".to_string()
            }
        };

        // 检查消息是否被中断
        {
            let versions = latest_version.try_read().map_err(|e| e.to_string())?;
            if versions.get(&chat_id).copied() != Some(version) {
                tracing::info!("[telegram] 消息 v{} 被中断", version);
                return Ok(());
            }
        }

        // 发送响应
        drop(typing_guard); // 停止 typing

        if !response.is_empty() {
            let mut remaining = response.as_str();
            while !remaining.is_empty() {
                let split_point = Self::find_best_split_point(remaining, TELEGRAM_MAX_MESSAGE_LENGTH);
                let chunk = &remaining[..split_point];
                remaining = &remaining[split_point..];

                let msg = TelegramMessage {
                    chat_id: chat_id.clone(),
                    text: chunk.to_string(),
                    parse_mode: None,
                    reply_to_message_id: None,
                };

                let _ = client
                    .post(format!("{}/sendMessage", api_url))
                    .json(&msg)
                    .send()
                    .await;
            }
        }

        Ok(())
    }
}

impl Default for TelegramPlugin {
    fn default() -> Self {
        Self::new(TelegramConfig::default())
    }
}

#[async_trait]
impl Plugin for TelegramPlugin {
    fn meta(&self, _path: &str) -> PluginResult<PluginMeta> {
        Ok(self.meta.clone())
    }

    fn invoke(&self, _path: &str, input: Value) -> PluginResult<InvokeStream> {
        let action = input.get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PluginError::ValidationError("缺少 action 参数".to_string()))?
            .to_string();

        let result = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                match action.as_str() {
                    "send" => self.handle_send(&input).await,
                    "get_updates" => self.handle_get_updates().await,
                    "configure" => self.handle_configure(&input).await,
                    "get_config" => self.handle_get_config().await,
                    "set_chat_id" => self.handle_set_chat_id(&input).await,
                    "start_listener" => {
                        // 尝试获取 LLM 插件引用（从 input 中）
                        let _llm_plugin = input.get("llm_plugin").and_then(|v| v.as_bool()).unwrap_or(false);
                        self.handle_start_listener(None).await
                    }
                    "stop_listener" => self.handle_stop_listener().await,
                    "status" => self.handle_status().await,
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