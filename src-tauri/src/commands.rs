//! Tauri 命令处理模块
//!
//! 提供核心命令：
//! - `meta`: 获取插件元数据
//! - `invoke`: 同步调用，返回完整结果
//! - `stream`: 流式调用，通过事件推送每个 chunk
//! - `docker_*`: Docker 执行环境命令

use crate::AppState;
use crate::core::types::{PluginMeta, StreamChunk, InvokeStream};
use crate::execution::{DockerExecutor, ExecutionConfig, ExecutionResult};
use serde::{Deserialize, Serialize};
use tauri::Emitter;

/// 获取插件元数据
#[tauri::command]
pub fn meta(
    state: tauri::State<AppState>,
    path: String,
) -> Result<MetaResponse, String> {
    let root = state.root.lock().map_err(|e| e.to_string())?;
    let meta = root.meta(&path).map_err(|e| e.to_string())?;

    Ok(MetaResponse {
        path: if path.is_empty() { "root".to_string() } else { path },
        meta,
    })
}

/// 同步调用插件，返回完整结果
///
/// 适用场景：需要一次性获取所有结果的简单场景
#[tauri::command]
pub async fn invoke(
    state: tauri::State<'_, AppState>,
    path: String,
    input: serde_json::Value,
) -> Result<Vec<StreamChunk>, String> {
    let stream: InvokeStream = {
        let root = state.root.lock().map_err(|e| e.to_string())?;
        root.invoke(&path, input).map_err(|e| e.to_string())?
    };
    
    Ok(stream.collect().await)
}

/// 流式调用插件，通过事件推送每个 chunk
///
/// 适用场景：需要渐进式渲染的流式场景
///
/// 前端使用示例：
/// ```typescript
/// const eventId = `stream-${Date.now()}`;
/// listen(eventId, (event) => {
///     const chunk = event.payload as StreamChunk;
///     // 处理每个 chunk
///     if (chunk.done) {
///         // 流结束
///     }
/// });
/// await invoke('stream', { path, input, eventId });
/// ```
#[tauri::command]
pub async fn stream(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    path: String,
    input: serde_json::Value,
    event_id: String,
) -> Result<(), String> {
    let stream: InvokeStream = {
        let root = state.root.lock().map_err(|e| e.to_string())?;
        root.invoke(&path, input).map_err(|e| e.to_string())?
    };
    
    match stream {
        InvokeStream::Single(chunk) => {
            app.emit(&event_id, chunk).map_err(|e| e.to_string())?;
        }
        InvokeStream::Stream(mut s) => {
            use futures::StreamExt;
            while let Some(chunk) = s.next().await {
                app.emit(&event_id, chunk).map_err(|e| e.to_string())?;
            }
        }
    }
    
    Ok(())
}

/// Meta 命令响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetaResponse {
    pub path: String,
    pub meta: PluginMeta,
}

// ===== Docker 执行命令 =====

/// 检查 Docker 是否可用
#[tauri::command]
pub fn docker_available() -> bool {
    DockerExecutor::with_defaults().is_docker_available()
}

/// 检查执行环境镜像是否存在
#[tauri::command]
pub fn docker_image_exists(tag: String) -> bool {
    DockerExecutor::with_defaults().image_exists(&tag)
}

/// 构建执行环境镜像
#[tauri::command]
pub fn docker_build_image(
    dockerfile_path: String,
    tag: String,
) -> Result<(), String> {
    DockerExecutor::with_defaults().build_image(&dockerfile_path, &tag)
}

/// 执行命令
#[tauri::command]
pub fn docker_execute(
    command: String,
    config: Option<ExecutionConfig>,
) -> Result<ExecutionResult, String> {
    let executor = match config {
        Some(c) => DockerExecutor::new(c),
        None => DockerExecutor::with_defaults(),
    };
    
    executor.execute(&command)
}

/// 执行脚本
#[tauri::command]
pub fn docker_execute_script(
    script_path: String,
    language: String,
    config: Option<ExecutionConfig>,
) -> Result<ExecutionResult, String> {
    let executor = match config {
        Some(c) => DockerExecutor::new(c),
        None => DockerExecutor::with_defaults(),
    };
    
    executor.execute_script(&script_path, &language)
}
