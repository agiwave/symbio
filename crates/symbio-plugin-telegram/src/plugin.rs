use super::schemas::{telegram_send, telegram_status};
use super::types::{TelegramConfig, TelegramMessage};
use super::typing::TypingGuard;
use symbio_deploy::PLUGIN_ID_TELEGRAM;
use symbio_core::PluginInvokeRequestExt;
use symbio_core::{
    plugin_dir_from_ctx, CapabilityMeta, Plugin, PluginConfigFile, PluginDir, PluginError,
    PluginInvokeRequest, PluginInvokeResponse, PluginMeta, PluginPayload, PluginStopReason,
};
use symbio_core::{DetailDefinition, DetailField, SuccessResponse};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

/// Telegram 消息最大长度
const TELEGRAM_MAX_MESSAGE_LENGTH: usize = 4096;

/// Telegram 配置的定义 —— **定义由配置的拥有者产出**。
///
/// 字段真源即 [`TelegramConfig`]：默认值从 `Default` 读出，不写第二份字面量。
fn config_definition() -> DetailDefinition {
    let d = TelegramConfig::default();
    DetailDefinition::form(
        "Telegram 设置",
        vec![
            DetailField::password(
                "bot_token",
                "Bot Token",
                "BotFather 签发的访问令牌",
                "123456:ABC-DEF...",
            ),
            DetailField::text("chat_id", "默认 Chat ID", "未指定时按会话推送"),
            DetailField::text(
                "api_base",
                "API 基址",
                "Bot API 上游地址；留空即官方端点（自建 / 代理端点填这里）",
            ),
            DetailField::toggle(
                "poll_enabled",
                "接收消息",
                "轮询更新，把用户消息交给会话处理",
                d.poll_enabled,
            ),
            // 游标是运行态事实，但它必须与 chat_id 存在同一个 owner 里（`PLUGIN.yml`），
            // 否则重启后要么重复处理、要么两处真相打架。表单里可改：改小 = 主动重放。
            DetailField::number(
                "update_offset",
                "更新游标",
                "已确认到的 update_id + 1；轮询自动推进，通常无需修改",
                0.0,
                9_007_199_254_740_991.0,
                serde_json::json!(d.update_offset),
            ),
            DetailField {
                key: "allowed_users".into(),
                label: "允许的用户 ID".into(),
                description: Some("每行一个用户 ID；留空表示不限制".into()),
                widget: "list".into(),
                full_width: true,
                ..Default::default()
            },
        ],
    )
}

/// 确认一批更新：推进游标、首次自动绑定 `chat_id`，**有变化才落盘**。
///
/// 手动 `get_updates` 与后台轮询共用这一份——若各写一遍，迟早有一处漏掉落盘，
/// 而漏掉的表现是「重启后重复处理历史更新」，只在重启时才显形。
///
/// 落盘是整份配置（与设置页保存同一个 owner，`PluginConfigFile` 原子写），
/// 因此两者并发时最后写入者赢；游标本身是单调的，退化只会发生在「设置页里
/// 打开了很久的旧表单被保存」，与手改游标的语义一致。
async fn acknowledge_updates(
    config: &RwLock<TelegramConfig>,
    config_file: &PluginConfigFile,
    updates: &[Value],
) {
    let mut changed = false;
    {
        let mut cfg = config.write().await;
        for update in updates {
            if let Some(uid) = update.get("update_id").and_then(Value::as_i64) {
                let next = uid + 1;
                if next > cfg.update_offset {
                    cfg.update_offset = next;
                    changed = true;
                }
            }
            if cfg.chat_id.is_none() {
                if let Some(id) = update
                    .get("message")
                    .and_then(|m| m.get("chat"))
                    .and_then(|c| c.get("id"))
                    .and_then(Value::as_i64)
                {
                    cfg.chat_id = Some(id.to_string());
                    changed = true;
                }
            }
        }
    }
    if !changed {
        return;
    }
    save_config(config, config_file).await;
}

/// 把配置槽写成 `PLUGIN.yml`（运行态字段的落盘入口）。
///
/// 日志而不是返回错误：调用点是后台轮询与路由臂，前者没有人能接收 `Err`；
/// 丢弃时**必须留痕**——重启后重复处理历史更新正是「静默落盘失败」的后果。
async fn save_config(config: &RwLock<TelegramConfig>, config_file: &PluginConfigFile) {
    let value = match serde_json::to_value(&*config.read().await) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("[telegram] 配置序列化失败，状态未能落盘: {e}");
            return;
        }
    };
    if let Err(e) = config_file.dir().save(&value) {
        tracing::warn!("[telegram] 配置落盘失败（游标/chat_id 重启后可能丢失）: {e}");
    }
}

/// Telegram 插件
#[derive(Clone)]
pub struct TelegramPlugin {
    config: Arc<RwLock<TelegramConfig>>,
    /// 配置文件的呈现与校验（`<根>/telegram/PLUGIN.yml`）
    config_file: PluginConfigFile,
    client: reqwest::Client,
    /// 监听器取消令牌
    listener_token: Arc<RwLock<Option<CancellationToken>>>,
    /// 监听器运行状态
    listener_running: Arc<AtomicBool>,
    /// 最新消息版本
    latest_version: Arc<RwLock<HashMap<String, u64>>>,
    // 这里曾有 `llm_plugin: Arc<RwLock<Option<Arc<dyn Plugin>>>>`——**按值持有兄弟插件**。
    // 已删除：跨插件调用一律经 `ctx.parent()` 走容器（见 `process_update`）。
}

impl TelegramPlugin {
    /// 静态工厂：从 PluginInvokeRequest 构造 Plugin 实例
    pub fn build(ctx: Arc<dyn PluginInvokeRequest>) -> Arc<dyn Plugin> {
        let dir = plugin_dir_from_ctx(&*ctx, PLUGIN_ID_TELEGRAM);
        let config: TelegramConfig = match dir.load::<TelegramConfig>() {
            Ok(Some(c)) => c,
            Ok(None) => TelegramConfig::default(),
            Err(e) => {
                symbio_core::plugin_warn!("telegram", "读取自身配置失败，改用默认值：{e}");
                TelegramConfig::default()
            }
        };

        Arc::new(TelegramPlugin::new(config, dir)) as Arc<dyn Plugin>
    }

    /// 主构造函数（Factory 机制使用）
    pub fn new(config: TelegramConfig, dir: PluginDir) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
            config_file: PluginConfigFile::new(dir, "Telegram 设置", config_definition()),
            client: reqwest::Client::new(),
            listener_token: Arc::new(RwLock::new(None)),
            listener_running: Arc::new(AtomicBool::new(false)),
            latest_version: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new("telegram", "Telegram")
            .with_description("Telegram Bot 的连接与推送配置。")
            .with_version("0.1.0")
            .with_order(10)
            .with_icon("send")
            .with_hidden(true)
    }

    fn api_url(&self) -> Option<String> {
        let config = self.config.try_read().ok()?;
        if config.bot_token.is_empty() {
            None
        } else {
            // 基址可配（缺省官方端点）：网络受限环境走自建 / 代理端点，e2e 也能打
            // 本地靶子。剥尾随 `/`——调用方拼 `{api_url}/sendMessage`，双斜杠会 404。
            Some(format!(
                "{}/bot{}",
                config.api_base.trim_end_matches('/'),
                config.bot_token
            ))
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
    async fn handle_send(&self, ctx: Arc<dyn PluginInvokeRequest>) -> PluginInvokeResponse<Value> {
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
    async fn handle_get_updates(&self) -> PluginInvokeResponse<Value> {
        let api_url = self
            .api_url()
            .ok_or_else(|| PluginError::ValidationError("未配置 bot_token".to_string()))?;

        let offset = self.config.read().await.update_offset;

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

            // 游标推进与首次自动绑定 chat_id 都在这里确认并落盘（与后台轮询同一条路径）。
            if let Some(result) = updates.get("result").and_then(|v| v.as_array()) {
                acknowledge_updates(&self.config, &self.config_file, result).await;
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
    ///
    /// **不接受「注入 LLM 插件」参数**：跨插件调用一律经 `ctx.parent()` 走容器，
    /// 见 `../../../../docs/003-design/006-插件路由与地址.md`。
    ///
    /// 原先的 `llm_plugin: Option<Arc<dyn Plugin>>` 参数是个从未生效的设计：
    /// 它只在传 `Some(..)` 时写入字段，而唯一调用点传的是 `None`，于是
    /// `process_update` 恒走「LLM 插件未配置」分支——**整条回复链路自始至终没通**。
    async fn handle_start_listener(
        &self,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<Value> {
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
        let config_file = self.config_file.clone();
        let client = self.client.clone();
        let latest_version = Arc::clone(&self.latest_version);
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
                        let offset = config.read().await.update_offset;
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
                                        // 先整批确认（游标 + 自动绑定，一次落盘），再逐条处理。
                                        acknowledge_updates(&config, &config_file, results).await;
                                        for update in results {
                                            // 处理每条消息。`client` / `api_url` 按值传：
                                            // 每条更新各起一个后台任务（见 `process_update`）。
                                            if let Err(e) = Self::process_update(
                                                client.clone(),
                                                api_url.clone(),
                                                update,
                                                &config,
                                                &latest_version,
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
    async fn handle_stop_listener(&self) -> PluginInvokeResponse<Value> {
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
    async fn handle_status(&self) -> PluginInvokeResponse<Value> {
        let config = self.config.read().await;
        Ok(serde_json::to_value(telegram_status::Response {
            configured: !config.bot_token.is_empty(),
            has_chat_id: config.chat_id.is_some(),
            poll_enabled: config.poll_enabled,
            listener_running: self.listener_running.load(Ordering::SeqCst),
            update_offset: config.update_offset,
        })
        .unwrap_or_default())
    }

    /// 处理单个更新
    ///
    /// `client` / `api_url` 按**值**收（而不是借用）：本方法把整轮会话调用交给
    /// 一个后台任务（见下），任务必须 `'static`。
    #[allow(clippy::too_many_arguments)]
    async fn process_update(
        client: reqwest::Client,
        api_url: String,
        update: &Value,
        config: &Arc<RwLock<TelegramConfig>>,
        latest_version: &Arc<RwLock<HashMap<String, u64>>>,
        ctx: Arc<dyn PluginInvokeRequest>,
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

        // ── 一轮会话调用交给后台任务：**不阻塞轮询** ──
        //
        // 一轮可能跑几分钟（长工具调用 / 子智能体），而 `getUpdates` 就在同一条
        // 循环里——同步等待会让整个 bot 在那段时间内收不到任何消息（`session/chat/send`
        // 立刻返回 `accepted` 只是把这个问题藏起来：回复永远是「无响应」）。
        //
        // 中断语义（`latest_version`）要求并发：同一 chat 的后来者把版本推高，前一轮
        // 据此放弃回复；串行 `await` 下那条判定恒不成立。
        // 同一会话的两轮由收件箱**单写者 FIFO** 串行（D-064），因此并发不会让
        // 两轮交错写同一条转写。
        let latest = Arc::clone(latest_version);
        let text = text.to_string();
        tokio::spawn(async move {
            // typing 指示器随任务起落：它跨整轮存在，一轮结束即停。
            let typing_guard = TypingGuard::new(client.clone(), api_url.clone(), chat_id.clone());

            // 跨插件调用必须**经父容器**：`telegram` 与 `session` 在 worker 下平级，
            // 只有容器的 `route` 认识绝对地址（容器挂载子插件时把自身 Weak 塞进子
            // ctx，`fork()` 会原样保留它）。直接调兄弟实例的 `route` 既绕过容器，
            // 又会在插件重建后钉住旧实例——这正是更早的写法。
            //
            // 调用形状（地址面入队 + 实时面取答复）见 [`super::turn`] 的模块文档。
            // 这里曾按 `session_chat::Response` 解析 `session/chat/send` 的回包，
            // 而它恒为 `{"status":"accepted"}`，于是回复永远是「无响应」。
            let response = match ctx.parent().and_then(|w| w.upgrade()) {
                Some(router) => match super::turn::run_turn(router, &ctx, &chat_id, &text).await {
                    super::turn::TurnOutcome::Replied(t) if t.trim().is_empty() => {
                        "（本轮没有产生文本输出）".to_string()
                    }
                    super::turn::TurnOutcome::Replied(t) => t,
                    super::turn::TurnOutcome::Failed(e) => format!("本轮以错误结束：{e}"),
                    super::turn::TurnOutcome::Broken(e) => format!("回复失败：{e}"),
                },
                None => "未挂载到插件容器，无法调用会话".to_string(),
            };

            // 检查消息是否被中断
            {
                let Ok(versions) = latest.try_read() else {
                    return;
                };
                if versions.get(&chat_id).copied() != Some(version) {
                    tracing::info!("[telegram] 消息 v{} 被中断", version);
                    return;
                }
            }

            // 发送响应
            drop(typing_guard); // 停止 typing

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
        });

        Ok(())
    }
}

/// 无装配上下文的实例（仅测试）。目录给临时目录——**不读全局系统根**。
#[cfg(test)]
impl Default for TelegramPlugin {
    fn default() -> Self {
        Self::new(
            TelegramConfig::default(),
            PluginDir::at(
                std::env::temp_dir().join("symbio-test/telegram"),
                PLUGIN_ID_TELEGRAM,
            ),
        )
    }
}

#[async_trait]
impl Plugin for TelegramPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    fn get_vfs_provider(self: Arc<Self>) -> Option<Arc<dyn symbio_core::VdfsProvider>> {
        Some(self)
    }

    /// 停用即收口长轮询：
    /// 复用 `stop_listener` 的取消令牌——路由臂与停用路径必须是**同一条**收口逻辑，
    /// 否则「面板上停了」与「插件被停用」会走出两种状态。
    async fn stop(self: Arc<Self>, _reason: PluginStopReason) -> Result<(), PluginError> {
        self.handle_stop_listener().await?;
        Ok(())
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let sub_path = ctx.get(symbio_core::PATH).unwrap_or_default();
        if sub_path != symbio_core::TRAVERSE_AVAILABLE_TOOLS {
            return Err(symbio_core::PluginError::NotFound(format!(
                "未知遍历路径: {}",
                sub_path
            )));
        }

        // 顺带声明「本插件有一份配置文档」（设置页据此列出并指路）
        symbio_core::capability_announce_configurable(&ctx, &self.config_file).await;

        Ok(PluginPayload::new(&Vec::<CapabilityMeta>::new()))
    }

    async fn route(
        self: Arc<Self>,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let path = ctx.get(symbio_core::PATH).unwrap_or_default();
        let path = path.strip_prefix('/').unwrap_or(&path);

        let data = match path {
            // 经 Telegram 发一条消息。
            "send" => self.invoke_send(ctx.clone()).await?,
            // 拉取 Telegram 更新（长轮询）。
            "get_updates" => self.invoke_get_updates().await?,
            // 绑定目标 chat id。
            "set_chat_id" => self.invoke_set_chat_id(ctx.clone()).await?,
            // 启动后台监听（长轮询入队，走会话地址面）。
            "start_listener" => self.invoke_start_listener(ctx.clone()).await?,
            // 停止后台监听。
            "stop_listener" => self.invoke_stop_listener().await?,
            // 通道运行状态。
            "status" => self.invoke_status().await?,
            _ => return Err(PluginError::NotFound(format!("未知路径: {path}"))),
        };

        Ok(PluginPayload::new(&data))
    }
}

impl TelegramPlugin {
    async fn invoke_send(&self, ctx: Arc<dyn PluginInvokeRequest>) -> PluginInvokeResponse<Value> {
        self.handle_send(ctx).await
    }

    async fn invoke_get_updates(&self) -> PluginInvokeResponse<Value> {
        self.handle_get_updates().await
    }

    async fn invoke_set_chat_id(
        &self,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<Value> {
        let new_cfg: TelegramConfig = ctx.payload()?;

        if let Some(id) = new_cfg.chat_id {
            self.config.write().await.chat_id = Some(id);
            // 绑定是**跨重启事实**：与游标同一条落盘路径（`PLUGIN.yml`）。
            save_config(&self.config, &self.config_file).await;
        }
        Ok(serde_json::to_value(SuccessResponse::default())?)
    }

    async fn invoke_start_listener(
        &self,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<Value> {
        self.handle_start_listener(ctx).await
    }

    async fn invoke_stop_listener(&self) -> PluginInvokeResponse<Value> {
        self.handle_stop_listener().await
    }

    async fn invoke_status(&self) -> PluginInvokeResponse<Value> {
        self.handle_status().await
    }
}

// ==================== VDFS：配置文档（`<根>/telegram/PLUGIN.yml`） ====================
//
// 本插件只有配置、没有资源树，因此挂载根的内容恒为「一个配置文件」。
// 节点形状、定义校验、落盘都在 [`PluginConfigFile`] 里，这里只做寻址分流。

#[async_trait]
impl symbio_core::PluginConfigMount for TelegramPlugin {
    type Config = TelegramConfig;
    const TITLE: &'static str = "Telegram";

    fn config_file(&self) -> &PluginConfigFile {
        &self.config_file
    }

    fn config_slot(&self) -> &RwLock<TelegramConfig> {
        &self.config
    }
}

#[cfg(test)]
#[path = "plugin.test.rs"]
mod tests;

symbio_core::submit_object_creator!(PLUGIN_ID_TELEGRAM, TelegramPlugin::build, dyn Plugin);

symbio_core::impl_config_mount_vdfs!(TelegramPlugin);
