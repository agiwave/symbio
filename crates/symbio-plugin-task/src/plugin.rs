//! Task 插件本体：本能力没有工具与路由，四条扩展面都走 `symbio-plugin-api` 的
//! 链接期收集，不需要插件实例参与。
//!
//! 实例仍然要有：装配清单按插件目录挂载，停用插件 ⇒ 实例不构造 ⇒ 它的行为 /
//! 贡献 / 段 / 声明随 crate 一起从进程里消失。这里是那个挂载点本身。

use async_trait::async_trait;
use std::sync::Arc;
use symbio_plugin_api::PLUGIN_ID_TASK;
use symbio_plugin_api::{
    plugin_dir_from_ctx, DetailDefinition, DetailField, Plugin, PluginConfigFile, PluginDir,
    PluginError, PluginInvokeRequest, PluginInvokeRequestExt, PluginInvokeResponse, PluginMeta,
    PluginPayload, PATH,
};

use crate::config::TaskConfig;

/// 插件的展示标题（元数据一处定义）。
pub const PLUGIN_TITLE: &str = "任务";

pub struct TaskPlugin {
    /// 本能力自己的配置（D-427）：真源在 `<根>/task/PLUGIN.yml`。
    config: TaskConfig,
    config_file: PluginConfigFile,
}

impl TaskPlugin {
    /// 工厂方法（满足 `submit_object_creator!` 协议）。
    pub fn build(ctx: Arc<dyn PluginInvokeRequest>) -> Arc<dyn Plugin> {
        let dir = plugin_dir_from_ctx(&*ctx, PLUGIN_ID_TASK);
        let config: TaskConfig = match dir.load::<TaskConfig>() {
            Ok(Some(c)) => c,
            Ok(None) => TaskConfig::default(),
            Err(e) => {
                symbio_plugin_api::plugin_warn!("task", "读取自身配置失败，改用默认值：{e}");
                TaskConfig::default()
            }
        };
        Arc::new(TaskPlugin::new(config, dir)) as Arc<dyn Plugin>
    }

    pub fn new(config: TaskConfig, dir: PluginDir) -> Self {
        Self {
            config,
            config_file: PluginConfigFile::new(dir, PLUGIN_TITLE, config_definition()),
        }
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new(PLUGIN_ID_TASK, PLUGIN_TITLE)
            .with_description("任务表声明、独立验证与返工、就绪集调度")
            .with_version("0.1.14")
    }
}

#[async_trait]
impl Plugin for TaskPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    /// 本插件不占用路由面：行为 / 声明 / 段 / 贡献都走链接期收集。
    async fn route(
        self: Arc<Self>,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let path = ctx.get(PATH).unwrap_or_default();
        Err(PluginError::NotFound(format!(
            "[task] 本插件没有路由：{path}"
        )))
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        // 本轮配置快照贡献（D-427）：容量是本能力的知识，由本能力交值；宿主只按
        // owner 合并，不认识键名。卸载 ⇒ 该格缺席 ⇒ 读方按 TASK_CAP 收敛。
        symbio_plugin_api::capability_announce_config_snapshot(
            &ctx,
            "readyset_cap",
            serde_json::json!(self.config.readyset_cap),
        )
        .await;
        symbio_plugin_api::capability_announce_configurable(&ctx, &self.config_file).await;
        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

fn config_definition() -> DetailDefinition {
    let d = TaskConfig::default();
    DetailDefinition::form(
        PLUGIN_TITLE,
        vec![DetailField::number(
            "readyset_cap",
            "就绪集容量",
            "一轮最多派几笔就绪任务；卸载本插件同样让这一格缺席，读方按出厂值\
             收敛（D-427）",
            1.0,
            64.0,
            serde_json::json!(d.readyset_cap),
        )],
    )
}

symbio_plugin_api::submit_object_creator!(PLUGIN_ID_TASK, TaskPlugin::build, dyn Plugin);
