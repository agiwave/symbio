//! `notify` 端口的世界提供者（DE-047 不变量 6 / D-302）。
//!
//! 环只按端口名取——Bot API 基址怎么拼、消息怎么切块、错误怎么分类，都是这里的私事。
//! 端口在 `traverse` 里随能力目录一起装：provider 行由 [`symbio_plugin_api::port::port_rows`]
//! 产出，注册面不随端口数写死。

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use symbio_plugin_api::port::{
    PortErrorClass, PortExecContext, PortExecutor, PortInput, PortOutcome, PortProvider,
};
use tokio::sync::RwLock;

use super::plugin::{api_url_from, send_text};
use super::types::TelegramConfig;

/// 本插件服务的端口名（数据：环不认这个名字，只按 `request` 载荷里的值取声明）。
pub(crate) const NOTIFY: &str = "notify";

/// `notify` 端口的提供者：一次外部消息推送。
#[derive(Clone)]
pub(crate) struct NotifyPort {
    client: reqwest::Client,
    config: Arc<RwLock<TelegramConfig>>,
}

impl NotifyPort {
    pub(crate) fn new(client: reqwest::Client, config: Arc<RwLock<TelegramConfig>>) -> Self {
        Self { client, config }
    }

    /// 端口载荷：`{ text, chat_id?, parse_mode? }`。
    ///
    /// 除了这三个字段，`request` 事实还带协议键（`port` / `id` / `attempt`）——执行体
    /// 不解析协议键，它们由环负责。
    async fn send(&self, input: &PortInput) -> PortOutcome {
        if input.port != NOTIFY {
            return PortOutcome::failed(
                PortErrorClass::NonRetryable,
                format!("本执行体只服务 `{NOTIFY}`，收到 `{}`", input.port),
            );
        }
        let Some(text) = input.payload.get("text").and_then(Value::as_str) else {
            return PortOutcome::failed(PortErrorClass::NonRetryable, "载荷缺少字符串字段 `text`");
        };
        let chat_id = input
            .payload
            .get("chat_id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| self.config.try_read().ok().and_then(|c| c.chat_id.clone()));
        let Some(chat_id) = chat_id else {
            return PortOutcome::failed(
                PortErrorClass::NonRetryable,
                "未指定 `chat_id`，且配置里没有默认 Chat ID",
            );
        };
        let Some(api_url) = self.config.try_read().ok().and_then(|c| api_url_from(&c)) else {
            return PortOutcome::failed(PortErrorClass::NonRetryable, "未配置 bot_token");
        };
        let parse_mode = input
            .payload
            .get("parse_mode")
            .and_then(Value::as_str)
            .map(str::to_owned);

        match send_text(
            &self.client,
            &api_url,
            &chat_id,
            text,
            parse_mode.as_deref(),
        )
        .await
        {
            Ok(_) => PortOutcome::ok(),
            // HTTP 4xx/5xx 是「这一次调用被上游拒绝」，重放同样的请求不会变好。
            Err(e) if e.starts_with("Telegram API 错误") => {
                PortOutcome::failed(PortErrorClass::NonRetryable, e)
            }
            // 连接失败 / 超时按可重试交给调用方。
            Err(e) => PortOutcome::failed(PortErrorClass::Retryable, e),
        }
    }
}

impl PortProvider for NotifyPort {
    fn ports(&self) -> Vec<String> {
        vec![NOTIFY.to_string()]
    }

    fn provider_id(&self) -> &str {
        "telegram"
    }

    fn executor(&self) -> Option<Arc<dyn PortExecutor>> {
        Some(Arc::new(self.clone()))
    }
}

#[async_trait]
impl PortExecutor for NotifyPort {
    async fn execute(&self, _ctx: PortExecContext, input: PortInput) -> PortOutcome {
        self.send(&input).await
    }
}

#[cfg(test)]
#[path = "port.test.rs"]
mod tests;
