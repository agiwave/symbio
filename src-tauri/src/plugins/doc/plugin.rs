//! Doc 插件 - 文档管理（持久化存储）

use crate::core::traits::Plugin;
use crate::core::types::{PluginMeta, PluginResult, PluginError, InvokeStream};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Weak};
use tokio::fs;
use uuid::Uuid;
use chrono::{DateTime, Utc};

/// 文档结构
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    pub id: String,
    pub title: String,
    pub content: String,
    pub parent_id: Option<String>,
    pub children: Vec<String>,
    pub order: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 文档存储
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentStore {
    pub documents: std::collections::HashMap<String, Document>,
    pub root_ids: Vec<String>,
}

impl DocumentStore {
    pub fn new() -> Self {
        DocumentStore {
            documents: std::collections::HashMap::new(),
            root_ids: Vec::new(),
        }
    }
}

pub struct DocPlugin {
    meta: PluginMeta,
    store: Arc<Mutex<DocumentStore>>,
    data_path: PathBuf,
    /// 父插件引用（用于获取工作区路径）
    parent: Option<Weak<dyn Plugin>>,
}

impl DocPlugin {
    fn create_meta() -> PluginMeta {
        PluginMeta {
            name: "doc".to_string(),
            description: "文档管理插件".to_string(),
            version: "0.1.0".to_string(),
            input: Some(json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["list", "create", "get", "update", "delete", "init", "save"],
                        "description": "操作类型"
                    },
                    "id": { "type": "string" },
                    "title": { "type": "string" },
                    "content": { "type": "string" },
                    "parentId": { "type": "string" }
                }
            })),
            output: Some(json!({
                "type": "object",
                "properties": {
                    "success": { "type": "boolean" },
                    "data": { "type": "object" },
                    "message": { "type": "string" }
                }
            })),
            author: Some("Symbio Team".to_string()),
        }
    }

    /// 主构造函数（Factory 机制使用）
    pub fn new(parent: Option<Weak<dyn Plugin>>) -> Self {
        // 获取应用数据目录
        let data_dir = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("symbio");
        
        let data_path = data_dir.join("documents.json");

        // 初始化存储
        let store = Arc::new(Mutex::new(DocumentStore::new()));

        DocPlugin { 
            meta: Self::create_meta(),
            store, 
            data_path,
            parent,
        }
    }

    /// 获取父插件引用
    fn get_parent(&self) -> Option<Arc<dyn Plugin>> {
        self.parent.as_ref().and_then(|w| w.upgrade())
    }

    /// 加载数据
    async fn load_data(&self) -> Result<(), PluginError> {
        if self.data_path.exists() {
            let content = fs::read_to_string(&self.data_path)
                .await
                .map_err(|e| PluginError::InternalError(format!("读取数据失败: {}", e)))?;
            
            let store: DocumentStore = serde_json::from_str(&content)
                .map_err(|e| PluginError::InternalError(format!("解析数据失败: {}", e)))?;
            
            let mut s = self.store.lock().map_err(|e| PluginError::InternalError(e.to_string()))?;
            *s = store;
        }
        Ok(())
    }

    /// 保存数据
    async fn save_data(&self) -> Result<(), PluginError> {
        // 确保目录存在
        if let Some(parent) = self.data_path.parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|e| PluginError::InternalError(format!("创建目录失败: {}", e)))?;
        }

        let s = self.store.lock().map_err(|e| PluginError::InternalError(e.to_string()))?;
        let content = serde_json::to_string_pretty(&*s)
            .map_err(|e| PluginError::InternalError(format!("序列化数据失败: {}", e)))?;
        
        fs::write(&self.data_path, content)
            .await
            .map_err(|e| PluginError::InternalError(format!("写入数据失败: {}", e)))?;
        
        Ok(())
    }

    /// 同步保存数据（用于同步上下文）
    fn save_data_sync(&self) -> Result<(), PluginError> {
        if let Some(parent) = self.data_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let s = self.store.lock().map_err(|e| PluginError::InternalError(e.to_string()))?;
        let content = serde_json::to_string_pretty(&*s)
            .map_err(|e| PluginError::InternalError(format!("序列化数据失败: {}", e)))?;
        
        std::fs::write(&self.data_path, content)
            .map_err(|e| PluginError::InternalError(format!("写入数据失败: {}", e)))?;
        
        Ok(())
    }

    /// 同步加载数据
    fn load_data_sync(&self) -> Result<(), PluginError> {
        if self.data_path.exists() {
            let content = std::fs::read_to_string(&self.data_path)
                .map_err(|e| PluginError::InternalError(format!("读取数据失败: {}", e)))?;
            
            let store: DocumentStore = serde_json::from_str(&content)
                .map_err(|e| PluginError::InternalError(format!("解析数据失败: {}", e)))?;
            
            let mut s = self.store.lock().map_err(|e| PluginError::InternalError(e.to_string()))?;
            *s = store;
        }
        Ok(())
    }

    /// 列出文档
    fn list_documents(&self) -> Value {
        let s = self.store.lock().unwrap();
        let docs: Vec<Value> = s.root_ids.iter()
            .filter_map(|id| s.documents.get(id))
            .map(|doc| json!({
                "id": doc.id,
                "title": doc.title,
                "parentId": doc.parent_id
            }))
            .collect();
        
        json!({
            "success": true,
            "data": { "documents": docs }
        })
    }

    /// 创建文档
    fn create_document(&self, title: &str, parent_id: Option<&str>) -> Value {
        let mut s = self.store.lock().unwrap();
        
        let now = Utc::now();
        let id = Uuid::new_v4().to_string();
        
        // 计算排序值
        let siblings: Vec<&Document> = s.documents.values()
            .filter(|d| d.parent_id.as_deref() == parent_id)
            .collect();
        let order = siblings.len() as i32 + 1;

        let doc = Document {
            id: id.clone(),
            title: title.to_string(),
            content: String::new(),
            parent_id: parent_id.map(|s| s.to_string()),
            children: Vec::new(),
            order,
            created_at: now,
            updated_at: now,
        };

        // 更新父文档的 children
        if let Some(pid) = parent_id {
            if let Some(parent) = s.documents.get_mut(pid) {
                parent.children.push(id.clone());
            }
        } else {
            s.root_ids.push(id.clone());
        }

        s.documents.insert(id.clone(), doc);

        json!({
            "success": true,
            "data": {
                "id": id,
                "title": title,
                "parentId": parent_id,
                "content": ""
            },
            "message": "文档创建成功"
        })
    }

    /// 获取文档
    fn get_document(&self, id: &str) -> Result<Value, PluginError> {
        let s = self.store.lock().unwrap();
        
        let doc = s.documents.get(id)
            .ok_or_else(|| PluginError::NotFound(format!("文档不存在: {}", id)))?;

        Ok(json!({
            "success": true,
            "data": {
                "id": doc.id,
                "title": doc.title,
                "content": doc.content,
                "parentId": doc.parent_id,
                "children": doc.children
            }
        }))
    }

    /// 更新文档
    fn update_document(&self, id: &str, updates: &Value) -> Result<Value, PluginError> {
        let mut s = self.store.lock().unwrap();
        
        let doc = s.documents.get_mut(id)
            .ok_or_else(|| PluginError::NotFound(format!("文档不存在: {}", id)))?;

        if let Some(title) = updates.get("title").and_then(|v| v.as_str()) {
            doc.title = title.to_string();
        }
        if let Some(content) = updates.get("content").and_then(|v| v.as_str()) {
            doc.content = content.to_string();
        }
        
        doc.updated_at = Utc::now();

        Ok(json!({
            "success": true,
            "data": { "id": id },
            "message": "文档更新成功"
        }))
    }

    /// 删除文档
    fn delete_document(&self, id: &str) -> Result<Value, PluginError> {
        let mut s = self.store.lock().unwrap();
        
        let doc = s.documents.get(id)
            .ok_or_else(|| PluginError::NotFound(format!("文档不存在: {}", id)))?;

        let parent_id = doc.parent_id.clone();
        let children = doc.children.clone();

        // 递归删除子文档
        fn delete_children(
            store: &mut std::collections::HashMap<String, Document>,
            children: &[String],
        ) {
            for child_id in children {
                if let Some(child) = store.remove(child_id) {
                    delete_children(store, &child.children);
                }
            }
        }

        delete_children(&mut s.documents, &children);

        // 从父文档或根列表中移除
        if let Some(pid) = parent_id {
            if let Some(parent) = s.documents.get_mut(&pid) {
                parent.children.retain(|cid| cid != id);
            }
        } else {
            s.root_ids.retain(|rid| rid != id);
        }

        s.documents.remove(id);

        Ok(json!({
            "success": true,
            "data": { "id": id },
            "message": "文档删除成功"
        }))
    }

    /// 初始化示例数据
    fn init_demo(&self) {
        let mut s = self.store.lock().unwrap();
        
        if !s.documents.is_empty() {
            return;
        }

        let now = Utc::now();

        // 创建示例文档
        let root_id = Uuid::new_v4().to_string();
        let design_id = Uuid::new_v4().to_string();
        let qc_id = Uuid::new_v4().to_string();

        let root = Document {
            id: root_id.clone(),
            title: "RNA-seq 差异表达分析".to_string(),
            content: "# RNA-seq 差异表达分析\n\n这是一个完整的 RNA-seq 分析流程示例。".to_string(),
            parent_id: None,
            children: vec![design_id.clone(), qc_id.clone()],
            order: 1,
            created_at: now,
            updated_at: now,
        };

        let design = Document {
            id: design_id.clone(),
            title: "实验设计".to_string(),
            content: "## 实验设计\n\n描述实验组和对照组的设置。".to_string(),
            parent_id: Some(root_id.clone()),
            children: vec![],
            order: 1,
            created_at: now,
            updated_at: now,
        };

        let qc = Document {
            id: qc_id.clone(),
            title: "数据预处理".to_string(),
            content: "## FastQC 质控\n\n```bash\nfastqc *.fastq.gz -o qc_results\n```".to_string(),
            parent_id: Some(root_id.clone()),
            children: vec![],
            order: 2,
            created_at: now,
            updated_at: now,
        };

        s.documents.insert(root_id.clone(), root);
        s.documents.insert(design_id, design);
        s.documents.insert(qc_id, qc);
        s.root_ids.push(root_id);
    }
}

impl Default for DocPlugin {
    fn default() -> Self {
        Self::new(None)
    }
}

#[async_trait::async_trait]
impl Plugin for DocPlugin {
    fn meta(&self, path: &str) -> PluginResult<PluginMeta> {
        if path == "config" {
            return Ok(PluginMeta {
                name: "config".to_string(),
                description: "Doc 配置管理".to_string(),
                version: "0.1.0".to_string(),
                input: Some(json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["get", "set", "schema"],
                            "description": "操作类型"
                        }
                    },
                    "required": ["action"]
                })),
                output: Some(json!({
                    "type": "object",
                    "properties": {
                        "success": { "type": "boolean" }
                    }
                })),
                author: Some("Symbio Team".to_string()),
            });
        }
        Ok(self.meta.clone())
    }

    fn invoke(&self, path: &str, input: Value) -> PluginResult<InvokeStream> {
        // 处理 config path
        if path == "config" {
            let action = input.get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("get");
            
            let result = match action {
                "get" => json!({ "success": true }),
                "schema" => json!({ "success": true, "schema": {} }),
                _ => json!({ "success": true }),
            };

            return Ok(InvokeStream::single(result));
        }

        let action = input.get("action").and_then(|v| v.as_str()).unwrap_or("list");

        let result = match action {
            "init" => {
                // 初始化并加载数据
                if self.data_path.exists() {
                    let _ = self.load_data_sync();
                } else {
                    self.init_demo();
                    let _ = self.save_data_sync();
                }
                json!({ "success": true, "message": "初始化完成" })
            }
            "list" => self.list_documents(),
            "create" => {
                let title = input.get("title").and_then(|v| v.as_str()).unwrap_or("新文档");
                let parent_id = input.get("parentId").and_then(|v| v.as_str());
                let result = self.create_document(title, parent_id);
                let _ = self.save_data_sync();
                result
            }
            "get" => {
                let id = input.get("id").and_then(|v| v.as_str())
                    .ok_or_else(|| PluginError::ValidationError("缺少 id 参数".to_string()))?;
                self.get_document(id)?
            }
            "update" => {
                let id = input.get("id").and_then(|v| v.as_str())
                    .ok_or_else(|| PluginError::ValidationError("缺少 id 参数".to_string()))?;
                let result = self.update_document(id, &input)?;
                let _ = self.save_data_sync();
                result
            }
            "delete" => {
                let id = input.get("id").and_then(|v| v.as_str())
                    .ok_or_else(|| PluginError::ValidationError("缺少 id 参数".to_string()))?;
                let result = self.delete_document(id)?;
                let _ = self.save_data_sync();
                result
            }
            "save" => {
                let _ = self.save_data_sync();
                json!({ "success": true, "message": "保存成功" })
            }
            _ => return Err(PluginError::ValidationError(format!("未知操作: {}", action))),
        };

        Ok(InvokeStream::single(result))
    }
}
