//! OpenAI Compatible 插件实现

use super::types::*;
use super::token::*;
use super::stream::ToolCallAccumulator;
use crate::core::traits::{Plugin, CAPABILITY_LLM};
use crate::core::types::{PluginMeta, PluginResult, PluginError, InvokeStream, StreamChunk};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::{Arc, Weak};
use tokio::sync::RwLock;

/// OpenAI Compatible 插件
pub struct OpenAiPlugin {
    meta: PluginMeta,
    config: Arc<RwLock<OpenAiConfig>>,
    client: reqwest::Client,
    /// 父插件引用（用于能力路由）
    parent: Option<Weak<dyn Plugin>>,
}

impl OpenAiPlugin {
    fn create_meta() -> PluginMeta {
        PluginMeta {
            name: "openai".to_string(),
            description: "OpenAI 兼容 LLM API 集成".to_string(),
            version: "0.1.0".to_string(),
            input: Some(json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["chat", "status", "list_models", "configure", "get_config", "compress_info"]
                    }
                },
                "required": ["action"]
            })),
            output: None,
            author: Some("Symbio Team".to_string()),
        }
    }

    /// 主构造函数（Factory 机制使用）
    pub fn new(parent: Option<Weak<dyn Plugin>>, config: OpenAiConfig) -> Self {
        Self {
            meta: Self::create_meta(),
            config: Arc::new(RwLock::new(config)),
            client: reqwest::Client::new(),
            parent,
        }
    }

    /// 获取父插件引用
    fn get_parent(&self) -> Option<Arc<dyn Plugin>> {
        self.parent.as_ref().and_then(|w| w.upgrade())
    }

    fn api_url(&self) -> String {
        let config = self.config.try_read();
        match config {
            Ok(c) => format!("{}/chat/completions", c.api_base),
            Err(_) => "https://api.openai.com/v1/chat/completions".to_string(),
        }
    }

    async fn handle_chat(&self, input: &Value) -> Result<StreamChunk, PluginError> {
        let message = input.get("message")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PluginError::ValidationError("缺少 message 参数".to_string()))?;

        let session_id = input.get("session_id")
            .and_then(|v| v.as_str())
            .unwrap_or("default");

        let config = self.config.read().await.clone();
        let counter = TokenCounter::for_model(&config.model);
        let context_config = ContextConfig::for_model(&config.model);

        // 通过 @session 能力路由获取上下文（包含 system_prompt, tools, history）
        let mut system_prompt = config.system_prompt.clone().unwrap_or_else(default_system_prompt);
        let mut tools: Vec<NativeToolSpec> = Vec::new();
        let mut context_messages: Vec<NativeMessage> = Vec::new();

        if let Some(parent) = self.get_parent() {
            let session_input = json!({
                "action": "get_context",
                "session_id": session_id,
                "history": true
            });

            if let Ok(stream) = parent.invoke("session", session_input) {
                if let InvokeStream::Single(chunk) = stream {
                    if chunk.error.is_none() {
                        // 解析 LlmContext：system_prompt + tools + history
                        if let Some(sys) = chunk.data.get("system_prompt").and_then(|v| v.as_str()) {
                            if !sys.is_empty() {
                                system_prompt = sys.to_string();
                            }
                        }
                        // 解析工具定义
                        if let Some(tools_arr) = chunk.data.get("tools").and_then(|v| v.as_array()) {
                            for tool in tools_arr {
                                if let Ok(spec) = serde_json::from_value(tool.clone()) {
                                    tools.push(spec);
                                }
                            }
                        }
                        // 解析上下文消息（可能是压缩后的历史或选定的上下文片段）
                        if let Some(history) = chunk.data.get("history").and_then(|v| v.as_array()) {
                            for msg in history {
                                if let (Some(role), Some(content)) = (
                                    msg.get("role").and_then(|r| r.as_str()),
                                    msg.get("content").and_then(|c| c.as_str())
                                ) {
                                    context_messages.push(NativeMessage {
                                        role: role.to_string(),
                                        content: Some(content.to_string()),
                                        tool_call_id: msg.get("tool_call_id").and_then(|v| v.as_str()).map(|s| s.to_string()),
                                        tool_calls: None,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }

        // 构建消息：system + context + current
        let mut messages: Vec<NativeMessage> = vec![
            NativeMessage {
                role: "system".into(),
                content: Some(system_prompt),
                tool_call_id: None,
                tool_calls: None,
            }
        ];

        // 添加上下文消息
        messages.extend(context_messages);

        // 记录历史长度（用于保存新消息）
        let history_len = messages.len();

        // 添加当前消息
        messages.push(NativeMessage {
            role: "user".into(),
            content: Some(message.to_string()),
            tool_call_id: None,
            tool_calls: None,
        });

        // 获取父插件引用用于工具调用
        let parent = self.get_parent();
        let api_key = config.api_key.clone().unwrap_or_default();

        // Agent loop - 工具调用循环
        let max_iterations = 255;
        let mut final_content = String::new();
        let mut final_usage: Option<TokenUsage> = None;

        for iteration in 0..max_iterations {
            // 估算 tokens 并裁剪
            let tools_ref = if tools.is_empty() { None } else { Some(&tools) };
            let estimated_tokens = count_total_tokens(&counter, &messages, tools_ref);
            if estimated_tokens > context_config.available_tokens() {
                trim_messages_to_fit(&counter, &mut messages, &context_config, tools_ref);
            }

            eprintln!("[openai] Iteration {} - messages: {}, tokens: {}", 
                iteration + 1, messages.len(), estimated_tokens);

            // 构建请求
            let mut request = json!({
                "model": config.model,
                "messages": &messages,
                "temperature": config.temperature,
                "stream": false,
            });

            if let Some(max_tokens) = config.max_tokens {
                request["max_tokens"] = json!(max_tokens);
            }

            // 添加工具定义（如果有）
            if !tools.is_empty() {
                request["tools"] = json!(tools);
                request["tool_choice"] = json!("auto");
            }

            // 发送请求
            let response = self.client
                .post(&self.api_url())
                .header("Authorization", format!("Bearer {}", api_key))
                .header("Content-Type", "application/json")
                .json(&request)
                .timeout(std::time::Duration::from_secs(config.timeout_secs))
                .send()
                .await
                .map_err(|e| PluginError::InternalError(format!("请求失败: {}", e)))?;

            if !response.status().is_success() {
                let error = response.text().await.unwrap_or_default();
                return Ok(StreamChunk {
                    data: json!({}),
                    done: true,
                    error: Some(format!("API 错误: {}", error)),
                });
            }

            let response_json: Value = response.json().await
                .map_err(|e| PluginError::InternalError(format!("解析响应失败: {}", e)))?;

            // 提取响应
            let msg_obj = response_json
                .get("choices")
                .and_then(|c| c.get(0))
                .and_then(|c| c.get("message"));

            let content = msg_obj
                .and_then(|m| m.get("content"))
                .and_then(|c| c.as_str())
                .unwrap_or("");

            // 提取 tool_calls
            let tool_calls: Vec<(String, String, Value)> = msg_obj
                .and_then(|m| m.get("tool_calls"))
                .and_then(|tc| tc.as_array())
                .map(|arr| {
                    arr.iter().filter_map(|tc| {
                        let id = tc.get("id").and_then(|v| v.as_str())?.to_string();
                        let name = tc.get("function")
                            .and_then(|f| f.get("name"))
                            .and_then(|n| n.as_str())?.to_string();
                        let args_str = tc.get("function")
                            .and_then(|f| f.get("arguments"))
                            .and_then(|a| a.as_str())?;
                        let args: Value = serde_json::from_str(args_str).unwrap_or(json!({}));
                        Some((id, name, args))
                    }).collect()
                })
                .unwrap_or_default();

            // 提取 usage
            if let Some(u) = response_json.get("usage") {
                final_usage = Some(TokenUsage {
                    input_tokens: u.get("prompt_tokens").and_then(|v| v.as_u64()),
                    output_tokens: u.get("completion_tokens").and_then(|v| v.as_u64()),
                });
            }

            // 构建 assistant 消息并添加到历史
            let assistant_msg = NativeMessage {
                role: "assistant".into(),
                content: if content.is_empty() { None } else { Some(content.to_string()) },
                tool_call_id: None,
                tool_calls: if tool_calls.is_empty() { None } else {
                    Some(tool_calls.iter().map(|(id, name, args)| {
                        NativeToolCall {
                            id: Some(id.clone()),
                            kind: Some("function".into()),
                            function: NativeFunctionCall {
                                name: name.clone(),
                                arguments: serde_json::to_string(args).unwrap_or_default(),
                            },
                        }
                    }).collect())
                },
            };
            messages.push(assistant_msg);

            // 没有工具调用 - 返回最终结果
            if tool_calls.is_empty() {
                final_content = content.to_string();
                break;
            }

            // 有工具调用 - 执行工具并将结果添加到消息历史
            eprintln!("[openai] Processing {} tool calls", tool_calls.len());

            for (id, name, args) in tool_calls {
                eprintln!("[openai] Executing tool: {} with args: {}", name, args);

                // 通过父插件调用工具，直接将工具名称作为 path
                let result = match &parent {
                    Some(p) => {
                        // 直接将工具名称作为 path 调用，由父插件的路由机制处理
                        match p.invoke(&name, args.clone()) {
                            Ok(InvokeStream::Single(chunk)) if chunk.error.is_none() => {
                                // 优先提取 content 字段，否则返回整个 data
                                if let Some(content) = chunk.data.get("content").and_then(|c| c.as_str()) {
                                    content.to_string()
                                } else if let Some(success) = chunk.data.get("success").and_then(|s| s.as_bool()) {
                                    if success {
                                        chunk.data.to_string()
                                    } else {
                                        format!("Error: {}", chunk.data.get("error").and_then(|e| e.as_str()).unwrap_or("unknown error"))
                                    }
                                } else {
                                    chunk.data.to_string()
                                }
                            }
                            Ok(InvokeStream::Single(chunk)) => {
                                format!("Error: {}", chunk.error.unwrap_or_default())
                            }
                            Ok(InvokeStream::Stream(mut s)) => {
                                // 收集流式响应
                                let mut result = String::new();
                                use futures::StreamExt;
                                while let Some(chunk) = s.next().await {
                                    if chunk.error.is_some() {
                                        result = format!("Error: {}", chunk.error.unwrap_or_default());
                                        break;
                                    }
                                    if let Some(text) = chunk.data.get("content").and_then(|c| c.as_str()) {
                                        result.push_str(text);
                                    } else if !chunk.data.is_null() {
                                        result.push_str(&chunk.data.to_string());
                                    }
                                }
                                result
                            }
                            Err(e) => format!("Error: {}", e),
                        }
                    }
                    None => "Error: No parent plugin available".to_string(),
                };

                eprintln!("[openai] Tool result: {}", result.chars().take(100).collect::<String>());

                // 添加工具结果到消息历史
                messages.push(NativeMessage {
                    role: "tool".into(),
                    content: Some(result),
                    tool_call_id: Some(id),
                    tool_calls: None,
                });
            }
        }

        // 保存消息到 session
        if let Some(ref p) = parent {
            let new_messages: Vec<Value> = messages[history_len..]
                .iter()
                .filter_map(|m| {
                    let role = m.role.as_str();
                    // 跳过空的 assistant 消息
                    if role == "assistant" && m.content.is_none() && m.tool_calls.is_none() {
                        return None;
                    }
                    
                    let mut msg = json!({
                        "role": role,
                        "content": m.content.clone().unwrap_or_default()
                    });
                    
                    if let Some(ref tc) = m.tool_calls {
                        msg["tool_calls"] = json!(tc);
                    }
                    if let Some(ref id) = m.tool_call_id {
                        msg["tool_call_id"] = json!(id);
                    }
                    
                    Some(msg)
                })
                .collect();

            if !new_messages.is_empty() {
                let append_input = json!({
                    "action": "append",
                    "session_id": session_id,
                    "messages": new_messages
                });
                let _ = p.invoke("session", append_input);
            }
        }

        Ok(StreamChunk {
            data: json!({
                "success": true,
                "content": final_content,
                "usage": final_usage,
                "model": config.model
            }),
            done: true,
            error: None,
        })
    }

    async fn handle_status(&self) -> Result<StreamChunk, PluginError> {
        let config = self.config.read().await;
        Ok(StreamChunk {
            data: json!({
                "success": true,
                "status": "ready",
                "model": config.model,
                "api_base": config.api_base,
                "has_api_key": config.api_key.is_some()
            }),
            done: true,
            error: None,
        })
    }

    fn handle_list_models(&self, input: &Value) -> Result<StreamChunk, PluginError> {
        // 如果指定了特定模型，返回详细信息
        if let Some(model_name) = input.get("model").and_then(|m| m.as_str()) {
            let config = get_model_config(model_name);
            return Ok(StreamChunk {
                data: json!({
                    "success": true,
                    "model": {
                        "name": config.name,
                        "max_context_tokens": config.max_context_tokens,
                        "encoding": match config.encoding {
                            TokenizerEncoding::Cl100kBase => "cl100k_base",
                            TokenizerEncoding::O200kBase => "o200k_base",
                        },
                        "max_output_tokens": config.max_output_tokens,
                        "supports_vision": config.supports_vision,
                        "supports_tools": config.supports_tools,
                        "reserved_tokens": config.reserved_tokens()
                    }
                }),
                done: true,
                error: None,
            });
        }

        // 返回已知模型列表
        let known_models = vec![
            "gpt-4o", "gpt-4o-mini", "gpt-4-turbo", "gpt-4", "gpt-3.5-turbo",
            "o1", "o1-preview", "o1-mini", "o3-mini",
            "claude-3-opus", "claude-3-sonnet", "claude-3-haiku", "claude-3-5-sonnet",
            "moonshot-v1-8k", "moonshot-v1-32k", "moonshot-v1-128k",
            "deepseek-chat", "deepseek-coder",
            "qwen-turbo", "qwen-plus", "qwen-max",
            "glm-4", "glm-4-plus",
        ];

        let models_info: Vec<Value> = known_models.iter().map(|name| {
            let config = get_model_config(name);
            json!({
                "name": name,
                "max_context_tokens": config.max_context_tokens,
                "supports_vision": config.supports_vision,
                "supports_tools": config.supports_tools
            })
        }).collect();

        Ok(StreamChunk {
            data: json!({
                "success": true,
                "models": models_info
            }),
            done: true,
            error: None,
        })
    }

    async fn handle_configure(&self, input: &Value) -> Result<StreamChunk, PluginError> {
        eprintln!("[openai] handle_configure called with: {:?}", input);
        
        {
            let mut config = self.config.write().await;

            if let Some(v) = input.get("api_base").and_then(|v| v.as_str()) {
                config.api_base = v.to_string();
            }
            if let Some(v) = input.get("api_key").and_then(|v| v.as_str()) {
                config.api_key = Some(v.to_string());
            }
            if let Some(v) = input.get("model").and_then(|v| v.as_str()) {
                config.model = v.to_string();
            }
            if let Some(v) = input.get("temperature").and_then(|v| v.as_f64()) {
                config.temperature = v as f32;
            }
            if let Some(v) = input.get("max_tokens").and_then(|v| v.as_u64()) {
                config.max_tokens = Some(v as u32);
            }
            if let Some(v) = input.get("system_prompt").and_then(|v| v.as_str()) {
                config.system_prompt = Some(v.to_string());
            }
            if let Some(v) = input.get("timeout_secs").and_then(|v| v.as_u64()) {
                config.timeout_secs = v;
            }
        }

        // 保存配置到文件
        eprintln!("[openai] calling save_config via parent...");
        if let Some(parent) = self.get_parent() {
            eprintln!("[openai] parent found, invoking save_config");
            let result = parent.invoke("save_config", json!({}));
            match result {
                Ok(_) => eprintln!("[openai] save_config call succeeded"),
                Err(e) => eprintln!("[openai] save_config call failed: {:?}", e),
            }
        } else {
            eprintln!("[openai] ERROR: no parent!");
        }

        Ok(StreamChunk {
            data: json!({
                "success": true,
                "message": "配置已更新"
            }),
            done: true,
            error: None,
        })
    }

    async fn handle_get_config(&self) -> Result<StreamChunk, PluginError> {
        let config = self.config.read().await;
        // 返回实际 api_key（前端需要判断是否已配置）
        let api_key_display = config.api_key.as_ref().map(|k| {
            if k.len() > 8 {
                format!("{}***{}", &k[..4], &k[k.len()-4..])
            } else {
                "***".to_string()
            }
        }).unwrap_or_default();
        
        Ok(StreamChunk {
            data: json!({
                "success": true,
                "config": {
                    "api_base": config.api_base,
                    "api_key": config.api_key,
                    "api_key_display": api_key_display,
                    "model": config.model,
                    "temperature": config.temperature,
                    "max_tokens": config.max_tokens,
                    "max_context_tokens": config.max_context_tokens,
                    "timeout_secs": config.timeout_secs
                }
            }),
            done: true,
            error: None,
        })
    }

    async fn handle_compress_info(&self, input: &Value) -> Result<StreamChunk, PluginError> {
        let history = input.get("history")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let config = self.config.read().await;
        let counter = TokenCounter::for_model(&config.model);
        let context_config = ContextConfig::for_model(&config.model);

        let mut total_tokens = 0;
        for msg in &history {
            if let Some(content) = msg.get("content").and_then(|c| c.as_str()) {
                total_tokens += counter.count_tokens(content);
            }
        }

        let usage_percent = (total_tokens as f64 / context_config.max_tokens as f64) * 100.0;
        let should_compress = context_config.should_compress(total_tokens);

        Ok(StreamChunk {
            data: json!({
                "success": true,
                "message_count": history.len(),
                "total_tokens": total_tokens,
                "max_tokens": context_config.max_tokens,
                "usage_percent": usage_percent.round() as u32,
                "should_compress": should_compress,
                "compression_threshold_percent": (context_config.compression_threshold * 100.0) as u32
            }),
            done: true,
            error: None,
        })
    }

    /// 流式处理聊天请求（支持工具调用）
    async fn handle_chat_stream(&self, input: &Value) -> PluginResult<InvokeStream> {
        let message = input.get("message")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PluginError::ValidationError("缺少 message 参数".to_string()))?;

        let session_id = input.get("session_id")
            .and_then(|v| v.as_str())
            .unwrap_or("default")
            .to_string();

        let config = self.config.read().await.clone();
        let parent = self.get_parent();
        let api_key = config.api_key.clone().unwrap_or_default();
        let api_url = self.api_url();

        eprintln!("[openai] handle_chat_stream started: session={}, model={}", session_id, config.model);

        // 通过 @session 能力路由获取上下文（包含 system_prompt, tools, history）
        let mut system_prompt = config.system_prompt.clone().unwrap_or_else(default_system_prompt);
        let mut tools: Vec<NativeToolSpec> = Vec::new();
        let mut context_messages: Vec<NativeMessage> = Vec::new();

        if let Some(p) = &parent {
            eprintln!("[openai] fetching context from session...");
            let session_input = json!({
                "action": "get_context",
                "session_id": session_id,
                "history": true
            });

            if let Ok(stream) = p.invoke("session", session_input) {
                if let InvokeStream::Single(chunk) = stream {
                    if chunk.error.is_none() {
                        eprintln!("[openai] context fetched successfully");
                        if let Some(sys) = chunk.data.get("system_prompt").and_then(|v| v.as_str()) {
                            if !sys.is_empty() {
                                system_prompt = sys.to_string();
                            }
                        }
                        if let Some(tools_arr) = chunk.data.get("tools").and_then(|v| v.as_array()) {
                            eprintln!("[openai] got {} tools from session", tools_arr.len());
                            for tool in tools_arr {
                                if let Ok(spec) = serde_json::from_value(tool.clone()) {
                                    tools.push(spec);
                                }
                            }
                        }
                        if let Some(history) = chunk.data.get("history").and_then(|v| v.as_array()) {
                            for msg in history {
                                if let (Some(role), Some(content)) = (
                                    msg.get("role").and_then(|r| r.as_str()),
                                    msg.get("content").and_then(|c| c.as_str())
                                ) {
                                    context_messages.push(NativeMessage {
                                        role: role.to_string(),
                                        content: Some(content.to_string()),
                                        tool_call_id: msg.get("tool_call_id").and_then(|v| v.as_str()).map(|s| s.to_string()),
                                        tool_calls: None,
                                    });
                                }
                            }
                        }
                    } else {
                        eprintln!("[openai] context fetch error: {:?}", chunk.error);
                    }
                }
            } else {
                eprintln!("[openai] context fetch failed");
            }
        } else {
            eprintln!("[openai] no parent plugin");
        }

        eprintln!("[openai] building messages with {} context messages", context_messages.len());

        // 构建消息：system + context + current
        let mut messages: Vec<NativeMessage> = vec![
            NativeMessage {
                role: "system".into(),
                content: Some(system_prompt),
                tool_call_id: None,
                tool_calls: None,
            }
        ];
        messages.extend(context_messages);
        let history_len = messages.len();
        messages.push(NativeMessage {
            role: "user".into(),
            content: Some(message.to_string()),
            tool_call_id: None,
            tool_calls: None,
        });

        // 创建流式返回
        let stream = async_stream::stream! {
            use futures::StreamExt;

            eprintln!("[openai] stream started, messages={}, tools={}", messages.len(), tools.len());

            // Agent loop - 工具调用循环
            let max_iterations = 255;
            let mut final_content = String::new();
            let last_stream_content = String::new();  // 保存最后一次流式内容

            for iteration in 0..max_iterations {
                eprintln!("[openai] iteration {}", iteration + 1);
                // 构建请求 - 使用流式 API
                let mut request = json!({
                    "model": config.model,
                    "messages": &messages,
                    "temperature": config.temperature,
                    "stream": true,
                });

                if let Some(max_tokens) = config.max_tokens {
                    request["max_tokens"] = json!(max_tokens);
                }

                // 添加工具定义（如果有）
                if !tools.is_empty() {
                    request["tools"] = json!(tools);
                    request["tool_choice"] = json!("auto");
                }

                eprintln!("[openai] sending request to {}", api_url);
                // 发送流式请求
                let response = match reqwest::Client::new()
                    .post(&api_url)
                    .header("Authorization", format!("Bearer {}", api_key))
                    .header("Content-Type", "application/json")
                    .json(&request)
                    .timeout(std::time::Duration::from_secs(config.timeout_secs))
                    .send()
                    .await
                {
                    Ok(r) => {
                        eprintln!("[openai] response status: {}", r.status());
                        r
                    },
                    Err(e) => {
                        eprintln!("[openai] request error: {}", e);
                        yield StreamChunk {
                            data: json!({}),
                            done: true,
                            error: Some(format!("请求失败: {}", e)),
                        };
                        return;
                    }
                };

                if !response.status().is_success() {
                    let error = response.text().await.unwrap_or_default();
                    eprintln!("[openai] API error: {}", error);
                    yield StreamChunk {
                        data: json!({}),
                        done: true,
                        error: Some(format!("API 错误: {}", error)),
                    };
                    return;
                }

                eprintln!("[openai] processing stream...");
                // 处理流式响应
                let mut stream_content = if last_stream_content.is_empty() { 
                    String::new() 
                } else { 
                    last_stream_content.clone() 
                };
                let mut tool_call_accumulator = ToolCallAccumulator::new();
                let mut stream = response.bytes_stream();

                while let Some(chunk_result) = stream.next().await {
                    let chunk_bytes = match chunk_result {
                        Ok(b) => b,
                        Err(e) => {
                            eprintln!("[openai] stream read error: {}", e);
                            yield StreamChunk {
                                data: json!({}),
                                done: true,
                                error: Some(format!("读取流失败: {}", e)),
                            };
                            return;
                        }
                    };

                    let chunk_text = String::from_utf8_lossy(&chunk_bytes);
                    eprintln!("[openai] received chunk: {} bytes", chunk_bytes.len());

                    for line in chunk_text.lines() {
                        if line.starts_with("data: ") {
                            let data = &line[6..];
                            if data == "[DONE]" {
                                continue;
                            }

                            if let Ok(chunk_json) = serde_json::from_str::<Value>(data) {
                                if let Some(choices) = chunk_json.get("choices") {
                                    if let Some(choice) = choices.get(0) {
                                        if let Some(delta) = choice.get("delta") {
                                            // 提取内容
                                            if let Some(content) = delta.get("content").and_then(|c| c.as_str()) {
                                                stream_content.push_str(content);
                                                
                                                // 实时返回累积内容
                                                yield StreamChunk {
                                                    data: json!({
                                                        "content": stream_content.clone(),
                                                        "done": false
                                                    }),
                                                    done: false,
                                                    error: None,
                                                };
                                            }

                                            // 使用 ToolCallAccumulator 处理 tool_calls
                                            if let Some(tc_arr) = delta.get("tool_calls").and_then(|t| t.as_array()) {
                                                for tc in tc_arr {
                                                    let index = tc.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                                                    let id = tc.get("id").and_then(|i| i.as_str());
                                                    let func = tc.get("function");
                                                    let name = func.and_then(|f| f.get("name")).and_then(|n| n.as_str());
                                                    let args = func.and_then(|f| f.get("arguments")).and_then(|a| a.as_str());

                                                    tool_call_accumulator.process_delta(index, id, name, args);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // 获取完成的 tool_calls
                let tool_calls = tool_call_accumulator.get_completed();

                // 构建 assistant 消息
                let assistant_msg = NativeMessage {
                    role: "assistant".into(),
                    content: if stream_content.is_empty() { None } else { Some(stream_content.clone()) },
                    tool_call_id: None,
                    tool_calls: if tool_calls.is_empty() { None } else {
                        Some(tool_calls.iter().map(|(id, name, args)| NativeToolCall {
                            id: Some(id.clone()),
                            kind: Some("function".into()),
                            function: NativeFunctionCall {
                                name: name.clone(),
                                arguments: serde_json::to_string(args).unwrap_or_default(),
                            },
                        }).collect())
                    },
                };
                messages.push(assistant_msg);

                // 没有工具调用 - 返回最终结果
                if tool_calls.is_empty() {
                    final_content = stream_content;
                    break;
                }

                // 有工具调用 - 先返回当前内容，然后执行工具
                yield StreamChunk {
                    data: json!({
                        "content": stream_content,
                        "tool_calls": tool_calls.iter().map(|(id, name, args)| {
                            json!({
                                "id": id,
                                "function": {
                                    "name": name,
                                    "arguments": args
                                }
                            })
                        }).collect::<Vec<_>>(),
                        "done": false
                    }),
                    done: false,
                    error: None,
                };

                // 执行每个工具调用
                for (id, name, args) in tool_calls {
                    eprintln!("[openai] Executing tool: {} with args: {}", name, args);

                    // 通过父插件调用工具，直接将工具名称作为 path
                    // 由父插件（agent）的路由机制处理工具名称解析
                    let result = match &parent {
                        Some(p) => {
                            match p.invoke(&name, args.clone()) {
                                Ok(InvokeStream::Single(chunk)) if chunk.error.is_none() => {
                                    if let Some(content) = chunk.data.get("content").and_then(|c| c.as_str()) {
                                        content.to_string()
                                    } else if let Some(success) = chunk.data.get("success").and_then(|s| s.as_bool()) {
                                        if success {
                                            chunk.data.to_string()
                                        } else {
                                            format!("Error: {}", chunk.data.get("error").and_then(|e| e.as_str()).unwrap_or("unknown error"))
                                        }
                                    } else {
                                        chunk.data.to_string()
                                    }
                                }
                                Ok(InvokeStream::Single(chunk)) => {
                                    format!("Error: {}", chunk.error.unwrap_or_default())
                                }
                                Ok(InvokeStream::Stream(mut s)) => {
                                    let mut result = String::new();
                                    while let Some(chunk) = s.next().await {
                                        if chunk.error.is_some() {
                                            result = format!("Error: {}", chunk.error.unwrap_or_default());
                                            break;
                                        }
                                        if let Some(text) = chunk.data.get("content").and_then(|c| c.as_str()) {
                                            result.push_str(text);
                                        } else if !chunk.data.is_null() {
                                            result.push_str(&chunk.data.to_string());
                                        }
                                    }
                                    result
                                }
                                Err(e) => format!("Error: {}", e),
                            }
                        }
                        None => "Error: No parent plugin available".to_string(),
                    };

                    eprintln!("[openai] Tool result: {}", result.chars().take(100).collect::<String>());

                    // 返回工具结果
                    yield StreamChunk {
                        data: json!({
                            "tool_result": {
                                "id": id,
                                "name": name,
                                "result": result
                            },
                            "done": false
                        }),
                        done: false,
                        error: None,
                    };

                    // 添加工具结果到消息历史
                    messages.push(NativeMessage {
                        role: "tool".into(),
                        content: Some(result),
                        tool_call_id: Some(id),
                        tool_calls: None,
                    });
                }
                
                // 工具调用完成后，从消息历史中获取 assistant 的 content
                if let Some(last_assistant_msg) = messages.iter().rev().find(|m| m.role == "assistant" && m.content.is_some()) {
                    if let Some(ref content) = last_assistant_msg.content {
                        final_content = content.clone();
                    }
                }
            }

            // 保存消息到 session
            if let Some(ref p) = parent {
                let new_messages: Vec<Value> = messages[history_len..]
                    .iter()
                    .filter_map(|m| {
                        let role = m.role.as_str();
                        if role == "assistant" && m.content.is_none() && m.tool_calls.is_none() {
                            return None;
                        }

                        let mut msg = json!({
                            "role": role,
                            "content": m.content.clone().unwrap_or_default()
                        });

                        if let Some(ref tc) = m.tool_calls {
                            msg["tool_calls"] = json!(tc);
                        }
                        if let Some(ref id) = m.tool_call_id {
                            msg["tool_call_id"] = json!(id);
                        }

                        Some(msg)
                    })
                    .collect();

                if !new_messages.is_empty() {
                    let append_input = json!({
                        "action": "append",
                        "session_id": session_id,
                        "messages": new_messages
                    });
                    let _ = p.invoke("session", append_input);
                }
            }

            // 返回最终结果
            yield StreamChunk {
                data: json!({
                    "content": final_content,
                    "done": true
                }),
                done: true,
                error: None,
            };
        };

        Ok(InvokeStream::Stream(Box::pin(stream)))
    }
}

impl Default for OpenAiPlugin {
    fn default() -> Self {
        Self::new(None, OpenAiConfig::default())
    }
}

#[async_trait]
impl Plugin for OpenAiPlugin {
    fn meta(&self, path: &str) -> PluginResult<PluginMeta> {
        if path == "config" {
            return Ok(PluginMeta {
                name: "config".to_string(),
                description: "OpenAI 配置管理".to_string(),
                version: "0.1.0".to_string(),
                input: Some(json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["get", "set"],
                            "description": "get获取配置，set设置配置"
                        },
                        "config": {
                            "type": "object",
                            "properties": {
                                "api_base": { "type": "string", "description": "API 基础 URL" },
                                "api_key": { "type": "string", "description": "API Key" },
                                "model": { "type": "string", "description": "模型名称" },
                                "temperature": { "type": "number", "description": "温度参数" },
                                "max_tokens": { "type": "integer", "description": "最大输出 tokens" },
                                "system_prompt": { "type": "string", "description": "系统提示词" }
                            }
                        }
                    },
                    "required": ["action"]
                })),
                output: Some(json!({
                    "type": "object",
                    "properties": {
                        "success": { "type": "boolean" },
                        "config": { "type": "object" }
                    }
                })),
                author: Some("Symbio Team".to_string()),
            });
        }
        Ok(self.meta.clone())
    }

    fn capabilities(&self) -> Vec<&'static str> {
        vec![CAPABILITY_LLM]
    }

    fn invoke(&self, path: &str, input: Value) -> PluginResult<InvokeStream> {
        // 处理 config path
        if path == "config" {
            let action = input.get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("get");

            let result = tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    match action {
                        "get" => {
                            let config = self.config.read().await;
                            Ok(StreamChunk {
                                data: json!({
                                    "api_base": config.api_base,
                                    "api_key": config.api_key,
                                    "model": config.model,
                                    "temperature": config.temperature,
                                    "max_tokens": config.max_tokens,
                                    "max_context_tokens": config.max_context_tokens,
                                    "system_prompt": config.system_prompt
                                }),
                                done: true,
                                error: None,
                            })
                        }
                        "set" => {
                            eprintln!("[openai] config set called with: {:?}", input);
                            if let Some(new_config) = input.get("config") {
                                let mut config = self.config.write().await;
                                if let Some(v) = new_config.get("api_base").and_then(|v| v.as_str()) {
                                    config.api_base = v.to_string();
                                }
                                if let Some(v) = new_config.get("api_key").and_then(|v| v.as_str()) {
                                    config.api_key = Some(v.to_string());
                                }
                                if let Some(v) = new_config.get("model").and_then(|v| v.as_str()) {
                                    config.model = v.to_string();
                                }
                                if let Some(v) = new_config.get("temperature").and_then(|v| v.as_f64()) {
                                    config.temperature = v as f32;
                                }
                                if let Some(v) = new_config.get("max_tokens").and_then(|v| v.as_u64()) {
                                    config.max_tokens = Some(v as u32);
                                }
                                if let Some(v) = new_config.get("system_prompt").and_then(|v| v.as_str()) {
                                    config.system_prompt = Some(v.to_string());
                                }
                                if let Some(v) = new_config.get("max_context_tokens").and_then(|v| v.as_u64()) {
                                    config.max_context_tokens = v as u32;
                                }
                                eprintln!("[openai] config updated: api_base={}, model={}", config.api_base, config.model);
                            }
                            // 通知父插件保存配置
                            if let Some(parent) = self.get_parent() {
                                let _ = parent.invoke("save_config", json!({}));
                            }
                            Ok(StreamChunk {
                                data: json!({ "success": true }),
                                done: true,
                                error: None,
                            })
                        }
                        _ => Ok(StreamChunk {
                            data: json!({}),
                            done: true,
                            error: Some(format!("未知操作: {}", action)),
                        }),
                    }
                })
            })?;

            return Ok(InvokeStream::Single(result));
        }

        // 原有的 action 处理
        let action = input.get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PluginError::ValidationError("缺少 action 参数".to_string()))?
            .to_string();

        let result = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                match action.as_str() {
                    "chat" => {
                        // 直接返回流式响应
                        self.handle_chat_stream(&input).await
                    }
                    "status" => self.handle_status().await.map(InvokeStream::Single),
                    "list_models" => self.handle_list_models(&input).map(InvokeStream::Single),
                    "configure" => self.handle_configure(&input).await.map(InvokeStream::Single),
                    "get_config" => self.handle_get_config().await.map(InvokeStream::Single),
                    "compress_info" => self.handle_compress_info(&input).await.map(InvokeStream::Single),
                    _ => Ok(InvokeStream::Single(StreamChunk {
                        data: json!({}),
                        done: true,
                        error: Some(format!("未知操作: {}", action)),
                    })),
                }
            })
        });

        result
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 辅助函数
// ═══════════════════════════════════════════════════════════════════════════

fn default_system_prompt() -> String {
    "You are a helpful AI assistant.".into()
}

fn count_message_tokens(counter: &TokenCounter, msg: &NativeMessage) -> usize {
    let mut tokens = 4; // 基础开销
    if let Some(ref content) = msg.content {
        tokens += counter.count_tokens(content);
    }
    if let Some(ref tool_calls) = msg.tool_calls {
        for tc in tool_calls {
            tokens += counter.count_tokens(&tc.function.name);
            tokens += counter.count_tokens(&tc.function.arguments);
            tokens += 4;
        }
    }
    tokens
}

fn count_total_tokens(counter: &TokenCounter, messages: &[NativeMessage], tools: Option<&Vec<NativeToolSpec>>) -> usize {
    let mut total = 0;
    for msg in messages {
        total += count_message_tokens(counter, msg);
    }
    if let Some(tool_list) = tools {
        for tool in tool_list {
            total += counter.count_tokens(&tool.function.name);
            total += counter.count_tokens(&tool.function.description);
            total += counter.count_tokens(&tool.function.parameters.to_string());
            total += 10;
        }
    }
    total
}

fn trim_messages_to_fit(counter: &TokenCounter, messages: &mut Vec<NativeMessage>, config: &ContextConfig, tools: Option<&Vec<NativeToolSpec>>) {
    let available_tokens = config.available_tokens();
    
    loop {
        let current_tokens = count_total_tokens(counter, messages, tools);
        if current_tokens <= available_tokens {
            break;
        }
        
        // 保留系统消息
        let has_system = messages.first().is_some_and(|m| m.role == "system");
        let start_index = if has_system { 1 } else { 0 };
        
        // 移除最旧的非系统消息
        let removable_count = messages.len().saturating_sub(start_index).saturating_sub(1);
        if removable_count == 0 {
            break;
        }
        
        messages.remove(start_index);
    }
}
