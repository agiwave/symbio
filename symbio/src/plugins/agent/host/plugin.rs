//! agent 插件 —— Agent 目录规范 v2 的宿主接入层。
//!
//! ## 定位（规范 §3.2 宿主）：**只管子智能体的目录与连接**
//!
//! 本插件是「**别处装进来的智能体**」域的唯一所有者，做两件事：
//!
//! - **托管**：为 `{homedir}/agent/<id>/` 下每个 Agent 构造一棵 composite 插件树
//!   （与系统 Agent 同构，§1.1），收集期把它的注册经 [`super::scope::SubAgentVisitor`]
//!   代理并进系统树（并集，§8.2）；
//! - **门槛**：manifest 不合 §5 / §10 时**拒绝接入**并明确报错——绝不静默降级成
//!   "没有人格的通用助手"。
//!
//! ## 边界：本插件**不拥有**当前智能体自身的东西
//!
//! 「当前智能体自身」的两样东西各有自己的插件，本插件一律不碰：
//!
//! | 东西 | 归谁 | 地址 |
//! |---|---|---|
//! | 智能体自身的记忆（`AGENTS.md`） | `memory` | `<根>/memory/AGENTS.md` |
//! | 智能体自身的信息设置 | `setting` | `<根>/setting/PLUGIN.yml` |
//!
//! 它们都是**分形**的：子树里各有自己的实例，落位由「本插件目录的父目录」推出。
//! 从前本插件用 `instruction` / `memory` 两个模块兼管这两个作用域（外加一份配置
//! 承载两道容量闸门），于是 `agent` 一边管「装进来的智能体」、一边管「自己是谁」——
//! 两件事各有一套落位与闸门，却挤在同一个插件里。现在这条线按**归属**划开：
//! 本插件只回答「有哪些智能体、怎么把它们连进来」。
//!
//! ⚠️ **指令文件不由子树里的插件实例解释**：子树按 agent 目录扫描，其中 `agent`
//! 实例只负责「再下一级子 Agent」（`<id>/agent/<sub-id>` 递归，即分形），并不解释
//! 本目录自身的指令——本目录的 `AGENTS.md` 归子树里的 `memory` 实例。而本插件
//! 恰恰是「认识 Agent 目录」的那个插件：它扫描目录、装配子树、把整包内容暴露成
//! VDFS，全在职责内。子树的注册经作用域 visitor 加前缀，故与系统侧不冲突（§8.2）。
//!
//! 能力（技能 / MCP / …）由 Agent 目录里的插件实例自己解释，**复用宿主已有的对应系统**
//! （§3.2 第 2 条）——宿主替 agent 目录再写一遍技能与 MCP 的解析，两条链必然长期不同步。
//! 子树因此挂**与父 Agent 同构的默认插件集**（见
//! [`crate::symbio_core::ASSEMBLY_SUB_AGENT_PLUGINS`]，与系统侧是**同一份清单**）：子树会构造
//! 自己的 `vdfs` 实例，但其注册经 `SubAgentVisitor` 在每一层丢弃（VDFS 根单槽归
//! 系统 Agent 独占），其余（含 `agent` 自身、`model`、`plugin_manager`、
//! `memory`、`setting`）全部与父树一致——UI 资源入口因此对齐。`model` 在子树里有
//! 实例：子智能体有自己的模型服务（子树会话以子容器为 parent 收集，自行解析）；
//! 父会话收集期该注册才被 `SubAgentVisitor` 丢弃（单槽防劫持）。
//!
//! 会话未选择智能体（`ctx[AGENT_ID]` 为空）时不装配任何 Agent，但 **agent_run
//! （子智能体委托）始终注册**。

use crate::plugins::agent::host::manifest;
use crate::plugins::agent::host::store::AgentDirStore;
use crate::symbio_core::schemas::detail::{DetailField, DetailOption};
use crate::symbio_core::{
    capability_report_error, creator_create_object, descend_addr, plugin_dir_from_ctx, Capability,
    CapabilityVisitor, Plugin, PluginDir, PluginError, PluginInvokeRequest, PluginInvokeRequestExt,
    PluginInvokeResponse, PluginMeta, PluginPayload, PluginSimpleRequest, AGENT_ID,
    ASSEMBLY_SUB_AGENT_PLUGINS, CAPABILITY_VISITOR, PATH, PLUGIN_DIR, PLUGIN_ID_AGENT,
    PLUGIN_ID_COMPOSITE, REQUIRED_PLUGINS, TRAVERSE_AVAILABLE_OPTIONS, TRAVERSE_AVAILABLE_TOOLS,
    VDFS_PARENT_ADDR, WORKDIR,
};
use crate::symbio_core::{VdfsAccess, VdfsProvider};
use async_trait::async_trait;
use std::sync::Arc;

/// 规范标识（见 `docs/design/agent-directory-spec.md`）
///
/// 子 Agent 目录只有在 `manifest.yaml` 声明了这个 `spec`、且通过 §10 版本门槛时
/// 才按 v2 装配（挂 composite 插件树）。**这是唯一的接入判据**——不是它的目录既
/// 列不出来也挂不上（见 [`super::store::AgentDirStore::load_record`]）。
pub(crate) const SPEC_V2: &str = "agent-dir/v2";

/// Agent 插件主结构（宿主接入层）。
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
    /// 本插件目录（agent 目录的**唯一发现根**；由父插件经 `PLUGIN_DIR` 告知）
    ///
    /// 同时是 `AgentDirStore` 的根与子树装配的起点——两个用途同一份，避免
    /// 「发现根」与「装配根」漂移。
    dir: PluginDir,
}

impl AgentPlugin {
    /// 静态工厂：从 PluginInvokeRequest 构造 Plugin 实例（composite 配置驱动）。
    pub fn build(ctx: Arc<dyn PluginInvokeRequest>) -> Arc<dyn Plugin> {
        let router = ctx.parent();
        let dir = plugin_dir_from_ctx(&*ctx, PLUGIN_ID_AGENT);
        Arc::new(Self::new_with(dir, router)) as Arc<dyn Plugin>
    }

    pub fn new_with(dir: PluginDir, router: Option<std::sync::Weak<dyn Plugin>>) -> Self {
        Self {
            sub_agents: tokio::sync::RwLock::new(std::collections::HashMap::new()),
            router,
            dir,
        }
    }

    /// 无装配上下文的实例（仅测试）：目录给临时目录——**不读全局系统根**。
    #[cfg(test)]
    pub fn new() -> Self {
        Self::new_with(
            PluginDir::at(
                std::env::temp_dir().join("symbio-test/agent"),
                PLUGIN_ID_AGENT,
            ),
            None,
        )
    }

    /// 测试用：把插件作用域到指定目录（等价于生产态由 `PLUGIN_DIR` 告知的目录）。
    ///
    /// 仅用于单测——让「导入位置」与「发现根」落在同一目录，验证导入→发现→装配链路。
    #[cfg(test)]
    pub fn new_with_dir(dir: PluginDir) -> Self {
        Self::new_with(dir, None)
    }

    /// 本插件目录 = agent 目录的唯一发现根
    pub(crate) fn dir(&self) -> &PluginDir {
        &self.dir
    }

    /// 取得（必要时构造）子 Agent 的插件树
    ///
    /// **惰性**：只在会话真的绑定它时才构造。全量预建会连带启动每个子 Agent 的
    /// MCP server，子 Agent 一多就撑不住（规范 §9.1）。
    ///
    /// 返回 `None` = 该 id 不是可接入的 Agent（目录不存在 / manifest 缺失 /
    /// 未过 §10 版本门槛）——列表与挂载共用同一判据，调用方无需区分这几种情况。
    pub(crate) async fn sub_agent(
        &self,
        id: &str,
        ctx: &Arc<dyn PluginInvokeRequest>,
    ) -> Option<Arc<dyn Plugin>> {
        if let Some(tree) = self.sub_agents.read().await.get(id) {
            return Some(Arc::clone(tree));
        }

        // agent 目录只由本插件自己的目录决定（与 workdir 无关）；嵌套子 Agent 的目录
        // 由 composite 经 `PLUGIN_DIR` 自动作用域到 `<subtree>/agent`，分形天然正确。
        let store = AgentDirStore::new(self.dir().dir());
        let record = store.get(id)?;
        let dir = record.dir.clone();

        // 与 `home` 造 `worker` 同形：把目录（子 Agent 的根）与必需插件清单告知
        // 容器，其余交给 composite 扫描装配——子 Agent 与系统 Agent 因此结构相同。
        let sub_context = Arc::new(PluginSimpleRequest::child_of(ctx, self.router.clone()));
        sub_context.set(PLUGIN_DIR, PluginDir::at(&dir, PLUGIN_ID_COMPOSITE));
        sub_context.set(
            REQUIRED_PLUGINS,
            ASSEMBLY_SUB_AGENT_PLUGINS
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
        );

        crate::plugin_info!("agent", "装配子 Agent `{}` -> {}", id, dir.display());
        let tree = creator_create_object::<dyn Plugin>(PLUGIN_ID_COMPOSITE, sub_context)?;
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
    ///
    /// ## 跨作用域必须改写上下文（与系统链路的 `VDFS_PARENT_ADDR` 同理）
    ///
    /// ⚠️ **不得改指 `WORKDIR`**：子树里的 `work` 只负责工作区信息（见
    /// [`ASSEMBLY_SUB_AGENT_PLUGINS`]），改指 Agent 目录会让它去解释 `<agentdir>/AGENTS.md`。
    /// 子树的 `WORKDIR` 与父会话一致（继承）。
    ///
    /// ⚠️ **`AGENT_ID` 必须清空**：它是**会话级「选中的智能体」**，只由**拥有该 id 的
    /// 那个 store 的实例**解析（系统根实例的 store = `{homedir}/agent`）。子树里的
    /// `agent` 实例（分形，见模块文档）用同一个键表达**它自己**那一层的选中
    /// （`<agentdir>/agent/<sub-id>`）；父 id 落进它的作用域会被当成「我这一层的选中」
    /// 去查 `<agentdir>/agent/<父id>`——必然查不到，于是按 §10 报「拒绝接入」，把整轮
    /// 能力收集打断（子树明明装好了，却被一个**本不属于该作用域**的 id 判死）。
    /// 故此处显式清空 = 「本作用域无选中」。委托子智能体不走本键，走 `agent_run`
    /// 的显式 `agent_id` 参数（它按自己的 store 解析）。
    ///
    /// ⚠️ **`VDFS_PARENT_ADDR` 必须续接 `<id>`**：子树里的插件（`memory` / `work` / …）
    /// 在收集期拼「给模型看的可编辑地址」，靠的就是上下文里的当前父地址。子树的挂载
    /// 点是 `<根>/agent/<id>`，而本插件自己的父地址只到 `<根>/agent`——不续接就会拼出
    /// `<根>/agent/memory/AGENTS.md`（一个不存在的地址），模型按它 `vdfs_read` 必然
    /// 落空。系统链路（[`super::vdfs`] 的 `sub_vfs`）本来就是这么续的，两条链路因此
    /// 说同一个地址。
    async fn forward_to_sub_agent(
        &self,
        tree: &Arc<dyn Plugin>,
        id: &str,
        ctx: &Arc<dyn PluginInvokeRequest>,
        visitor: &Arc<dyn CapabilityVisitor>,
    ) {
        let sub = ctx.fork();
        sub.set(PATH, TRAVERSE_AVAILABLE_TOOLS.to_string());
        // 清空而非继承：父作用域的选中 id 由**父**的 store 解析；带进来只会被本层的
        // `agent` 实例当成自己的子目录选择（见上方文档）。
        sub.set(AGENT_ID, String::new());
        // 续接而非继承：子树挂在 `<根>/agent/<id>` 下（见上方文档）。
        sub.set(
            VDFS_PARENT_ADDR,
            descend_addr(&ctx.get(VDFS_PARENT_ADDR).unwrap_or_default(), id),
        );
        let scoped: Arc<dyn CapabilityVisitor> =
            Arc::new(super::scope::SubAgentVisitor::new(Arc::clone(visitor), id));
        sub.set(CAPABILITY_VISITOR, scoped.clone());

        if let Err(e) = tree.clone().traverse(String::new(), sub).await {
            crate::plugin_warn!("agent", "子 Agent `{id}` 能力收集失败：{e:?}");
        }
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new(PLUGIN_ID_AGENT, "智能体")
            .with_description(
                "智能体域：管理装进来的 Agent 目录（安装/导出/删除）与它们的连接，\
                 会话绑定 Agent 时把它整棵插件树的能力并进会话（技能 / MCP 由目录里的\
                 插件实例自己解释）",
            )
            .with_version("0.1.0")
            .with_order(3)
            .with_icon(PLUGIN_ID_AGENT)
            // 根可列举 + 可递归遍历（agent 目录内部有子条目）。
            // 「根下可新建类型」不在这里——agent 目录只能整包导入（没有「先建空壳
            // 再填字段」的形态），清单由 provider 自持（见 `super::vdfs`）
            .with_root_access(VdfsAccess::LIST_TRAVERSE)
    }

    /// 参与 `available_options` 收集：贡献「智能体」字段。
    ///
    /// 候选 = 「不使用 Agent」 + 各可用 agent 目录（值 = `agent_id`，空串 = 显式解绑）。
    ///
    /// **不回填当前值**：「值 → 标签」由 `field.options` 承担，前端查表即得；当前值
    /// 来自会话 `metadata.agent_id`。因此这里既不读 `ctx[AGENT_ID]`，也不算
    /// `current_label`（见 `docs/archive/session-options-unification.md` §6）。
    async fn contribute_options(&self, ctx: &Arc<dyn PluginInvokeRequest>) {
        let Some(visitor) = ctx.get(crate::symbio_core::OPTION_VISITOR) else {
            return;
        };

        // 展示顺序号段约定：20 = 智能体（见 session::options 模块文档）
        const ORDER: i32 = 20;

        let store = AgentDirStore::new(self.dir().dir());
        let agent_dirs = store.list();

        visitor
            .register_option_field(ORDER, agent_field(&agent_dirs))
            .await;
    }
}

/// 「智能体」选项的字段声明（`node.schema` 用）。
///
/// 候选 = 「不使用 Agent」（空串 = 显式解绑，后端 orchestrator 对空值按「未选择」
/// 处理）+ 各可用 agent 目录。`default` 也取空串：会话 metadata 里没有
/// `agent_id` 时，前端按 `default` 显示「不使用 Agent」。
fn agent_field(agent_dirs: &[crate::plugins::agent::host::store::AgentDirRecord]) -> DetailField {
    let mut options = vec![DetailOption {
        value: String::new(),
        label: "不使用 Agent".to_string(),
        description: Some("纯工具模式：直接与 Model 对话，可用文件/搜索等基础工具".to_string()),
    }];
    options.extend(agent_dirs.iter().map(|record| {
        let m = &record.manifest;
        DetailOption {
            value: m.id.clone(),
            label: m.name.clone(),
            description: (!m.description.is_empty()).then(|| m.description.clone()),
        }
    }));

    DetailField {
        key: "agent_id".to_string(),
        label: "智能体".to_string(),
        description: Some("选择认知人格（可不选）".to_string()),
        widget: "select".to_string(),
        icon: Some("agent".to_string()),
        default: Some(serde_json::json!("")),
        options,
        ..Default::default()
    }
}

#[cfg(test)]
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

    fn get_vfs_provider(self: Arc<Self>) -> Option<Arc<dyn crate::symbio_core::VdfsProvider>> {
        Some(self)
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
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
        // 统一解析：请求显式 > metadata.agent_id）；本插件把它解析为 agent 目录实例 id。
        // 注意：agent_run（子智能体委托）**始终注册**，不受"是否选择智能体"影响——
        // 它是会话基础能力：未指定 agent_id 时默认沿用当前会话的智能体，
        // 两者皆空则子会话以无智能体的纯对话模式运行。
        let agent_id = ctx
            .get(AGENT_ID)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let workdir = ctx.get(WORKDIR);

        let Some(tool_visitor) = ctx.get(crate::symbio_core::CAPABILITY_VISITOR) else {
            return Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()));
        };

        // ── VDFS 挂载点（`<根>/agent`）──
        // 本插件自身就是 provider：agent 目录由 AgentDirStore 自管目录（本插件自己的
        // 目录，与 workdir 无关），列 / 读 / 写（整包导入）/ 删 / 导出 直接由
        // `impl VdfsProvider for AgentPlugin` 承载（见 `super::vdfs`）。
        // 智能体自身的记忆 / 设置不在这里——它们各有自己的插件与挂载点。
        {
            let vdfs_provider: Arc<dyn VdfsProvider> = self.clone();
            tool_visitor
                .register_vdfs_provider(PLUGIN_ID_AGENT, vdfs_provider)
                .await;
        }

        // ── agent_run：无条件注册 ──
        let store = AgentDirStore::new(self.dir().dir());
        tool_visitor
            .register_batch(vec![super::subagent::AgentRunCapability::new(
                workdir.clone(),
                self.dir().dir().to_path_buf(),
                super::subagent::format_agent_dirs_brief(&store.list()),
                self.router.clone(),
            ) as Arc<dyn Capability>])
            .await;

        // ── 已选择智能体 → 装配它的能力 ──
        if let Some(agent_id) = agent_id {
            match self.sub_agent(&agent_id, &ctx).await {
                // 子 Agent 是一棵 composite 插件树，能力经代理层并集进来
                Some(tree) => {
                    self.forward_to_sub_agent(&tree, &agent_id, &ctx, &tool_visitor)
                        .await
                }
                // §10：不匹配必须拒绝接入，且不得静默降级为「无人格的通用助手」。
                // 走到这里说明目录没有可接入的 manifest，错误信息写明双侧版本。
                None => {
                    let dir = self.dir().dir().join(&agent_id);
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
                    capability_report_error(
                        &ctx,
                        PLUGIN_ID_AGENT,
                        format!("智能体 `{agent_id}` 拒绝接入：{reason}"),
                    )
                    .await;
                }
            }
        }

        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }

    /// 路由入口：**无自有协议**。
    ///
    /// agent 目录的浏览 / 导入 / 删除 / 导出全部由 VDFS 承接（`<根>/agent/…`）：
    /// `vdfs/list` / `vdfs/write`（二进制 = 导入）/ `vdfs/delete` / 节点动作
    /// `export`。因此这里不再有任何路由——插件只对宿主暴露装配能力。
    async fn route(
        self: Arc<Self>,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let path = ctx.get(PATH).unwrap_or_default();
        // 绝对地址 = 上下文父地址 + 相对地址（协议级指路信息，封装入口统一）
        Err(PluginError::NotFound(format!(
            "agent 无自有协议路由 `{path}`：agent 目录一律经 VDFS 访问（{}）",
            crate::symbio_core::absolute_addr(&ctx, &format!("{PLUGIN_ID_AGENT}/…"))
        )))
    }
}

crate::submit_object_creator!(PLUGIN_ID_AGENT, AgentPlugin::build, dyn Plugin);
