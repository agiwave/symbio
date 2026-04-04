//! Tauri 命令处理模块
//!
//! 提供三个核心命令：
//! - `meta`: 获取插件元数据
//! - `invoke`: 同步调用，返回完整结果
//! - `stream`: 流式调用，通过 Channel 实时推送每个 chunk
//!
//! 所有插件能力都通过这三个标准命令访问，遵循统一的插件架构。

use crate::AppState;
use crate::core::types::{PluginMeta, StreamChunk, InvokeStream};
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

/// 流式调用插件，通过 Channel 实时推送每个 chunk
///
/// 适用场景：需要渐进式渲染的流式场景
///
/// 前端使用示例：
/// ```typescript
/// import { invoke } from '@tauri-apps/api/core'
/// 
/// await invoke('stream', {
///   path: 'agent/chat',
///   input: { messages: [...], session_id: 'xxx' }
/// })
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
                eprintln!("[commands] emitting chunk: done={}, error={:?}", chunk.done, chunk.error);
                if let Err(e) = app.emit(&event_id, &chunk) {
                    eprintln!("[commands] emit error: {}", e);
                }
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