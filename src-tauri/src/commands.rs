//! Tauri 命令处理模块
//!
//! 只提供三个核心命令：
//! - `get`: 通过路径获取插件的 PluginMeta 信息
//! - `invoke`: 同步调用插件
//! - `sinvoke`: 流式调用插件

use crate::AppState;
use crate::core::types::{PluginMeta, StreamChunk};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 通过路径获取插件的 PluginMeta 信息
///
/// path 规则：
/// - 空路径：返回 root 插件自身的元数据
/// - ["plugin_name"]：返回指定插件的元数据
/// - ["plugin_name", "child_name", ...]：逐级获取子插件的元数据（分形模式）
#[tauri::command]
pub fn meta(
    state: tauri::State<AppState>,
    path: Vec<String>,
) -> Result<MetaResponse, String> {
    let root = state.root.lock().map_err(|e| e.to_string())?;

    // 空路径返回 root 自身元数据
    if path.is_empty() {
        return Ok(MetaResponse {
            path: "root".to_string(),
            meta: root.meta(),
        });
    }

    // 逐级查找子插件
    let plugin = root.plugin(&path)
        .ok_or_else(|| format!("插件路径 '{}' 未找到", path.join("/")))?;

    Ok(MetaResponse {
        path: path.join("/"),
        meta: plugin.meta(),
    })
}

/// 同步调用插件
///
/// path 规则：
/// - 空路径：调用 root 插件的 invoke 方法
/// - ["plugin_name"]：调用指定插件
/// - ["plugin_name", "child_name", ...]：逐级调用子插件（分形模式）
#[tauri::command]
pub fn invoke(
    state: tauri::State<AppState>,
    path: Vec<String>,
    input: Value,
) -> Result<Value, String> {
    let root = state.root.lock().map_err(|e| e.to_string())?;

    // 空路径调用 root 自身
    if path.is_empty() {
        let rt = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
        return Ok(rt.block_on(root.invoke(input))?);
    }

    // 逐级查找并调用子插件
    let plugin = root.plugin(&path)
        .ok_or_else(|| format!("插件路径 '{}' 未找到", path.join("/")))?;

    let rt = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
    Ok(rt.block_on(plugin.invoke(input))?)
}

/// 流式调用插件
///
/// path 规则：
/// - ["plugin_name"]：调用指定插件的流式接口
/// - ["plugin_name", "child_name", ...]：逐级调用子插件的流式接口（分形模式）
///
/// 注意：空路径不支持流式调用
#[tauri::command]
pub fn sinvoke(
    state: tauri::State<AppState>,
    path: Vec<String>,
    input: Value,
) -> Result<Vec<StreamChunk>, String> {
    let root = state.root.lock().map_err(|e| e.to_string())?;

    // 空路径不支持流式调用
    if path.is_empty() {
        return Err("路径不能为空".to_string());
    }

    // 逐级查找并调用子插件
    let plugin = root.plugin(&path)
        .ok_or_else(|| format!("插件路径 '{}' 未找到", path.join("/")))?;

    let rt = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
    Ok(rt.block_on(plugin.sinvoke(input))?)
}

/// Meta 命令响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetaResponse {
    /// 插件路径
    pub path: String,
    /// 插件元数据
    pub meta: PluginMeta,
}
