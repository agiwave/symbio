//! 任务清单工具 - 实现 Tool trait（等价于 Trae 的 TodoWrite）
//!
//! 以会话（session_id）为作用域维护一份结构化任务清单，
//! 帮助 Agent 跟踪复杂多步任务的进度。纯内存状态，不落盘。

use super::policy::SecurityPolicy;
use crate::symbio_core::{
    Capability, CapabilityMeta, InvokeRequest, InvokeRequestExt, InvokeResponse, PluginError,
    PluginPayload, AGENT_ID, SESSION_ID, WORKDIR,
};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{OnceCell, RwLock};

/// 会话级任务清单存储：session_id -> 任务数组
static TODO_STORE: OnceCell<RwLock<HashMap<String, Vec<Value>>>> = OnceCell::const_new();

async fn todo_store() -> &'static RwLock<HashMap<String, Vec<Value>>> {
    TODO_STORE
        .get_or_init(|| async { RwLock::new(HashMap::new()) })
        .await
}

/// 任务清单工具
#[derive(Clone)]
pub struct TodoWriteTool {
    #[allow(dead_code)]
    security: Arc<SecurityPolicy>,
}

impl TodoWriteTool {
    pub fn new(_security: Arc<SecurityPolicy>) -> Self {
        Self {
            security: _security,
        }
    }

    async fn execute_inner(&self, args: &Value, key: &str) -> InvokeResponse<Value> {
        let todos = args
            .get("todos")
            .and_then(|v| v.as_array())
            .ok_or_else(|| PluginError::ValidationError("缺少 'todos' 数组参数".to_string()))?;
        let merge = args.get("merge").and_then(|v| v.as_bool()).unwrap_or(false);

        let mut items: Vec<Value> = Vec::with_capacity(todos.len());
        for (i, t) in todos.iter().enumerate() {
            let content = t
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let status = t
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("pending")
                .to_string();
            let priority = t
                .get("priority")
                .and_then(|v| v.as_str())
                .unwrap_or("medium")
                .to_string();
            let id = t
                .get("id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("task_{}", i + 1));
            if content.is_empty() {
                return Err(PluginError::ValidationError(format!(
                    "第 {} 项任务缺少 content",
                    i + 1
                )));
            }
            items.push(json!({
                "id": id,
                "content": content,
                "status": status,
                "priority": priority,
            }));
        }

        let store = todo_store().await;
        let mut map = store.write().await;
        if merge {
            let entry = map.entry(key.to_string()).or_default();
            for it in items {
                let id = it["id"].as_str().unwrap_or("").to_string();
                if let Some(pos) = entry
                    .iter()
                    .position(|e| e["id"].as_str() == Some(id.as_str()))
                {
                    entry[pos] = it;
                } else {
                    entry.push(it);
                }
            }
        } else {
            map.insert(key.to_string(), items);
        }
        let current = map.get(key).cloned().unwrap_or_default();

        let mut md = String::from("## 任务清单\n\n");
        for it in &current {
            let mark = match it["status"].as_str() {
                Some("completed") => "x",
                Some("in_progress") => ">",
                _ => " ",
            };
            let pri = it["priority"].as_str().unwrap_or("medium");
            let content = it["content"].as_str().unwrap_or("");
            md.push_str(&format!("- [{}] ({}) {}\n", mark, pri, content));
        }

        let summary = args.get("summary").and_then(|v| v.as_str()).unwrap_or("");
        let message = if summary.is_empty() {
            format!("已更新任务清单，共 {} 项。", current.len())
        } else {
            summary.to_string()
        };

        Ok(json!({
            "success": true,
            "count": current.len(),
            "todos": current,
            "markdown": md,
            "message": message,
        }))
    }
}

#[async_trait]
impl Capability for TodoWriteTool {
    fn meta(&self) -> CapabilityMeta {
        CapabilityMeta {
            name: "todo_write".to_string(),
            description:
                "管理结构化任务清单，用于跟踪复杂多步任务进度。todos 整体替换清单；merge=true 时按 id 合并。以会话为作用域。"
                    .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "todos": {
                        "type": "array",
                        "description": "任务项数组；merge=false 时整体替换，merge=true 时按 id 合并",
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": { "type": "string", "description": "任务唯一标识（省略则自动生成 task_N）" },
                                "content": { "type": "string", "description": "任务描述" },
                                "status": { "type": "string", "enum": ["pending", "in_progress", "completed"], "description": "状态" },
                                "priority": { "type": "string", "enum": ["high", "medium", "low"], "description": "优先级" }
                            },
                            "required": ["content"]
                        }
                    },
                    "merge": { "type": "boolean", "description": "true=按 id 合并进已有清单；false=整体替换（默认）" },
                    "summary": { "type": "string", "description": "可选：完成时的用户可见摘要" }
                },
                "required": ["todos"]
            }),
            category: Some(crate::symbio_core::CapabilityCategory::SystemOperation),
            examples: Some(vec![
                "todos=[{content:'分析架构',status:'in_progress',priority:'high'}]".to_string(),
            ]),
            ..Default::default()
        }
    }

    async fn execute(&self, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload> {
        let args: Value = ctx.payload()?;
        let session = ctx.get(SESSION_ID).unwrap_or_default();
        let wd = ctx.get(WORKDIR).unwrap_or_default();
        let aid = ctx.get(AGENT_ID).unwrap_or_default();
        let key = if session.is_empty() {
            format!("{}::{}", wd, aid)
        } else {
            session
        };
        let data = self.execute_inner(&args, &key).await?;
        Ok(PluginPayload::new(&data))
    }
}
