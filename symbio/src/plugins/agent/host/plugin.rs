//! agent 插件 —— OAB 协议的宿主接入层（单协议、约定目录装配）。
//!
//! ## 定位（规范 §12 宿主参考实现）
//!
//! symbio 是 OAB 协议的第一个接入方：本插件把 bundle 的约定目录装配语义
//! 映射进 symbio 的会话机制——
//!
//! - **提示词**：`traverse(available_tools)` 时扫描 `prompts/` `skills/`，
//!   把片段汇总为 [`BundleIdentityCapability`]（`agent_identity` 工具），
//!   宿主据此追加/影响系统提示词（老 agent 插件同款语义，**零会话编排改动**）；
//! - **MCP**：扫描 `mcps/` 得到 MCP server 声明，**交给宿主已有的 MCP 客户端**启动
//!   （OAB 不重新发明工具运行时，工具唯一来源即 MCP）；
//! - **版本匹配**：bundle 校验（含 `requires.spec` 硬门槛）失败 = 收集期
//!   硬错误，会话中止并明确报错——绝不静默降级成"没有人格的通用助手"。
//!
//! 会话未选择智能体（`ctx[AGENT_ID]` 为空）时，本插件仅静默跳过 bundle 装配，
//! 但 **agent_run（子智能体委托）始终注册**——未指定 agent_id 时默认沿用当前
//! 会话的智能体，两者皆空则子会话以无智能体的纯对话模式运行。

use crate::plugins::agent::core::spec::assembly::{assemble_bundle, Assembly};
use crate::plugins::agent::host::capability::BundleIdentityCapability;
use crate::plugins::agent::host::handlers;
use crate::plugins::agent::host::store::{BundleRecord, BundleStore};
use crate::symbio_core::{
    report_error, Capability, InvokeRequest, InvokeRequestExt, InvokeResponse, Plugin, PluginError,
    PluginMeta, PluginPayload, AGENT_ID, PATH, PLUGIN_AGENT, SESSION_ID, TRAVERSE_AVAILABLE_TOOLS,
    WORKDIR,
};
use async_trait::async_trait;
use std::sync::Arc;

/// AgentBundle 插件主结构。
pub struct AgentPlugin {
    /// 插件间路由入口（composite 容器的弱引用）。
    ///
    /// 在 `build(ctx)` 时捕获：composite 构造子插件时注入的 ctx 已携带
    /// `PARENT` 弱引用（见 composite 的子上下文构造）。agent_run 工具执行期
    /// 凭它把 `session/chat/send` 等请求路由给兄弟插件——工具 ctx 经 chat
    /// loop 一路 fork，自身并不携带路由入口。
    router: Option<std::sync::Weak<dyn Plugin>>,
}

impl AgentPlugin {
    /// 静态工厂：从 InvokeRequest 构造 Plugin 实例（composite 配置驱动）。
    pub fn build(ctx: Arc<dyn InvokeRequest>) -> Arc<dyn Plugin> {
        // 消费配置以兼容未来扩展（当前使用默认值）
        let _config = ctx.config().as_ref().and_then(|c| c.get("agent"));
        let router = ctx.parent();
        Arc::new(Self { router }) as Arc<dyn Plugin>
    }

    pub fn new() -> Self {
        Self { router: None }
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new(PLUGIN_AGENT, "Agent Bundle（OAB 协议）")
            .with_description(
                "OAB（Open Agent Bundle）协议宿主：管理 bundle 实例（安装/导出/删除），\
                 会话绑定 bundle 时按约定目录装配系统提示词（prompts/ skills/）\
                 与 MCP server 声明（mcps/）",
            )
            .with_version("0.1.0")
    }

    /// traverse 主逻辑：扫描约定目录 → 装配 → 身份工具注册。
    async fn attach_bundle(
        self: &Arc<Self>,
        _ctx: &Arc<dyn InvokeRequest>,
        bundle_id: &str,
        workdir: Option<String>,
        _session_id: Option<String>,
        tool_visitor: &Arc<dyn crate::symbio_core::CapabilityVisitor>,
    ) -> Result<(), PluginError> {
        // ── 1. 加载 bundle ──
        let store = BundleStore::new(workdir.as_deref());
        let record: BundleRecord = store.get(bundle_id).ok_or_else(|| {
            PluginError::NotFound(format!(
                "bundle `{bundle_id}` 不存在（workdir={workdir:?}），无法开始对话。\
                 请重新选择 bundle。"
            ))
        })?;

        // ── 2. 约定目录装配（纯数据扫描，规范 §3 / §5）──
        let assembly: Assembly = assemble_bundle(&record.dir, &record.manifest);

        // 软故障诊断：转 warning 日志，不阻断装配
        for d in &assembly.diagnostics {
            tracing::warn!(code = %d.code, message = %d.message, "[oab] 装配诊断");
        }
        // MCP server 声明：交给宿主已有的 MCP 客户端启动（工具唯一来源）。
        // 参考实现尚未桥接时如实告警，不静默吞掉——宿主接入 MCP 客户端即在此消费。
        for s in &assembly.mcp_servers {
            tracing::warn!(
                server = %s.name,
                "[oab] mcp server `{}` 已声明，等待宿主 MCP 客户端桥接启动",
                s.name
            );
        }

        // ── 3. 提示词片段 → agent_identity 身份工具（老 agent 同款语义）──
        let mut caps: Vec<Arc<dyn Capability>> = Vec::new();
        let identity_text = assembly.identity_text();
        if !identity_text.trim().is_empty() {
            caps.push(
                BundleIdentityCapability::new(identity_text, record.manifest.id.clone())
                    as Arc<dyn Capability>,
            );
        }

        tool_visitor.register_batch(caps).await;
        Ok(())
    }
}

impl Default for AgentPlugin {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Plugin for AgentPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        ctx: Arc<dyn InvokeRequest>,
    ) -> InvokeResponse<PluginPayload> {
        let sub_path = ctx.get(PATH).unwrap_or_default();
        if sub_path != TRAVERSE_AVAILABLE_TOOLS {
            return Err(PluginError::NotFound("Invalid traverse path".to_string()));
        }

        // ── 会话级「智能体选择」复用既有通用机制 ctx[AGENT_ID]（orchestrator
        // 统一解析：请求显式 > metadata.agent_id）；本插件把它解析为 bundle 实例 id。
        // 注意：agent_run（子智能体委托）**始终注册**，不受"是否选择智能体"影响——
        // 它是会话基础能力：未指定 agent_id 时默认沿用当前会话的智能体，
        // 两者皆空则子会话以无智能体的纯对话模式运行。
        let bundle_id = ctx
            .get(AGENT_ID)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let workdir = ctx.get(WORKDIR);
        let session_id = ctx.get(SESSION_ID);

        let Some(tool_visitor) = ctx.get(crate::symbio_core::CAPABILITY_VISITOR) else {
            return Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()));
        };

        // ── agent_run：无条件注册 ──
        let store = BundleStore::new(workdir.as_deref());
        tool_visitor
            .register_batch(vec![super::subagent::AgentRunCapability::new(
                workdir.clone(),
                super::subagent::format_bundles_brief(&store.list()),
                self.router.clone(),
            ) as Arc<dyn Capability>])
            .await;

        // ── 已选择智能体 → 约定目录装配（身份工具等）──
        if let Some(bundle_id) = bundle_id {
            if let Err(e) = self
                .attach_bundle(&ctx, &bundle_id, workdir, session_id, &tool_visitor)
                .await
            {
                // 硬错误：会话绑定了一个不合规/不存在的 bundle——必须中止并明确提示，
                // 绝不静默降级（与老 agent 插件的收集期错误语义一致）。
                report_error(
                    &ctx,
                    PLUGIN_AGENT,
                    format!("bundle `{bundle_id}` 装配失败: {e}"),
                )
                .await;
            }
        }

        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }

    async fn route(self: Arc<Self>, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload> {
        let path = ctx.get(PATH).unwrap_or_default();
        let path = path.strip_prefix('/').unwrap_or(&path);
        handlers::route(&self, path, ctx).await
    }
}

crate::submit_object_creator!(PLUGIN_AGENT, AgentPlugin::build, dyn Plugin);
