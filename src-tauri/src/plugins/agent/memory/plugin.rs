//! Memory 插件实现
//!
//! 提供持久化记忆存储功能

use super::types::MemoryEntry;
use crate::core::traits::Plugin;
use crate::core::types::{PluginMeta, PluginResult, PluginError, InvokeStream, StreamChunk};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::{Arc, Weak};
use tokio::sync::RwLock;

/// Memory 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConfig {
    /// 存储目录
    #[serde(default = "default_storage_dir")]
    pub storage_dir: String,
    /// 最大条目数
    #[serde(default = "default_max_entries")]
    pub max_entries: usize,
    /// 预定义分类
    #[serde(default = "default_categories")]
    pub categories: Vec<String>,
}

fn default_storage_dir() -> String { 
    dirs::data_local_dir()
        .map(|p| p.join("symbio").join("memory").to_string_lossy().to_string())
        .unwrap_or_else(|| "~/.local/share/symbio/memory".to_string())
}
fn default_max_entries() -> usize { 1000 }
fn default_categories() -> Vec<String> { 
    vec!["preference".to_string(), "fact".to_string(), "instruction".to_string()] 
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            storage_dir: default_storage_dir(),
            max_entries: default_max_entries(),
            categories: default_categories(),
        }
    }
}

/// Memory 插件
pub struct MemoryPlugin {
    meta: PluginMeta,
    config: Arc<RwLock<MemoryConfig>>,
    storage_dir: Arc<RwLock<PathBuf>>,
    /// 父插件引用（用于保存配置）
    parent: Option<Weak<dyn Plugin>>,
}

impl MemoryPlugin {
    fn create_meta() -> PluginMeta {
        PluginMeta {
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
        }
    }

    /// 主构造函数（Factory 机制使用）
    pub fn new(parent: Option<Weak<dyn Plugin>>, config: MemoryConfig) -> Self {
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
                "description": "记忆数据存储目录",
                "default": default_storage_dir()
            },
            "max_entries": {
                "type": "integer",
                "title": "最大条目数",
                "description": "存储的最大记忆条目数量",
                "minimum": 100,
                "maximum": 10000,
                "default": 1000
            },
            "categories": {
                "type": "array",
                "title": "预定义分类",
                "description": "记忆的预定义分类列表",
                "items": { "type": "string" },
                "default": ["preference", "fact", "instruction"]
            }
        })
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
        Self::new(None, MemoryConfig::default())
    }
}

#[async_trait]
impl Plugin for MemoryPlugin {
    fn meta(&self, path: &str) -> PluginResult<PluginMeta> {
        if path == "config" {
            return Ok(PluginMeta {
                name: "config".to_string(),
                description: "Memory 配置管理".to_string(),
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

    fn available_tools(&self) -> Vec<PluginMeta> {
        vec![
            PluginMeta {
                name: "store".to_string(),
                description: "存储记忆（将键值对保存到记忆存储）".to_string(),
                version: "0.1.0".to_string(),
                input: Some(json!({
                    "type": "object",
                    "properties": {
                        "key": {
                            "type": "string",
                            "description": "记忆的唯一标识键"
                        },
                        "content": {
                            "type": "string",
                            "description": "记忆的内容"
                        },
                        "category": {
                            "type": "string",
                            "description": "记忆的分类（可选）"
                        }
                    },
                    "required": ["key", "content"]
                })),
                output: Some(json!({
                    "type": "object",
                    "properties": {
                        "success": { "type": "boolean" },
                        "key": { "type": "string" },
                        "message": { "type": "string" }
                    }
                })),
                author: Some("Symbio Team".to_string()),
            },
            PluginMeta {
                name: "recall".to_string(),
                description: "回忆记忆（根据键名检索记忆）".to_string(),
                version: "0.1.0".to_string(),
                input: Some(json!({
                    "type": "object",
                    "properties": {
                        "key": {
                            "type": "string",
                            "description": "要检索的记忆键"
                        }
                    },
                    "required": ["key"]
                })),
                output: Some(json!({
                    "type": "object",
                    "properties": {
                        "success": { "type": "boolean" },
                        "entry": { "type": "object" }
                    }
                })),
                author: Some("Symbio Team".to_string()),
            },
            PluginMeta {
                name: "forget".to_string(),
                description: "忘记记忆（删除指定的记忆）".to_string(),
                version: "0.1.0".to_string(),
                input: Some(json!({
                    "type": "object",
                    "properties": {
                        "key": {
                            "type": "string",
                            "description": "要删除的记忆键"
                        }
                    },
                    "required": ["key"]
                })),
                output: Some(json!({
                    "type": "object",
                    "properties": {
                        "success": { "type": "boolean" },
                        "key": { "type": "string" },
                        "message": { "type": "string" }
                    }
                })),
                author: Some("Symbio Team".to_string()),
            },
            PluginMeta {
                name: "list".to_string(),
                description: "列出所有记忆（显示所有已存储的记忆摘要）".to_string(),
                version: "0.1.0".to_string(),
                input: Some(json!({
                    "type": "object",
                    "properties": {}
                })),
                output: Some(json!({
                    "type": "object",
                    "properties": {
                        "success": { "type": "boolean" },
                        "memories": { "type": "array" },
                        "count": { "type": "integer" }
                    }
                })),
                author: Some("Symbio Team".to_string()),
            },
            PluginMeta {
                name: "search".to_string(),
                description: "搜索记忆（根据查询内容搜索记忆库）".to_string(),
                version: "0.1.0".to_string(),
                input: Some(json!({
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "搜索查询词"
                        }
                    },
                    "required": ["query"]
                })),
                output: Some(json!({
                    "type": "object",
                    "properties": {
                        "success": { "type": "boolean" },
                        "results": { "type": "array" },
                        "query": { "type": "string" },
                        "count": { "type": "integer" }
                    }
                })),
                author: Some("Symbio Team".to_string()),
            },
        ]
    }

    fn invoke(&self, path: &str, input: Value) -> PluginResult<InvokeStream> {
        // 处理 available_tools path - 返回所有记忆工具的 meta（通用接口）
        if path == "available_tools" {
            let tools = self.available_tools();
            return Ok(InvokeStream::single(json!({
                "success": true,
                "tools": tools
            })));
        }

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
                                    "max_entries": cfg.max_entries,
                                    "categories": cfg.categories
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
                                    let mut dir = storage_dir.write().await;
                                    *dir = PathBuf::from(v);
                                }
                                if let Some(v) = new_config.get("max_entries").and_then(|v| v.as_u64()) {
                                    cfg.max_entries = v as usize;
                                }
                                if let Some(v) = new_config.get("categories").and_then(|v| v.as_array()) {
                                    cfg.categories = v.iter()
                                        .filter_map(|s| s.as_str().map(|s| s.to_string()))
                                        .collect();
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

        // 工具调用路由：当 path 是工具名称时，直接执行对应工具
        // 这样 LLM 调用 memory/store 时，path="store"，直接执行 store 操作
        let action = if path.is_empty() {
            // 空路径时，从 input 中获取 action（兼容旧格式）
            input.get("action")
                .and_then(|v| v.as_str())
                .ok_or_else(|| PluginError::ValidationError("缺少 action 参数".to_string()))?
                .to_string()
        } else {
            // 非空路径，path 就是工具名称
            path.to_string()
        };

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_plugin_available_tools() {
        // 创建 MemoryPlugin 实例
        let memory_plugin = MemoryPlugin::new(None, MemoryConfig::default());

        // 调用 available_tools 方法
        let tools = memory_plugin.available_tools();

        // 验证返回的工具列表不为空
        assert!(!tools.is_empty(), "MemoryPlugin should return non-empty tools list");

        // 打印所有工具名称
        println!("MemoryPlugin available tools ({} total):", tools.len());
        for tool in &tools {
            println!("  - {} ({})", tool.name, tool.description);
        }

        // 验证包含预期的工具
        let tool_names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        
        // 验证所有记忆工具存在（子插件返回原始名称，由父插件添加前缀）
        assert!(tool_names.contains(&"store"), "Should have store tool");
        assert!(tool_names.contains(&"recall"), "Should have recall tool");
        assert!(tool_names.contains(&"forget"), "Should have forget tool");
        assert!(tool_names.contains(&"list"), "Should have list tool");
        assert!(tool_names.contains(&"search"), "Should have search tool");

        // 验证工具数量为 5
        assert_eq!(tools.len(), 5, "MemoryPlugin should have exactly 5 tools");
    }
}
