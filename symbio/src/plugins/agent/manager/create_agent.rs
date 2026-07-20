use crate::plugins::agent::core::AgentConfig;
use crate::plugins::agent::core::CognitiveUnit;
use crate::plugins::agent::core::{AgentError, AgentResult};
use crate::plugins::agent::manager::manager::AgentManager;
use crate::plugins::agent::manager::model::AgentProfile;
use crate::plugins::agent::plugin::AgentPlugin;
use crate::plugins::agent::store::build_store;
use crate::symbio_core::schemas::common::SimpleResponse;
use crate::symbio_core::CAPABILITY_AGENT_CREATE;
use crate::symbio_core::{
    Capability, CapabilityMeta, InvokeRequest, InvokeRequestExt, InvokeResponse, PluginError,
    PluginPayload,
};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs;
use tracing::{error, info};

/// `agent_create` 能力：创建一个新智能体（人格 + 认知单元）
///
/// 重构要点：
/// 1. 消除 `downcast_ref::<AgentManager>` 反模式，直接通过 `plugin.manager: Arc<AgentManager>` 访问
/// 2. 抽出 `resolve_workspace_dir` 调用走 `path` 模块统一校验
/// 3. 错误消息中文化 + 包含示例
///
/// **存储策略**：智能体统一存储到全局目录 `~/.symbio/plugins/agent/{id}/`，
/// 不再区分 `is_global`（deprecated，保留仅为 API 兼容）。这样：
/// - 所有 workdir 共享同一份 agent 定义
/// - AgentView 等 UI 不需要按 workdir 过滤
/// - 删除/创建操作都走同一个目录
pub struct AgentCreateTool {
    plugin: Arc<AgentPlugin>,
}

impl AgentCreateTool {
    pub fn new(plugin: Arc<AgentPlugin>) -> Self {
        Self { plugin }
    }

    /// 创建 agent 物理目录（统一写入全局目录）
    ///
    /// `is_global` 参数**已弃用**，保留仅为 API 兼容。所有 agent 都写入全局目录。
    async fn create_agent_dir(
        &self,
        manager: &AgentManager,
        _workdir: Option<&str>,
        profile: &AgentProfile,
        _is_global: bool,
    ) -> std::io::Result<PathBuf> {
        // ⭐ 统一存储：所有 agent 都写到 ~/.symbio/plugins/agent/{id}/
        let base_dir = manager.global_dir().to_path_buf();
        let agent_dir = base_dir.join(&profile.id);
        fs::create_dir_all(&agent_dir).await?;
        Ok(agent_dir)
    }

    /// 创建智能体（不含 Capability payload 解析，专为 handler/外部调用设计）
    ///
    /// 与 `Capability::execute` 共享同一份创建逻辑。
    /// `pub` 暴露给 `handlers::create::handle` 与未来的种子/脚本工具使用。
    ///
    /// **注意**：`is_global` 参数已弃用，所有 agent 都写入全局目录。
    pub async fn create_agent(
        &self,
        workdir: Option<&str>,
        id: &str,
        is_global: bool,
        cognition_units: &[CognitiveUnit],
    ) -> AgentResult<AgentProfile> {
        // 关键重构：直接用 `self.plugin.manager: Arc<AgentManager>`，不再 downcast
        let manager: &AgentManager = &self.plugin.manager;

        // 1. 校验：必须存在 id=identity 的认知单元
        let identity_unit = cognition_units
            .iter()
            .find(|unit| unit.id() == "identity")
            .ok_or_else(|| {
                AgentError::validation("缺少必需的认知单元: 'identity'。\n\n\
                     请确保 cognition_units 数组中包含 id 为 'identity' 的认知单元。\n\
                     示例:\n\
                     {\n  \"id\": \"my_agent\",\n  \"cognition_units\": [\n    {\n      \"id\": \"identity\",\n      \"is_a\": [\"fact\"],\n      \"name\": \"My Agent\",\n      \"description\": \"A description of this agent\"\n    }\n  ]\n}\n\n\
                     如需查看完整的使用说明，请调用: agent_create({\"help\": true})".to_string())
            })?;

        // 2. 校验：identity 单元必须有 name
        let name = identity_unit.name().ok_or_else(|| {
            AgentError::validation("identity 认知单元缺少必需的 'name' 字段。\n\n\
                 请确保 identity 单元包含 name 字段。\n\
                 示例:\n\
                 {\n  \"id\": \"identity\",\n  \"is_a\": [\"fact\"],\n  \"name\": \"My Agent\",\n  \"description\": \"...\"\n}\n\n\
                 如需查看完整的使用说明，请调用: agent_create({\"help\": true})".to_string())
        })?;

        let description = identity_unit.description().unwrap_or_default();

        let profile = AgentProfile {
            id: id.to_string(),
            name: name.to_string(),
            description: description.to_string(),
            ..Default::default()
        };

        // 3. 创建目录（统一到全局目录）
        let _agent_dir = self
            .create_agent_dir(manager, workdir, &profile, is_global)
            .await
            .map_err(AgentError::Io)?;

        // 4. 【修复】先直接用 build_store 在磁盘上建出 identity CU，
        //    这样后续 get_mindscape → get_agent → list → scan_dir 才能找到这个 agent。
        //    之前调用顺序是错的：先 get_mindscape 时 index 还是空的。
        let config = AgentConfig::default();
        let store = match build_store(&config, &_agent_dir).await {
            Ok(s) => s,
            Err(e) => {
                error!(error = %e, "build_store FAILED");
                return Err(e.into());
            },
        };
        info!(dir = ?_agent_dir, identity_id = %identity_unit.id(), "build_store");
        match store.upsert(identity_unit).await {
            Ok(_) => info!("identity upsert OK"),
            Err(e) => error!(error = %e, "identity upsert FAILED"),
        }

        // 5. 获取（或构建）mindscape 引擎（经过 index → engine_pool 复用）
        let mindscape = self
            .plugin
            .get_mindscape(workdir, id)
            .await
            .ok_or_else(|| {
                error!(agent_id = %id, dir = ?_agent_dir, "get_mindscape returned None");
                AgentError::profile("无法获取新创建的智能体的 mindscape")
            })?;

        // 6. 写入所有认知单元
        for unit in cognition_units {
            mindscape
                .upsert(unit)
                .await
                .map_err(|e| AgentError::profile(e.to_string()))?;
        }

        // 7. 失效缓存
        manager.invalidate_cache_for_workdir(workdir).await;
        Ok(profile)
    }

    async fn get_help_documentation(&self) -> String {
        let content = include_str!("CREATE_AGENT_SKILL.md");
        content.to_string()
    }
}

#[async_trait]
impl Capability for AgentCreateTool {
    fn meta(&self) -> CapabilityMeta {
        CapabilityMeta {
            name: CAPABILITY_AGENT_CREATE.to_string(),
            description:
                "从认知单元创建智能体。调用 agent_create({\"help\": true}) 查看完整使用说明。"
                    .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "智能体 ID（必需）" },
                    "is_global": { "type": "boolean", "description": "是否全局", "default": false },
                    "cognition_units": {
                        "type": "array",
                        "description": "认知单元数组，必须包含 id 为 'identity' 的单元"
                    },
                    "help": { "type": "boolean", "description": "true=查看帮助文档", "default": false }
                }
            }),
            category: Some(crate::symbio_core::CapabilityCategory::Chat),
            examples: Some(vec!["id='my-agent', cognition_units=[{...}]".to_string()]),
            ..Default::default()
        }
    }

    async fn execute(&self, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload> {
        let tool_ctx = ctx.fork();
        let workdir: Option<String> = tool_ctx.get(crate::symbio_core::WORKDIR);
        let payload: Value = ctx.payload()?;

        // 1. 优先响应 help
        if payload.get("help").and_then(|v| v.as_bool()) == Some(true) {
            let help_content = self.get_help_documentation().await;
            return Ok(PluginPayload::new(&json!({
                "status": "success",
                "message": "agent_create 工具使用文档",
                "documentation": help_content
            })));
        }

        // 2. 解析参数
        #[derive(serde::Deserialize, Clone)]
        struct CreateRequest {
            id: Option<String>,
            #[serde(default)]
            is_global: bool,
            cognition_units: Option<Vec<CognitiveUnit>>,
        }

        let req: CreateRequest = serde_json::from_value(payload.clone()).map_err(|e| {
            PluginError::ValidationError(format!(
                "参数解析错误: {}\n\n请检查您的参数格式是否正确。\
                 如需查看完整的使用说明，请调用: agent_create({{\"help\": true}})",
                e
            ))
        })?;

        // 3. 必填校验：id
        if req.id.is_none() || req.id.as_ref().map(|s| s.trim().is_empty()).unwrap_or(true) {
            return Err(PluginError::ValidationError(
                "缺少必需参数: 'id'。\n\n\
                 请提供智能体的唯一标识符，例如:\n\
                 {\n  \"id\": \"my_agent\",\n  \"cognition_units\": [...]\n}\n\n\
                 如需查看完整的使用说明，请调用: agent_create({\"help\": true})"
                    .to_string(),
            ));
        }

        // 4. 必填校验：cognition_units 非空
        if req.cognition_units.is_none()
            || req
                .cognition_units
                .as_ref()
                .map(|v| v.is_empty())
                .unwrap_or(true)
        {
            return Err(PluginError::ValidationError(
                "缺少必需参数: 'cognition_units'。\n\n\
                 请提供认知单元数组，至少包含一个 id 为 'identity' 的单元。\n\
                 如需查看完整的使用说明，请调用: agent_create({\"help\": true})"
                    .to_string(),
            ));
        }

        // 5. 调用 create_agent
        // 必填校验已在步骤 3/4 完成，unwrap 安全
        let id = req.id.expect("id 已校验非空");
        let is_global = req.is_global;
        let cognition_units = req.cognition_units.expect("cognition_units 已校验非空");

        let result = self
            .create_agent(
                workdir.as_deref(),
                &id,
                is_global,
                cognition_units.as_slice(),
            )
            .await
            .map_err(|e| PluginError::InternalError(e.to_string()))?;

        Ok(PluginPayload::new(&SimpleResponse::success_with_message(
            format!("智能体 '{}' 已成功创建", result.id),
        )))
    }
}

#[cfg(test)]
#[path = "create_agent_tests.rs"]
mod tests;

// ═══════════════════════════════════════════════════════════════════════════
// 工厂 + 自注册
// ---------------------------------------------------------------------
// 与 `capabilities/*` 一致：工厂签名遵循 `submit_object_creator!` 协议；
// 运行期依赖（`AgentPlugin`）通过 `AGENT_CAPABILITY_CONTEXT` 键注入。
// ═══════════════════════════════════════════════════════════════════════════

pub fn build_create_agent(ctx: Arc<dyn InvokeRequest>) -> Arc<dyn Capability> {
    let cap_ctx = crate::plugins::agent::capabilities::get_capability_context(ctx.as_ref());
    Arc::new(AgentCreateTool::new(cap_ctx.plugin.clone())) as Arc<dyn Capability>
}

crate::submit_object_creator!(CAPABILITY_AGENT_CREATE, build_create_agent, dyn Capability);
