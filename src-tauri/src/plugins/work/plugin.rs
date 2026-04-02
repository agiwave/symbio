//! Work 插件 - 工作区管理（持久化存储）

use crate::core::traits::Plugin;
use crate::core::types::{PluginMeta, PluginResult, PluginError, InvokeStream};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Weak};
use tokio::fs;
use uuid::Uuid;
use chrono::{DateTime, Utc};

/// Work 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkConfig {
    /// 工作区路径
    #[serde(default = "default_workspace_path")]
    pub workspace_path: String,
    /// 自动保存
    #[serde(default = "default_auto_save")]
    pub auto_save: bool,
    /// 自动保存间隔（毫秒）
    #[serde(default = "default_auto_save_interval")]
    pub auto_save_interval: u64,
    /// 最近文件列表
    #[serde(default)]
    pub recent_files: Vec<String>,
}

fn default_workspace_path() -> String { 
    dirs::home_dir()
        .map(|p| p.join("projects").to_string_lossy().to_string())
        .unwrap_or_else(|| "~/projects".to_string())
}
fn default_auto_save() -> bool { true }
fn default_auto_save_interval() -> u64 { 30000 }

impl Default for WorkConfig {
    fn default() -> Self {
        Self {
            workspace_path: default_workspace_path(),
            auto_save: default_auto_save(),
            auto_save_interval: default_auto_save_interval(),
            recent_files: Vec::new(),
        }
    }
}

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

pub struct WorkPlugin {
    meta: PluginMeta,
    config: Arc<Mutex<WorkConfig>>,
    store: Arc<Mutex<DocumentStore>>,
    data_path: PathBuf,
    /// 父插件引用（用于保存配置）
    parent: Option<Weak<dyn Plugin>>,
}

impl WorkPlugin {
    fn create_meta() -> PluginMeta {
        PluginMeta {
            name: "work".to_string(),
            description: "工作区管理插件".to_string(),
            version: "0.1.0".to_string(),
            input: Some(json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["list", "create", "get", "update", "delete", "init", "save", "workspace_path", "get_workspace", "set_workspace"],
                        "description": "操作类型"
                    },
                    "id": { "type": "string" },
                    "title": { "type": "string" },
                    "content": { "type": "string" },
                    "parentId": { "type": "string" },
                    "path": { "type": "string", "description": "工作区路径（set_workspace 时使用）" }
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
        
        let data_path = data_dir.join("workspace.json");

        // 初始化存储
        let store = Arc::new(Mutex::new(DocumentStore::new()));
        let config = Arc::new(Mutex::new(WorkConfig::default()));

        WorkPlugin { 
            meta: Self::create_meta(),
            config,
            store, 
            data_path,
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
            "workspace_path": {
                "type": "string",
                "title": "工作区路径",
                "description": "默认工作区目录",
                "default": default_workspace_path()
            },
            "auto_save": {
                "type": "boolean",
                "title": "自动保存",
                "description": "自动保存编辑内容",
                "default": true
            },
            "auto_save_interval": {
                "type": "integer",
                "title": "自动保存间隔",
                "description": "自动保存间隔时间（毫秒）",
                "minimum": 1000,
                "maximum": 300000,
                "default": 30000
            },
            "recent_files": {
                "type": "array",
                "title": "最近文件",
                "description": "最近打开的文件列表",
                "items": { "type": "string" }
            }
        })
    }

    /// 加载数据（保留用于将来扩展）
    #[allow(dead_code)]
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

    /// 保存数据（保留用于将来扩展）
    #[allow(dead_code)]
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
            content: "## FastQC 质控\n\n```bash run\nfastqc *.fastq.gz -o qc_results\n```".to_string(),
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

impl Default for WorkPlugin {
    fn default() -> Self {
        Self::new(None)
    }
}

#[async_trait::async_trait]
impl Plugin for WorkPlugin {
    fn meta(&self, path: &str) -> PluginResult<PluginMeta> {
        if path == "config" {
            return Ok(PluginMeta {
                name: "config".to_string(),
                description: "Work 配置管理".to_string(),
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

    fn invoke(&self, path: &str, input: Value) -> PluginResult<InvokeStream> {
        // 处理 config path
        if path == "config" {
            let action = input.get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("get");
            
            let config = Arc::clone(&self.config);
            let parent = self.get_parent();

            let result = match action {
                "get" => {
                    let cfg = config.lock().unwrap();
                    InvokeStream::single(json!({
                        "workspace_path": cfg.workspace_path,
                        "auto_save": cfg.auto_save,
                        "auto_save_interval": cfg.auto_save_interval,
                        "recent_files": cfg.recent_files
                    }))
                }
                "set" => {
                    if let Some(new_config) = input.get("config") {
                        let mut cfg = config.lock().unwrap();
                        if let Some(v) = new_config.get("workspace_path").and_then(|v| v.as_str()) {
                            cfg.workspace_path = v.to_string();
                        }
                        if let Some(v) = new_config.get("auto_save").and_then(|v| v.as_bool()) {
                            cfg.auto_save = v;
                        }
                        if let Some(v) = new_config.get("auto_save_interval").and_then(|v| v.as_u64()) {
                            cfg.auto_save_interval = v;
                        }
                        if let Some(v) = new_config.get("recent_files").and_then(|v| v.as_array()) {
                            cfg.recent_files = v.iter()
                                .filter_map(|s| s.as_str().map(|s| s.to_string()))
                                .collect();
                        }
                    }
                    // 通知父插件保存配置
                    if let Some(p) = parent {
                        let _ = p.invoke("save_config", json!({}));
                    }
                    InvokeStream::single(json!({ "success": true }))
                }
                "schema" => {
                    InvokeStream::single(json!({
                        "success": true,
                        "schema": Self::config_schema()
                    }))
                }
                _ => InvokeStream::single(json!({
                    "error": format!("未知操作: {}", action)
                })),
            };

            return Ok(result);
        }

        let action = input.get("action").and_then(|v| v.as_str()).unwrap_or("list");

        // 对于需要异步加载的操作，使用同步方式处理
        // 在实际应用中，应该在初始化时加载数据
        let result = match action {
            "init" => {
                // 初始化并加载数据
                // 注意：这里使用同步版本，因为 invoke 是同步的
                // 实际生产环境应该使用 async
                if self.data_path.exists() {
                    if let Ok(content) = std::fs::read_to_string(&self.data_path) {
                        if let Ok(store) = serde_json::from_str::<DocumentStore>(&content) {
                            let mut s = self.store.lock().unwrap();
                            *s = store;
                        }
                    }
                } else {
                    self.init_demo();
                    // 保存初始数据
                    if let Ok(s) = self.store.lock() {
                        if let Some(parent) = self.data_path.parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        if let Ok(content) = serde_json::to_string_pretty(&*s) {
                            let _ = std::fs::write(&self.data_path, content);
                        }
                    }
                }
                json!({ "success": true, "message": "初始化完成" })
            }
            "list" => self.list_documents(),
            "create" => {
                let title = input.get("title").and_then(|v| v.as_str()).unwrap_or("新文档");
                let parent_id = input.get("parentId").and_then(|v| v.as_str());
                let result = self.create_document(title, parent_id);
                
                // 保存数据
                if let Ok(s) = self.store.lock() {
                    if let Some(parent) = self.data_path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    if let Ok(content) = serde_json::to_string_pretty(&*s) {
                        let _ = std::fs::write(&self.data_path, content);
                    }
                }
                
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
                
                // 保存数据
                if let Ok(s) = self.store.lock() {
                    if let Some(parent) = self.data_path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    if let Ok(content) = serde_json::to_string_pretty(&*s) {
                        let _ = std::fs::write(&self.data_path, content);
                    }
                }
                
                result
            }
            "delete" => {
                let id = input.get("id").and_then(|v| v.as_str())
                    .ok_or_else(|| PluginError::ValidationError("缺少 id 参数".to_string()))?;
                let result = self.delete_document(id)?;
                
                // 保存数据
                if let Ok(s) = self.store.lock() {
                    if let Some(parent) = self.data_path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    if let Ok(content) = serde_json::to_string_pretty(&*s) {
                        let _ = std::fs::write(&self.data_path, content);
                    }
                }
                
                result
            }
            "save" => {
                // 手动保存
                if let Ok(s) = self.store.lock() {
                    if let Some(parent) = self.data_path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    if let Ok(content) = serde_json::to_string_pretty(&*s) {
                        let _ = std::fs::write(&self.data_path, content);
                    }
                }
                json!({ "success": true, "message": "保存成功" })
            }
            "workspace_path" | "get_workspace" => {
                // 获取工作区路径
                let cfg = self.config.lock().unwrap();
                let path = shellexpand::tilde(&cfg.workspace_path).to_string();
                json!({ 
                    "success": true, 
                    "workspace_path": cfg.workspace_path,
                    "expanded_path": path
                })
            }
            "set_workspace" => {
                // 设置工作区路径
                let new_path = input.get("path").and_then(|v| v.as_str())
                    .or_else(|| input.get("workspace_path").and_then(|v| v.as_str()));

                eprintln!("[work] set_workspace: path={:?}", new_path);

                if let Some(path) = new_path {
                    let mut cfg = self.config.lock().unwrap();
                    cfg.workspace_path = path.to_string();
                    eprintln!("[work] set_workspace: saved path={}", cfg.workspace_path);

                    // 通知父插件保存配置（后台执行，不阻塞）
                    if let Some(p) = self.get_parent() {
                        eprintln!("[work] set_workspace: save_config scheduled");
                        // 不等待 save_config 完成
                        let _ = p.invoke("save_config", json!({}));
                    }

                    let result = json!({ "success": true, "workspace_path": path });
                    eprintln!("[work] set_workspace: returning result");
                    return Ok(InvokeStream::single(result));
                } else {
                    eprintln!("[work] set_workspace: missing path parameter");
                    return Err(PluginError::ValidationError("缺少 path 参数".to_string()));
                }
            }
            _ => return Err(PluginError::ValidationError(format!("未知操作: {}", action))),
        };

        Ok(InvokeStream::single(result))
    }
}