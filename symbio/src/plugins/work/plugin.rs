//! work 插件 —— **工作区记忆**（机制与配置面见 [`super`] 的模块文档 / `README.md`）。
//!
//! 本插件只有一件事：让模型**记得住**工作区里长期有效的东西，并且**改得动**它。
//! 因此它在 `traverse` 里同时交出两样东西——
//!
//! 1. **系统提示词**（`register_system_prompt`）：记忆正文 + 文件地址 + 容量与写法，
//!    由会话侧按注册顺序拼接后送达模型；
//! 2. **VDFS 挂载点**（`<根>/work`）：记忆文件本身，模型用 `vdfs_read` / `vdfs_write`
//!    读写，用户在界面上也能直接编辑。
//!
//! 两样必须同时存在：只注入不给地址 = 只读记忆，永远长不大；只给地址不注入 =
//! 模型不知道它存在，等于没有。

use super::config::WorkConfig;
use super::memory::{self, SEGMENT_NAME, SEGMENT_TITLE, WORK_MEMORY_FILE};
use crate::providers::MemoryFile;
use crate::symbio_core::schemas::detail::{DetailDefinition, DetailField};
use crate::symbio_core::VdfsAccess;
use crate::symbio_core::{
    capability_announce_configurable, plugin_dir_from_ctx, Plugin, PluginConfigFile, PluginError,
    PluginInvokeRequest, PluginInvokeRequestExt, PluginInvokeResponse, PluginMeta, PluginPayload,
    CAPABILITY_VISITOR, PATH, PLUGIN_ID_WORK, TRAVERSE_AVAILABLE_TOOLS, WORKDIR,
};
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;
use tokio::sync::RwLock;

/// 配置表单的定义 —— **定义由配置的拥有者产出**，默认值从 [`WorkConfig::default`]
/// 读出，不写第二份字面量（避免面板显示值与实际行为漂移）。
fn config_definition() -> DetailDefinition {
    let d = WorkConfig::default();
    DetailDefinition::form(
        SEGMENT_TITLE,
        vec![
            DetailField::toggle(
                "memory_enabled",
                "注入工作区记忆",
                "每轮请求把工作区记忆追加进系统提示词",
                d.memory_enabled,
            ),
            DetailField::number(
                "memory_max_bytes",
                "写入上限（字节）",
                "记忆文件单次写入的字节上限，超出会被拒绝",
                1.0,
                1_048_576.0,
                json!(d.memory_max_bytes),
            ),
            DetailField::number(
                "memory_inject_max_bytes",
                "注入上限（字节）",
                "每轮注入系统提示词的记忆正文字节上限，超出部分截断",
                1.0,
                1_048_576.0,
                json!(d.memory_inject_max_bytes),
            ),
        ],
    )
}

/// 工作区记忆插件。
#[derive(Clone)]
pub struct WorkPlugin {
    /// 生效配置（配置文档的写入经 [`PluginConfigFile::apply`] 落在这里）
    config: Arc<RwLock<WorkConfig>>,
    /// 配置文件的呈现与校验（`<根>/work/PLUGIN.yml`）
    config_file: PluginConfigFile,
}

impl WorkPlugin {
    /// 静态工厂：从 PluginInvokeRequest 构造 Plugin 实例
    pub fn build(ctx: Arc<dyn PluginInvokeRequest>) -> Arc<dyn Plugin> {
        let dir = plugin_dir_from_ctx(&*ctx, PLUGIN_ID_WORK);
        let config: WorkConfig = match dir.load::<WorkConfig>() {
            Ok(Some(c)) => c,
            Ok(None) => WorkConfig::default(),
            Err(e) => {
                crate::plugin_warn!("work", "读取自身配置失败，改用默认值：{e}");
                WorkConfig::default()
            }
        };
        Arc::new(Self::new(dir, config)) as Arc<dyn Plugin>
    }

    pub fn new(dir: crate::symbio_core::PluginDir, config: WorkConfig) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
            config_file: PluginConfigFile::new(dir, SEGMENT_TITLE, config_definition()),
        }
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new(PLUGIN_ID_WORK, SEGMENT_TITLE)
            .with_description("工作区记忆：跨会话保留的长期事实与约定，模型可读写。")
            .with_version("0.1.0")
            // 在既有挂载点之后（session 1 / model 2 / agent 3 / skill 4 / mcp 5 /
            // plugin_manager 6 / local 7 / web 8 / gateway 9 / telegram 10）
            .with_order(11)
            .with_icon(PLUGIN_ID_WORK)
            .with_hidden(true)
            // 根可列举 + 可递归遍历（记忆文件在根下，树视图要能走到它）
            .with_root_access(VdfsAccess::LIST_TRAVERSE)
    }

    /// 由请求上下文 + 生效配置构造记忆门面。
    ///
    /// **作用域闸门**（`ctx[WORKDIR]` 缺失 / 空 → 无作用域）与**两道容量闸门**
    /// 都在这一处注入，因此调用点不必各自判断。
    fn store_with(ctx: &Arc<dyn PluginInvokeRequest>, cfg: &WorkConfig) -> MemoryFile {
        memory::store(
            ctx.get(WORKDIR).as_deref(),
            cfg.effective_max_bytes(),
            cfg.effective_inject_bytes(),
        )
    }

    /// 依请求上下文构造记忆门面（闸门取自当前配置）
    pub(crate) async fn store_of(&self, ctx: &Arc<dyn PluginInvokeRequest>) -> MemoryFile {
        let cfg = self.config.read().await;
        Self::store_with(ctx, &cfg)
    }
    /// 配置文档（呈现 / 校验 / 落盘）——VDFS 侧读写的入口
    pub(crate) fn config_file(&self) -> &PluginConfigFile {
        &self.config_file
    }

    /// 配置槽位（[`PluginConfigFile::read`] / [`PluginConfigFile::apply`] 的读写对象）
    pub(crate) fn config_slot(&self) -> &RwLock<WorkConfig> {
        &self.config
    }

    /// 参与能力收集：交出**系统提示词**（记忆注入）。
    ///
    /// 无工作区时静默跳过——「工作区记忆」在没有工作区时不存在，
    /// 这不是故障，不该往收集期错误桶里塞东西。
    async fn contribute_prompt(
        &self,
        ctx: &Arc<dyn PluginInvokeRequest>,
        visitor: &Arc<dyn crate::symbio_core::CapabilityVisitor>,
    ) {
        let cfg = self.config.read().await.clone();
        if !cfg.memory_enabled {
            return;
        }
        let store = Self::store_with(ctx, &cfg);
        // 绝对地址 = 上下文父地址 + 相对地址（容器转发时已写入父地址）
        let address = crate::symbio_core::absolute_addr(ctx, WORK_MEMORY_FILE);
        match store.segment(&memory::segment_spec(&address)) {
            Ok(Some(segment)) => {
                visitor.register_system_prompt(SEGMENT_NAME, segment).await;
            }
            // 无工作区：不注入（正常状态，不报错）
            Ok(None) => {}
            Err(e) => crate::plugin_warn!("work", "读取工作区记忆失败，本轮不注入：{e}"),
        }
    }
}

/// 无装配上下文的实例（仅测试）。目录给临时目录——**不读全局系统根**。
#[cfg(test)]
impl Default for WorkPlugin {
    fn default() -> Self {
        Self::new(
            crate::symbio_core::PluginDir::at(
                std::env::temp_dir().join("symbio-test/work"),
                PLUGIN_ID_WORK,
            ),
            WorkConfig::default(),
        )
    }
}

#[async_trait]
impl Plugin for WorkPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    fn get_vfs_provider(self: Arc<Self>) -> Option<Arc<dyn crate::symbio_core::VdfsProvider>> {
        Some(self)
    }

    /// 路由入口：**无自有协议**。
    ///
    /// 记忆的读 / 写 / 编辑全部由 VDFS 承接（`<根>/work/AGENTS.md`）——
    /// 与 agent 插件同一取舍：宿主与模型走同一条链路，不为「记忆」再造一条协议。
    async fn route(
        self: Arc<Self>,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let path = ctx.get(PATH).unwrap_or_default();
        Err(PluginError::NotFound(format!(
            "work 无自有协议路由 `{path}`：工作区记忆一律经 VDFS 访问（{}）",
            crate::symbio_core::absolute_addr(&ctx, WORK_MEMORY_FILE)
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
            // ① VDFS 挂载点：记忆文件本体（模型与用户共用的编辑面）
            let me: crate::symbio_core::DynVdfsProvider = self.clone();
            visitor.register_vdfs_provider(PLUGIN_ID_WORK, me).await;

            // ② 系统提示词：记忆注入（含地址与容量口径）
            self.contribute_prompt(&ctx, &visitor).await;
        }

        // 顺带声明「本插件有一份配置文档」（设置页据此列出并指路）
        capability_announce_configurable(&ctx, &self.config_file).await;

        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

crate::submit_object_creator!(PLUGIN_ID_WORK, WorkPlugin::build, dyn Plugin);

#[cfg(test)]
#[path = "plugin.test.rs"]
mod tests;
