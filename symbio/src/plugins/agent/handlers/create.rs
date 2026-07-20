//! `agent/create` 路由 — 程序化创建智能体
//!
//! 设计动机：seed 工具、外部脚本、CI 流水线需要**不经 LLM** 就能批量创建 agent。
//! 复用 [`AgentCreateTool::create_agent`] 的核心创建逻辑，避免与 Capability 入口走两套代码。
//!
//! ## 调用方
//!
//! - `bin/seed_agents.rs`：批量创建 7 角色的软件项目开发智能体体系
//! - 未来：CI 流水线 / 远程管理端
//!
//! ## PATH
//!
//! `agent/create`（composite 分发后会到 agent plugin，PATH 变成 `create`）

use std::sync::Arc;

use serde::Deserialize;
use serde_json::Value;

use crate::plugins::agent::core::CognitiveUnit;
use crate::plugins::agent::manager::create_agent::AgentCreateTool;
use crate::plugins::agent::plugin::AgentPlugin;
use crate::symbio_core::schemas::common::SimpleResponse;
use crate::symbio_core::{
    InvokeRequest, InvokeRequestExt, InvokeResponse, PluginError, PluginPayload,
};

#[derive(Deserialize)]
struct CreateReq {
    id: String,
    #[serde(default)]
    is_global: bool,
    cognition_units: Vec<CognitiveUnit>,
}

pub async fn handle(
    plugin: Arc<AgentPlugin>,
    ctx: Arc<dyn InvokeRequest>,
    workdir_opt: Option<&str>,
) -> InvokeResponse<PluginPayload> {
    let payload: Value = ctx.payload()?;

    // 必填校验 — 简单直接（不走 LLM 提示符路径）
    let req: CreateReq = serde_json::from_value(payload).map_err(|e| {
        PluginError::ValidationError(format!(
            "agent/create 参数解析失败: {}。期望字段: id, is_global?, cognition_units[]",
            e
        ))
    })?;

    if req.id.trim().is_empty() {
        return Err(PluginError::ValidationError(
            "agent/create: id 不能为空".to_string(),
        ));
    }
    if req.cognition_units.is_empty() {
        return Err(PluginError::ValidationError(
            "agent/create: cognition_units 不能为空，至少需要一个 id='identity' 的单元".to_string(),
        ));
    }

    let tool = AgentCreateTool::new(plugin);
    let profile = tool
        .create_agent(workdir_opt, &req.id, req.is_global, &req.cognition_units)
        .await
        .map_err(|e| PluginError::InternalError(e.to_string()))?;

    Ok(PluginPayload::new(&SimpleResponse::success_with_message(
        format!("智能体 '{}' (name='{}') 已创建", profile.id, profile.name),
    )))
}
