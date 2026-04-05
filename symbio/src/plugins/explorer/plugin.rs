//! Explorer 插件 - 工作区资源浏览器（文件系统浏览）

use crate::symbio_core::traits::Plugin;
use crate::symbio_core::types::{Connection, PluginMeta, PluginResult, PluginError, InvokeStream};
use crate::symbio_core::event::OptionalEventSender;
use super::watcher::FileWatcher;
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

#[derive(Clone)]
pub struct ExplorerPlugin {
    meta: PluginMeta,
    config: Arc<Mutex<ExplorerConfig>>,
    /// 父插件引用（用于获取工作区路径）
    parent: Arc<Mutex<Option<Weak<dyn Plugin>>>>,
    /// 文件监听器（延迟初始化）
    watcher: Arc<Mutex<Option<FileWatcher>>>,
    /// 事件发送器（用于发送文件变化事件）
    event_sender: OptionalEventSender,
    /// 活跃的连接（用于 connect 双向通信）
    active_connections: Arc<Mutex<Vec<Connection>>>,
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
                        "enum": ["list", "get", "config", "read", "write", "exists", "start_watch", "stop_watch"],
                        "description": "操作类型"
                    },
                    "path": { "type": "string", "description": "文件/目录路径" },
                    "recursive": { "type": "boolean", "description": "是否递归列出" },
                    "content": { "type": "string", "description": "文件内容（write 操作时使用）" }
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
    pub fn new(parent: Option<Weak<dyn Plugin>>, event_sender: OptionalEventSender) -> Self {
        ExplorerPlugin {
            meta: Self::create_meta(),
            config: Arc::new(Mutex::new(ExplorerConfig::default())),
            parent: Arc::new(Mutex::new(parent)),
            watcher: Arc::new(Mutex::new(None)),
            event_sender,
            active_connections: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// 获取父插件引用
    fn get_parent(&self) -> Option<Arc<dyn Plugin>> {
        self.parent.lock().unwrap().as_ref().and_then(|w| w.upgrade())
    }

    /// 获取工作区路径
    fn get_workspace_path(&self) -> Result<PathBuf, PluginError> {
        eprintln!("[explorer] get_workspace_path: start");
        
        let parent = self.get_parent()
            .ok_or_else(|| {
                eprintln!("[explorer] get_workspace_path: parent not found");
                PluginError::InternalError("父插件不存在，无法获取工作区路径".to_string())
            })?;

        eprintln!("[explorer] get_workspace_path: calling work/workspace_path");
        
        // 使用 work/workspace_path 获取工作区
        let result = parent.invoke("work/workspace_path", json!({}))
            .map_err(|e| {
                eprintln!("[explorer] get_workspace_path: invoke error: {}", e);
                PluginError::InternalError(format!("调用父插件失败：{}", e))
            })?;

        eprintln!("[explorer] get_workspace_path: invoke completed");

        // 解析结果
        if let InvokeStream::Single(chunk) = result {
            eprintln!("[explorer] get_workspace_path: chunk = {:?}", chunk);
            if chunk.error.is_none() {
                if let Some(data) = chunk.data.get("expanded_path") {
                    if let Some(path_str) = data.as_str() {
                        eprintln!("[explorer] got expanded_path: {}", path_str);
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
                        eprintln!("[explorer] got workspace_path: {} -> {}", path_str, expanded);
                        let path = PathBuf::from(&expanded);
                        if path.exists() {
                            return Ok(path);
                        }
                        return Err(PluginError::NotFound(format!("工作区路径不存在：{}", expanded)));
                    }
                }
            } else {
                let err = chunk.error.unwrap_or_default();
                eprintln!("[explorer] chunk error: {}", err);
                return Err(PluginError::InternalError(format!("获取工作区路径失败：{}", err)));
            }
        }

        eprintln!("[explorer] get_workspace_path: failed to parse result");
        Err(PluginError::InternalError("无法从父插件获取工作区路径，请先在首页选择工作区".to_string()))
    }

    /// 启动文件监听（通过 connect 机制）
    fn start_watch_with_connection(&self, conn: &Connection) -> Result<(), String> {
        let workspace = self.get_workspace_path().map_err(|e| e.to_string())?;

        // 检查是否已经在监听
        {
            let watcher = self.watcher.lock().unwrap();
            if watcher.is_some() {
                conn.emit("watch_status", json!({
                    "success": true,
                    "message": "已在监听中"
                })).map_err(|e| e.to_string())?;
                return Ok(());
            }
        }

        // 创建新的 FileWatcher，使用连接的事件发送器
        let conn_sender = conn.clone();
        let workspace_for_callback = workspace.clone();
        let watcher = FileWatcher::new_with_callback(
            move |event_name, payload| {
                // 将绝对路径转换为相对于工作区的路径
                let mut updated_payload = payload.clone();
                if let Some(path_str) = payload.get("path").and_then(|v| v.as_str()) {
                    let abs_path = std::path::Path::new(path_str);
                    if let Ok(rel_path) = abs_path.strip_prefix(&workspace_for_callback) {
                        updated_payload["path"] = json!(rel_path.to_string_lossy().to_string());
                    }
                }

                // 通过连接发送事件
                if let Err(e) = conn_sender.emit(&event_name, updated_payload) {
                    eprintln!("[explorer] Failed to send watch event: {}", e);
                }
            }
        );
        let workspace_clone = workspace.clone();
        let watcher_clone = watcher.clone();

        // 在后台线程启动监听
        tokio::spawn(async move {
            if let Err(e) = watcher_clone.start(workspace_clone).await {
                eprintln!("[explorer] Failed to start watcher: {}", e);
            }
        });

        // 保存 watcher 引用
        {
            let mut w = self.watcher.lock().unwrap();
            *w = Some(watcher);
        }

        // 保存连接到活跃列表
        {
            let mut connections = self.active_connections.lock().unwrap();
            connections.push(conn.clone());
        }

        conn.emit("watch_started", json!({
            "success": true,
            "message": "开始监听",
            "path": workspace.to_string_lossy()
        })).map_err(|e| e.to_string())?;

        Ok(())
    }

    /// 启动文件监听（旧方式，保持兼容）
    fn start_watch(&self) -> PluginResult<InvokeStream> {
        let workspace = self.get_workspace_path()?;
        
        // 检查是否已经在监听
        {
            let watcher = self.watcher.lock().unwrap();
            if watcher.is_some() {
                return Ok(InvokeStream::single(json!({
                    "success": true,
                    "message": "已在监听中"
                })));
            }
        }

        // 使用注入的事件发送器创建 FileWatcher
        let watcher = FileWatcher::new(self.event_sender.clone());
        let workspace_clone = workspace.clone();
        let watcher_clone = watcher.clone();
        
        // 在后台线程启动监听
        tokio::spawn(async move {
            if let Err(e) = watcher_clone.start(workspace_clone).await {
                eprintln!("[explorer] Failed to start watcher: {}", e);
            }
        });

        // 保存 watcher 引用
        {
            let mut w = self.watcher.lock().unwrap();
            *w = Some(watcher);
        }

        eprintln!("[explorer] start_watch: {}", workspace.display());
        
        Ok(InvokeStream::single(json!({
            "success": true,
            "message": "开始监听",
            "path": workspace.to_string_lossy()
        })))
    }

    /// 停止文件监听（旧方式，保持兼容）
    fn stop_watch(&self) -> PluginResult<InvokeStream> {
        let watcher = {
            let mut w = self.watcher.lock().unwrap();
            w.take()
        };

        if let Some(w) = watcher {
            // 在后台线程停止监听
            tokio::spawn(async move {
                w.stop().await;
            });
            eprintln!("[explorer] stop_watch: stopping");
        }

        Ok(InvokeStream::single(json!({
            "success": true,
            "message": "停止监听"
        })))
    }

    /// 停止文件监听（connect 方式）
    fn stop_watch_for_connection(&self) -> Result<(), String> {
        let watcher = {
            let mut w = self.watcher.lock().unwrap();
            w.take()
        };

        if let Some(w) = watcher {
            // 在后台线程停止监听
            tokio::spawn(async move {
                w.stop().await;
            });
            eprintln!("[explorer] stop_watch_for_connection: stopping");
        }

        // 清理已关闭的连接
        {
            let mut connections = self.active_connections.lock().unwrap();
            connections.retain(|c| !c.is_closed());
        }

        Ok(())
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
        eprintln!("[explorer] list_directory: START, path={:?}, recursive={}", path, recursive);
        
        let workspace = self.get_workspace_path()?;
        eprintln!("[explorer] list_directory: workspace={:?}", workspace);

        let target_path = match path {
            Some(p) if !p.is_empty() => workspace.join(p),
            _ => workspace.clone(),
        };

        eprintln!("[explorer] list_directory: target_path={:?}", target_path);

        if !target_path.exists() {
            eprintln!("[explorer] list_directory: path not found");
            return Err(PluginError::NotFound(format!("路径不存在：{}", target_path.display())));
        }

        if !target_path.is_dir() {
            eprintln!("[explorer] list_directory: not a directory");
            return Err(PluginError::InternalError("不是目录".to_string()));
        }

        let config = self.config.lock().unwrap();
        let show_hidden = config.show_hidden;
        let file_filter = config.file_filter.clone();
        drop(config);

        let mut items = Vec::new();
        self.collect_directory(&target_path, &workspace, recursive, show_hidden, &file_filter, &mut items)?;

        eprintln!("[explorer] list_directory: found {} items", items.len());
        for item in &items {
            eprintln!("[explorer]   - {} ({})", item.name, if item.is_dir { "dir" } else { "file" });
        }

        let result = json!({
            "success": true,
            "data": {
                "path": target_path.to_string_lossy().to_string(),
                "items": items
            }
        });
        
        eprintln!("[explorer] list_directory: result = {:?}", result);
        Ok(result)
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

    /// 写入文件内容
    fn write_file(&self, path: &str, content: &str) -> Result<Value, PluginError> {
        let workspace = self.get_workspace_path()?;
        let target_path = workspace.join(path);

        // 确保父目录存在
        if let Some(parent) = target_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| PluginError::InternalError(format!("创建目录失败：{}", e)))?;
        }

        // 写入文件
        fs::write(&target_path, content)
            .map_err(|e| PluginError::InternalError(format!("写入文件失败：{}", e)))?;

        eprintln!("[explorer] write_file: {} ({} bytes)", target_path.display(), content.len());

        Ok(json!({
            "success": true,
            "data": {
                "path": path,
                "size": content.len()
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
        Self::new(None, OptionalEventSender::new(None))
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

        // 处理文件监听 action
        if action == "start_watch" {
            return self.start_watch();
        }
        if action == "stop_watch" {
            return self.stop_watch();
        }

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
            "write" => {
                let path = input.get("path").and_then(|v| v.as_str())
                    .ok_or_else(|| PluginError::ValidationError("缺少 path 参数".to_string()))?;
                let content = input.get("content").and_then(|v| v.as_str())
                    .ok_or_else(|| PluginError::ValidationError("缺少 content 参数".to_string()))?;
                self.write_file(path, content)?
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

    async fn connect(
        &self,
        _path: &str,
        input: Value,
        conn: Connection,
    ) -> PluginResult<()> {
        // 处理连接请求
        let action = input.get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("watch");

        match action {
            "watch" => {
                // 启动文件监听并通过此连接发送事件
                self.start_watch_with_connection(&conn).map_err(|e| PluginError::InternalError(e.to_string()))?;

                // 注册消息处理器
                let plugin = self.clone();
                let conn_for_handler = conn.clone();
                conn.on_message(move |message| {
                    let plugin_clone = plugin.clone();
                    let conn_clone = conn_for_handler.clone();
                    tokio::spawn(async move {
                        if let Err(e) = plugin_clone.handle_connect_message(&conn_clone, message).await {
                            eprintln!("[explorer] connect message error: {}", e);
                        }
                    });
                });

                // 发送连接建立确认
                conn.emit("connected", json!({
                    "message": "已连接到 Explorer 插件",
                    "watching": self.watcher.lock().unwrap().is_some()
                })).map_err(|e| PluginError::InternalError(e))?;
            }
            _ => {
                conn.emit("error", json!({
                    "message": format!("未知操作：{}", action)
                })).map_err(|e| PluginError::InternalError(e)).ok();
            }
        }

        Ok(())
    }
}

impl ExplorerPlugin {
    /// 处理连接后的客户端消息
    async fn handle_connect_message(&self, conn: &Connection, message: Value) -> PluginResult<()> {
        let action = message.get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        match action {
            "start_watch" => {
                self.start_watch_with_connection(conn).map_err(|e| PluginError::InternalError(e.to_string()))?;
            }
            "stop_watch" => {
                self.stop_watch_for_connection().map_err(|e| PluginError::InternalError(e.to_string()))?;
                conn.emit("watch_stopped", json!({
                    "message": "已停止监听"
                })).map_err(|e| PluginError::InternalError(e)).ok();
            }
            "list" => {
                let path = message.get("path").and_then(|v| v.as_str());
                let recursive = message.get("recursive").and_then(|v| v.as_bool()).unwrap_or(false);
                match self.list_directory(path, recursive) {
                    Ok(result) => {
                        conn.emit("list_result", result).map_err(|e| PluginError::InternalError(e)).ok();
                    }
                    Err(e) => {
                        conn.emit("error", json!({ "message": e.to_string() })).map_err(|e| PluginError::InternalError(e)).ok();
                    }
                }
            }
            _ => {
                conn.emit("error", json!({
                    "message": format!("未知操作：{}", action)
                })).map_err(|e| PluginError::InternalError(e)).ok();
            }
        }

        Ok(())
    }
}
