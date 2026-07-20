//! Explorer 插件 - 工作区资源浏览器（文件系统浏览）

use super::watcher::FileWatcher;
use crate::plugin_error;
use crate::symbio_core::event_bus::EventBus;
pub use crate::symbio_core::schemas::common::SimpleResponse;
use crate::symbio_core::schemas::explorer::explorer_config::ExplorerConfig;
use crate::symbio_core::schemas::explorer::explorer_event::StatusInput;
use crate::symbio_core::schemas::explorer::{
    explorer_event, explorer_list, explorer_read, explorer_write,
};
use crate::symbio_core::{
    InvokeRequest, InvokeRequestExt, InvokeResponse, Plugin, PluginChannel, PluginError,
    PluginFrame, PluginMeta, PluginPayload, CONFIG_GET, CONFIG_SET, PLUGIN_EXPLORER,
};
use async_trait::async_trait;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};
use tokio::sync::{mpsc, Mutex};

use dashmap::DashMap;

#[derive(Clone)]
pub struct ExplorerPlugin {
    config: Arc<Mutex<ExplorerConfig>>,
    /// 父插件引用
    parent: Arc<Mutex<Option<Weak<dyn Plugin>>>>,
    /// 文件监听器 (Key: workdir)
    watchers: Arc<DashMap<String, FileWatcher>>,
    /// 活跃的路由会话发送端 (Key: workdir)
    active_sessions: Arc<DashMap<String, Vec<mpsc::Sender<PluginFrame>>>>,
}

impl ExplorerPlugin {
    /// 静态工厂：从 InvokeRequest 构造 Plugin 实例
    pub fn build(ctx: Arc<dyn InvokeRequest>) -> Arc<dyn Plugin> {
        let config: ExplorerConfig = ctx
            .config()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();

        let parent = ctx.parent();

        Arc::new(ExplorerPlugin::new(parent, config)) as Arc<dyn Plugin>
    }

    /// 主构造函数
    pub fn new(parent: Option<Weak<dyn Plugin>>, config: ExplorerConfig) -> Self {
        ExplorerPlugin {
            config: Arc::new(Mutex::new(config)),
            parent: Arc::new(Mutex::new(parent)),
            watchers: Arc::new(DashMap::new()),
            active_sessions: Arc::new(DashMap::new()),
        }
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new("explorer", "资源管理器")
            .with_description("工作区资源浏览器（文件系统浏览）")
            .with_version("0.1.0")
    }

    async fn get_parent(&self) -> Option<Arc<dyn Plugin>> {
        let guard = self.parent.lock().await;
        guard.as_ref().and_then(|w| w.upgrade())
    }

    async fn start_watch_internal(
        &self,
        tx: mpsc::Sender<PluginFrame>,
        workdir: PathBuf,
    ) -> Result<(), PluginError> {
        let workdir_key = workdir.to_string_lossy().to_string();

        {
            let mut sessions = self.active_sessions.entry(workdir_key.clone()).or_default();
            sessions.retain(|s| !s.is_closed());
            sessions.push(tx);
        }

        if !self.watchers.contains_key(&workdir_key) {
            let active_sessions = self.active_sessions.clone();
            let workspace_for_callback = workdir.clone();
            let workdir_key_clone = workdir_key.clone();
            let watcher = FileWatcher::new_with_callback(move |event_type, input| {
                if let Ok(mut event) = serde_json::from_value::<explorer_event::Event>(input) {
                    if let explorer_event::Event::FileChange { ref mut path, .. } = event {
                        let abs_path = std::path::Path::new(path);
                        if let Ok(rel_path) = abs_path.strip_prefix(&workspace_for_callback) {
                            *path = rel_path.to_string_lossy().to_string();
                        }
                    }

                    let broadcast_input = serde_json::to_value(StatusInput {
                        r#type: event_type,
                        data: serde_json::to_value(event).unwrap_or_default(),
                    })
                    .unwrap_or_default();

                    if let Some(mut sessions) = active_sessions.get_mut(&workdir_key_clone) {
                        sessions.retain(|s| !s.is_closed());
                        for sender in sessions.iter() {
                            let _ = sender.try_send(PluginFrame::Data(broadcast_input.clone()));
                        }
                    }

                    // 同时通过 EventBus 转发（workdir 作为 session_id 标识）
                    EventBus::try_publish("explorer", Some(&workdir_key_clone), broadcast_input);
                }
            });
            let watcher_clone = watcher.clone();
            let workspace = workdir;
            tokio::spawn(async move {
                if let Err(e) = watcher_clone.start(workspace).await {
                    plugin_error!("explorer", format!("Watcher error: {}", e));
                }
            });
            self.watchers.insert(workdir_key, watcher);
        }
        Ok(())
    }

    async fn list_directory(
        &self,
        workspace: &Path,
        path: Option<&str>,
        recursive: bool,
    ) -> Result<Value, PluginError> {
        let target_path = match path {
            Some(p) if !p.is_empty() => workspace.join(p),
            _ => workspace.to_path_buf(),
        };

        if !target_path.exists() || !target_path.is_dir() {
            return Err(PluginError::NotFound("目录不存在".to_string()));
        }

        let config = self.config.lock().await;
        let (show_hidden, file_filter) = (config.show_hidden, config.file_filter.clone());
        drop(config);

        let mut items = Vec::new();
        self.collect_directory_recursive(
            &target_path,
            workspace,
            recursive,
            show_hidden,
            &file_filter,
            &mut items,
            0,
        )
        .await?;

        Ok(serde_json::to_value(explorer_list::Response {
            path: target_path.to_string_lossy().to_string(),
            items,
        })
        .unwrap_or_default())
    }

    #[allow(clippy::too_many_arguments)]
    async fn collect_directory_recursive(
        &self,
        dir: &Path,
        workspace: &Path,
        recursive: bool,
        show_hidden: bool,
        file_filter: &[String],
        items: &mut Vec<explorer_list::FileItem>,
        depth: usize,
    ) -> Result<(), PluginError> {
        if depth > 10 {
            return Ok(());
        }
        let mut entries = tokio::fs::read_dir(dir)
            .await
            .map_err(|e| PluginError::InternalError(e.to_string()))?;
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| PluginError::InternalError(e.to_string()))?
        {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if !show_hidden && name.starts_with('.') {
                continue;
            }

            let is_dir = path.is_dir();
            let mut children = None;
            if is_dir && recursive {
                let mut sub_items = Vec::new();
                // 使用 Box::pin 处理 async 递归
                Box::pin(self.collect_directory_recursive(
                    &path,
                    workspace,
                    false,
                    show_hidden,
                    file_filter,
                    &mut sub_items,
                    depth + 1,
                ))
                .await?;
                children = Some(sub_items);
            }

            items.push(explorer_list::FileItem {
                name,
                path: path
                    .strip_prefix(workspace)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .to_string(),
                is_dir,
                size: if is_dir {
                    None
                } else {
                    Some(entry.metadata().await.map(|m| m.len()).unwrap_or(0))
                },
                children,
            });
        }
        Ok(())
    }
}

#[async_trait]
impl Plugin for ExplorerPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    async fn route(self: Arc<Self>, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload> {
        let path = ctx.get(crate::symbio_core::PATH).unwrap_or_default();
        let path = path.strip_prefix('/').unwrap_or(&path);

        match path {
            CONFIG_GET => {
                let cfg = self.config.lock().await;
                Ok(PluginPayload::new(&*cfg))
            },
            CONFIG_SET => {
                let new_cfg: ExplorerConfig = ctx.payload()?;
                {
                    let mut cfg = self.config.lock().await;
                    *cfg = new_cfg;
                }
                if let Some(p) = self.get_parent().await {
                    let save_ctx = ctx.fork();
                    save_ctx.set(crate::symbio_core::PATH, "save_config".to_string());
                    p.route(save_ctx).await?;
                }
                Ok(PluginPayload::new(&SimpleResponse::success()))
            },
            "list" => {
                let workdir_str = ctx.get(crate::symbio_core::WORKDIR).ok_or_else(|| {
                    PluginError::ValidationError("Missing workdir in context".to_string())
                })?;
                if workdir_str.is_empty() {
                    return Err(PluginError::ValidationError(
                        "Empty workdir in context".to_string(),
                    ));
                }
                let workspace = PathBuf::from(shellexpand::tilde(&workdir_str).to_string());
                let req: explorer_list::Request = ctx.payload()?;
                let res = self
                    .list_directory(&workspace, req.path.as_deref(), req.recursive)
                    .await?;
                Ok(PluginPayload::new(&res))
            },
            "read" => {
                let workdir_str = ctx.get(crate::symbio_core::WORKDIR).ok_or_else(|| {
                    PluginError::ValidationError("Missing workdir in context".to_string())
                })?;
                if workdir_str.is_empty() {
                    return Err(PluginError::ValidationError(
                        "Empty workdir in context".to_string(),
                    ));
                }
                let workspace = PathBuf::from(shellexpand::tilde(&workdir_str).to_string());
                let req: explorer_read::Request = ctx.payload()?;
                let abs_path = workspace.join(&req.path);
                let content = tokio::fs::read_to_string(&abs_path)
                    .await
                    .map_err(|e| PluginError::InternalError(e.to_string()))?;
                Ok(PluginPayload::new(&explorer_read::ReadData {
                    path: req.path.clone(),
                    content,
                    file_type: "text".to_string(),
                    size: Some(
                        tokio::fs::metadata(&abs_path)
                            .await
                            .map(|m| m.len())
                            .unwrap_or(0),
                    ),
                }))
            },
            "write" => {
                let workdir_str = ctx.get(crate::symbio_core::WORKDIR).ok_or_else(|| {
                    PluginError::ValidationError("Missing workdir in context".to_string())
                })?;
                if workdir_str.is_empty() {
                    return Err(PluginError::ValidationError(
                        "Empty workdir in context".to_string(),
                    ));
                }
                let workspace = PathBuf::from(shellexpand::tilde(&workdir_str).to_string());
                let req: explorer_write::Request = ctx.payload()?;
                let abs_path = workspace.join(&req.path);
                tokio::fs::write(abs_path, &req.content)
                    .await
                    .map_err(|e| PluginError::InternalError(e.to_string()))?;
                Ok(PluginPayload::new(&SimpleResponse::success()))
            },
            "watch" => {
                let workdir_str = ctx.get(crate::symbio_core::WORKDIR).ok_or_else(|| {
                    PluginError::ValidationError("Missing workdir in context".to_string())
                })?;
                if workdir_str.is_empty() {
                    return Err(PluginError::ValidationError(
                        "Empty workdir in context".to_string(),
                    ));
                }
                let workspace = PathBuf::from(shellexpand::tilde(&workdir_str).to_string());
                let (my_channel, peer_channel) = PluginChannel::pair(64);
                let this = self.clone();
                tokio::spawn(async move {
                    let _ = this.start_watch_internal(my_channel.tx, workspace).await;
                });
                Ok(PluginPayload::Session(peer_channel))
            },
            _ => Err(PluginError::NotFound(format!("Unknown path: {path}"))),
        }
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        _ctx: Arc<dyn InvokeRequest>,
    ) -> InvokeResponse<PluginPayload> {
        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

crate::submit_object_creator!(PLUGIN_EXPLORER, ExplorerPlugin::build, dyn Plugin);
