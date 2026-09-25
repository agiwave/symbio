//! V2.7 新版分形路由指令 (影子文件 - 极致调试版)

use crate::AppState;
use serde_json::Value;
use std::sync::Arc;
use symbio::symbio_core::{
    PluginFrame, PluginMessageWire, PluginPayload, PluginPayloadWire, KEY_PAYLOAD,
};
use tauri::Emitter;
use tracing::{debug, error, info, warn};

// 线路层消息容器（PluginMessageWire / PluginPayloadWire）统一定义于
// `symbio_core::plugin::transport`，与 HTTP/WebSocket 网关入站共用同一份线上格式，
// 壳层不再重复定义（避免双份漂移）。

#[tauri::command]
pub async fn route_v2(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    request: PluginMessageWire,
    client_id: Option<String>,
) -> Result<PluginMessageWire, String> {
    // 从 metadata 中提取 path
    let path = request
        .metadata
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // 提取并记录 trace_id (如果存在)
    let trace_id = request
        .metadata
        .get("trace_id")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    // 调用**来源**（诊断键，不是协议键）。`trace_id` 只能把同一条链的几次请求
    // 串起来，回答不了「谁发的、为什么发」——而同一个路由名常有多个调用方
    // （如 `vdfs/stat` 既是「实时面缺基线补读」也是「资源信号分辨删除」）。
    // 缺省 `-`：**恒打这个字段**，好让「有没有来源」本身可判、日志行形状稳定。
    let origin = request
        .metadata
        .get("origin")
        .and_then(|v| v.as_str())
        .unwrap_or("-")
        .to_string();

    info!(trace_id = %trace_id, origin = %origin, path = %path, "Start routing");

    // 创建插件上下文 (模拟 from_message 行为)
    let mut extensions = std::collections::HashMap::new();

    // 存储 payload 到扩展桶（桶名与 gateway 的 `build_ctx` 同一份契约）
    extensions.insert(
        KEY_PAYLOAD.to_string(),
        std::sync::Arc::new(request.payload) as std::sync::Arc<dyn std::any::Any + Send + Sync>,
    );

    // 存储 metadata 到扩展桶中
    for (k, v) in request
        .metadata
        .as_object()
        .unwrap_or(&serde_json::Map::new())
    {
        if let Some(s) = v.as_str() {
            extensions.insert(
                k.clone(),
                std::sync::Arc::new(s.to_string())
                    as std::sync::Arc<dyn std::any::Any + Send + Sync>,
            );
        } else {
            extensions.insert(
                k.clone(),
                std::sync::Arc::new(v.clone()) as std::sync::Arc<dyn std::any::Any + Send + Sync>,
            );
        }
    }

    let context = Arc::new(symbio::symbio_core::PluginSimpleRequest {
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
            // 连接的取消令牌：**只用于订阅表收口**（会话/总线插件的反注册任务在等它），
            // 不用于中止业务任务——会话中止走 `session/chat/abort` + ExecAbortSignal。
            // 见 `route_connection::RouteConnection::cancel` 的说明。
            let cancel = chan.cancel_token.clone();
            // 注册连接: 优先使用前端指定的 ID (client_id) 避免握手竞态丢失首帧
            let conn_id = if let Some(id) = client_id {
                rm.register_fixed(id.clone(), chan.tx, cancel.clone()).await;
                id
            } else {
                rm.register(chan.tx, cancel.clone()).await
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

                // EOF：通道已关，订阅该收口了。取消是**幂等**的，与
                // `remove_connection` 的取消重复无害；这里补一次是为了覆盖
                // 「前端没调 close 就消失」的路径。
                cancel.cancel();

                // EOF 通知前端 (channel drop)
                let _ = app.emit(&format!("{event_name}/eof"), ());
            });

            Ok(PluginMessageWire {
                metadata,
                payload: serde_json::to_value(PluginPayloadWire::Connection(conn_id)).unwrap(),
            })
        }
        PluginPayload::Empty => Ok(PluginMessageWire {
            metadata,
            payload: Value::Null,
        }),
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
