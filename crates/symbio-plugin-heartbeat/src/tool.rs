//! 心跳设置工具 —— 模型侧的 `heartbeat` 能力（set / get / cancel）
//!
//! ## 它读写的是**别人的存储**，走的是地址面
//!
//! 切出本插件前它直接持有 `Weak<SessionPlugin>` 并调 `get_or_create_session` /
//! `save_session`（宿主的私有项，[D-075](../../../../docs/003-design/002-架构总览.md#d-075-卸载能力先于目录分离session-切分立三条前置会话记忆与存储留守 "DE-002-架构总览") P3 那条
//! 「被切的那片不再 `use super::` 宿主的私有项」要消掉的正是这个）。现在它对会话只说两句话：
//! `stat(<会话>)` 读旧值、`write(<会话>)` 写 `{metadata:{heartbeat:{…}}}`——浅合并由会话侧
//! 的写窗口做（`Session::merge_metadata_object` 是 metadata 写入的唯一实现），本文件不复制那份算术。
//!
//! ## 为什么是**两跳**而不是一跳
//!
//! `set` 是**部分更新**语义（未提供的参数沿用已有值），而写通道按**顶层键整块替换**
//! `heartbeat`。先读后写就是把「部分更新」在调用方补齐——代价是多一次 `stat`，收益是
//! 存储侧不需要知道 `heartbeat` 里有几格、哪格可以缺。
//!
//! ## 校验留在这里，不留在那里
//!
//! 间隔下限与非空提示词是**设置的判据**，不是存储的判据（会话写臂不认识 `heartbeat`
//! 的内容）。写进存储的必须已经是合法的配置，否则调度侧要替它兜底，一份数据两个 owner。

use super::plugin::HeartbeatPlugin;
use super::setting::HeartbeatConfig;
use symbio_deploy::PLUGIN_ID_SESSION;
use symbio_core::{
    vdfs_context, Capability, CapabilityMeta, DynVdfsProvider, ExecEnv, PluginError,
    PluginInvokeRequest, PluginInvokeRequestExt, VdfsContent, VdfsContext, VdfsRequest,
    VdfsResponse, SESSION_ID,
};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::{Arc, Weak};

/// 心跳间隔下限（秒）：调度器扫描周期 15s + 抖动 0-30s，低于 10s 的间隔
/// 没有可观测意义，只会制造密集空转。
const MIN_INTERVAL_SECS: u64 = 10;

/// 心跳设置工具
///
/// 由 [`HeartbeatPlugin`] 的 `Plugin::traverse` 注册进
/// 能力表；弱引用宿主插件，避免 插件 ↔ 工具 的 Arc 循环。
pub struct HeartbeatTool {
    plugin: Weak<HeartbeatPlugin>,
}

impl HeartbeatTool {
    pub fn new(plugin: Weak<HeartbeatPlugin>) -> Self {
        Self { plugin }
    }
}

#[async_trait]
impl Capability for HeartbeatTool {
    fn meta(&self) -> CapabilityMeta {
        CapabilityMeta {
            name: "heartbeat".to_string(),
            description: "设置当前会话的心跳任务：会话空闲达到指定间隔后，自动发送提示词触发一轮工作（后台无人值守执行）。action=set 启用并更新参数；action=cancel 停用；action=get 查看当前配置（默认）。".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["set", "get", "cancel"],
                        "description": "set=启用并更新参数；get=查看当前配置（默认）；cancel=停用（配置保留，可再次 set 启用）"
                    },
                    "interval_seconds": {
                        "type": "integer",
                        "description": "空闲多少秒后触发（set 可选，默认 300，最小 10）"
                    },
                    "prompt": {
                        "type": "string",
                        "description": "心跳触发时发送的提示词（set 可选；启用时必须非空——沿用已有值或本次提供）"
                    },
                    "include_history": {
                        "type": "boolean",
                        "description": "触发时是否携带会话历史（默认 true；false 为无上下文心跳）"
                    }
                }
            }),
            category: Some(symbio_core::CapabilityCategory::SystemOperation),
            examples: Some(vec![
                r#"{"action":"set","interval_seconds":1800,"prompt":"检查任务清单并继续推进待办事项"}"#.to_string(),
                r#"{"action":"cancel"}"#.to_string(),
            ]),
            ..Default::default()
        }
    }

    async fn execute(
        &self,
        args: Value,
        _env: &ExecEnv,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> Result<Value, PluginError> {
        let plugin = self
            .plugin
            .upgrade()
            .ok_or_else(|| PluginError::InternalError("心跳工具的宿主插件已被释放".to_string()))?;
        let provider = plugin.provider().ok_or_else(|| {
            PluginError::InternalError("无法取得本作用域的 VDFS 视图".to_string())
        })?;

        let session_id = ctx
            .get(SESSION_ID)
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| {
                PluginError::ValidationError("心跳设置需要会话上下文（session_id）".to_string())
            })?;
        let vctx = vdfs_context(&ctx);
        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("get");

        let mut hb =
            HeartbeatConfig::from_metadata(&read_metadata(&provider, &vctx, &session_id).await?);

        let data = match action {
            // 查看当前配置（未设置过则返回默认值：未启用）
            "get" => json!({
                "success": true,
                "session_id": session_id,
                "heartbeat": hb.view(),
            }),

            // 启用 / 更新：未提供的参数沿用已有值（部分更新语义）
            "set" => {
                if let Some(v) = args.get("interval_seconds").and_then(|v| v.as_u64()) {
                    if v < MIN_INTERVAL_SECS {
                        return Err(PluginError::ValidationError(format!(
                            "interval_seconds 不能小于 {MIN_INTERVAL_SECS}（调度器扫描周期 15s）"
                        )));
                    }
                    hb.interval_seconds = v;
                }
                if let Some(v) = args.get("prompt").and_then(|v| v.as_str()) {
                    hb.prompt = v.trim().to_string();
                }
                if let Some(v) = args.get("include_history").and_then(|v| v.as_bool()) {
                    hb.include_history = v;
                }
                if hb.prompt.is_empty() {
                    return Err(PluginError::ValidationError(
                        "心跳任务需要非空 prompt（本次提供，或此前已设置）".to_string(),
                    ));
                }
                hb.enabled = true;
                save(&provider, &vctx, &session_id, &hb).await?;

                json!({
                    "success": true,
                    "message": "心跳任务已启用（会话空闲达到间隔后自动触发）",
                    "session_id": session_id,
                    "heartbeat": hb.view(),
                })
            }

            // 停用：配置保留，便于再次 set 启用
            "cancel" => {
                hb.enabled = false;
                save(&provider, &vctx, &session_id, &hb).await?;

                json!({
                    "success": true,
                    "message": "心跳任务已停用（配置保留，可再次 set 启用）",
                    "session_id": session_id,
                    "heartbeat": hb.view(),
                })
            }

            other => {
                return Err(PluginError::ValidationError(format!(
                    "未知 action: {other}（可选 set/get/cancel）"
                )));
            }
        };

        Ok(serde_json::to_value(&data)?)
    }
}

/// 读会话的 metadata 整块（`stat` 的 `attributes.metadata`，与清单同一投影口径）。
async fn read_metadata(
    provider: &DynVdfsProvider,
    vctx: &VdfsContext,
    session_id: &str,
) -> Result<Value, PluginError> {
    match provider
        .dispatch(
            vctx,
            &format!("{PLUGIN_ID_SESSION}/{session_id}"),
            VdfsRequest::Stat,
        )
        .await?
    {
        VdfsResponse::Stat(node) => Ok(node
            .attributes
            .get("metadata")
            .cloned()
            .unwrap_or_else(|| json!({}))),
        _ => Err(PluginError::InternalError(
            "vdfs/stat 未返回节点".to_string(),
        )),
    }
}

/// 写回整块 `heartbeat` 配置（浅合并按顶层键替换，与切片前的 `write_back` 逐字同义）。
async fn save(
    provider: &DynVdfsProvider,
    vctx: &VdfsContext,
    session_id: &str,
    hb: &HeartbeatConfig,
) -> Result<(), PluginError> {
    let body = json!({ "metadata": { "heartbeat": hb.view() } });
    provider
        .dispatch(
            vctx,
            &format!("{PLUGIN_ID_SESSION}/{session_id}"),
            VdfsRequest::Write {
                content: VdfsContent::text(body.to_string()),
            },
        )
        .await
        .map(|_| ())
        .map_err(Into::into)
}

#[cfg(test)]
#[path = "tool.test.rs"]
mod tests;
