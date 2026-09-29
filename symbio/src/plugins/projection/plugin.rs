//! projection 插件 —— **可选**的投影内省口：把登记过的投影暴露为只读路由。
//!
//! ## 它解决什么
//!
//! B2 的投影表（[`symbio_core::Projection`]）是**进程内**的登记机制：投影住各自插件，
//! 编译期登记进 core 的表。但"表里有哪些投影""某个投影跑出来是什么"在**系统外部
//! 不可见**——审计者 / 测试 / 调试者无从观察。
//!
//! 本插件给这张表加一个**只读窗口**：两条路由，`list` 列名、`run` 跑一个并回结果。
//!
//! ## 三条设计边界（与 `fact_log` 同款）
//!
//! | 边界 | 为什么 |
//! |---|---|
//! | **可选**：停用 / 未装配 = 系统照常运行（J2 平凡值） | 它提供的是**可观测性**，不是地基。不在 `ASSEMBLY_SUB_AGENT_PLUGINS` 里 |
//! | **只读**：只查表 / 运行纯投影，不写任何域 | 投影本身无副作用；本插件只是它的出口 |
//! | **零插件依赖**：只经 `symbio_core` 的投影 API，不 import 任何兄弟插件 | 插件独立原则 |
//!
//! ## 为什么**不**是新能力去挂的钩子
//!
//! 消费方（检索者 / 巩固者）**不**经本插件取投影——它们直接调 `symbio_core::projection_run`。
//! 本插件纯属**人看的口**（HTTP / VDFS 边界），与"机制面"分离。
//! 这与 `fact_log` 把 `list` 定位为"机制面内省口、非 LLM 工具"是同一取舍。

use crate::symbio_core::schemas::detail::{DetailDefinition, DetailField};
use crate::symbio_core::{
    plugin_dir_from_ctx, projection_list, projection_run, Fact, Plugin, PluginConfigFile,
    PluginDir, PluginError, PluginInvokeRequest, PluginInvokeResponse, PluginMeta, PluginPayload,
    ProjectionInput, PLUGIN_ID_PROJECTION,
};
use crate::symbio_core::{PluginInvokeRequestExt, PATH};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

const SEGMENT_TITLE: &str = "投影表";

/// 本插件自身配置 —— 只有一个开关（与 `fact_log` 同形）。
#[derive(Debug, Clone, Deserialize)]
pub struct ProjectionConfig {
    /// 是否开放内省路由。关掉 = 路由不可达（平凡值），但**不影响投影机制本身**
    /// ——投影仍登记在表里，消费方照常经 `symbio_core::projection_run` 取用。
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

impl Default for ProjectionConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

fn config_definition() -> DetailDefinition {
    let d = ProjectionConfig::default();
    DetailDefinition::form(
        SEGMENT_TITLE,
        vec![DetailField::toggle(
            "enabled",
            "启用投影内省口",
            "把已登记的投影暴露为只读查询路由（审计 / 调试用）",
            d.enabled,
        )],
    )
}

/// 投影插件。
pub struct ProjectionPlugin {
    config: ProjectionConfig,
    config_file: PluginConfigFile,
}

impl ProjectionPlugin {
    pub fn build(ctx: Arc<dyn PluginInvokeRequest>) -> Arc<dyn Plugin> {
        let dir = plugin_dir_from_ctx(&*ctx, PLUGIN_ID_PROJECTION);
        let config: ProjectionConfig = match dir.load::<ProjectionConfig>() {
            Ok(Some(c)) => c,
            Ok(None) => ProjectionConfig::default(),
            Err(e) => {
                crate::plugin_warn!("projection", "读取自身配置失败，改用默认值：{e}");
                ProjectionConfig::default()
            }
        };
        Arc::new(Self::new(dir, config)) as Arc<dyn Plugin>
    }

    pub fn new(dir: PluginDir, config: ProjectionConfig) -> Self {
        Self {
            config,
            config_file: PluginConfigFile::new(dir, SEGMENT_TITLE, config_definition()),
        }
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new(PLUGIN_ID_PROJECTION, SEGMENT_TITLE)
            .with_description("投影内省口：列出 / 运行已登记的纯投影（只读）。")
            .with_version("0.1.0")
            .with_order(13)
            .with_icon(PLUGIN_ID_PROJECTION)
            .with_hidden(true)
    }
}

/// 取载荷中的 `facts` 数组并反序列化为事实；失败 ⇒ 空（只读口宽容）。
fn facts_of(payload: &serde_json::Value) -> Vec<Fact> {
    payload
        .get("facts")
        .cloned()
        .and_then(|v| serde_json::from_value::<Vec<Fact>>(v).ok())
        .unwrap_or_default()
}

#[async_trait]
impl Plugin for ProjectionPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    /// 两条只读内省路由：
    /// - `list` → 已登记的投影名（排序去重）；
    /// - `run`  → 运行一个投影，回 `{ name, view }`（`view` 即 `View<V>` 的 JSON）。
    ///
    /// **[`ProjectionConfig::enabled`] = false 时两条都不可达**（平凡值，J2）：
    /// 内省口关掉不影响投影机制，只影响"这个窗口"。
    async fn route(
        self: Arc<Self>,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let path = ctx.get(PATH).unwrap_or_default();
        let path = path.strip_prefix('/').unwrap_or(&path);

        if !self.config.enabled {
            return Err(PluginError::NotFound(format!(
                "[projection] 内省口已停用（enabled=false），路由 {path} 不可达"
            )));
        }

        match path {
            "list" => Ok(PluginPayload::new(&projection_list())),
            "run" => {
                // 载荷可为空（最小调用）——取不到就按空对象处理，投影跑出确定结果。
                let body: serde_json::Value = ctx.payload().unwrap_or(serde_json::Value::Null);
                let name = body.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let facts = facts_of(&body);
                let at_ms = body.get("at_ms").and_then(|v| v.as_i64()).unwrap_or(0);
                let input = ProjectionInput::new(&facts, at_ms);
                match projection_run(name, &input) {
                    Ok(view) => Ok(PluginPayload::new(&json!({ "name": name, "view": view }))),
                    Err(e) => Err(PluginError::NotFound(format!("[projection] {e}"))),
                }
            }
            _ => Err(PluginError::NotFound(format!(
                "[projection] 无路由子命令: {path}"
            ))),
        }
    }

    /// 本插件**没有能力面**：它是纯内省口，不贡献工具 / 选项 / 配置收集。
    ///
    /// 仍需实现（`Plugin` trait 要求）：声明有配置文档，于是设置页能列出它。
    async fn traverse(
        self: Arc<Self>,
        _path: String,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        crate::symbio_core::capability_announce_configurable(&ctx, &self.config_file).await;
        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

crate::submit_object_creator!(PLUGIN_ID_PROJECTION, ProjectionPlugin::build, dyn Plugin);

#[cfg(test)]
#[path = "plugin.test.rs"]
mod tests;
