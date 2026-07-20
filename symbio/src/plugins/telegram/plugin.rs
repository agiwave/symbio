use super::types::{TelegramConfig, TelegramMessage};
use super::typing::TypingGuard;
use crate::symbio_core::InvokeRequestExt;
use crate::symbio_core::{
    schemas::{
        common,
        session::{session_chat, session_chat_response},
        telegram::{telegram_send, telegram_status},
    },
    CapabilityMeta, InvokeRequest, InvokeResponse, Plugin, PluginError, PluginFrame, PluginMeta,
    PluginPayload, CONFIG_GET, CONFIG_SET, PLUGIN_TELEGRAM, SESSION_CHAT,
};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Weak};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

/// Telegram 消息最大长度
const TELEGRAM_MAX_MESSAGE_LENGTH: usize = 4096;

/// Telegram 插件
#[derive(Clone)]
pub struct TelegramPlugin {
    config: Arc<RwLock<TelegramConfig>>,
    client: reqwest::Client,
    /// 更新偏移量
    update_offset: Arc<AtomicI64>,
    /// 监听器取消令牌
    listener_token: Arc<RwLock<Option<CancellationToken>>>,
    /// 监听器运行状态
    listener_running: Arc<AtomicBool>,
    /// 最新消息版本
    latest_version: Arc<RwLock<HashMap<String, u64>>>,
    /// LLM 插件引用（用于调用 chat）
    llm_plugin: Arc<RwLock<Option<Arc<dyn Plugin>>>>,
    /// 父插件引用
    parent: Arc<RwLock<Option<Weak<dyn Plugin>>>>,
}

impl TelegramPlugin {
    /// 静态工厂：从 InvokeRequest 构造 Plugin 实例
    pub fn build(ctx: Arc<dyn InvokeRequest>) -> Arc<dyn Plugin> {
        let config: TelegramConfig = ctx
            .config()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();

        let parent = ctx.parent();

        Arc::new(TelegramPlugin::new(parent, config)) as Arc<dyn Plugin>
    }

    /// 主构造函数（Factory 机制使用）
    pub fn new(parent: Option<Weak<dyn Plugin>>, config: TelegramConfig) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
            client: reqwest::Client::new(),
            update_offset: Arc::new(AtomicI64::new(0)),
            listener_token: Arc::new(RwLock::new(None)),
            listener_running: Arc::new(AtomicBool::new(false)),
            latest_version: Arc::new(RwLock::new(HashMap::new())),
            llm_plugin: Arc::new(RwLock::new(None)),
            parent: Arc::new(RwLock::new(parent)),
        }
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new("telegram", "Telegram 集成")
            .with_description("提供与 Telegram Bot 的连接和消息推送功能")
            .with_version("0.1.0")
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
    async fn send_message(
        &self,
        chat_id: &str,
        text: &str,
        parse_mode: Option<&str>,
    ) -> Result<i32, String> {
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

            let resp = self
                .client
                .post(format!("{api_url}/sendMessage"))
                .json(&msg)
                .send()
                .await
                .map_err(|e| format!("发送失败: {e}"))?;

            if !resp.status().is_success() {
                return Err(format!("Telegram API 错误: {}", resp.status()));
            }
            sent += 1;
        }

        Ok(sent)
    }

    /// 处理发送消息
    async fn handle_send(&self, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<Value> {
        let req: telegram_send::Request = ctx.payload()?;

        let chat_id = req
            .chat_id
            .or_else(|| self.get_chat_id())
            .ok_or_else(|| PluginError::ValidationError("未配置 chat_id".to_string()))?;

        match self
            .send_message(&chat_id, &req.text, req.parse_mode.as_deref())
            .await
        {
            Ok(sent) => Ok(serde_json::to_value(telegram_send::Response {
                sent,
                message: format!("已发送 {sent} 条消息"),
            })
            .unwrap_or_default()),
            Err(e) => Err(PluginError::InternalError(e)),
        }
    }

    /// 获取更新
    async fn handle_get_updates(&self) -> InvokeResponse<Value> {
        let api_url = self
            .api_url()
            .ok_or_else(|| PluginError::ValidationError("未配置 bot_token".to_string()))?;

        let offset = self.update_offset.load(Ordering::SeqCst);

        let resp = self
            .client
            .post(format!("{api_url}/getUpdates"))
            .json(&serde_json::json!({
                "offset": offset,
                "timeout": 0,
                "allowed_updates": ["message"]
            }))
            .send()
            .await
            .map_err(|e| PluginError::InternalError(format!("请求失败: {e}")))?;

        if resp.status().is_success() {
            let updates: Value = resp
                .json()
                .await
                .map_err(|e| PluginError::InternalError(format!("解析响应失败: {e}")))?;

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

            Ok(json!({
                "success": true,
                "updates": updates.get("result")
            }))
        } else {
            Err(PluginError::InternalError(format!(
                "Telegram API 错误: {}",
                resp.status()
            )))
        }
    }

    /// 启动监听器
    async fn handle_start_listener(
        &self,
        llm_plugin: Option<Arc<dyn Plugin>>,
        ctx: Arc<dyn InvokeRequest>,
    ) -> InvokeResponse<Value> {
        // 设置 LLM 插件引用
        if let Some(ref plugin) = llm_plugin {
            if let Ok(mut p) = self.llm_plugin.try_write() {
                *p = Some(plugin.clone());
            }
        }

        // 检查是否已运行
        if self.listener_running.load(Ordering::SeqCst) {
            return Ok(json!({
                "success": true,
                "message": "监听器已在运行"
            }));
        }

        let api_url = self
            .api_url()
            .ok_or_else(|| PluginError::ValidationError("未配置 bot_token".to_string()))?;

        let cancel_token = CancellationToken::new();
        if let Ok(mut token) = self.listener_token.try_write() {
            *token = Some(cancel_token.clone());
        }
        self.listener_running.store(true, Ordering::SeqCst);

        let config = Arc::clone(&self.config);
        let client = self.client.clone();
        let update_offset = Arc::clone(&self.update_offset);
        let latest_version = Arc::clone(&self.latest_version);
        let llm_plugin_arc = Arc::clone(&self.llm_plugin);
        let base_ctx = ctx.fork();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = cancel_token.cancelled() => {
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
                        let resp = client
                            .post(format!("{api_url}/getUpdates"))
                            .json(&serde_json::json!({
                                "offset": offset,
                                "timeout": 30,
                                "allowed_updates": ["message"]
                            }))
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
                                                base_ctx.clone(),
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

        Ok(serde_json::json!({
            "success": true,
            "message": "监听器已启动"
        }))
    }

    /// 停止监听器
    async fn handle_stop_listener(&self) -> InvokeResponse<Value> {
        if let Ok(mut token) = self.listener_token.try_write() {
            if let Some(t) = token.take() {
                t.cancel();
            }
        }
        self.listener_running.store(false, Ordering::SeqCst);

        Ok(serde_json::json!({
            "success": true,
            "message": "监听器已停止"
        }))
    }

    /// 状态
    async fn handle_status(&self) -> InvokeResponse<Value> {
        let config = self.config.try_read();
        let (has_token, has_chat, streaming, poll) = match config {
            Ok(c) => (
                !c.bot_token.is_empty(),
                c.chat_id.is_some(),
                c.streaming_enabled,
                c.poll_enabled,
            ),
            Err(_) => (false, false, true, true),
        };

        Ok(serde_json::to_value(telegram_status::Response {
            configured: has_token,
            has_chat_id: has_chat,
            streaming_enabled: streaming,
            poll_enabled: poll,
            listener_running: self.listener_running.load(Ordering::SeqCst),
            update_offset: self.update_offset.load(Ordering::SeqCst),
        })
        .unwrap_or_default())
    }

    /// 处理单个更新
    #[allow(clippy::too_many_arguments)]
    async fn process_update(
        client: &reqwest::Client,
        api_url: &str,
        update: &Value,
        config: &Arc<RwLock<TelegramConfig>>,
        update_offset: &Arc<AtomicI64>,
        latest_version: &Arc<RwLock<HashMap<String, u64>>>,
        llm_plugin: &Arc<RwLock<Option<Arc<dyn Plugin>>>>,
        ctx: Arc<dyn InvokeRequest>,
    ) -> Result<(), String> {
        // 解析消息
        let message = update.get("message").ok_or("无消息")?;
        let chat_id = message
            .get("chat")
            .and_then(|c| c.get("id"))
            .and_then(|id| id.as_i64())
            .ok_or("无 chat_id")?
            .to_string();
        let user_id = message
            .get("from")
            .and_then(|f| f.get("id"))
            .and_then(|id| id.as_i64())
            .ok_or("无 user_id")?;
        let text = message.get("text").and_then(|t| t.as_str()).unwrap_or("");

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

        tracing::info!(
            "[telegram] 消息 v{} 来自 {}: {}",
            version,
            user_id,
            text.chars().take(50).collect::<String>()
        );

        // 启动 typing 指示器
        let typing_guard = TypingGuard::new(client.clone(), api_url.to_string(), chat_id.clone());

        // 获取 LLM 响应
        let response = {
            let llm = llm_plugin.try_read().map_err(|e| e.to_string())?;
            if let Some(ref plugin) = *llm {
                let chat_input = serde_json::to_value(session_chat::Request {
                    session_id: Some(chat_id.clone()),
                    agent_id: None,
                    message: Some(crate::symbio_core::schemas::session::chat_message::ChatMessage {
                        role: Some(crate::symbio_core::schemas::session::chat_message::MessageRole::User),
                        content: Some(
                            crate::symbio_core::schemas::session::chat_message::MessageContent::Text(
                                text.to_string(),
                            ),
                        ),
                        ..Default::default()
                    }),
                    provider_id: None,
                    include_history: None,
                    // Telegram 接入是无人值守通道，统一走 auto 模式
                    // （需交互工具返回友好错误，由 LLM 自行决策；不产 user_prompt 阻塞）
                    mode: Some("auto".to_string()),
                    // risk_level 留空：由 orchestrator 回退到会话 metadata.risk_level（默认 medium）。
                    // Telegram 通道应尊重会话自身的风险等级设置。
                    risk_level: None,
                    resume: None,
                })
                .unwrap_or_default();

                let sub_ctx = ctx.fork();
                sub_ctx.set(crate::symbio_core::PATH, SESSION_CHAT.to_string());
                let _ = sub_ctx.set_payload(chat_input);
                sub_ctx.set(crate::symbio_core::WORKDIR, ".".to_string());
                sub_ctx.set(crate::symbio_core::SESSION_ID, chat_id.clone());

                match plugin.clone().route(sub_ctx).await {
                    Ok(payload) => {
                        let mut full_text = String::new();
                        match payload {
                            PluginPayload::Data(_) => {
                                if let Ok(chat_resp) =
                                    payload.get::<session_chat_response::Response>()
                                {
                                    full_text.push_str(
                                        &chat_resp
                                            .message
                                            .content
                                            .as_ref()
                                            .map(|c| c.to_text())
                                            .unwrap_or_default(),
                                    );
                                }
                            },
                            PluginPayload::Session(mut chan) => {
                                while let Some(frame) = chan.rx.recv().await {
                                    match frame {
                                        PluginFrame::Data(data) => {
                                            if let Ok(
                                                session_chat_response::StreamEvent::Update {
                                                    message,
                                                },
                                            ) = serde_json::from_value::<
                                                session_chat_response::StreamEvent,
                                            >(
                                                data
                                            ) {
                                                full_text.push_str(
                                                    &message
                                                        .content
                                                        .as_ref()
                                                        .map(|c| c.to_text())
                                                        .unwrap_or_default(),
                                                );
                                            }
                                        },
                                        PluginFrame::Error(e, _) => {
                                            tracing::error!("LLM Error: {}", e);
                                            break;
                                        },
                                    }
                                }
                            },
                            _ => {},
                        }

                        if full_text.is_empty() {
                            "无响应".to_string()
                        } else {
                            full_text
                        }
                    },
                    Err(e) => format!("LLM 调用失败: {e}"),
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
                let split_point =
                    Self::find_best_split_point(remaining, TELEGRAM_MAX_MESSAGE_LENGTH);
                let chunk = &remaining[..split_point];
                remaining = &remaining[split_point..];

                let msg = TelegramMessage {
                    chat_id: chat_id.clone(),
                    text: chunk.to_string(),
                    parse_mode: None,
                    reply_to_message_id: None,
                };

                let _ = client
                    .post(format!("{api_url}/sendMessage"))
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
        Self::new(None, TelegramConfig::default())
    }
}

#[async_trait]
impl Plugin for TelegramPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        ctx: Arc<dyn InvokeRequest>,
    ) -> InvokeResponse<PluginPayload> {
        let sub_path = ctx.get(crate::symbio_core::PATH).unwrap_or_default();
        if sub_path != crate::symbio_core::TRAVERSE_AVAILABLE_TOOLS {
            return Err(crate::symbio_core::PluginError::NotFound(format!(
                "未知遍历路径: {}",
                sub_path
            )));
        }

        Ok(PluginPayload::new(&Vec::<CapabilityMeta>::new()))
    }

    async fn route(self: Arc<Self>, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload> {
        let path = ctx.get(crate::symbio_core::PATH).unwrap_or_default();
        let path = path.strip_prefix('/').unwrap_or(&path);

        let data = match path {
            "send" => self.invoke_send(ctx.clone()).await?,
            "get_updates" => self.invoke_get_updates().await?,
            CONFIG_SET => self.invoke_configure(ctx.clone()).await?,
            CONFIG_GET => self.invoke_get_config().await?,
            "set_chat_id" => self.invoke_set_chat_id(ctx.clone()).await?,
            "start_listener" => self.invoke_start_listener(ctx.clone()).await?,
            "stop_listener" => self.invoke_stop_listener().await?,
            "status" => self.invoke_status().await?,
            _ => return Err(PluginError::NotFound(format!("未知路径: {path}"))),
        };

        Ok(PluginPayload::new(&data))
    }
}

impl TelegramPlugin {
    async fn invoke_send(&self, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<Value> {
        self.handle_send(ctx).await
    }

    async fn invoke_get_updates(&self) -> InvokeResponse<Value> {
        self.handle_get_updates().await
    }

    async fn invoke_configure(&self, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<Value> {
        let new_cfg: TelegramConfig = ctx.payload()?;

        if let Ok(mut config) = self.config.try_write() {
            *config = new_cfg;
        }

        if let Some(p) = self.get_parent_plugin().await {
            let save_ctx = ctx.fork();
            save_ctx.set(crate::symbio_core::PATH, "save_config".to_string());
            p.route(save_ctx).await?;
        }

        Ok(serde_json::to_value(common::SuccessResponse::default())?)
    }

    async fn get_parent_plugin(&self) -> Option<Arc<dyn Plugin>> {
        let guard = self.parent.read().await;
        guard.as_ref().and_then(|w| w.upgrade())
    }

    async fn invoke_get_config(&self) -> InvokeResponse<Value> {
        let config = self.config.read().await;
        Ok(serde_json::to_value(&*config)?)
    }

    async fn invoke_set_chat_id(&self, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<Value> {
        let new_cfg: TelegramConfig = ctx.payload()?;

        if let Some(id) = new_cfg.chat_id {
            if let Ok(mut config) = self.config.try_write() {
                config.chat_id = Some(id);
            }
        }
        Ok(serde_json::to_value(common::SuccessResponse::default())?)
    }

    async fn invoke_start_listener(&self, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<Value> {
        self.handle_start_listener(None, ctx).await
    }

    async fn invoke_stop_listener(&self) -> InvokeResponse<Value> {
        self.handle_stop_listener().await
    }

    async fn invoke_status(&self) -> InvokeResponse<Value> {
        self.handle_status().await
    }
}

crate::submit_object_creator!(PLUGIN_TELEGRAM, TelegramPlugin::build, dyn Plugin);
