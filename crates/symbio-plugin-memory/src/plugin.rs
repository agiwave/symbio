//! memory 插件本体 —— **智能体 / 工作区 / 会话三层文件记忆**（机制与配置面见
//! [`super`] 的模块文档 / `README.md`）。
//!
//! 本插件只有一件事：让模型**记得住**长期有效的东西，并且**改得动**它们。因此它在
//! `traverse` 里同时交出——
//!
//! 1. **系统提示词**（`CapabilityRow::prompt`，三段）：三层记忆的正文 + 文件地址 +
//!    容量与写法，由会话侧按注册顺序拼接后送达模型；
//! 2. **VDFS 挂载点**（`<根>/memory`）：三份记忆文件本体（物理均 `MEMORY.md`，
//!    挂载名 `MEMORY.md` / `WORKSPACE.md` / `SESSION.md`），模型用 `vdfs_read` / `vdfs_write` 读写，用户
//!    在界面上也能直接编辑。
//!
//! 两样必须同时存在：只注入不给地址 = 只读记忆，永远长不大；只给地址不注入 =
//! 模型不知道它存在，等于没有。
//!
//! ## 三个作用域，三条腿
//!
//! | 作用域 | 物理落位 | 作用域来源 | 挂载名 |
//! |---|---|---|---|
//! | 智能体自身 | `{agent_dir}/MEMORY.md` | **装配**（本插件目录的父目录，分形） | `MEMORY.md` |
//! | 工作区 | `{work_dir}/MEMORY.md` | **会话**（`ctx[WORKDIR]`，可选） | `WORKSPACE.md` |
//! | 会话 | `{session_dir}/MEMORY.md` | **会话**（`ctx[SESSION_DIR]`，可选） | `SESSION.md` |
//!
//! 工作区腿的闸门（没选工作区 → 无作用域；`workspace_enabled` 关 → 不注入）在
//! [`Self::workspace_store_of`] / [`Self::contribute_workspace_prompt`] 两处收口；
//! 会话腿的作用域闸门在 [`Self::session_store_of`] / [`Self::contribute_session_prompt`]
//! 两处收口。
//!
//! ## 本插件不认识「系统」与「子智能体」
//!
//! 智能体腿的落位规则只有一条（记忆 = 本插件目录的父目录下的 `MEMORY.md`），两个
//! 装配作用域因此是**同一个插件的两个实例**，不是两段代码。子树实例的注册经
//! 作用域访问器加 `agent/<id>/` 前缀，与系统侧并集且不撞名。

use super::config::MemoryConfig;
use super::memory::{self, PLUGIN_TITLE, SEGMENT_NAME};
use super::session;
use super::workspace;
use crate::memory_file::MemoryFile;
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;
use symbio_plugin_api::VdfsAccess;
use symbio_plugin_api::PLUGIN_ID_MEMORY;
use symbio_plugin_api::{
    capability_announce_configurable, plugin_dir_from_ctx, CapabilityRow, Plugin, PluginConfigFile,
    PluginDir, PluginError, PluginInvokeRequest, PluginInvokeRequestExt, PluginInvokeResponse,
    PluginMeta, PluginPayload, AGENT_VDFS, CAPABILITY_VISITOR, PATH, SESSION_VDFS,
    TRAVERSE_AVAILABLE_TOOLS, WORKSPACE_VDFS,
};
use symbio_plugin_api::{DetailDefinition, DetailField};
use tokio::sync::RwLock;

/// 配置表单的定义 —— **定义由配置的拥有者产出**，默认值从 [`MemoryConfig::default`]
/// 读出，不写第二份字面量（避免面板显示值与实际行为漂移）。
///
/// 三个作用域的字段分组排列：工作区在前（有开关，是用户最常调的），会话居中，
/// 智能体在后（闸门对，与出厂行为一致）。
fn config_definition() -> DetailDefinition {
    let d = MemoryConfig::default();
    DetailDefinition::form(
        PLUGIN_TITLE,
        vec![
            // 三层记忆的**正文**都不是配置项：配置只存开关与闸门，正文一律经各自的
            // VDFS 地址（`<根>/memory/MEMORY.md` / `WORKSPACE.md` / `SESSION.md`）读写。
            // 稳定指令 `AGENTS.md` 归 `setting`，更不在这里。
            DetailField::toggle(
                "agent_enabled",
                "注入智能体记忆",
                "每轮请求把本智能体的记忆追加进系统提示词（关掉只影响注入，文件照旧可编辑）",
                d.agent_enabled,
            ),
            DetailField::toggle(
                "workspace_enabled",
                "注入工作区记忆",
                "每轮请求把工作区记忆追加进系统提示词（会话已选工作区时生效）",
                d.workspace_enabled,
            ),
            DetailField::toggle(
                "session_enabled",
                "注入会话记忆",
                "每轮请求把本会话的记忆追加进系统提示词（关掉只影响注入，文件照旧可编辑）",
                d.session_enabled,
            ),
            DetailField::number(
                "workspace_max_bytes",
                "工作区写入上限（字节）",
                "工作区记忆文件单次写入的字节上限，超出会被拒绝",
                1.0,
                1_048_576.0,
                json!(d.workspace_max_bytes),
            ),
            DetailField::number(
                "workspace_inject_max_bytes",
                "工作区注入上限（字节）",
                "每轮注入系统提示词的工作区记忆正文字节上限，超出部分截断并指路 vdfs_read 取全文",
                1.0,
                1_048_576.0,
                json!(d.workspace_inject_max_bytes),
            ),
            DetailField::number(
                "session_max_bytes",
                "会话写入上限（字节）",
                "会话记忆文件单次写入的字节上限，超出会被拒绝",
                1.0,
                1_048_576.0,
                json!(d.session_max_bytes),
            ),
            DetailField::number(
                "session_inject_max_bytes",
                "会话注入上限（字节）",
                "每轮注入系统提示词的会话记忆正文字节上限，超出部分截断并指路 vdfs_read 取全文",
                1.0,
                1_048_576.0,
                json!(d.session_inject_max_bytes),
            ),
            DetailField::number(
                "memory_max_bytes",
                "智能体写入上限（字节）",
                "智能体自身记忆文件单次写入的字节上限，超出会被拒绝",
                1.0,
                1_048_576.0,
                json!(d.memory_max_bytes),
            ),
            DetailField::number(
                "memory_inject_max_bytes",
                "智能体注入上限（字节）",
                "每轮注入系统提示词的智能体记忆正文字节上限，超出部分截断并指路 vdfs_read 取全文",
                1.0,
                1_048_576.0,
                json!(d.memory_inject_max_bytes),
            ),
        ],
    )
}

/// 记忆插件（智能体自身 / 工作区 / 本会话，三个作用域）。
pub struct MemoryPlugin {
    /// 生效配置（配置文档的写入经 [`PluginConfigFile::apply`] 落在这里）
    config: Arc<RwLock<MemoryConfig>>,
    /// 配置文件的呈现与校验（`<根>/memory/PLUGIN.yml`）
    ///
    /// 同时是**本插件目录的持有者**：智能体记忆的落位由它的父目录推出
    /// （见 `super::memory`）。
    config_file: PluginConfigFile,
}

impl MemoryPlugin {
    /// 静态工厂：从 PluginInvokeRequest 构造 Plugin 实例
    pub fn build(ctx: Arc<dyn PluginInvokeRequest>) -> Arc<dyn Plugin> {
        let dir = plugin_dir_from_ctx(&*ctx, PLUGIN_ID_MEMORY);
        let config: MemoryConfig = match dir.load::<MemoryConfig>() {
            Ok(Some(c)) => c,
            Ok(None) => MemoryConfig::default(),
            Err(e) => {
                symbio_plugin_api::plugin_warn!("memory", "读取自身配置失败，改用默认值：{e}");
                MemoryConfig::default()
            }
        };
        Arc::new(Self::new(dir, config)) as Arc<dyn Plugin>
    }

    pub fn new(dir: PluginDir, config: MemoryConfig) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
            config_file: PluginConfigFile::new(dir, PLUGIN_TITLE, config_definition()),
        }
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new(PLUGIN_ID_MEMORY, PLUGIN_TITLE)
            .with_description(
                "记忆：智能体自身 / 工作区 / 本会话三个作用域的长期记忆，模型可读写。",
            )
            .with_version("0.2.0")
            // 在既有挂载点之后（session 1 / model 2 / agent 3 / skill 4 / mcp 5 /
            // plugin_manager 6 / local 7 / web 8 / gateway 9 / telegram 10）
            .with_order(11)
            .with_icon(r#"<path d="M2 3h6a4 4 0 0 1 4 4v14a3 3 0 0 0-3-3H2z"/><path d="M22 3h-6a4 4 0 0 0-4 4v14a3 3 0 0 1 3-3h7z"/>"#)
            // 根可列举 + 可递归遍历（记忆文件在根下，树视图要能走到它）
            .with_root_access(VdfsAccess::LIST_TRAVERSE)
            // 入口在**设置页**（plugin_manager 的 memory 条目），不进侧边栏：
            // 记忆没有列表内容，用户要改的是智能体记忆正文与闸门，不是浏览一个资源集合。
            .with_hidden(true)
    }

    /// 依生效配置构造**智能体记忆**门面（两道闸门在此注入，调用点不必各自判断）。
    ///
    /// D-515：作用域来自 `ctx[AGENT_VDFS]`（provider 缺失 = 无此层），
    /// 不再由插件目录反推物理路径。
    pub(crate) async fn store(&self, ctx: &Arc<dyn PluginInvokeRequest>) -> MemoryFile {
        let cfg = self.config.read().await;
        memory::store(
            ctx.get(AGENT_VDFS).flatten(),
            cfg.effective_memory_max_bytes(),
            cfg.effective_memory_inject_bytes(),
        )
    }

    /// 由请求上下文 + 生效配置构造**工作区记忆**门面。
    ///
    /// **作用域闸门**（`ctx[WORKSPACE_VDFS]` 缺失 → 无作用域）与**两道容量闸门**
    /// 都在这一处注入，因此调用点不必各自判断。
    pub(crate) async fn workspace_store_of(
        &self,
        ctx: &Arc<dyn PluginInvokeRequest>,
    ) -> MemoryFile {
        let cfg = self.config.read().await;
        workspace::store(
            ctx.get(WORKSPACE_VDFS).flatten(),
            cfg.effective_workspace_max_bytes(),
            cfg.effective_workspace_inject_bytes(),
        )
    }

    /// 由请求上下文 + 生效配置构造**会话记忆**门面。
    ///
    /// **作用域闸门**（`ctx[SESSION_VDFS]` 缺失 → 无作用域）与**两道容量闸门**
    /// 都在这一处注入。provider 由会话插件按唯一布局构造、宿主随轮递来；
    /// 本插件不认识 `SESSION_ID`，也不从它拼路径（D-513 / D-515）。
    pub(crate) async fn session_store_of(&self, ctx: &Arc<dyn PluginInvokeRequest>) -> MemoryFile {
        let cfg = self.config.read().await;
        session::store(
            ctx.get(SESSION_VDFS).flatten(),
            cfg.effective_session_max_bytes(),
            cfg.effective_session_inject_bytes(),
        )
    }

    /// 配置文档（呈现 / 校验 / 落盘）——VDFS 侧读写的入口
    pub(crate) fn config_file(&self) -> &PluginConfigFile {
        &self.config_file
    }

    /// 配置槽位（[`PluginConfigFile::read`] / [`PluginConfigFile::apply`] 的读写对象）
    pub(crate) fn config_slot(&self) -> &RwLock<MemoryConfig> {
        &self.config
    }

    /// 参与能力收集：交出**智能体记忆**的系统提示词。
    ///
    /// `agent_enabled` 关 → 不注入（文件与挂载照旧可用）。
    /// 空文件 / 还没写过 → 静默跳过——「还没写过」不是故障，
    /// 不该往收集期错误桶里塞东西。
    async fn contribute_prompt(
        &self,
        ctx: &Arc<dyn PluginInvokeRequest>,
        visitor: &Arc<dyn symbio_plugin_api::Registry>,
    ) {
        let cfg = self.config.read().await;
        if !cfg.agent_enabled {
            return;
        }
        drop(cfg);
        let store = self.store(ctx).await;
        // 绝对地址 = 上下文父地址 + 相对地址（容器转发时已写入父地址）
        let address = symbio_plugin_api::absolute_addr(ctx, memory::rel_path());
        match memory::segment(&store, &address).await {
            Ok(Some(segment)) => {
                visitor
                    .put(CapabilityRow::prompt(SEGMENT_NAME, segment))
                    .await
            }
            Ok(None) => {}
            Err(e) => {
                symbio_plugin_api::plugin_warn!("memory", "读取智能体记忆失败，本轮不注入：{e}")
            }
        }
    }

    /// 参与能力收集：交出**工作区记忆**的系统提示词。
    ///
    /// 两道闸门：
    /// - `workspace_enabled` 关 → 不注入（但挂载点照旧交出——关掉注入不等于关掉编辑面）；
    /// - 没选工作区 → 无作用域，静默跳过——这不是故障，不该往收集期错误桶里塞东西。
    ///
    /// 与智能体腿不同：工作区腿**空文件也注入**（带空内容提示）——工作区记忆需要
    /// 教会模型「这里可以建立记忆」，智能体记忆则是「没有就别占上下文」。
    async fn contribute_workspace_prompt(
        &self,
        ctx: &Arc<dyn PluginInvokeRequest>,
        visitor: &Arc<dyn symbio_plugin_api::Registry>,
    ) {
        let cfg = self.config.read().await.clone();
        if !cfg.workspace_enabled {
            return;
        }
        let store = self.workspace_store_of(ctx).await;
        // 绝对地址 = 上下文父地址 + 相对地址（容器转发时已写入父地址）
        let address = symbio_plugin_api::absolute_addr(ctx, workspace::rel_path());
        match store.segment(&workspace::segment_spec(&address)).await {
            Ok(Some(segment)) => {
                visitor
                    .put(CapabilityRow::prompt(workspace::SEGMENT_NAME, segment))
                    .await;
            }
            // 无工作区：不注入（正常状态，不报错）
            Ok(None) => {}
            Err(e) => {
                symbio_plugin_api::plugin_warn!("memory", "读取工作区记忆失败，本轮不注入：{e}")
            }
        }
    }

    /// 参与能力收集：交出**会话记忆**的系统提示词。
    ///
    /// `session_enabled` 关 → 不注入（文件与挂载照旧可用）。
    /// 与工作区腿同形：`ctx[SESSION_DIR]` 缺失 / 空 → 无作用域，静默跳过；有作用域
    /// 时空文件也注入（带空内容提示），让模型知道这里可以建立本会话的置顶约定。
    async fn contribute_session_prompt(
        &self,
        ctx: &Arc<dyn PluginInvokeRequest>,
        visitor: &Arc<dyn symbio_plugin_api::Registry>,
    ) {
        let cfg = self.config.read().await;
        if !cfg.session_enabled {
            return;
        }
        drop(cfg);
        let store = self.session_store_of(ctx).await;
        let address = symbio_plugin_api::absolute_addr(ctx, session::rel_path());
        match store.segment(&session::segment_spec(&address)).await {
            Ok(Some(segment)) => {
                visitor
                    .put(CapabilityRow::prompt(session::SEGMENT_NAME, segment))
                    .await;
            }
            Ok(None) => {}
            Err(e) => {
                symbio_plugin_api::plugin_warn!("memory", "读取会话记忆失败，本轮不注入：{e}")
            }
        }
    }
}

/// 无装配上下文的实例（仅测试）。目录给临时目录——**不读全局系统根**。
#[cfg(test)]
impl Default for MemoryPlugin {
    fn default() -> Self {
        Self::new(
            PluginDir::at(
                std::env::temp_dir().join("symbio-test/homedir/memory"),
                PLUGIN_ID_MEMORY,
            ),
            MemoryConfig::default(),
        )
    }
}

#[async_trait]
impl Plugin for MemoryPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    fn get_vfs_provider(self: Arc<Self>) -> Option<Arc<dyn symbio_plugin_api::VdfsProvider>> {
        Some(self)
    }

    /// 路由入口：**无自有协议**。
    ///
    /// 记忆的读 / 写 / 编辑全部由 VDFS 承接（`<根>/memory/MEMORY.md` /
    /// `<根>/memory/WORKSPACE.md` / `<根>/memory/SESSION.md`）——与 work / agent
    /// 插件同一取舍：宿主与模型走同一条链路，不为「记忆」再造一条协议。
    async fn route(
        self: Arc<Self>,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let path = ctx.get(PATH).unwrap_or_default();
        Err(PluginError::NotFound(format!(
            "memory 无自有协议路由 `{path}`：记忆一律经 VDFS 访问（{} / {} / {}）",
            symbio_plugin_api::absolute_addr(&ctx, memory::rel_path()),
            symbio_plugin_api::absolute_addr(&ctx, workspace::rel_path()),
            symbio_plugin_api::absolute_addr(&ctx, session::rel_path()),
        )))
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let sub_path = ctx.get(PATH).unwrap_or_default();
        if sub_path != TRAVERSE_AVAILABLE_TOOLS {
            return Err(PluginError::NotFound(format!("未知遍历路径: {sub_path}")));
        }

        if let Some(visitor) = ctx.get(CAPABILITY_VISITOR) {
            // 巩固的债 + 还债的端口：两处在同一次收集里交出（D-268 订正）。
            symbio_plugin_api::register_ports(&ctx, super::port::memory_port_provider().as_ref())
                .await;
            for row in super::debt::memory_debt_rows() {
                visitor.put(row).await;
            }
            // 系统提示词：三段注入（各自含地址与容量口径）
            self.contribute_prompt(&ctx, &visitor).await;
            self.contribute_workspace_prompt(&ctx, &visitor).await;
            self.contribute_session_prompt(&ctx, &visitor).await;
        }

        // 顺带声明「本插件有一份配置文档」（设置页据此列出并指路）
        capability_announce_configurable(&ctx, &self.config_file).await;

        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

symbio_plugin_api::submit_object_creator!(PLUGIN_ID_MEMORY, MemoryPlugin::build, dyn Plugin);

#[cfg(test)]
#[path = "plugin.test.rs"]
mod tests;
