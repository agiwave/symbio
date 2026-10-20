//! V2.7 新版分形路由指令 (影子文件 - 极致调试版)

use crate::AppState;
use serde_json::Value;
use std::sync::Arc;
use symbio::symbio_core::{
    PluginFrame, PluginMessageWire, PluginPayload, PluginPayloadWire, SymbioKey,
    PLUGIN_PAYLOAD_KEY, TRACE_ID,
};
use tauri::Emitter;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

/// 把一条会话通道的帧泵到前端，并在 EOF 时收口订阅。
///
/// ## 为什么单独成一个函数
///
/// 泵里有两条**都必须发生**的 EOF 动作：取消订阅令牌（会话/总线插件的反注册任务
/// 在等它）与通知前端 `…/eof`。它们此前只在 `route_v2` 的 `spawn` 块里，而那一块
/// 需要 `AppHandle`，单元测试够不着——于是「前端没调 close 就消失」这条路径
/// 长期只有人工实机验证。把泵抽成只依赖 `(rx, cancel, emit)` 的形状之后，
/// 命令层可以用一个记录闭包直接钉住这两条动作。
pub(crate) fn spawn_forward_pump<F>(
    mut rx: mpsc::Receiver<PluginFrame>,
    event_name: String,
    cancel: CancellationToken,
    emit: F,
) -> tokio::task::JoinHandle<()>
where
    F: Fn(&str, Option<&PluginFrame>) + Send + 'static,
{
    tokio::spawn(async move {
        while let Some(frame) = rx.recv().await {
            emit(&event_name, Some(&frame));
        }

        // EOF：通道已关，订阅该收口了。取消是**幂等**的，与
        // `remove_connection` 的取消重复无害；这里补一次是为了覆盖
        // 「前端没调 close 就消失」的路径。
        cancel.cancel();

        // EOF 通知前端 (channel drop)
        emit(&format!("{event_name}/eof"), None);
    })
}

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

    // 提取并记录 trace_id (如果存在)。键名来自键面（`TRACE_ID.name()`）而不是再写
    // 一遍字面量：协议 `metadata` 的键与上下文键是**同一张表**，两处各写一遍必然漂移。
    let trace_id = request
        .metadata
        .get(TRACE_ID.name())
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
        PLUGIN_PAYLOAD_KEY.to_string(),
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
            // 不用于中止业务任务——会话中止走 `vdfs/action(<会话地址>, "abort")`
            // + ExecAbortSignal。见 `route_connection::RouteConnection::cancel` 的说明。
            let cancel = chan.cancel_token.clone();
            // 注册连接: 优先使用前端指定的 ID (client_id) 避免握手竞态丢失首帧
            let conn_id = if let Some(id) = client_id {
                rm.register_fixed(id.clone(), chan.tx, cancel.clone()).await;
                id
            } else {
                rm.register(chan.tx, cancel.clone()).await
            };
            let event_name = format!("route/{conn_id}");

            // 启动转发泵：帧 → 前端；EOF → 取消订阅令牌 + `…/eof`。
            // 泵本身不认识 `AppHandle`（见 [`spawn_forward_pump`]），这里只把
            // 「怎么发」作为闭包交进去。
            let conn_id_clone = conn_id.clone();
            spawn_forward_pump(
                chan.rx,
                event_name,
                cancel.clone(),
                move |event, frame| match frame {
                    Some(frame) => {
                        if let Err(e) = app.emit(event, frame) {
                            warn!(conn_id = %conn_id_clone, error = %e, "Emit error to frontend");
                        }
                    }
                    None => {
                        let _ = app.emit(event, ());
                    }
                },
            );

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

#[cfg(test)]
#[path = "commands.test.rs"]
mod tests;
