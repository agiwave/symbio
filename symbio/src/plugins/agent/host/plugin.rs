//! agent 插件 —— Agent 目录规范 v2 的宿主接入层。
//!
//! ## 定位（规范 §3.2 宿主）
//!
//! 本插件**不再装配能力**——它只做两件事：
//!
//! - **托管**：为 `{homedir}/agent/<id>/` 下每个 Agent 构造一棵 composite 插件树
//!   （与系统 Agent 同构，§1.1），收集期把它的注册经 [`super::scope::SubAgentVisitor`]
//!   代理并进系统树（并集，§8.2）；
//! - **门槛**：manifest 不合 §5 / §10 时**拒绝接入**并明确报错——绝不静默降级成
//!   "没有人格的通用助手"。
//!
//! 能力（技能 / MCP / 记忆）由 Agent 目录里的插件实例自己解释，**复用宿主已有
//! 的对应系统**（§3.2 第 2 条）——这正是 v1 的失败之处：那时宿主为 bundle 再写
//! 一遍技能与 MCP 的解析，两条链长期不同步。
//!
//! 会话未选择智能体（`ctx[AGENT_ID]` 为空）时不装配任何 Agent，但 **agent_run
//! （子智能体委托）始终注册**。

use crate::plugins::agent::host::config::AgentConfig;
use crate::plugins::agent::host::manifest;
use crate::plugins::agent::host::memory;
use crate::plugins::agent::host::store::BundleStore;
use crate::symbio_core::schemas::detail::{DetailDefinition, DetailField};
use crate::symbio_core::vdfs_provider::VdfsProvider;
use crate::symbio_core::{
    announce_configurable, create_object, dir_from_ctx, report_error, Capability,
    CapabilityVisitor, ConfigFile, InvokeRequest, InvokeRequestExt, InvokeResponse, Plugin,
    PluginDir, PluginError, PluginMeta, PluginPayload, SimpleRequest, AGENT_ID, CAPABILITY_VISITOR,
    PATH, PLUGIN_AGENT, PLUGIN_COMPOSITE, PLUGIN_DIR, REQUIRED_PLUGINS,
    TRAVERSE_AVAILABLE_OPTIONS, TRAVERSE_AVAILABLE_TOOLS, WORKDIR,
};
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;
use tokio::sync::RwLock;

/// 配置表单的定义 —— 默认值从 [`AgentConfig::default`] 读出，不写第二份字面量
fn config_definition() -> DetailDefinition {
    let d = AgentConfig::default();
    DetailDefinition::form(
        "智能体设置",
        vec![
            DetailField::number(
                "item_max_bytes",
                "条目写入上限（字节）",
                "bundle 内单个条目文件（提示词 / 技能 / MCP）的写入字节上限，超出会被拒绝",
                1.0,
                1_048_576.0,
                json!(d.item_max_bytes),
            ),
            DetailField::number(
                "memory_max_bytes",
                "记忆写入上限（字节）",
                "智能体记忆（bundle 目录下的 AGENTS.md）的写入字节上限，超出会被拒绝",
                1.0,
                1_048_576.0,
                json!(d.memory_max_bytes),
            ),
            DetailField::number(
                "memory_inject_max_bytes",
                "记忆注入上限（字节）",
                "每轮注入系统提示词的智能体记忆字节上限，超出部分截断",
                1.0,
                1_048_576.0,
                json!(d.memory_inject_max_bytes),
            ),
        ],
    )
}

/// v2 规范标识（见 `docs/design/agent-directory-spec.md`）
///
/// 子 Agent 目录只有在 `manifest.yaml` 声明了这个 `spec` 时才按 v2 装配（挂
/// composite 插件树）。声明 `oab/v1` 的旧目录走 legacy 装配路径——两者按 manifest
/// 分流，迁移只需改写 manifest 与目录（规范 §12）。
pub(crate) const SPEC_V2: &str = "agent-dir/v2";

/// v1 的规范标识（OAB 约定目录装配形态，见 [`super::migrate`]）
pub(crate) const SPEC_V1: &str = "oab/v1";

/// 子 Agent 的必需插件清单（规范 §7.2 推荐的最小集合）
///
/// 只有与「认知能力」相关的插件才属于子 Agent；会话编排、模型网关、宿主基础设施
/// 属于系统 Agent，不在这里。
const SUB_AGENT_PLUGINS: &[&str] = &["mcp", "skill", "work"];

/// 读 Agent 目录下 `manifest.yaml` 的 `spec` 字段（读不到 / 解析不了 = `None`）
fn manifest_spec(dir: &std::path::Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join("manifest.yaml")).ok()?;
    let value: serde_yaml_ng::Value = serde_yaml_ng::from_str(&text).ok()?;
    value
        .get("spec")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// AgentBundle 插件主结构。
pub struct AgentPlugin {
    /// 已构造的子 Agent 插件树（惰性构造 + 缓存，见 [`Self::sub_agent`]）
    sub_agents: tokio::sync::RwLock<std::collections::HashMap<String, Arc<dyn Plugin>>>,
    /// 插件间路由入口（composite 容器的弱引用）。
    ///
    /// 在 `build(ctx)` 时捕获：composite 构造子插件时注入的 ctx 已携带
    /// `PARENT` 弱引用（见 composite 的子上下文构造）。agent_run 工具执行期
    /// 凭它把 `session/chat/send` 等请求路由给兄弟插件——工具 ctx 经 chat
    /// loop 一路 fork，自身并不携带路由入口。
    router: Option<std::sync::Weak<dyn Plugin>>,
    /// 生效配置（两道容量闸门的取值点）
    config: Arc<RwLock<AgentConfig>>,
    /// 配置文件的呈现与校验（`.vdfs/agent/PLUGIN.yml`）
    config_file: ConfigFile,
}

impl AgentPlugin {
    /// 静态工厂：从 InvokeRequest 构造 Plugin 实例（composite 配置驱动）。
    pub fn build(ctx: Arc<dyn InvokeRequest>) -> Arc<dyn Plugin> {
        let router = ctx.parent();
        let dir = dir_from_ctx(&*ctx, PLUGIN_AGENT);
        let config: AgentConfig = match dir.load::<AgentConfig>() {
            Ok(Some(c)) => c,
            Ok(None) => AgentConfig::default(),
            Err(e) => {
                crate::plugin_warn!("agent", "读取自身配置失败，改用默认值：{e}");
                AgentConfig::default()
            }
        };
        Arc::new(Self {
            sub_agents: tokio::sync::RwLock::new(std::collections::HashMap::new()),
            router,
            config: Arc::new(RwLock::new(config)),
            config_file: ConfigFile::new(dir, "智能体设置", config_definition()),
        }) as Arc<dyn Plugin>
    }

    /// 无装配上下文的实例（测试 / 默认构造）：配置落常规位置，读写仍自洽。
    pub fn new() -> Self {
        Self {
            sub_agents: tokio::sync::RwLock::new(std::collections::HashMap::new()),
            router: None,
            config: Arc::new(RwLock::new(AgentConfig::default())),
            config_file: ConfigFile::new(
                PluginDir::of(PLUGIN_AGENT),
                "智能体设置",
                config_definition(),
            ),
        }
    }

    /// 生效的条目写入上限（**写入闸门的唯一取值点**）
    pub(crate) async fn item_max_bytes(&self) -> usize {
        self.config.read().await.effective_item_max_bytes()
    }

    /// 依 bundle 记录构造记忆门面（**记忆的唯一构造点**：作用域 + 两道闸门在此收口）。
    ///
    /// 内核只认「上限是多少」，不关心它从哪个配置来；bundle 不存在 → 无作用域，
    /// 之后读 / 注入 / 写三条路各自降级，调用点不需要重复判断。
    pub(crate) async fn memory_store(
        &self,
        bundles: &BundleStore,
        bundle_id: &str,
    ) -> crate::symbio_core::MemoryFile {
        let cfg = self.config.read().await;
        memory::store(
            bundles,
            bundle_id,
            cfg.effective_memory_max_bytes(),
            cfg.effective_memory_inject_bytes(),
        )
    }

    /// 配置文档（VDFS 侧读写的入口）
    pub(crate) fn config_file(&self) -> &ConfigFile {
        &self.config_file
    }

    /// 配置槽位（[`ConfigFile::read`] / [`ConfigFile::apply`] 的读写对象）
    pub(crate) fn config_slot(&self) -> &RwLock<AgentConfig> {
        &self.config
    }

    /// 取得（必要时构造）子 Agent 的插件树
    ///
    /// **惰性**：只在会话真的绑定它时才构造。全量预建会连带启动每个子 Agent 的
    /// MCP server，子 Agent 一多就撑不住（规范 §9.1）。
    ///
    /// 返回 `None` = 该 id 不是 v2 子 Agent（目录不存在 / manifest 不是
    /// `agent-dir/v2`）——调用方据此回退到 legacy 约定目录装配。
    async fn sub_agent(&self, id: &str, ctx: &Arc<dyn InvokeRequest>) -> Option<Arc<dyn Plugin>> {
        if let Some(tree) = self.sub_agents.read().await.get(id) {
            return Some(Arc::clone(tree));
        }

        // 目录经 store 解析：**两级都找**（工作区级覆盖全局级）。只在全局根拼
        // 路径会让工作区里安装的 Agent 找不到——v1 的 `attach_bundle` 走的就是
        // store，v2 不能比它少看一层。
        let store = BundleStore::new(self.config_file.dir().dir(), ctx.get(WORKDIR).as_deref());
        let record = store.get(id)?;
        let dir = record.dir.clone();

        // v1 目录 → 就地迁移成 v2 后再装配。迁移是**幂等**的，且失败不阻断：
        // 返回 `None` 让调用方拒接（§10），宁可显式报错，也不要在半迁移状态下继续。
        if manifest_spec(&dir).as_deref() == Some(SPEC_V1) {
            match super::migrate::migrate_v1_to_v2(&dir) {
                Ok(true) => {
                    crate::plugin_info!("agent", "已把 `{}` 从 oab/v1 迁移为 agent-dir/v2", id)
                }
                Ok(false) => {}
                Err(e) => {
                    crate::plugin_warn!("agent", "`{}` 的 v1→v2 迁移失败：{e}", id)
                }
            }
        }

        if manifest_spec(&dir).as_deref() != Some(SPEC_V2) {
            return None;
        }

        // 与 `home` 造 `worker` 同形：把目录（子 Agent 的根）与必需插件清单告知
        // 容器，其余交给 composite 扫描装配——子 Agent 与系统 Agent 因此结构相同。
        let sub_context = Arc::new(SimpleRequest::new(self.router.clone(), None));
        if let Some(std_ctx) = ctx.as_any().downcast_ref::<SimpleRequest>() {
            let mut envs = sub_context.envs.write().unwrap();
            *envs = std_ctx.envs.read().unwrap().clone();
        }
        sub_context.set(PLUGIN_DIR, PluginDir::at(&dir, PLUGIN_COMPOSITE));
        sub_context.set(
            REQUIRED_PLUGINS,
            SUB_AGENT_PLUGINS.iter().map(|s| (*s).to_string()).collect(),
        );

        crate::plugin_info!("agent", "装配子 Agent `{}` -> {}", id, dir.display());
        let tree = create_object::<dyn Plugin>(PLUGIN_COMPOSITE, sub_context)?;
        self.sub_agents
            .write()
            .await
            .insert(id.to_string(), Arc::clone(&tree));
        Some(tree)
    }

    /// 把能力收集转发进子 Agent 的插件树
    ///
    /// 注册经 [`super::scope::SubAgentVisitor`] 代理（加来源前缀），因此与系统树的
    /// 同名注册**不冲突**，两份都生效——并集，且结果与遍历顺序无关。
    async fn forward_to_sub_agent(
        &self,
        tree: &Arc<dyn Plugin>,
        id: &str,
        dir: &std::path::Path,
        ctx: &Arc<dyn InvokeRequest>,
        visitor: &Arc<dyn CapabilityVisitor>,
    ) {
        let sub = ctx.fork();
        sub.set(PATH, TRAVERSE_AVAILABLE_TOOLS.to_string());
        // 子 Agent 的作用域 = 它自己的目录：其中的 `work` 实例因此拥有
        // `<agent dir>/AGENTS.md`，而不是沿用父的 workdir（规范 §6.2：一个作用域
        // 只有一个所有者，否则同一份记忆会被注入两次）。
        sub.set(WORKDIR, dir.to_string_lossy().to_string());
        let scoped: Arc<dyn CapabilityVisitor> =
            Arc::new(super::scope::SubAgentVisitor::new(Arc::clone(visitor), id));
        sub.set(CAPABILITY_VISITOR, scoped);

        if let Err(e) = tree.clone().traverse(String::new(), sub).await {
            crate::plugin_warn!("agent", "子 Agent `{id}` 能力收集失败：{e:?}");
        }
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new(PLUGIN_AGENT, "智能体（Agent 目录规范 v2）")
            .with_description(
                "Agent 目录宿主：管理 Agent 实例（安装/导出/删除），会话绑定 Agent 时\
                 把它整棵插件树的能力并进会话（技能 / MCP / 记忆由目录里的插件实例自己解释）",
            )
            .with_version("0.1.0")
    }

    /// 参与 `available_options` 收集：贡献「智能体」选择项。
    ///
    /// 形态：`sub` 节点，子项 = 「不使用 Agent」 + 各可用 bundle；每个子项
    /// 是「会话状态落库」invoke（`metadata.agent_id`），选中即持久化。
    /// 当前选中值由宿主注入的 `ctx[AGENT_ID]` 回填——本插件无需加载会话。
    async fn contribute_options(&self, ctx: &Arc<dyn InvokeRequest>) {
        let Some(visitor) = ctx.get(crate::symbio_core::OPTION_VISITOR) else {
            return;
        };

        let workdir = ctx.get(WORKDIR);
        let current = ctx
            .get(AGENT_ID)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        // 展示顺序号段约定：20 = 智能体（见 session::options 模块文档）
        const ORDER: i32 = 20;

        let store = BundleStore::new(self.config_file.dir().dir(), workdir.as_deref());
        let bundles = store.list();

        let mut children: Vec<crate::symbio_core::schemas::options::OptionNode> =
            Vec::with_capacity(bundles.len() + 1);
        children.push(
            crate::symbio_core::schemas::options::OptionNode::session_state(
                "agent:none",
                "不使用 Agent",
                // 空串 = 显式解绑（后端 orchestrator 对空值按「未选择」处理，
                // 与 metadata 缺省同语义），亦使子项 value 与父节点 value 可直接比较
                "agent_id",
                serde_json::json!(""),
            )
            .with_description("纯工具模式：直接与 Model 对话，可用文件/搜索等基础工具"),
        );

        let mut current_label: Option<String> = if current.is_none() {
            Some("不使用 Agent".to_string())
        } else {
            None
        };
        for record in &bundles {
            let m = &record.manifest;
            if current.as_deref() == Some(m.id.as_str()) {
                current_label = Some(m.name.clone());
            }
            children.push(
                crate::symbio_core::schemas::options::OptionNode::session_state(
                    format!("agent:{}", m.id),
                    m.name.clone(),
                    "agent_id",
                    serde_json::json!(m.id),
                )
                .with_description(m.description.clone()),
            );
        }

        let node =
            crate::symbio_core::schemas::options::OptionNode::sub("agent", "智能体", children)
                .with_icon("agent")
                .with_order(ORDER)
                .with_description("选择认知人格（可不选）");

        // 回填当前选中值（值 = agent_id；展示文本 = bundle 名 / 不使用 Agent）
        let node = match current_label {
            Some(label) => {
                let value = current.clone().unwrap_or_default();
                node.with_value_label(value, label)
            }
            // 选中的 bundle 已不存在（陈旧 id）：仅展示值本身，前端仍可重选
            None => node.with_value(current.clone().unwrap_or_default()),
        };

        visitor.register_option(node).await;
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
        match sub_path.as_str() {
            // 选项收集（与能力收集同一广播机制的第二通道）：贡献「智能体」选择项
            TRAVERSE_AVAILABLE_OPTIONS => {
                self.contribute_options(&ctx).await;
                return Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()));
            }
            TRAVERSE_AVAILABLE_TOOLS => {}
            other => {
                return Err(PluginError::NotFound(format!("未知遍历路径: {other}")));
            }
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

        let Some(tool_visitor) = ctx.get(crate::symbio_core::CAPABILITY_VISITOR) else {
            return Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()));
        };

        // ── VDFS 挂载点（`.vdfs/agent`）──
        // 本插件自身就是 provider：bundle 由 BundleStore 自管目录（工作区级 +
        // 全局级双层），列 / 读 / 写（整包导入）/ 删 / 导出 直接由
        // `impl VdfsProvider for AgentPlugin` 承载（见 `super::vdfs`）。
        {
            let vdfs_provider: Arc<dyn VdfsProvider> = self.clone();
            tool_visitor
                .register_vdfs_provider(PLUGIN_AGENT, vdfs_provider)
                .await;
        }

        // ── agent_run：无条件注册 ──
        let store = BundleStore::new(self.config_file.dir().dir(), workdir.as_deref());
        tool_visitor
            .register_batch(vec![super::subagent::AgentRunCapability::new(
                workdir.clone(),
                self.config_file.dir().dir().to_path_buf(),
                super::subagent::format_bundles_brief(&store.list()),
                self.router.clone(),
            ) as Arc<dyn Capability>])
            .await;

        // ── 已选择智能体 → 装配它的能力 ──
        if let Some(bundle_id) = bundle_id {
            match self.sub_agent(&bundle_id, &ctx).await {
                // v2：子 Agent 是一棵 composite 插件树，能力经代理层并集进来
                Some(tree) => {
                    let dir = BundleStore::new(
                        self.config_file.dir().dir(),
                        ctx.get(WORKDIR).as_deref(),
                    )
                    .get(&bundle_id)
                    .map(|r| r.dir)
                    .unwrap_or_else(|| self.config_file.dir().dir().join(&bundle_id));
                    self.forward_to_sub_agent(&tree, &bundle_id, &dir, &ctx, &tool_visitor)
                        .await
                }
                // §10：不匹配必须拒绝接入，且不得静默降级为「无人格的通用助手」。
                // 迁移（v1 → v2）已在 [`Self::sub_agent`] 里试过；走到这里说明目录
                // 根本没有合规 manifest，错误信息写明双侧版本。
                None => {
                    let dir = self.config_file.dir().dir().join(&bundle_id);
                    let reason = match manifest::load(&dir) {
                        Some(m) => manifest::validate(&m).unwrap_err(),
                        None => format!(
                            "`{}` 下没有可解析的 manifest.yaml（已尝试 {}）；\
                             本宿主只支持 `agent-dir/v{}`",
                            dir.display(),
                            manifest::MANIFEST_NAMES.join(" / "),
                            manifest::SPEC_MAJOR
                        ),
                    };
                    report_error(
                        &ctx,
                        PLUGIN_AGENT,
                        format!("智能体 `{bundle_id}` 拒绝接入：{reason}"),
                    )
                    .await;
                }
            }
        }

        // 顺带声明「本插件有一份配置文档」（设置页据此列出并指路）
        announce_configurable(&ctx, &self.config_file).await;

        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }

    /// 路由入口：**无自有协议**。
    ///
    /// bundle 的浏览 / 导入 / 删除 / 导出全部由 VDFS 承接（`.vdfs/agent/…`）：
    /// `vdfs/list` / `vdfs/write`（二进制 = 导入）/ `vdfs/delete` / 节点动作
    /// `export`。因此这里不再有任何路由——插件只对宿主暴露装配能力。
    async fn route(self: Arc<Self>, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload> {
        let path = ctx.get(PATH).unwrap_or_default();
        Err(PluginError::NotFound(format!(
            "agent 无自有协议路由 `{path}`：bundle 一律经 VDFS 访问（.vdfs/agent/…）"
        )))
    }
}

crate::submit_object_creator!(PLUGIN_AGENT, AgentPlugin::build, dyn Plugin);
