//! delegate 插件本体 —— `Plugin` 实现 + **两条只读路由**（判定 / 进展）。
//!
//! ## 它登记什么
//!
//! 什么都不登记：不注册能力（主会话不持有工具正是本插件的**前提**）、不注册
//! 事实源、不注册 VDFS 挂载点。它只在被调用时**读**——判定读配置，进展读磁盘。
//!
//! ## 两条路由
//!
//! | 路径 | 输入 | 输出 |
//! |---|---|---|
//! | `delegate/decide` | `payload = { "text": "用户消息" }` | `{ dispatch, reason }` |
//! | `delegate/progress` | `payload = { "parent": "主会话 id" }`（可选） | `{ parent, workers, rendered, count }` |
//!
//! 两者都是**只读内省口**（与 `fact_log/list`、`retrieval/list` 同款），
//! 不是 LLM 工具：本插件的消费者是**主会话的提示词组装**（下一步 R1-a 的注入点）
//! 与诊断面。
//!
//! ## 平凡值（J2）
//!
//! - `enabled = false` ⇒ 两条路由都 `NotFound`，系统照常（会话、其余路由零影响）；
//! - 没有 worker 会话 ⇒ `progress` 回空表 + `rendered: ""`（**不是错误**）。

use super::config::DelegateConfig;
use super::decide::decide;
use super::progress;
use crate::symbio_core::schemas::detail::{DetailDefinition, DetailField};
use crate::symbio_core::PATH;
use crate::symbio_core::{
    plugin_dir_from_ctx, Plugin, PluginConfigFile, PluginDir, PluginError, PluginInvokeRequest,
    PluginInvokeRequestExt, PluginInvokeResponse, PluginMeta, PluginPayload, PLUGIN_ID_DELEGATE,
};
use async_trait::async_trait;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const SEGMENT_TITLE: &str = "委派者";

/// 会话插件目录名（= 其 VDFS 挂载点 / 工厂 id）。**字面量而非 import**。
pub const PLUGIN_ID_SESSION_DIR: &str = "session";

fn config_definition(cfg: &DelegateConfig) -> DetailDefinition {
    DetailDefinition::form(
        SEGMENT_TITLE,
        vec![
            DetailField::toggle(
                "enabled",
                "启用委派者",
                "提供委派判定与后台任务快照两条只读出口（主会话据此决定是否开 worker）",
                cfg.enabled,
            ),
            DetailField::number(
                "digest_max",
                "能力目录条数上限",
                "折叠给主会话的能力目录最多列几项（截断会在目录里标注）",
                1.0,
                200.0,
                json!(cfg.effective_digest_max()),
            ),
        ],
    )
}

/// delegate 插件。
pub struct DelegatePlugin {
    config: DelegateConfig,
    config_file: PluginConfigFile,
    /// 读取根（`<homedir>`）——与 `fact_log` / `retrieval` 同款取法：
    /// 本插件目录是 `<root>/delegate`，父目录即系统根。
    root: PathBuf,
}

impl DelegatePlugin {
    pub fn build(ctx: Arc<dyn PluginInvokeRequest>) -> Arc<dyn Plugin> {
        let dir = plugin_dir_from_ctx(&*ctx, PLUGIN_ID_DELEGATE);
        let config: DelegateConfig = match dir.load::<DelegateConfig>() {
            Ok(Some(c)) => c,
            Ok(None) => DelegateConfig::default(),
            Err(e) => {
                crate::plugin_warn!("delegate", "读取自身配置失败，改用默认值：{e}");
                DelegateConfig::default()
            }
        };
        Arc::new(Self::new(dir, config)) as Arc<dyn Plugin>
    }

    pub fn new(dir: PluginDir, config: DelegateConfig) -> Self {
        let root = dir
            .dir()
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| dir.dir().to_path_buf());
        let config_file = PluginConfigFile::new(dir, SEGMENT_TITLE, config_definition(&config));
        Self {
            config,
            config_file,
            root,
        }
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new(PLUGIN_ID_DELEGATE, SEGMENT_TITLE)
            .with_description(
                "委派者：判定该不该开后台 worker 会话，并给出 worker 进展快照（只读）。",
            )
            .with_version("0.1.0")
            .with_order(16)
            .with_icon(PLUGIN_ID_DELEGATE)
            .with_hidden(true)
    }

    /// worker 快照（测试直接打这里，避免走 ctx）
    pub fn scan(&self) -> Vec<progress::WorkerState> {
        progress::scan(self.root.as_path())
    }
}

#[async_trait]
impl Plugin for DelegatePlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    async fn route(
        self: Arc<Self>,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let path = ctx.get(PATH).unwrap_or_default();
        let path = path.strip_prefix('/').unwrap_or(&path);

        if !self.config.enabled {
            return Err(PluginError::NotFound(format!(
                "[delegate] 委派者已停用（enabled=false），路由 {path} 不可达"
            )));
        }

        match path {
            "decide" => {
                let text = ctx
                    .payload::<serde_json::Value>()
                    .ok()
                    .and_then(|v| v.get("text").and_then(|t| t.as_str()).map(str::to_string))
                    .unwrap_or_default();
                let d = decide(&text, &self.config);
                Ok(PluginPayload::new(&json!({
                    "dispatch": d.dispatch.wire(),
                    "reason": d.reason,
                })))
            }
            "progress" => {
                // 可选 `parent`：给了就只报这一个主会话名下的 worker（注入自用），
                // 不给 = 全部（诊断面看全局）。空串按"不给"处理，不是"看全部"。
                let parent = ctx
                    .payload::<serde_json::Value>()
                    .ok()
                    .and_then(|v| {
                        v.get("parent")
                            .and_then(|p| p.as_str())
                            .map(str::trim)
                            .map(str::to_string)
                    })
                    .filter(|p| !p.is_empty());
                let workers = match parent.as_deref() {
                    Some(p) => progress::scan_of(self.root.as_path(), p),
                    None => self.scan(),
                };
                let rendered = progress::render(&workers);
                Ok(PluginPayload::new(&json!({
                    "parent": parent,
                    "count": workers.len(),
                    "workers": workers
                        .iter()
                        .map(|w| json!({
                            "session_id": w.session_id,
                            "parent": w.parent,
                            "title": w.title,
                            "rounds": w.rounds,
                            "state": w.state,
                            "last_step": w.last_step,
                        }))
                        .collect::<Vec<_>>(),
                    "rendered": rendered,
                })))
            }
            _ => Err(PluginError::NotFound(format!(
                "[delegate] 无路由子命令: {path}"
            ))),
        }
    }

    /// 本插件**没有能力面**（不贡献工具 / 选项），只声明配置文档。
    async fn traverse(
        self: Arc<Self>,
        _path: String,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        crate::symbio_core::capability_announce_configurable(&ctx, &self.config_file).await;
        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

crate::submit_object_creator!(PLUGIN_ID_DELEGATE, DelegatePlugin::build, dyn Plugin);

#[cfg(test)]
#[path = "plugin.test.rs"]
mod tests;
