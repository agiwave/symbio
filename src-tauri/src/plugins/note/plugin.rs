//! Note 插件 - 笔记管理（持久化存储）
//!
//! 存储路径: <workspace>/.symbio/note/notes.json

use crate::core::traits::Plugin;
use crate::core::types::{PluginMeta, PluginResult, PluginError, InvokeStream};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Weak};
use uuid::Uuid;
use chrono::{DateTime, Utc};

/// 笔记结构
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Note {
    pub id: String,
    pub title: String,
    pub content: String,
    pub parent_id: Option<String>,
    pub children: Vec<String>,
    pub order: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 笔记存储
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteStore {
    pub notes: std::collections::HashMap<String, Note>,
    pub root_ids: Vec<String>,
}

impl NoteStore {
    pub fn new() -> Self {
        NoteStore {
            notes: std::collections::HashMap::new(),
            root_ids: Vec::new(),
        }
    }
}

pub struct NotePlugin {
    meta: PluginMeta,
    store: Arc<Mutex<NoteStore>>,
    /// 父插件引用（用于获取工作区路径）
    parent: Option<Weak<dyn Plugin>>,
    /// 缓存的工作区路径
    cached_workspace: Arc<Mutex<Option<String>>>,
}

impl NotePlugin {
    fn create_meta() -> PluginMeta {
        PluginMeta {
            name: "note".to_string(),
            description: "笔记管理插件".to_string(),
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
        // 初始化存储
        let store = Arc::new(Mutex::new(NoteStore::new()));

        NotePlugin { 
            meta: Self::create_meta(),
            store, 
            parent,
            cached_workspace: Arc::new(Mutex::new(None)),
        }
    }

    /// 获取父插件引用
    fn get_parent(&self) -> Option<Arc<dyn Plugin>> {
        self.parent.as_ref().and_then(|w| w.upgrade())
    }

    /// 获取工作区路径（通过父插件调用 work 插件）
    fn get_workspace_path(&self) -> PathBuf {
        // 尝试从缓存获取
        {
            let cached = self.cached_workspace.lock().unwrap();
            if let Some(ref path) = *cached {
                return PathBuf::from(path);
            }
        }

        // 通过父插件获取工作区路径
        let workspace_path = if let Some(parent) = self.get_parent() {
            // 调用 home 的 _workspace 快捷路径，或直接调用 work
            match parent.invoke("_workspace", json!({})) {
                Ok(InvokeStream::Single(chunk)) if chunk.error.is_none() => {
                    chunk.data.get("expanded_path")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                }
                _ => {
                    // 尝试直接调用 work 插件
                    match parent.invoke("work/workspace_path", json!({})) {
                        Ok(InvokeStream::Single(chunk)) if chunk.error.is_none() => {
                            chunk.data.get("expanded_path")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string())
                        }
                        _ => None,
                    }
                }
            }
        } else {
            None
        };

        // 如果获取到工作区路径，缓存并返回
        if let Some(path) = workspace_path {
            let mut cached = self.cached_workspace.lock().unwrap();
            *cached = Some(path.clone());
            return PathBuf::from(path);
        }

        // 回退到默认路径
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("projects")
    }

    /// 获取数据存储路径: <workspace>/.symbio/note/notes.json
    fn get_data_path(&self) -> PathBuf {
        self.get_workspace_path()
            .join(".symbio")
            .join("note")
            .join("notes.json")
    }

    /// 同步保存数据
    fn save_data_sync(&self) -> Result<(), PluginError> {
        let data_path = self.get_data_path();
        if let Some(parent) = data_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let s = self.store.lock().map_err(|e| PluginError::InternalError(e.to_string()))?;
        let content = serde_json::to_string_pretty(&*s)
            .map_err(|e| PluginError::InternalError(format!("序列化数据失败: {}", e)))?;
        
        std::fs::write(&data_path, content)
            .map_err(|e| PluginError::InternalError(format!("写入数据失败: {}", e)))?;
        
        Ok(())
    }

    /// 同步加载数据
    fn load_data_sync(&self) -> Result<(), PluginError> {
        let data_path = self.get_data_path();
        if data_path.exists() {
            let content = std::fs::read_to_string(&data_path)
                .map_err(|e| PluginError::InternalError(format!("读取数据失败: {}", e)))?;
            
            let store: NoteStore = serde_json::from_str(&content)
                .map_err(|e| PluginError::InternalError(format!("解析数据失败: {}", e)))?;
            
            let mut s = self.store.lock().map_err(|e| PluginError::InternalError(e.to_string()))?;
            *s = store;
        }
        Ok(())
    }

    /// 列出笔记
    fn list_notes(&self) -> Value {
        let s = self.store.lock().unwrap();
        let notes: Vec<Value> = s.root_ids.iter()
            .filter_map(|id| s.notes.get(id))
            .map(|note| json!({
                "id": note.id,
                "title": note.title,
                "parentId": note.parent_id
            }))
            .collect();
        
        json!({
            "success": true,
            "data": { "documents": notes }
        })
    }

    /// 创建笔记
    fn create_note(&self, title: &str, parent_id: Option<&str>) -> Value {
        let mut s = self.store.lock().unwrap();
        
        let now = Utc::now();
        let id = Uuid::new_v4().to_string();
        
        // 计算排序值
        let siblings: Vec<&Note> = s.notes.values()
            .filter(|n| n.parent_id.as_deref() == parent_id)
            .collect();
        let order = siblings.len() as i32 + 1;

        let note = Note {
            id: id.clone(),
            title: title.to_string(),
            content: String::new(),
            parent_id: parent_id.map(|s| s.to_string()),
            children: Vec::new(),
            order,
            created_at: now,
            updated_at: now,
        };

        // 更新父笔记的 children
        if let Some(pid) = parent_id {
            if let Some(parent) = s.notes.get_mut(pid) {
                parent.children.push(id.clone());
            }
        } else {
            s.root_ids.push(id.clone());
        }

        s.notes.insert(id.clone(), note);

        json!({
            "success": true,
            "data": {
                "id": id,
                "title": title,
                "parentId": parent_id,
                "content": ""
            },
            "message": "笔记创建成功"
        })
    }

    /// 获取笔记
    fn get_note(&self, id: &str) -> Result<Value, PluginError> {
        let s = self.store.lock().unwrap();
        
        let note = s.notes.get(id)
            .ok_or_else(|| PluginError::NotFound(format!("笔记不存在: {}", id)))?;

        Ok(json!({
            "success": true,
            "data": {
                "id": note.id,
                "title": note.title,
                "content": note.content,
                "parentId": note.parent_id,
                "children": note.children
            }
        }))
    }

    /// 更新笔记
    fn update_note(&self, id: &str, updates: &Value) -> Result<Value, PluginError> {
        let mut s = self.store.lock().unwrap();
        
        let note = s.notes.get_mut(id)
            .ok_or_else(|| PluginError::NotFound(format!("笔记不存在: {}", id)))?;

        if let Some(title) = updates.get("title").and_then(|v| v.as_str()) {
            note.title = title.to_string();
        }
        if let Some(content) = updates.get("content").and_then(|v| v.as_str()) {
            note.content = content.to_string();
        }
        
        note.updated_at = Utc::now();

        Ok(json!({
            "success": true,
            "data": { "id": id },
            "message": "笔记更新成功"
        }))
    }

    /// 删除笔记
    fn delete_note(&self, id: &str) -> Result<Value, PluginError> {
        let mut s = self.store.lock().unwrap();
        
        let note = s.notes.get(id)
            .ok_or_else(|| PluginError::NotFound(format!("笔记不存在: {}", id)))?;

        let parent_id = note.parent_id.clone();
        let children = note.children.clone();

        // 递归删除子笔记
        fn delete_children(
            store: &mut std::collections::HashMap<String, Note>,
            children: &[String],
        ) {
            for child_id in children {
                if let Some(child) = store.remove(child_id) {
                    delete_children(store, &child.children);
                }
            }
        }

        delete_children(&mut s.notes, &children);

        // 从父笔记或根列表中移除
        if let Some(pid) = parent_id {
            if let Some(parent) = s.notes.get_mut(&pid) {
                parent.children.retain(|cid| cid != id);
            }
        } else {
            s.root_ids.retain(|rid| rid != id);
        }

        s.notes.remove(id);

        Ok(json!({
            "success": true,
            "data": { "id": id },
            "message": "笔记删除成功"
        }))
    }

    /// 初始化示例数据
    fn init_demo(&self) {
        let mut s = self.store.lock().unwrap();
        
        if !s.notes.is_empty() {
            return;
        }

        let now = Utc::now();

        // 创建示例笔记
        let root_id = Uuid::new_v4().to_string();
        let design_id = Uuid::new_v4().to_string();
        let qc_id = Uuid::new_v4().to_string();

        let root = Note {
            id: root_id.clone(),
            title: "RNA-seq 差异表达分析".to_string(),
            content: "# RNA-seq 差异表达分析\n\n这是一个完整的 RNA-seq 分析流程示例。".to_string(),
            parent_id: None,
            children: vec![design_id.clone(), qc_id.clone()],
            order: 1,
            created_at: now,
            updated_at: now,
        };

        let design = Note {
            id: design_id.clone(),
            title: "实验设计".to_string(),
            content: "## 实验设计\n\n描述实验组和对照组的设置。".to_string(),
            parent_id: Some(root_id.clone()),
            children: vec![],
            order: 1,
            created_at: now,
            updated_at: now,
        };

        let qc = Note {
            id: qc_id.clone(),
            title: "数据预处理".to_string(),
            content: "## FastQC 质控\n\n```bash\nfastqc *.fastq.gz -o qc_results\n```".to_string(),
            parent_id: Some(root_id.clone()),
            children: vec![],
            order: 2,
            created_at: now,
            updated_at: now,
        };

        s.notes.insert(root_id.clone(), root);
        s.notes.insert(design_id, design);
        s.notes.insert(qc_id, qc);
        s.root_ids.push(root_id);
    }
}

impl Default for NotePlugin {
    fn default() -> Self {
        Self::new(None)
    }
}

#[async_trait::async_trait]
impl Plugin for NotePlugin {
    fn meta(&self, path: &str) -> PluginResult<PluginMeta> {
        if path == "config" {
            return Ok(PluginMeta {
                name: "config".to_string(),
                description: "Note 配置管理".to_string(),
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
                let data_path = self.get_data_path();
                if data_path.exists() {
                    let _ = self.load_data_sync();
                } else {
                    self.init_demo();
                    let _ = self.save_data_sync();
                }
                json!({ "success": true, "message": "初始化完成" })
            }
            "list" => self.list_notes(),
            "create" => {
                let title = input.get("title").and_then(|v| v.as_str()).unwrap_or("新笔记");
                let parent_id = input.get("parentId").and_then(|v| v.as_str());
                let result = self.create_note(title, parent_id);
                let _ = self.save_data_sync();
                result
            }
            "get" => {
                let id = input.get("id").and_then(|v| v.as_str())
                    .ok_or_else(|| PluginError::ValidationError("缺少 id 参数".to_string()))?;
                self.get_note(id)?
            }
            "update" => {
                let id = input.get("id").and_then(|v| v.as_str())
                    .ok_or_else(|| PluginError::ValidationError("缺少 id 参数".to_string()))?;
                let result = self.update_note(id, &input)?;
                let _ = self.save_data_sync();
                result
            }
            "delete" => {
                let id = input.get("id").and_then(|v| v.as_str())
                    .ok_or_else(|| PluginError::ValidationError("缺少 id 参数".to_string()))?;
                let result = self.delete_note(id)?;
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
