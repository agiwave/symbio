use crate::plugins::agent::core::AgentConfig;
use crate::plugins::agent::core::AgentStore;
use crate::plugins::agent::core::PromptBudget;

use crate::plugins::agent::handlers::system_prompt;
use crate::plugins::agent::manager::{AgentManager, AgentRegistry};

use crate::symbio_core::{
    Capability, InvokeRequest, InvokeRequestExt, InvokeResponse, Plugin, PluginError, PluginMeta,
    PluginPayload, SymbioKey, CAPABILITY_AGENT_CHAT, CAPABILITY_AGENT_COGNITION,
    CAPABILITY_AGENT_CREATE, CAPABILITY_AGENT_IDENTITY, PLUGIN_AGENT,
};
use async_trait::async_trait;

use std::sync::Arc;
use tokio::sync::RwLock;

/// Agent 插件主结构
///
/// ## 架构定位（重构后）
///
/// Agent 插件与 local / web / mcp / skill 等插件**完全同构**：
/// 唯一参与会话的方式是 `traverse(TRAVERSE_AVAILABLE_TOOLS)` 向会话贡献工具。
/// 是否贡献取决于 `ctx[AGENT_ID]`——未选择智能体的会话里，本插件不挂载任何工具，
/// 会话照常以"纯工具模式"运行。
///
/// 由此带来两个 API 上的简化：
/// - **不再有 capability 列表缓存**：能力说明里现在嵌入了动态人格文本
///   （身份 / 规则 / 策略 / 预算），缓存会随智能体学习而腐化；且注册动作本来就
///   每次请求都必须重做，缓存只剩"跳过一次 list_capability"这点收益。
/// - **不再持有 chat 编排职责**：会话编排归 session 插件
///   （见 `plugins/session/orchestrator.rs`）。
///
/// 字段说明：
/// - `manager`：直接持有 `Arc<AgentManager>` 具体类型，避免空 trait 抽象和 downcast
/// - `parent`：父插件弱引用，用于 `parent.route()` 反向调用
/// - `config`：可热更新的运行时配置
pub struct AgentPlugin {
    pub(crate) manager: Arc<AgentManager>,
    pub(crate) parent: Arc<RwLock<Option<std::sync::Weak<dyn Plugin>>>>,
    pub(crate) config: Arc<RwLock<AgentConfig>>,
}

impl AgentPlugin {
    /// 静态工厂：从 InvokeRequest 构造 Plugin 实例
    pub fn build(ctx: Arc<dyn InvokeRequest>) -> Arc<dyn Plugin> {
        let config: AgentConfig = ctx
            .config()
            .as_ref()
            .and_then(|c| c.get("agent"))
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let parent = ctx.parent();

        Arc::new(Self::new(parent, config)) as Arc<dyn Plugin>
    }

    pub fn new(parent: Option<std::sync::Weak<dyn Plugin>>, config: AgentConfig) -> Self {
        Self {
            manager: Arc::new(AgentManager::new()),
            parent: Arc::new(RwLock::new(parent)),
            config: Arc::new(RwLock::new(config)),
        }
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new("agent", "智能体与心智流形")
            .with_description(
                "管理智能体人格与代理预设，并承载 Mindscape 心智流形数据引擎；\
                 会话选定智能体时，通过 traverse 向会话附加智能体工具与人格",
            )
            .with_version("0.6.0")
    }

    pub(crate) async fn get_parent(&self) -> Option<Arc<dyn Plugin>> {
        let guard = self.parent.read().await;
        guard.as_ref().and_then(|w| w.upgrade())
    }

    pub(crate) async fn get_mindscape(
        &self,
        workdir: Option<&str>,
        agent_id: &str,
    ) -> Option<Arc<dyn AgentStore>> {
        {
            let agent_config = self.config.read().await;
            // 不再静默吞掉初始化错误
            // 之前 `let _ = AgentRegistry::ensure_initialized(...)` 会吞掉所有错误，
            // 导致初始化失败时无日志、后续 get_agent_engine 会得到 confusing 错误
            if let Err(e) =
                AgentRegistry::ensure_initialized(&self.manager, workdir, &agent_config).await
            {
                crate::plugin_warn!(
                    "agent",
                    "[AgentPlugin] Failed to initialize agent registry: {}",
                    e
                );
                // 初始化失败，直接返回 None，避免后续 get_agent_engine 报 confusing 错误
                return None;
            }
        }

        let agent_config = self.config.read().await;
        self.manager
            .get_agent_engine(workdir, agent_id, &agent_config)
            .await
    }

    /// 渲染指定智能体的人格文本（身份 / 规则 / 策略 / 预算状态）
    ///
    /// ## 用途
    ///
    /// 重构后人格不再写入 `system_prompt`，而是嵌入 `agent_identity` 能力的
    /// `description` 随工具定义送达 LLM。本方法在 `traverse`（异步）阶段调用——
    /// 能力工厂签名是同步的，访问存储的活儿必须在工厂之外先干完。
    ///
    /// 预算取自 `AgentConfig.prompt_budget_tokens` / `prompt_overhead_tokens`。
    pub(crate) async fn render_persona(
        &self,
        workdir: Option<&str>,
        agent_id: &str,
    ) -> Option<String> {
        let mindscape = self.get_mindscape(workdir, agent_id).await?;

        let cfg = self.config.read().await;
        let budget = PromptBudget::new(cfg.prompt_budget_tokens, cfg.prompt_overhead_tokens);
        drop(cfg);

        let result = system_prompt::build_persona(mindscape.as_ref(), &budget, None).await;
        Some(result.prompt)
    }

    /// 从请求上下文中解析 mindscape 引擎
    ///
    /// 消除各能力文件中重复的 boilerplate：
    /// - 提取 workdir 和 agent_id
    /// - 获取 mindscape 引擎
    pub(crate) async fn resolve_mindscape_from_ctx(
        &self,
        ctx: &dyn InvokeRequest,
    ) -> Result<(Arc<dyn AgentStore>, Option<String>), PluginError> {
        let workdir: Option<String> = ctx
            .get_raw(crate::symbio_core::WORKDIR.name())
            .and_then(|any| any.downcast::<String>().ok())
            .map(|arc| (*arc).clone());

        let agent_id: String = ctx
            .get_raw(crate::symbio_core::AGENT_ID.name())
            .and_then(|any| any.downcast::<String>().ok())
            .map(|arc| (*arc).clone())
            .ok_or_else(|| {
                PluginError::ValidationError("Missing 'agent_id' in context".to_string())
            })?;

        let mindscape = self
            .get_mindscape(workdir.as_deref(), &agent_id)
            .await
            .ok_or_else(|| PluginError::NotFound(format!("Agent '{agent_id}' not found")))?;

        Ok((mindscape, workdir))
    }
}

/// Agent 插件装载的能力清单（按 `AGENT_CAPABILITY_IDS` 使用）
///
/// - `agent_identity`：人格载体（身份 / 规则 / 策略 / 预算），**说明即提示词**
/// - `agent_cognition`：认知读写（记忆存取 / 推理 / 反思 / 整理）
/// - `agent_chat`：子智能体委托
/// - `agent_create`：创建新智能体
pub const AGENT_CAPABILITY_IDS: &[&str] = &[
    CAPABILITY_AGENT_IDENTITY,
    CAPABILITY_AGENT_CHAT,
    CAPABILITY_AGENT_COGNITION,
    CAPABILITY_AGENT_CREATE,
];

/// Agent 插件的能力清单——通过 `symbio_core::AGENT_CAPABILITY_IDS` 统一声明
///
/// 该常量是 Agent 插件**对外**装载的全部 capability id 的**单一真相源**：
/// - 注册侧（`submit_object_creator!` 第一参）必须使用 `CAPABILITY_AGENT_*` 常量
/// - 装载侧（本 `traverse`）通过 `AGENT_CAPABILITY_IDS` 列表精确构造
/// - 增减能力只改 `symbio_core::registry_ids` 一处
///
/// 作用域隔离：不通过 `inventory` 全局遍历 `dyn Capability`，避免误装载
/// 其它插件注册的同名/同 trait 类型。
async fn register_all_capabilities(
    ctx: Arc<dyn InvokeRequest>,
    tool_manager: &Arc<dyn crate::symbio_core::CapabilityManager>,
) {
    for &id in AGENT_CAPABILITY_IDS {
        // 通过公开 API 调用注册时包装的构造函数（内部已做 TypeId 校验）。
        match crate::symbio_core::create_object::<dyn Capability>(id, ctx.clone()) {
            Some(cap) => tool_manager.register(cap).await,
            None => crate::plugin_warn!(
                "agent",
                "[AgentPlugin] capability `{}` not registered via submit_object_creator! (create_object returned None)",
                id
            ),
        }
    }
}

#[async_trait]
impl Plugin for AgentPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    /// 统一的能力贡献入口——与其它插件（local / web / mcp / skill）**同一机制**
    ///
    /// ## 触发条件
    ///
    /// **仅当 `ctx[AGENT_ID]` 非空时**贡献工具。会话未选择智能体时本插件完全静默，
    /// 会话以"纯工具模式"照常运行（local / web / mcp / skill 的工具不受影响）。
    ///
    /// ## 失败语义
    ///
    /// `Composite::traverse` 会吞掉子插件的 Err，因此这里不能靠返回值让会话中止。
    /// 智能体解析失败（绑定了不存在的 id）属于**硬错误**，通过
    /// `chat_pipeline::report_error` 写入 ctx，由 session 编排方统一裁决并报错——
    /// 绝不静默降级成"没有人格的通用助手"。
    async fn traverse(
        self: Arc<Self>,
        _path: String,
        ctx: Arc<dyn InvokeRequest>,
    ) -> InvokeResponse<PluginPayload> {
        let sub_path = ctx.get(crate::symbio_core::PATH).unwrap_or_default();
        if sub_path != crate::symbio_core::TRAVERSE_AVAILABLE_TOOLS {
            return Err(crate::symbio_core::PluginError::NotFound(
                "Invalid traverse path".to_string(),
            ));
        }

        // ── 未选择智能体 → 静默退出（会话仍可正常使用其它插件的工具）──
        let agent_id = ctx
            .get(crate::symbio_core::AGENT_ID)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let Some(agent_id) = agent_id else {
            return Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()));
        };

        let workdir_opt = ctx.get(crate::symbio_core::WORKDIR);

        // 智能体必须真实存在：不存在就报告硬错误，让会话中止并明确提示，
        // 而不是"带一个空人格继续跑"（那会让模型凭空臆造身份）。
        let Some(persona) = self.render_persona(workdir_opt.as_deref(), &agent_id).await else {
            crate::symbio_core::report_error(
                &ctx,
                "agent",
                format!(
                    "智能体 '{}' 不存在（workdir={:?}），无法开始对话。请重新选择智能体。",
                    agent_id, workdir_opt
                ),
            )
            .await;
            return Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()));
        };

        let Some(tool_manager) = ctx.get(crate::symbio_core::CAPABILITY_MANAGER) else {
            return Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()));
        };

        let agents = self.manager.list_agents(workdir_opt.as_deref()).await;

        // 把构造 capability 所需的运行期依赖注入到 InvokeRequest，
        // 各能力工厂统一通过 `AGENT_CAPABILITY_CONTEXT` 键读取。
        // `persona` 在此一并注入（异步阶段预渲染，能力工厂是同步的）。
        ctx.set_raw(
            crate::plugins::agent::capabilities::AGENT_CAPABILITY_CONTEXT.name(),
            Arc::new(
                crate::plugins::agent::capabilities::AgentCapabilityContext {
                    plugin: self.clone(),
                    agents,
                    agent_id: Some(agent_id),
                    persona: Some(persona),
                },
            ),
        );

        // 通过系统级 `submit_object_creator!` 机制发现并装配全部已注册 capability
        register_all_capabilities(ctx.clone(), &tool_manager).await;

        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }

    async fn route(self: Arc<Self>, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload> {
        let path = ctx.get(crate::symbio_core::PATH).unwrap_or_default();
        let path = path.strip_prefix('/').unwrap_or(&path);
        let workdir = ctx.get(crate::symbio_core::WORKDIR);
        let workdir_opt = workdir.as_deref();

        // 路由分发：统一入口（避免 plugin 直接依赖各 handler 实现模块）
        // 用 Arc<Self> 而非 &self，让 handler 内部能继续 new 需要 Arc<AgentPlugin> 的工具
        super::handlers::route(self, path, ctx, workdir_opt).await
    }
}

crate::submit_object_creator!(PLUGIN_AGENT, AgentPlugin::build, dyn Plugin);

#[cfg(test)]
mod tests;
