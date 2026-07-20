use crate::plugins::agent::core::AgentConfig;
use crate::plugins::agent::core::AgentStore;

use crate::plugins::agent::manager::{resolve_workspace_dir, AgentManager, AgentRegistry};

use crate::symbio_core::{
    Capability, CapabilityManager, CapabilityMeta, InvokeRequest, InvokeRequestExt, InvokeResponse,
    Plugin, PluginError, PluginMeta, PluginPayload, SimpleRequest, SymbioKey,
    CAPABILITY_AGENT_CHAT, CAPABILITY_AGENT_COGNITION, CAPABILITY_AGENT_CREATE, PLUGIN_AGENT,
};
use async_trait::async_trait;

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Agent 插件主结构
///
/// 字段说明：
/// - `manager`：直接持有 `Arc<AgentManager>` 具体类型，避免空 trait 抽象和 downcast
/// - `parent`：父插件弱引用，用于 `parent.route()` 反向调用
/// - `config`：可热更新的运行时配置
/// - `capability_cache`：，按 (workdir, agent_id) 缓存 capability 列表，
///   避免每次 chat 请求都重做 `parent.traverse` + 全量 cap 构造
pub struct AgentPlugin {
    pub(crate) manager: Arc<AgentManager>,
    pub(crate) parent: Arc<RwLock<Option<std::sync::Weak<dyn Plugin>>>>,
    pub(crate) config: Arc<RwLock<AgentConfig>>,
    /// Capability 列表缓存
    ///
    /// key 格式：`"{workdir}::{agent_id}"`，None workdir 编码为空字符串
    /// value：当前 agent 已注册的 capability 列表
    /// **失效策略**：写入/删除 agent 时手动 `invalidate_capability_cache`；
    /// 配置热更新时全量清空（见 `config` 写锁 guard 释放时）。
    /// 不做 TTL：capability 注册是写少读多的场景，TTL 反而引入复杂度。
    ///
    /// 缓存只缓存"该 agent 应有哪些 capability"这一**元信息**，
    /// 每次 chat 请求都会拿到**全新的** `DefaultToolManager`，所以 `parent.traverse`
    /// 把工具注册到新 manager 的工作必须**每次都做**，不能因为缓存命中而跳过。
    /// 当前缓存仅在命中时跳过廉价的 `list_capability` 调用。
    pub(crate) capability_cache: Arc<RwLock<HashMap<String, Vec<CapabilityMeta>>>>,
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
            capability_cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new("agent", "智能体与心智流形")
            .with_description("管理智能体人格与代理预设，并承载 Mindscape 心智流形数据引擎")
            .with_version("0.5.0")
    }

    /// 缓存 key 标准化：`{workdir}::{agent_id}`，workdir=None 编码为空字符串
    pub fn cache_key(workdir: Option<&str>, agent_id: &str) -> String {
        format!("{}::{}", workdir.unwrap_or(""), agent_id)
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

    pub(crate) async fn fetch_tools_with_manager(
        &self,
        workdir: Option<String>,
        agent_id: &str,
        tool_manager: Arc<dyn CapabilityManager>,
    ) -> Vec<CapabilityMeta> {
        // 缓存命中时也必须调用 `parent.traverse` 把工具注册到
        // 调用方传入的 `tool_manager` 实例中。
        //
        // 历史 bug：原实现缓存命中后 `return cached.clone();` 直接返回，
        // 跳过了 `parent.traverse(...)`，导致新传入的 `DefaultToolManager` 是空的；
        // 但 `UnifiedCapabilityManager` 把"cached_capabilities"（含 agent_memory）作为
        // 精确匹配索引 → 命中后调用 `inner_manager.invoke("agent_memory", ...)` →
        // 空 HashMap 查表失败 → "Tool not found: agent_memory"。
        //
        // 正确语义：缓存只缓存"该 agent 应有哪些 capability"这一**元信息**，
        // 但每次请求都得到一个全新的 `tool_manager`，所以注册工作必须每次都做。
        // 缓存的价值仅在于命中时跳过 `list_capability`（廉价的小优化）。
        let key = Self::cache_key(workdir.as_deref(), agent_id);

        // 1. 缓存命中检查（仅跳过 list_capability，不跳过注册）
        {
            let cache = self.capability_cache.read().await;
            if let Some(cached) = cache.get(&key) {
                // 即便命中，也必须把工具实际注册到本次的 tool_manager
                Self::register_capabilities_into(
                    self.get_parent().await.as_ref(),
                    workdir.as_deref(),
                    agent_id,
                    &tool_manager,
                )
                .await;
                return cached.clone();
            }
        }

        // 2. 缓存未命中：traverse + list
        Self::register_capabilities_into(
            self.get_parent().await.as_ref(),
            workdir.as_deref(),
            agent_id,
            &tool_manager,
        )
        .await;

        let caps = tool_manager.list_capability().await;

        // 3. 写缓存
        {
            let mut cache = self.capability_cache.write().await;
            cache.insert(key, caps.clone());
        }

        caps
    }

    /// 把当前 agent 的所有 capability 工厂注册到给定的 `tool_manager`。
    ///
    /// 抽出来是为了 `fetch_tools_with_manager` 在缓存命中/未命中两条路径上
    /// 都能复用，避免遗漏注册导致 `Tool not found: <name>`。
    async fn register_capabilities_into(
        parent: Option<&Arc<dyn Plugin>>,
        workdir: Option<&str>,
        agent_id: &str,
        tool_manager: &Arc<dyn CapabilityManager>,
    ) {
        let Some(parent) = parent else {
            return;
        };
        let ctx = Arc::new(SimpleRequest::new(None, None));
        ctx.set(
            crate::symbio_core::PATH,
            crate::symbio_core::TRAVERSE_AVAILABLE_TOOLS.to_string(),
        );
        if let Some(wd) = workdir {
            ctx.set(crate::symbio_core::WORKDIR, wd.to_string());
        }
        ctx.set(crate::symbio_core::AGENT_ID, agent_id.to_string());
        ctx.set(crate::symbio_core::CAPABILITY_MANAGER, tool_manager.clone());

        // traverse 失败不再静默吞错
        if let Err(e) = parent.clone().traverse("".to_string(), ctx).await {
            crate::plugin_warn!(
                "agent",
                "register_capabilities_into: traverse failed for agent_id={} err={:?}",
                agent_id,
                e
            );
        }
    }

    /// 加载 `<homedir>/AGENTS.md` 全局指令
    ///
    /// homedir 来自 [`crate::symbio_core::HomedirRegistry`]
    pub(crate) async fn load_system_agents_md(&self) -> Option<String> {
        let p = crate::symbio_core::HomedirRegistry::get().join("AGENTS.md");
        Self::read_to_string_safe(&p).await
    }

    /// 加载 `{workdir}/AGENTS.md` 工作区指令
    ///
    /// 安全说明：
    /// - workdir 在工厂创建时已被前端规范化为绝对路径
    /// - 此处走 `path::resolve_workspace_dir` 统一校验（拒绝 `..` 逃逸）
    pub(crate) async fn load_workspace_agents_md(&self, workdir: Option<&str>) -> Option<String> {
        let p = resolve_workspace_dir(workdir)?.join("AGENTS.md");
        Self::read_to_string_safe(&p).await
    }

    async fn read_to_string_safe(path: &Path) -> Option<String> {
        if !path.exists() {
            return None;
        }
        tokio::fs::read_to_string(path).await.ok()
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
pub const AGENT_CAPABILITY_IDS: &[&str] = &[
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

        let workdir_opt = ctx.get(crate::symbio_core::WORKDIR);
        let agents = self.manager.list_agents(workdir_opt.as_deref()).await;

        if let Some(tool_manager) = ctx.get(crate::symbio_core::CAPABILITY_MANAGER) {
            // 把构造 capability 所需的运行期依赖注入到 InvokeRequest，
            // 各能力工厂统一通过 `AGENT_CAPABILITY_CONTEXT` 键读取
            ctx.set_raw(
                crate::plugins::agent::capabilities::AGENT_CAPABILITY_CONTEXT.name(),
                Arc::new(
                    crate::plugins::agent::capabilities::AgentCapabilityContext {
                        plugin: self.clone(),
                        agents,
                    },
                ),
            );

            // 通过系统级 `submit_object_creator!` 机制发现并装配全部已注册 capability
            register_all_capabilities(ctx.clone(), &tool_manager).await;
        }

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
