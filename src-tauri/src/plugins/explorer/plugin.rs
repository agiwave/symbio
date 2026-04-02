//! Explorer 插件 - 工作区资源浏览器（文件系统浏览）

use crate::core::traits::Plugin;
use crate::core::types::{PluginMeta, PluginResult, PluginError, InvokeStream};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::fs;
use shellexpand;

/// Explorer 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExplorerConfig {
    /// 工作区路径
    #[serde(default = "default_workspace_path")]
    pub workspace_path: String,
    /// 是否显示隐藏文件
    #[serde(default = "default_show_hidden")]
    pub show_hidden: bool,
    /// 文件过滤扩展名
    #[serde(default)]
    pub file_filter: Vec<String>,
}

fn default_workspace_path() -> String {
    dirs::home_dir()
        .map(|p| p.join("projects").to_string_lossy().to_string())
        .unwrap_or_else(|| "~/projects".to_string())
}
fn default_show_hidden() -> bool { false }

impl Default for ExplorerConfig {
    fn default() -> Self {
        Self {
            workspace_path: default_workspace_path(),
            show_hidden: default_show_hidden(),
            file_filter: Vec::new(),
        }
    }
}

/// 文件/目录项
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileItem {
    /// 文件/目录名
    pub name: String,
    /// 完整路径
    pub path: String,
    /// 是否为目录
    pub is_dir: bool,
    /// 文件大小（字节），目录为 None
    pub size: Option<u64>,
    /// 子项（仅在展开时填充）
    pub children: Option<Vec<FileItem>>,
}

pub struct ExplorerPlugin {
    meta: PluginMeta,
    config: Arc<Mutex<ExplorerConfig>>,
    /// 父插件引用（用于获取工作区路径）
    parent: Arc<Mutex<Option<Weak<dyn Plugin>>>>,
}

impl ExplorerPlugin {
    fn create_meta() -> PluginMeta {
        PluginMeta {
            name: "explorer".to_string(),
            description: "工作区资源浏览器".to_string(),
            version: "0.1.0".to_string(),
            input: Some(json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["list", "get", "config", "read", "exists"],
                        "description": "操作类型"
                    },
                    "path": { "type": "string", "description": "文件/目录路径" },
                    "recursive": { "type": "boolean", "description": "是否递归列出" }
                }
            })),
            output: Some(json!({
                "type": "object",
                "properties": {
                    "success": { "type": "boolean" },
                    "data": { "type": "object" }
                }
            })),
            author: Some("Symbio Team".to_string()),
        }
    }

    /// 主构造函数
    pub fn new(parent: Option<Weak<dyn Plugin>>) -> Self {
        ExplorerPlugin {
            meta: Self::create_meta(),
            config: Arc::new(Mutex::new(ExplorerConfig::default())),
            parent: Arc::new(Mutex::new(parent)),
        }
    }

    /// 获取父插件引用
    fn get_parent(&self) -> Option<Arc<dyn Plugin>> {
        self.parent.lock().unwrap().as_ref().and_then(|w| w.upgrade())
    }

    /// 获取工作区路径
    fn get_workspace_path(&self) -> Result<PathBuf, PluginError> {
        // 尝试从父插件获取工作区路径
        let parent = self.get_parent()
            .ok_or_else(|| PluginError::InternalError("父插件不存在，无法获取工作区路径".to_string()))?;

        // 使用绝对路径 /work/workspace_path 获取工作区
        let result = parent.invoke("/work/workspace_path", json!({}))
            .map_err(|e| PluginError::InternalError(format!("调用父插件失败：{}", e)))?;

        // 解析结果
        if let InvokeStream::Single(chunk) = result {
            if chunk.error.is_none() {
                if let Some(data) = chunk.data.get("expanded_path") {
                    if let Some(path_str) = data.as_str() {
                        let path = PathBuf::from(path_str);
                        if path.exists() {
                            return Ok(path);
                        }
                        return Err(PluginError::NotFound(format!("工作区路径不存在：{}", path_str)));
                    }
                }
                if let Some(data) = chunk.data.get("workspace_path") {
                    if let Some(path_str) = data.as_str() {
                        // 展开 ~ 为 home 目录
                        let expanded = shellexpand::tilde(path_str).to_string();
                        let path = PathBuf::from(&expanded);
                        if path.exists() {
                            return Ok(path);
                        }
                        return Err(PluginError::NotFound(format!("工作区路径不存在：{}", expanded)));
                    }
                }
            } else {
                let err = chunk.error.unwrap_or_default();
                return Err(PluginError::InternalError(format!("获取工作区路径失败：{}", err)));
            }
        }

        Err(PluginError::InternalError("无法从父插件获取工作区路径，请先在首页选择工作区".to_string()))
    }

    /// 获取配置 Schema
    fn config_schema() -> Value {
        json!({
            "show_hidden": {
                "type": "boolean",
                "title": "显示隐藏文件",
                "description": "是否显示隐藏文件和目录",
                "default": false
            },
            "file_filter": {
                "type": "array",
                "title": "文件过滤",
                "description": "要显示的文件扩展名列表（空表示显示所有）",
                "items": { "type": "string" }
            }
        })
    }

    /// 判断是否为隐藏文件
    fn is_hidden(path: &Path) -> bool {
        path.file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.starts_with('.'))
            .unwrap_or(false)
    }

    /// 列出目录内容
    fn list_directory(&self, path: Option<&str>, recursive: bool) -> Result<Value, PluginError> {
        let workspace = self.get_workspace_path()?;

        let target_path = match path {
            Some(p) if !p.is_empty() => workspace.join(p),
            _ => workspace.clone(),
        };

        if !target_path.exists() {
            return Err(PluginError::NotFound(format!("路径不存在：{}", target_path.display())));
        }

        if !target_path.is_dir() {
            return Err(PluginError::InternalError("不是目录".to_string()));
        }

        let config = self.config.lock().unwrap();
        let show_hidden = config.show_hidden;
        let file_filter = config.file_filter.clone();
        drop(config);

        let mut items = Vec::new();
        self.collect_directory(&target_path, &workspace, recursive, show_hidden, &file_filter, &mut items)?;

        Ok(json!({
            "success": true,
            "data": {
                "path": target_path.to_string_lossy().to_string(),
                "items": items
            }
        }))
    }

    /// 递归收集目录项
    fn collect_directory(
        &self,
        dir: &Path,
        workspace: &Path,
        recursive: bool,
        show_hidden: bool,
        file_filter: &[String],
        items: &mut Vec<FileItem>,
    ) -> Result<(), PluginError> {
        let entries = fs::read_dir(dir)
            .map_err(|e| PluginError::InternalError(format!("读取目录失败：{}", e)))?;

        let mut entries: Vec<_> = entries
            .filter_map(|e| e.ok())
            .collect();

        // 排序：目录在前，文件在后，按名称排序
        entries.sort_by(|a, b| {
            let a_is_dir = a.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
            let b_is_dir = b.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
            match (a_is_dir, b_is_dir) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a.file_name().cmp(&b.file_name()),
            }
        });

        for entry in entries {
            let entry_path = entry.path();

            // 跳过隐藏文件
            if !show_hidden && Self::is_hidden(&entry_path) {
                continue;
            }

            let name = entry.file_name().to_string_lossy().to_string();
            let rel_path = entry_path.strip_prefix(workspace)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| entry_path.to_string_lossy().to_string());

            let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);

            // 文件过滤
            if !is_dir && !file_filter.is_empty() {
                if let Some(ext) = entry_path.extension().and_then(|e| e.to_str()) {
                    let ext_lower = ext.to_lowercase();
                    if !file_filter.iter().any(|f| f.to_lowercase() == ext_lower || f == "*") {
                        continue;
                    }
                } else {
                    continue;
                }
            }

            let size = if is_dir { None } else { entry.metadata().ok().map(|m| m.len()) };

            let mut children: Option<Vec<FileItem>> = None;
            if is_dir && recursive {
                let mut child_items = Vec::new();
                let _ = self.collect_directory(&entry_path, workspace, false, show_hidden, file_filter, &mut child_items);
                children = Some(child_items);
            }

            items.push(FileItem {
                name,
                path: rel_path,
                is_dir,
                size,
                children,
            });
        }

        Ok(())
    }

    /// 获取单个文件/目录详情
    fn get_item(&self, path: &str) -> Result<Value, PluginError> {
        let workspace = self.get_workspace_path()?;
        let target_path = workspace.join(path);

        if !target_path.exists() {
            return Err(PluginError::NotFound(format!("路径不存在：{}", target_path.display())));
        }

        let is_dir = target_path.is_dir();
        let size = if is_dir { None } else { target_path.metadata().ok().map(|m| m.len()) };

        let config = self.config.lock().unwrap();
        let show_hidden = config.show_hidden;
        let file_filter = config.file_filter.clone();
        drop(config);

        let mut children: Option<Vec<FileItem>> = None;
        if is_dir {
            let mut child_items = Vec::new();
            self.collect_directory(&target_path, &workspace, false, show_hidden, &file_filter, &mut child_items)?;
            children = Some(child_items);
        }

        let rel_path = target_path.strip_prefix(workspace)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| target_path.to_string_lossy().to_string());

        Ok(json!({
            "success": true,
            "data": {
                "name": target_path.file_name().and_then(|n| n.to_str()).unwrap_or(""),
                "path": rel_path,
                "is_dir": is_dir,
                "size": size,
                "children": children
            }
        }))
    }

    /// 读取文件内容
    fn read_file(&self, path: &str) -> Result<Value, PluginError> {
        let workspace = self.get_workspace_path()?;
        let target_path = workspace.join(path);

        if !target_path.exists() {
            return Err(PluginError::NotFound(format!("文件不存在：{}", target_path.display())));
        }

        if target_path.is_dir() {
            return Err(PluginError::InternalError("不能读取目录内容".to_string()));
        }

        let content = fs::read_to_string(&target_path)
            .map_err(|e| PluginError::InternalError(format!("读取文件失败：{}", e)))?;

        // 尝试检测文件类型
        let file_type = target_path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_else(|| "txt".to_string());

        Ok(json!({
            "success": true,
            "data": {
                "path": path,
                "content": content,
                "file_type": file_type,
                "size": target_path.metadata().ok().map(|m| m.len())
            }
        }))
    }

    /// 检查文件/目录是否存在
    fn exists(&self, path: &str) -> Result<Value, PluginError> {
        let workspace = self.get_workspace_path()?;
        let target_path = workspace.join(path);

        Ok(json!({
            "success": true,
            "data": {
                "path": path,
                "exists": target_path.exists()
            }
        }))
    }
}

impl Default for ExplorerPlugin {
    fn default() -> Self {
        Self::new(None)
    }
}

#[async_trait::async_trait]
impl Plugin for ExplorerPlugin {
    fn meta(&self, path: &str) -> PluginResult<PluginMeta> {
        if path == "config" {
            return Ok(PluginMeta {
                name: "config".to_string(),
                description: "Explorer 配置管理".to_string(),
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
                        "show_hidden": cfg.show_hidden,
                        "file_filter": cfg.file_filter
                    }))
                }
                "set" => {
                    if let Some(new_config) = input.get("config") {
                        let mut cfg = config.lock().unwrap();
                        if let Some(v) = new_config.get("show_hidden").and_then(|v| v.as_bool()) {
                            cfg.show_hidden = v;
                        }
                        if let Some(v) = new_config.get("file_filter").and_then(|v| v.as_array()) {
                            cfg.file_filter = v.iter()
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
                    "error": format!("未知操作：{}", action)
                })),
            };

            return Ok(result);
        }

        let action = input.get("action").and_then(|v| v.as_str()).unwrap_or("list");

        let result = match action {
            "list" => {
                let path = input.get("path").and_then(|v| v.as_str());
                let recursive = input.get("recursive").and_then(|v| v.as_bool()).unwrap_or(false);
                self.list_directory(path, recursive)?
            }
            "get" => {
                let path = input.get("path").and_then(|v| v.as_str())
                    .ok_or_else(|| PluginError::ValidationError("缺少 path 参数".to_string()))?;
                self.get_item(path)?
            }
            "read" => {
                let path = input.get("path").and_then(|v| v.as_str())
                    .ok_or_else(|| PluginError::ValidationError("缺少 path 参数".to_string()))?;
                self.read_file(path)?
            }
            "exists" => {
                let path = input.get("path").and_then(|v| v.as_str())
                    .ok_or_else(|| PluginError::ValidationError("缺少 path 参数".to_string()))?;
                self.exists(path)?
            }
            _ => return Err(PluginError::ValidationError(format!("未知操作：{}", action))),
        };

        Ok(InvokeStream::single(result))
    }
}
