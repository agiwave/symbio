//! Tauri 命令处理模块
//!
//! 提供核心命令：
//! - `meta`: 获取插件元数据
//! - `invoke`: 同步调用，返回完整结果
//! - `stream`: 流式调用，通过 Channel 实时推送每个 chunk
//! - `connect`: 建立持久双向连接
//! - `connect.send`: 通过连接发送消息
//! - `connect.close`: 关闭连接
//! - `connect.status`: 查询连接状态
//!
//! 所有插件能力都通过这六个标准命令访问，遵循统一的插件架构。

use crate::AppState;
use symbio::{PluginMeta, StreamChunk, InvokeStream};
use serde::{Deserialize, Serialize};
use tauri::Emitter;
use std::sync::Arc;

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

// ==================== Connect 相关命令 ====================

/// 建立持久连接
///
/// 前端调用后，会返回一个 connection_id。
/// 前端通过监听 `connect/{connection_id}` 事件接收插件消息。
/// 前端通过 `connect.send` 向插件发送消息。
#[tauri::command]
pub async fn connect(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    path: String,
    input: serde_json::Value,
) -> Result<String, String> {
    let root = state.root.lock().map_err(|e| e.to_string())?;
    let connection_manager = state.connection_manager.lock().map_err(|e| e.to_string())?;

    // 创建连接
    let conn = connection_manager.create(
        Arc::new(crate::event_sender::TauriEventSender::new(app.clone())),
    );
    let conn_id = conn.id.clone();

    // 调用 root plugin 的 connect 方法，让它自行处理路径路由
    let conn_for_plugin = conn.clone();
    let path_clone = path.clone();
    let root_clone = root.clone();
    tokio::spawn(async move {
        if let Err(e) = root_clone.connect(&path_clone, input, conn_for_plugin).await {
            eprintln!("[connect] Plugin connect error: {}", e);
        }
    });

    Ok(conn_id)
}

/// 通过连接发送消息到插件
#[tauri::command]
pub async fn connect_send(
    state: tauri::State<'_, AppState>,
    connection_id: String,
    message: serde_json::Value,
) -> Result<(), String> {
    let connection_manager = state.connection_manager.lock().map_err(|e| e.to_string())?;

    let conn = connection_manager.get(&connection_id)
        .ok_or_else(|| format!("Connection not found: {}", connection_id))?;

    if conn.is_closed() {
        return Err("Connection is closed".to_string());
    }

    // 触发连接的消息处理器
    conn.handle_message(message);

    Ok(())
}

/// 关闭连接
#[tauri::command]
pub async fn connect_close(
    state: tauri::State<'_, AppState>,
    connection_id: String,
) -> Result<(), String> {
    let connection_manager = state.connection_manager.lock().map_err(|e| e.to_string())?;

    if let Some(conn) = connection_manager.remove(&connection_id) {
        conn.close("client_closed").ok();
        eprintln!("[connect] Connection closed: {}", connection_id);
    }

    Ok(())
}

/// 查询连接状态
#[tauri::command]
pub async fn connect_status(
    state: tauri::State<'_, AppState>,
    connection_id: String,
) -> Result<ConnectStatusResponse, String> {
    let connection_manager = state.connection_manager.lock().map_err(|e| e.to_string())?;

    let alive = connection_manager.is_alive(&connection_id);

    Ok(ConnectStatusResponse {
        connection_id,
        alive,
    })
}

/// 连接状态响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectStatusResponse {
    pub connection_id: String,
    pub alive: bool,
}