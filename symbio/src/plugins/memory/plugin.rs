//! memory 插件本体 —— **智能体自身的记忆 + 工作区记忆**（机制与配置面见
//! [`super`] 的模块文档 / `README.md`）。
//!
//! 本插件只有一件事：让模型**记得住**长期有效的东西（智能体自己的 + 所在工作区的），
//! 并且**改得动**它们。因此它在 `traverse` 里同时交出——
//!
//! 1. **系统提示词**（`register_system_prompt`，两段）：智能体记忆与工作区记忆的
//!    正文 + 文件地址 + 容量与写法，由会话侧按注册顺序拼接后送达模型；
//! 2. **VDFS 挂载点**（`<根>/memory`）：两份记忆文件本体（`AGENTS.md` /
//!    `WORKSPACE.md`），模型用 `vdfs_read` / `vdfs_write` 读写，用户在界面上也能
//!    直接编辑。
//!
//! 两样必须同时存在：只注入不给地址 = 只读记忆，永远长不大；只给地址不注入 =
//! 模型不知道它存在，等于没有。
//!
//! ## 两个作用域，两条腿
//!
//! | 作用域 | 物理落位 | 作用域来源 | 挂载名 |
//! |---|---|---|---|
//! | 智能体自身 | `{宿主目录}/AGENTS.md` | **装配**（本插件目录的父目录，分形） | `AGENTS.md` |
//! | 工作区 | `{workdir}/AGENTS.md` | **会话**（`ctx[WORKDIR]`，可选） | `WORKSPACE.md` |
//!
//! 工作区腿的闸门（没选工作区 → 无作用域；`workspace_enabled` 关 → 不注入）在
//! [`Self::workspace_store_of`] / [`Self::contribute_workspace_prompt`] 两处收口。
//!
//! ## 本插件不认识「系统」与「子智能体」
//!
//! 智能体腿的落位规则只有一条（记忆 = 本插件目录的父目录下的 `AGENTS.md`），两个
//! 装配作用域因此是**同一个插件的两个实例**，不是两段代码。子树实例的注册经
//! `agent` 插件的 `SubAgentVisitor` 加 `agent/<id>/` 前缀，与系统侧并集且不撞名。

use super::config::MemoryConfig;
use super::memory::{self, PLUGIN_TITLE, SEGMENT_NAME};
use super::workspace;
use crate::providers::MemoryFile;
use crate::symbio_core::VdfsAccess;
use crate::symbio_core::{
    capability_announce_configurable, plugin_dir_from_ctx, Plugin, PluginConfigFile, PluginDir,
    PluginError, PluginInvokeRequest, PluginInvokeRequestExt, PluginInvokeResponse, PluginMeta,
    PluginPayload, CAPABILITY_VISITOR, PATH, PLUGIN_ID_MEMORY, TRAVERSE_AVAILABLE_TOOLS, WORKDIR,
};
use crate::symbio_core::{DetailDefinition, DetailField};
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;
use tokio::sync::RwLock;

/// 配置表单的定义 —— **定义由配置的拥有者产出**，默认值从 [`MemoryConfig::default`]
/// 读出，不写第二份字面量（避免面板显示值与实际行为漂移）。
///
/// 两个作用域的字段分组排列：工作区在前（有开关，是用户最常调的），
/// 智能体在后（闸门对，与出厂行为一致）。
fn config_definition() -> DetailDefinition {
    let d = MemoryConfig::default();
    DetailDefinition::form(
        PLUGIN_TITLE,
        vec![
            DetailField::toggle(
                "workspace_enabled",
                "注入工作区记忆",
                "每轮请求把工作区记忆追加进系统提示词（会话已选工作区时生效）",
                d.workspace_enabled,
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

/// 记忆插件（智能体自身 + 工作区，两个作用域）。
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
                crate::plugin_warn!("memory", "读取自身配置失败，改用默认值：{e}");
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
                "记忆：智能体自身的长期记忆（分形）+ 所在工作区的长期记忆，模型可读写。",
            )
            .with_version("0.2.0")
            // 在既有挂载点之后（session 1 / model 2 / agent 3 / skill 4 / mcp 5 /
            // plugin_manager 6 / local 7 / web 8 / gateway 9 / telegram 10）
            .with_order(11)
            .with_icon(PLUGIN_ID_MEMORY)
            // 根可列举 + 可递归遍历（记忆文件在根下，树视图要能走到它）
            .with_root_access(VdfsAccess::LIST_TRAVERSE)
    }

    /// 宿主目录 = **本插件目录的父目录**（智能体记忆落位规则的唯一出处，见
    /// `super::memory`）
    pub(crate) fn host_dir(&self) -> std::path::PathBuf {
        memory::host_dir(self.config_file.dir().dir())
    }

    /// 依生效配置构造**智能体记忆**门面（两道闸门在此注入，调用点不必各自判断）
    pub(crate) async fn store(&self) -> MemoryFile {
        let cfg = self.config.read().await;
        memory::store(
            &self.host_dir(),
            cfg.effective_memory_max_bytes(),
            cfg.effective_memory_inject_bytes(),
        )
    }

    /// 由请求上下文 + 生效配置构造**工作区记忆**门面。
    ///
    /// **作用域闸门**（`ctx[WORKDIR]` 缺失 / 空 → 无作用域）与**两道容量闸门**
    /// 都在这一处注入，因此调用点不必各自判断。
    pub(crate) async fn workspace_store_of(
        &self,
        ctx: &Arc<dyn PluginInvokeRequest>,
    ) -> MemoryFile {
        let cfg = self.config.read().await;
        workspace::store(
            ctx.get(WORKDIR).as_deref(),
            cfg.effective_workspace_max_bytes(),
            cfg.effective_workspace_inject_bytes(),
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
    /// 空文件 / 还没写过 → 静默跳过——「还没写过」不是故障，
    /// 不该往收集期错误桶里塞东西。
    async fn contribute_prompt(
        &self,
        ctx: &Arc<dyn PluginInvokeRequest>,
        visitor: &Arc<dyn crate::symbio_core::CapabilityVisitor>,
    ) {
        let store = self.store().await;
        // 绝对地址 = 上下文父地址 + 相对地址（容器转发时已写入父地址）
        let address = crate::symbio_core::absolute_addr(ctx, memory::rel_path());
        match memory::segment(&store, &address) {
            Ok(Some(segment)) => visitor.register_system_prompt(SEGMENT_NAME, segment).await,
            Ok(None) => {}
            Err(e) => crate::plugin_warn!("memory", "读取智能体记忆失败，本轮不注入：{e}"),
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
        visitor: &Arc<dyn crate::symbio_core::CapabilityVisitor>,
    ) {
        let cfg = self.config.read().await.clone();
        if !cfg.workspace_enabled {
            return;
        }
        let store = self.workspace_store_of(ctx).await;
        // 绝对地址 = 上下文父地址 + 相对地址（容器转发时已写入父地址）
        let address = crate::symbio_core::absolute_addr(ctx, workspace::rel_path());
        match store.segment(&workspace::segment_spec(&address)) {
            Ok(Some(segment)) => {
                visitor
                    .register_system_prompt(workspace::SEGMENT_NAME, segment)
                    .await;
            }
            // 无工作区：不注入（正常状态，不报错）
            Ok(None) => {}
            Err(e) => crate::plugin_warn!("memory", "读取工作区记忆失败，本轮不注入：{e}"),
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

    fn get_vfs_provider(self: Arc<Self>) -> Option<Arc<dyn crate::symbio_core::VdfsProvider>> {
        Some(self)
    }

    /// 路由入口：**无自有协议**。
    ///
    /// 记忆的读 / 写 / 编辑全部由 VDFS 承接（`<根>/memory/AGENTS.md` 与
    /// `<根>/memory/WORKSPACE.md`）——与 work / agent 插件同一取舍：宿主与模型走
    /// 同一条链路，不为「记忆」再造一条协议。
    async fn route(
        self: Arc<Self>,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let path = ctx.get(PATH).unwrap_or_default();
        Err(PluginError::NotFound(format!(
            "memory 无自有协议路由 `{path}`：记忆一律经 VDFS 访问（{} 与 {}）",
            crate::symbio_core::absolute_addr(&ctx, memory::rel_path()),
            crate::symbio_core::absolute_addr(&ctx, workspace::rel_path()),
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
            // ① VDFS 挂载点：两份记忆文件本体（模型与用户共用的编辑面）
            let me: crate::symbio_core::DynVdfsProvider = self.clone();
            visitor.register_vdfs_provider(PLUGIN_ID_MEMORY, me).await;

            // ② 系统提示词：两段注入（各自含地址与容量口径）
            self.contribute_prompt(&ctx, &visitor).await;
            self.contribute_workspace_prompt(&ctx, &visitor).await;
        }

        // 顺带声明「本插件有一份配置文档」（设置页据此列出并指路）
        capability_announce_configurable(&ctx, &self.config_file).await;

        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

crate::submit_object_creator!(PLUGIN_ID_MEMORY, MemoryPlugin::build, dyn Plugin);

#[cfg(test)]
#[path = "plugin.test.rs"]
mod tests;
