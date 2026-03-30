//! Memory 插件实现
//!
//! 提供持久化记忆存储功能

use super::types::MemoryEntry;
use crate::core::traits::Plugin;
use crate::core::types::{PluginMeta, PluginResult, PluginError, InvokeStream, StreamChunk};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Memory 插件
pub struct MemoryPlugin {
    meta: PluginMeta,
    storage_dir: Arc<RwLock<PathBuf>>,
}

impl MemoryPlugin {
    pub fn new(storage_dir: PathBuf) -> Self {
        Self {
            meta: PluginMeta {
                name: "memory".to_string(),
                description: "持久化记忆存储".to_string(),
                version: "0.1.0".to_string(),
                input: Some(json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["store", "recall", "forget", "list", "search"]
                        },
                        "key": { "type": "string" },
                        "content": { "type": "string" },
                        "category": { "type": "string" },
                        "query": { "type": "string" }
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
            .join("memory");
        Self::new(dir)
    }

    async fn ensure_storage_dir(&self) -> Result<PathBuf, PluginError> {
        let dir = self.storage_dir.read().await.clone();
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| PluginError::InternalError(format!("创建存储目录失败: {}", e)))?;
        Ok(dir)
    }
}

impl Default for MemoryPlugin {
    fn default() -> Self {
        Self::default_dir()
    }
}

#[async_trait]
impl Plugin for MemoryPlugin {
    fn meta(&self, _path: &str) -> PluginResult<PluginMeta> {
        Ok(self.meta.clone())
    }

    fn invoke(&self, _path: &str, input: Value) -> PluginResult<InvokeStream> {
        let action = input.get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PluginError::ValidationError("缺少 action 参数".to_string()))?
            .to_string();

        let storage_dir = Arc::clone(&self.storage_dir);

        let stream = async_stream::stream! {
            match action.as_str() {
                "store" => {
                    let key = input.get("key").and_then(|v| v.as_str());
                    let content = input.get("content").and_then(|v| v.as_str());
                    
                    match (key, content) {
                        (Some(key), Some(content)) => {
                            let category = input.get("category").and_then(|v| v.as_str());
                            let entry = MemoryEntry::new(key.to_string(), content.to_string(), category.map(|s| s.to_string()));
                            
                            let dir = storage_dir.read().await.clone();
                            if let Err(e) = tokio::fs::create_dir_all(&dir).await {
                                yield StreamChunk {
                                    data: json!({}),
                                    done: true,
                                    error: Some(format!("创建存储目录失败: {}", e)),
                                };
                                return;
                            }
                            
                            let file_path = dir.join(format!("{}.json", key));
                            let json_str = match serde_json::to_string_pretty(&entry) {
                                Ok(s) => s,
                                Err(e) => {
                                    yield StreamChunk {
                                        data: json!({}),
                                        done: true,
                                        error: Some(format!("序列化失败: {}", e)),
                                    };
                                    return;
                                }
                            };
                            
                            if let Err(e) = tokio::fs::write(&file_path, json_str).await {
                                yield StreamChunk {
                                    data: json!({}),
                                    done: true,
                                    error: Some(format!("写入文件失败: {}", e)),
                                };
                                return;
                            }
                            
                            yield StreamChunk {
                                data: json!({
                                    "success": true,
                                    "key": key,
                                    "message": "记忆已存储"
                                }),
                                done: true,
                                error: None,
                            };
                        }
                        _ => {
                            yield StreamChunk {
                                data: json!({}),
                                done: true,
                                error: Some("缺少 key 或 content 参数".to_string()),
                            };
                        }
                    }
                }
                "recall" => {
                    let key = input.get("key").and_then(|v| v.as_str());
                    
                    match key {
                        Some(key) => {
                            let dir = storage_dir.read().await.clone();
                            let file_path = dir.join(format!("{}.json", key));
                            
                            match tokio::fs::read_to_string(&file_path).await {
                                Ok(json_str) => {
                                    match serde_json::from_str::<MemoryEntry>(&json_str) {
                                        Ok(entry) => {
                                            yield StreamChunk {
                                                data: json!({
                                                    "success": true,
                                                    "entry": entry
                                                }),
                                                done: true,
                                                error: None,
                                            };
                                        }
                                        Err(e) => {
                                            yield StreamChunk {
                                                data: json!({}),
                                                done: true,
                                                error: Some(format!("解析记忆失败: {}", e)),
                                            };
                                        }
                                    }
                                }
                                Err(_) => {
                                    yield StreamChunk {
                                        data: json!({}),
                                        done: true,
                                        error: Some(format!("未找到记忆: {}", key)),
                                    };
                                }
                            }
                        }
                        None => {
                            yield StreamChunk {
                                data: json!({}),
                                done: true,
                                error: Some("缺少 key 参数".to_string()),
                            };
                        }
                    }
                }
                "forget" => {
                    let key = input.get("key").and_then(|v| v.as_str());
                    
                    match key {
                        Some(key) => {
                            let dir = storage_dir.read().await.clone();
                            let file_path = dir.join(format!("{}.json", key));
                            
                            match tokio::fs::remove_file(&file_path).await {
                                Ok(_) => {
                                    yield StreamChunk {
                                        data: json!({
                                            "success": true,
                                            "key": key,
                                            "message": "记忆已删除"
                                        }),
                                        done: true,
                                        error: None,
                                    };
                                }
                                Err(_) => {
                                    yield StreamChunk {
                                        data: json!({}),
                                        done: true,
                                        error: Some(format!("未找到记忆: {}", key)),
                                    };
                                }
                            }
                        }
                        None => {
                            yield StreamChunk {
                                data: json!({}),
                                done: true,
                                error: Some("缺少 key 参数".to_string()),
                            };
                        }
                    }
                }
                "list" => {
                    let dir = storage_dir.read().await.clone();
                    
                    match tokio::fs::read_dir(&dir).await {
                        Ok(mut entries) => {
                            let mut memories = Vec::new();
                            while let Ok(Some(entry)) = entries.next_entry().await {
                                if entry.path().extension().map(|e| e == "json").unwrap_or(false) {
                                    if let Ok(content) = tokio::fs::read_to_string(entry.path()).await {
                                        if let Ok(mem) = serde_json::from_str::<MemoryEntry>(&content) {
                                            memories.push(json!({
                                                "key": mem.key,
                                                "category": mem.category,
                                                "created_at": mem.created_at
                                            }));
                                        }
                                    }
                                }
                            }
                            
                            yield StreamChunk {
                                data: json!({
                                    "success": true,
                                    "memories": memories,
                                    "count": memories.len()
                                }),
                                done: true,
                                error: None,
                            };
                        }
                        Err(e) => {
                            yield StreamChunk {
                                data: json!({}),
                                done: true,
                                error: Some(format!("读取目录失败: {}", e)),
                            };
                        }
                    }
                }
                "search" => {
                    let query = input.get("query").and_then(|v| v.as_str());
                    
                    match query {
                        Some(query) => {
                            let dir = storage_dir.read().await.clone();
                            let query_lower = query.to_lowercase();
                            
                            match tokio::fs::read_dir(&dir).await {
                                Ok(mut entries) => {
                                    let mut results = Vec::new();
                                    while let Ok(Some(entry)) = entries.next_entry().await {
                                        if entry.path().extension().map(|e| e == "json").unwrap_or(false) {
                                            if let Ok(content) = tokio::fs::read_to_string(entry.path()).await {
                                                if let Ok(mem) = serde_json::from_str::<MemoryEntry>(&content) {
                                                    if mem.content.to_lowercase().contains(&query_lower)
                                                        || mem.key.to_lowercase().contains(&query_lower)
                                                    {
                                                        results.push(json!({
                                                            "key": mem.key,
                                                            "content": mem.content,
                                                            "category": mem.category,
                                                            "relevance": "match"
                                                        }));
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    
                                    yield StreamChunk {
                                        data: json!({
                                            "success": true,
                                            "results": results,
                                            "query": query,
                                            "count": results.len()
                                        }),
                                        done: true,
                                        error: None,
                                    };
                                }
                                Err(e) => {
                                    yield StreamChunk {
                                        data: json!({}),
                                        done: true,
                                        error: Some(format!("读取目录失败: {}", e)),
                                    };
                                }
                            }
                        }
                        None => {
                            yield StreamChunk {
                                data: json!({}),
                                done: true,
                                error: Some("缺少 query 参数".to_string()),
                            };
                        }
                    }
                }
                _ => {
                    yield StreamChunk {
                        data: json!({}),
                        done: true,
                        error: Some(format!("未知操作: {}", action)),
                    };
                }
            }
        };

        Ok(InvokeStream::Stream(Box::pin(stream)))
    }
}
