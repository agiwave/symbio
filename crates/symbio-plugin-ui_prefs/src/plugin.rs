//! ui_prefs 插件本体 —— 界面偏好的**后端权威**。
//!
//! ## 为什么独立成插件
//!
//! 外观 / 提示音此前是 `plugin_manager` 的自有分区，数据在前端 localStorage 里，
//! 后端没有任何归属——这破坏了「一切皆插件」的一致性（D-521）。本插件把那两份
//! 数据接成一份配置文档 `<根>/ui_prefs/PLUGIN.yml`，由 `PluginConfigMount` 提供
//! 读写与落盘；前端 localStorage 退为**首屏快照 + 离线降级**（见 `main.ts`）。
//!
//! ## 不声明 configurable（刻意）
//!
//! 普通配置型插件会经 `capability_announce_configurable` 把自己的表单声明挂到
//! 插件管理条目上。本插件**不做这件事**：外观 / 提示音的入口是
//! `<插件管理>/appearance`，由 `plugin_manager` 把读写转发到这里；若再声明一份
//! configurable，用户会在 `ui_prefs` 条目里看到第二张外观表单——同一份数据的两个
//! 入口，其中一张还是通用表单，不是专属面板。

use super::config::{config_definition, UiPrefsConfig};
use async_trait::async_trait;
use std::sync::Arc;
use symbio_plugin_api::PLUGIN_ID_UI_PREFS;
use symbio_plugin_api::{
    plugin_dir_from_ctx, Plugin, PluginConfigFile, PluginConfigMount, PluginDir, PluginError,
    PluginInvokeRequest, PluginInvokeRequestExt, PluginInvokeResponse, PluginMeta, PluginPayload,
    PATH,
};
use tokio::sync::RwLock;

/// 界面偏好插件
pub struct UiPrefsPlugin {
    /// 生效配置（写入经 [`PluginConfigFile::apply`] 落盘）
    config: Arc<RwLock<UiPrefsConfig>>,
    /// 配置文件的呈现与校验（`<根>/ui_prefs/PLUGIN.yml`）
    config_file: PluginConfigFile,
}

impl UiPrefsPlugin {
    /// 静态工厂：从 PluginInvokeRequest 构造 Plugin 实例
    pub fn build(ctx: Arc<dyn PluginInvokeRequest>) -> Arc<dyn Plugin> {
        let dir = plugin_dir_from_ctx(&*ctx, PLUGIN_ID_UI_PREFS);
        let config: UiPrefsConfig = match dir.load::<UiPrefsConfig>() {
            Ok(Some(c)) => c,
            Ok(None) => UiPrefsConfig::default(),
            Err(e) => {
                symbio_plugin_api::plugin_warn!("ui_prefs", "读取界面偏好失败，改用默认值：{e}");
                UiPrefsConfig::default()
            }
        };
        Arc::new(Self::new(dir, config)) as Arc<dyn Plugin>
    }

    pub fn new(dir: PluginDir, config: UiPrefsConfig) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
            config_file: PluginConfigFile::new(dir, "界面偏好", config_definition()),
        }
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new(PLUGIN_ID_UI_PREFS, "界面偏好")
            .with_description("外观与提示音的后端权威数据；入口在插件管理的「外观」分区。")
            .with_version("0.1.0")
            .with_order(14)
            .with_icon(PLUGIN_ID_UI_PREFS)
            // 配置型挂载点：入口在**插件管理的外观分区**，不进侧边栏
            .with_hidden(true)
    }
}

/// 无装配上下文的实例（仅测试）
#[cfg(test)]
impl Default for UiPrefsPlugin {
    fn default() -> Self {
        Self::new(
            PluginDir::at(
                std::env::temp_dir().join("symbio-test/ui_prefs"),
                PLUGIN_ID_UI_PREFS,
            ),
            UiPrefsConfig::default(),
        )
    }
}

#[async_trait]
impl Plugin for UiPrefsPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    fn get_vfs_provider(self: Arc<Self>) -> Option<Arc<dyn symbio_plugin_api::VdfsProvider>> {
        Some(self)
    }

    /// 无自有协议路由：界面偏好一律经 VDFS 访问。
    async fn route(
        self: Arc<Self>,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let path = ctx.get(PATH).unwrap_or_default();
        Err(PluginError::NotFound(format!(
            "ui_prefs 无自有协议路由 `{path}`；界面偏好一律经 VDFS 访问（{}）",
            symbio_plugin_api::absolute_addr(&ctx, symbio_plugin_api::PLUGIN_FILE)
        )))
    }

    /// 不参与能力收集：这份配置只给前端读，没有任何模型侧消费者。
    async fn traverse(
        self: Arc<Self>,
        _path: String,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

// ==================== VDFS：`<根>/ui_prefs/PLUGIN.yml` ====================

impl PluginConfigMount for UiPrefsPlugin {
    type Config = UiPrefsConfig;
    const TITLE: &'static str = "界面偏好";

    fn config_file(&self) -> &PluginConfigFile {
        &self.config_file
    }

    fn config_slot(&self) -> &RwLock<UiPrefsConfig> {
        &self.config
    }
}

symbio_plugin_api::submit_object_creator!(PLUGIN_ID_UI_PREFS, UiPrefsPlugin::build, dyn Plugin);

symbio_plugin_api::impl_config_mount_vdfs!(UiPrefsPlugin);

#[cfg(test)]
#[path = "plugin.test.rs"]
mod tests;
