//! LLM API 客户端

use crate::core::types::PluginError;
use super::types::*;
use reqwest::Client;
use reqwest_eventsource::{Event, EventSource};
use futures::stream::{Stream, StreamExt};
use std::time::Duration;

/// LLM 客户端
#[derive(Clone)]
pub struct LlmClient {
    client: Client,
    config: ProviderConfig,
}

impl LlmClient {
    pub fn new(config: ProviderConfig) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .expect("Failed to create HTTP client");

        Self { client, config }
    }

    /// 发送普通对话请求
    pub async fn chat(&self, messages: Vec<ChatMessage>) -> Result<String, PluginError> {
        if self.config.api_key.is_empty() {
            return Err(PluginError::ValidationError("API Key 未配置".to_string()));
        }

        let request = ChatRequest {
            messages,
            model: Some(self.config.model.clone()),
            temperature: self.config.temperature,
            max_tokens: self.config.max_tokens,
            stream: Some(false),
        };

        let url = format!("{}/chat/completions", self.config.api_base);
        
        let response = self.client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| PluginError::InternalError(format!("请求失败: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(PluginError::InternalError(format!("API 错误 ({}): {}", status, body)));
        }

        let chat_response: ChatResponse = response
            .json()
            .await
            .map_err(|e| PluginError::ParseError(format!("解析响应失败: {}", e)))?;

        chat_response
            .choices
            .first()
            .and_then(|c| c.message.as_ref())
            .map(|m| m.content.clone())
            .ok_or_else(|| PluginError::ParseError("响应中没有内容".to_string()))
    }

    /// 发送流式对话请求
    pub async fn chat_stream(
        &self,
        messages: Vec<ChatMessage>,
    ) -> Result<impl Stream<Item = Result<StreamEvent, PluginError>>, PluginError> {
        if self.config.api_key.is_empty() {
            return Err(PluginError::ValidationError("API Key 未配置".to_string()));
        }

        let request = ChatRequest {
            messages,
            model: Some(self.config.model.clone()),
            temperature: self.config.temperature,
            max_tokens: self.config.max_tokens,
            stream: Some(true),
        };

        let url = format!("{}/chat/completions", self.config.api_base);
        
        let es = EventSource::new(
            self.client
                .post(&url)
                .header("Authorization", format!("Bearer {}", self.config.api_key))
                .header("Content-Type", "application/json")
                .json(&request)
        ).map_err(|e| PluginError::InternalError(format!("创建 SSE 连接失败: {}", e)))?;

        let stream = es.map(|event| match event {
            Ok(Event::Open) => Ok(StreamEvent {
                event_type: StreamEventType::Start,
                content: String::new(),
            }),
            Ok(Event::Message(message)) => {
                if message.data == "[DONE]" {
                    Ok(StreamEvent {
                        event_type: StreamEventType::Done,
                        content: String::new(),
                    })
                } else {
                    let response: Result<ChatResponse, _> = serde_json::from_str(&message.data);
                    match response {
                        Ok(chat_response) => {
                            if let Some(choice) = chat_response.choices.first() {
                                if let Some(delta) = &choice.delta {
                                    return Ok(StreamEvent {
                                        event_type: StreamEventType::Delta,
                                        content: delta.content.clone().unwrap_or_default(),
                                    });
                                }
                            }
                            Ok(StreamEvent {
                                event_type: StreamEventType::Delta,
                                content: String::new(),
                            })
                        }
                        Err(e) => Err(PluginError::ParseError(format!("解析 SSE 数据失败: {}", e))),
                    }
                }
            }
            Err(e) => Err(PluginError::InternalError(format!("SSE 错误: {}", e))),
        });

        Ok(stream)
    }
}
