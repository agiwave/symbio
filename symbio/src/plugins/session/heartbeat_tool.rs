//! 心跳设置工具 —— session 插件内聚实现
//!
//! 心跳机制的全部环节都在 session 插件内闭环：
//! - 配置存储：`Session.metadata.heartbeat`（[`HeartbeatConfig`]，types.rs）
//! - 后台调度：`heartbeat.rs` 的调度循环（随插件树构建启动，常驻运行）
//! - 触发执行：`trigger_heartbeat` 复用 chat/send 统一入口
//!
//! 本工具是这套机制的**设置入口**：模型在会话内调用它启用/调整/停用本会话
//! 的心跳任务。实现上直接读写本插件的会话存储（get_or_create_session /
//! save_session），不经任何跨模块路由——机制与设置工具同插件，零跨模块依赖。

use super::plugin::SessionPlugin;
use super::types::HeartbeatConfig;
use crate::symbio_core::{
    Capability, CapabilityMeta, ExecEnv, InvokeRequest, InvokeRequestExt, PluginError, SESSION_ID,
};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::{Arc, Weak};

/// 心跳间隔下限（秒）：调度器扫描周期 15s + 抖动 0-30s，低于 10s 的间隔
/// 没有可观测意义，只会制造密集空转。
const MIN_INTERVAL_SECS: u64 = 10;

/// 心跳设置工具
///
/// 由 [`SessionPlugin::traverse`] 按需创建并注册进 `CAPABILITY_VISITOR`；
/// 弱引用宿主插件，避免 插件 ↔ 工具 的 Arc 循环。
pub struct HeartbeatTool {
    plugin: Weak<SessionPlugin>,
}

impl HeartbeatTool {
    pub fn new(plugin: Weak<SessionPlugin>) -> Self {
        Self { plugin }
    }

    /// 配置的对外视图（与 metadata.heartbeat 存储结构一致）
    fn config_view(hb: &HeartbeatConfig) -> Value {
        json!({
            "enabled": hb.enabled,
            "interval_seconds": hb.interval_seconds,
            "prompt": hb.prompt,
            "include_history": hb.include_history,
        })
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
            category: Some(crate::symbio_core::CapabilityCategory::SystemOperation),
            examples: Some(vec![
                r#"{"action":"set","interval_seconds":1800,"prompt":"检查任务清单并继续推进待办事项"}"#.to_string(),
                r#"{"action":"cancel"}"#.to_string(),
            ]),
            // 配置型写入：每次全量回显生效配置，历史版本对后续推理无参考价值
            // → 仅保留最近一次调用的完整参数/结果（机制化声明，压缩层通用执行）。
            context_retention: Some(crate::symbio_core::ToolContextRetention::LastOnly),
            ..Default::default()
        }
    }

    async fn execute(
        &self,
        args: Value,
        _env: &ExecEnv,
        ctx: Arc<dyn InvokeRequest>,
    ) -> Result<Value, PluginError> {
        let plugin = self
            .plugin
            .upgrade()
            .ok_or_else(|| PluginError::InternalError("心跳工具的宿主插件已被释放".to_string()))?;

        let session_id = ctx
            .get(SESSION_ID)
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| {
                PluginError::ValidationError("心跳设置需要会话上下文（session_id）".to_string())
            })?;

        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("get");

        let mut session = plugin.get_or_create_session(&session_id).await?;
        let mut hb = HeartbeatConfig::from_metadata(&session.metadata);

        let data = match action {
            // 查看当前配置（未设置过则返回默认值：未启用）
            "get" => json!({
                "success": true,
                "session_id": session_id,
                "heartbeat": Self::config_view(&hb),
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

                Self::write_back(&mut session, &hb);
                plugin.save_session(&session).await?;

                json!({
                    "success": true,
                    "message": "心跳任务已启用（会话空闲达到间隔后自动触发）",
                    "session_id": session_id,
                    "heartbeat": Self::config_view(&hb),
                })
            }

            // 停用：配置保留，便于再次 set 启用
            "cancel" => {
                hb.enabled = false;
                Self::write_back(&mut session, &hb);
                plugin.save_session(&session).await?;

                json!({
                    "success": true,
                    "message": "心跳任务已停用（配置保留，可再次 set 启用）",
                    "session_id": session_id,
                    "heartbeat": Self::config_view(&hb),
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

impl HeartbeatTool {
    /// 把配置写回会话 metadata（整体替换 heartbeat 子对象）并刷新 updated_at。
    ///
    /// updated_at 即调度器的磁盘侧空闲基准：进程重启后内存活动表清空，
    /// 调度器以 updated_at 追补判定，重启不丢心跳节奏。
    fn write_back(session: &mut super::types::Session, hb: &HeartbeatConfig) {
        let hb_json = serde_json::to_value(hb).unwrap_or_else(|_| json!({}));
        if let Some(obj) = session.metadata.as_object_mut() {
            obj.insert("heartbeat".to_string(), hb_json);
        } else {
            session.metadata = json!({ "heartbeat": hb_json });
        }
        session.updated_at = crate::symbio_core::now_ms();
    }
}

#[cfg(test)]
#[path = "heartbeat_tool.test.rs"]
mod tests;
