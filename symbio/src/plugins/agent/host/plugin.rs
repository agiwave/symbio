//! agent 插件 —— OAB 协议的宿主接入层（单协议、约定目录装配）。
//!
//! ## 定位（规范 §12 宿主参考实现）
//!
//! symbio 是 OAB 协议的第一个接入方：本插件把 bundle 的约定目录装配语义
//! 映射进 symbio 的会话机制——
//!
//! - **提示词**：`traverse(available_tools)` 时扫描 `prompts/` `skills/`，
//!   把片段汇总后交出**两份**——系统提示词片段（每轮注入的人格本体 + 可编辑
//!   地址，见 [`identity_segment`]）与 [`BundleIdentityCapability`]
//!   （`agent_identity` 工具，取回超出注入预算的全文）；
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
use crate::plugins::agent::host::config::AgentConfig;
use crate::plugins::agent::host::memory;
use crate::plugins::agent::host::prompt::{identity_segment, SEGMENT_NAME as IDENTITY_SEGMENT};
use crate::plugins::agent::host::store::{BundleRecord, BundleStore};
use crate::symbio_core::schemas::detail::{DetailDefinition, DetailField};
use crate::symbio_core::vdfs_provider::VdfsProvider;
use crate::symbio_core::{
    announce_configurable, dir_from_ctx, report_error, Capability, ConfigFile, InvokeRequest,
    InvokeRequestExt, InvokeResponse, Plugin, PluginDir, PluginError, PluginMeta, PluginPayload,
    AGENT_ID, PATH, PLUGIN_AGENT, SESSION_ID, TRAVERSE_AVAILABLE_OPTIONS, TRAVERSE_AVAILABLE_TOOLS,
    WORKDIR,
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
                "identity_inject_max_bytes",
                "人格注入上限（字节）",
                "每轮注入系统提示词的人格字节上限，超出部分截断（可用 agent_identity 取回全文）",
                1.0,
                1_048_576.0,
                json!(d.identity_inject_max_bytes),
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

/// AgentBundle 插件主结构。
pub struct AgentPlugin {
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
            router,
            config: Arc::new(RwLock::new(config)),
            config_file: ConfigFile::new(dir, "智能体设置", config_definition()),
        }) as Arc<dyn Plugin>
    }

    /// 无装配上下文的实例（测试 / 默认构造）：配置落常规位置，读写仍自洽。
    pub fn new() -> Self {
        Self {
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
        let store = BundleStore::new(self.config_file.dir().dir(), workdir.as_deref());
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

        // ── 3. 提示词片段 → agent_identity 身份工具 ──
        let mut caps: Vec<Arc<dyn Capability>> = Vec::new();
        let identity_text = assembly.identity_text();
        if !identity_text.trim().is_empty() {
            caps.push(BundleIdentityCapability::new(
                identity_text.clone(),
                record.manifest.id.clone(),
            ) as Arc<dyn Capability>);
        }

        tool_visitor.register_batch(caps).await;

        // ── 4. 人格 + 智能体记忆 → 系统提示词（每轮注入，带可编辑地址与容量口径）──
        // 与身份工具**同时机、同一次广播**注册：同一个人格的两个面——提示词负责
        // 「一开始就知道自己是谁」，工具负责「取回超出注入预算的全文」。
        let cfg = self.config.read().await.clone();
        tool_visitor
            .register_system_prompt(
                IDENTITY_SEGMENT,
                identity_segment(&record.manifest.id, &identity_text, &cfg),
            )
            .await;
        // 记忆：机制在内核（读写 / 两道闸门 / 排版），本插件只给落位与地址。
        // 读失败**不注册**（而不是降级成一段「暂无记忆」）——那会让模型以为确实没有，
        // 比不注入更坏；人格在上面已经注册，不受影响。
        let memory = self.memory_store(&store, &record.manifest.id).await;
        let address = memory::memory_address(&record.manifest.id);
        match memory.segment(&memory::segment_spec(&address)) {
            Ok(Some(segment)) => {
                tool_visitor
                    .register_system_prompt(memory::SEGMENT_NAME, segment)
                    .await;
            }
            Ok(None) => {}
            Err(e) => crate::plugin_warn!("agent", "读取智能体记忆失败，本轮不注入：{e}"),
        }

        Ok(())
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
        let session_id = ctx.get(SESSION_ID);

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

        // ── 已选择智能体 → 约定目录装配（人格片段 + 身份工具）──
        if let Some(bundle_id) = bundle_id {
            if let Err(e) = self
                .attach_bundle(&ctx, &bundle_id, workdir, session_id, &tool_visitor)
                .await
            {
                // 硬错误：会话绑定了一个不合规/不存在的 bundle——必须中止并明确提示，
                // 绝不静默降级为「无人格的通用助手」；装配失败一律以收集期错误上报。
                report_error(
                    &ctx,
                    PLUGIN_AGENT,
                    format!("bundle `{bundle_id}` 装配失败: {e}"),
                )
                .await;
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
