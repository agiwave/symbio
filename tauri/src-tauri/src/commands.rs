//! V2.7 新版分形路由指令 (影子文件 - 极致调试版)

use crate::AppState;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use symbio::symbio_core::{PluginFrame, PluginPayload, SymbioKey};
use tauri::Emitter;
use tracing::{debug, error, info, warn};

/// FFI 传输层的统一消息结构
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginMessageWire {
    pub metadata: Value,
    pub payload: Value,
}

/// FFI 传输层的载荷结构
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum PluginPayloadWire {
    /// 立即响应数据
    Data(Value),
    /// 连接已建立，返回 connection_id
    Connection(String),
}

#[tauri::command]
pub async fn route_v2(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    request: PluginMessageWire,
    client_id: Option<String>,
) -> Result<PluginMessageWire, String> {
    // 从 metadata 中提取 path
    let path = request.metadata.get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    
    // 提取并记录 trace_id (如果存在)
    let trace_id = request.metadata.get("trace_id")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    
    info!(trace_id = %trace_id, path = %path, "Start routing");

    // 创建插件上下文 (模拟 from_message 行为)
    let mut extensions = std::collections::HashMap::new();

    // 存储 payload 到扩展桶
    #[allow(deprecated)]
    extensions.insert(
        symbio::symbio_core::PAYLOAD.name().to_string(),
        std::sync::Arc::new(request.payload) as std::sync::Arc<dyn std::any::Any + Send + Sync>,
    );
    
    // 存储 metadata 到扩展桶中
    for (k, v) in request.metadata.as_object().unwrap_or(&serde_json::Map::new()) {
        if let Some(s) = v.as_str() {
            extensions.insert(k.clone(), std::sync::Arc::new(s.to_string()) as std::sync::Arc<dyn std::any::Any + Send + Sync>);
        } else {
            extensions.insert(k.clone(), std::sync::Arc::new(v.clone()) as std::sync::Arc<dyn std::any::Any + Send + Sync>);
        }
    }
    
    let context = Arc::new(symbio::symbio_core::SimpleRequest {
        envs: std::sync::Arc::new(std::sync::RwLock::new(std::collections::HashMap::new())),
        extensions: std::sync::Arc::new(std::sync::RwLock::new(extensions)),
    });

    let root = state.root.clone();
    let rm = state.route_manager.clone();
    let payload = root.route(context).await.map_err(|e| {
        error!(trace_id = %trace_id, path = %path, error = %e, "Error routing");
        e.to_string()
    })?;

    debug!(trace_id = %trace_id, path = %path, "Routing finished");

    let metadata = serde_json::json!({});
    match payload {
        PluginPayload::Data(_) => {
            let value = payload.serialize().map_err(|e| {
                error!(trace_id = %trace_id, path = %path, error = %e, "Failed to serialize payload");
                e
            })?;
            Ok(PluginMessageWire {
                metadata,
                payload: serde_json::to_value(PluginPayloadWire::Data(value)).unwrap(),
            })
        }
        PluginPayload::Session(chan) => {
            // 注册连接: 优先使用前端指定的 ID (client_id) 避免握手竞态丢失首帧
            let conn_id = if let Some(id) = client_id {
                rm.register_fixed(id.clone(), chan.tx).await;
                id
            } else {
                rm.register(chan.tx).await
            };
            let event_name = format!("route/{conn_id}");

            // 启动转发泵
            let conn_id_clone = conn_id.clone();
            tokio::spawn(async move {
                let mut rx = chan.rx;
                while let Some(frame) = rx.recv().await {
                    if let Err(e) = app.emit(&event_name, &frame) {
                        warn!(conn_id = %conn_id_clone, error = %e, "Emit error to frontend");
                    }
                }
                
                // EOF 通知前端 (channel drop)
                let _ = app.emit(&format!("{event_name}/eof"), ());
            });

            Ok(PluginMessageWire { 
                metadata, 
                payload: serde_json::to_value(PluginPayloadWire::Connection(conn_id)).unwrap(),
            })
        }
        PluginPayload::Native(_) => {
            info!(trace_id = %trace_id, path = %path, "Native payload returned, skipping for FFI");
            Ok(PluginMessageWire {
                metadata,
                payload: Value::Null,
            })
        }
        PluginPayload::Empty => {
            Ok(PluginMessageWire {
                metadata,
                payload: Value::Null,
            })
        }
    }
}

#[tauri::command]
pub async fn route_v2_send(
    state: tauri::State<'_, AppState>,
    connection_id: String,
    frame: PluginFrame,
) -> Result<(), String> {
    state.route_manager.send(&connection_id, frame).await
}

#[tauri::command]
pub async fn route_v2_close(
    state: tauri::State<'_, AppState>,
    connection_id: String,
) -> Result<(), String> {
    // 移除连接会自动 drop channel tx，关闭前端连接
    // 此处不发送 abort 信号，使得后端业务任务能独立于前端连接继续运行
    state.route_manager.remove_connection(&connection_id).await;
    Ok(())
}
