//! setting 插件本体 —— 本智能体自身的**信息设置**（见 [`super`] 的模块文档）。
//!
//! 本插件只有一件事：**把「我是谁、我怎么说话」交给模型看，并让用户改得动它**。
//! 因此它在 `traverse` 里交出两样东西——
//!
//! 1. **系统提示词**（`register_system_prompt`）：[`segment`] 拼出的【本智能体】段；
//! 2. **VDFS 挂载点**（`<根>/setting`）：配置文档本身，用户在设置页 / 该地址上编辑。
//!
//! 挂载点不自己写 `VdfsProvider`：它是**纯配置挂载点**，四臂 dispatch 由
//! `symbio_core::PluginConfigMount` 的 blanket impl 提供（与 web / local / gateway
//! 同一条机制），本文件只声明差异。
//!
//! ## 本插件不认识「系统」与「子智能体」
//!
//! 它被装配在哪个智能体目录下，就描述哪个智能体——两个作用域是**同一个插件的两个
//! 实例**。子树实例的注册经 `agent` 插件的 `SubAgentVisitor` 加 `agent/<id>/` 前缀，
//! 与系统侧并集且不撞名。

use super::config::{config_definition, SettingConfig};
use super::segment;
use crate::symbio_core::{
    capability_announce_configurable, plugin_dir_from_ctx, Plugin, PluginConfigFile,
    PluginConfigMount, PluginDir, PluginError, PluginInvokeRequest, PluginInvokeRequestExt,
    PluginInvokeResponse, PluginMeta, PluginPayload, CAPABILITY_VISITOR, PATH, PLUGIN_ID_SETTING,
    TRAVERSE_AVAILABLE_TOOLS,
};
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::RwLock;

/// setting 插件。
pub struct SettingPlugin {
    /// 生效配置（配置文档的写入经 [`PluginConfigFile::apply`] 落在这里）
    config: Arc<RwLock<SettingConfig>>,
    /// 配置文件的呈现与校验（`<根>/setting/PLUGIN.yml`）
    config_file: PluginConfigFile,
}

impl SettingPlugin {
    /// 静态工厂：从 PluginInvokeRequest 构造 Plugin 实例
    pub fn build(ctx: Arc<dyn PluginInvokeRequest>) -> Arc<dyn Plugin> {
        let dir = plugin_dir_from_ctx(&*ctx, PLUGIN_ID_SETTING);
        let config: SettingConfig = match dir.load::<SettingConfig>() {
            Ok(Some(c)) => c,
            Ok(None) => SettingConfig::default(),
            Err(e) => {
                crate::plugin_warn!("setting", "读取自身配置失败，改用默认值：{e}");
                SettingConfig::default()
            }
        };
        Arc::new(Self::new(dir, config)) as Arc<dyn Plugin>
    }

    pub fn new(dir: PluginDir, config: SettingConfig) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
            config_file: PluginConfigFile::new(dir, "智能体设置", config_definition()),
        }
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new(PLUGIN_ID_SETTING, "智能体设置")
            .with_description("本智能体自身的信息：显示名、简介与回答偏好（语言 / 详略）。")
            .with_version("0.1.0")
            // 在既有挂载点之后（… / work 11 / memory 12）
            .with_order(13)
            .with_icon(PLUGIN_ID_SETTING)
            // 配置型挂载点：入口在**设置页**，不进侧边栏（与 web / local / gateway 同一口径）
            .with_hidden(true)
    }

    /// 参与能力收集：交出**系统提示词**（本智能体的自述与偏好）。
    ///
    /// 全空 → 静默跳过：缺省状态不该往提示词里塞一个空标题，也不该进收集期错误桶。
    async fn contribute_prompt(&self, visitor: &Arc<dyn crate::symbio_core::CapabilityVisitor>) {
        let cfg = self.config.read().await;
        if let Some(text) = segment::segment(&cfg) {
            visitor
                .register_system_prompt(segment::SEGMENT_NAME, text)
                .await;
        }
    }
}

/// 无装配上下文的实例（仅测试）。目录给临时目录——**不读全局系统根**。
#[cfg(test)]
impl Default for SettingPlugin {
    fn default() -> Self {
        Self::new(
            PluginDir::at(
                std::env::temp_dir().join("symbio-test/setting"),
                PLUGIN_ID_SETTING,
            ),
            SettingConfig::default(),
        )
    }
}

#[async_trait]
impl Plugin for SettingPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    fn get_vfs_provider(self: Arc<Self>) -> Option<Arc<dyn crate::symbio_core::VdfsProvider>> {
        Some(self)
    }

    /// 路由入口：**无自有协议**——本插件的全部内容就是一份配置文档，走 VDFS。
    async fn route(
        self: Arc<Self>,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let path = ctx.get(PATH).unwrap_or_default();
        Err(PluginError::NotFound(format!(
            "setting 无自有协议路由 `{path}`：智能体设置一律经 VDFS 访问（{}）",
            crate::symbio_core::absolute_addr(&ctx, crate::symbio_core::PLUGIN_FILE)
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
            // ① VDFS 挂载点：配置文档本体（用户在设置页 / 该地址上编辑）
            let me: crate::symbio_core::DynVdfsProvider = self.clone();
            visitor.register_vdfs_provider(PLUGIN_ID_SETTING, me).await;

            // ② 系统提示词：本智能体的自述与偏好
            self.contribute_prompt(&visitor).await;
        }

        // 顺带声明「本插件有一份配置文档」（设置页据此列出并指路）
        capability_announce_configurable(&ctx, &self.config_file).await;

        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

// ==================== VDFS：配置文档（`<根>/setting/PLUGIN.yml`） ====================

impl PluginConfigMount for SettingPlugin {
    type Config = SettingConfig;
    const TITLE: &'static str = "智能体设置";

    fn config_file(&self) -> &PluginConfigFile {
        &self.config_file
    }

    fn config_slot(&self) -> &RwLock<SettingConfig> {
        &self.config
    }
}

crate::submit_object_creator!(PLUGIN_ID_SETTING, SettingPlugin::build, dyn Plugin);

#[cfg(test)]
#[path = "plugin.test.rs"]
mod tests;
