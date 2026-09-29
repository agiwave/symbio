//! actor 插件 —— **可选**的判定者登记方与 Actor 表内省口。
//!
//! ## 它解决什么
//!
//! B3 的 Actor 表（[`symbio_core::ActorSpec`]）是**进程内**的登记机制：行住各自插件，
//! 编译期登记进 core 的表。但"表里有哪些行""某一行长什么样"在**系统外部不可见**
//! ——审计者 / 测试 / 调试者无从观察。
//!
//! 本插件做两件事：
//!
//! 1. **登记** §2.3 的"抢占判定者"一行 —— `session.decider`，`budget_ms = 80`；
//! 2. 给表加一个**只读窗口** —— `list` 路由列出全部行。
//!
//! ## 为什么第 1 件是 B3 交付判据 #2 的证明
//!
//! > "新注册的 Actor 可在**不修改** `chat_loop` 代码的前提下接入"
//!
//! 本插件登记判定者只调 [`ActorSource::register_all`]；`chat_loop.rs` 里
//! **没有任何一行**提到 `decider` / 本插件名。于是这条判据可以用
//! `git diff -- symbio/src/plugins/session/chat_loop.rs` 直接核对。
//!
//! ## 三条设计边界（与 `fact_log` / `projection` 同款）
//!
//! | 边界 | 为什么 |
//! |---|---|
//! | **可选**：停用 / 未装配 = 系统照常运行（J2 平凡值） | 它提供的是**可观测性 + 一个演示行**，不是地基。不在 `ASSEMBLY_SUB_AGENT_PLUGINS` 里 |
//! | **只读**：只登记一行、只查表，不写任何域 | 登记不是"写业务数据"；本插件没有任何业务副作用 |
//! | **零插件依赖**：只经 `symbio_core` 的 actor API，不 import 任何兄弟插件 | 插件独立原则 |
//!
//! ## 为什么登记放在 `build` 而不是 `route`
//!
//! 判定者行**没有请求**也要在表里（它是"这个构建有什么执行者"的陈述，不是
//! "某次请求要干什么"）。放 `build` 则插件一装配就登记；`route` 只在调用时登记
//! 会让"表里有没有这一行"取决于"有没有人调过 `actor/list`"——那是自欺。
//!
//! 装配顺序无关：`actor_register` 是幂等的同 name 覆盖，且 core 的表**先于**任何
//! 插件存在（`OnceLock` 惰性初始化）。

use crate::symbio_core::schemas::detail::{DetailDefinition, DetailField};
use crate::symbio_core::{
    actor_list, plugin_dir_from_ctx, ActorPattern, ActorScope, ActorSource, ActorSpec, Plugin,
    PluginConfigFile, PluginDir, PluginError, PluginInvokeRequest, PluginInvokeResponse,
    PluginMeta, PluginPayload, PLUGIN_ID_ACTOR,
};
use crate::symbio_core::{PluginInvokeRequestExt, PATH};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

const SEGMENT_TITLE: &str = "执行者";

/// 判定者行的名字。**字面量副本**（不是 `use session::ACTOR_NAME_*`）——
/// 本插件不认识 session（插件独立原则）；两个名字各自独立地描述各自的行。
const ACTOR_NAME_SESSION_DECIDER: &str = "session.decider";

/// §2.3 的抢占判定者预算：80ms（只够一次极短裁决，不够一次完整推理）。
const DECIDER_BUDGET_MS: u64 = 80;

/// 本插件自身配置 —— 只有一个开关（与 `projection` 同形）。
#[derive(Debug, Clone, Deserialize)]
pub struct ActorConfig {
    /// 是否登记判定者行 + 开放内省路由。关掉 = 行不登记、路由不可达（平凡值），
    /// 但**不影响 Actor 机制本身**——session 侧那一行照常，会话照常运行。
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

impl Default for ActorConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

fn config_definition() -> DetailDefinition {
    let d = ActorConfig::default();
    DetailDefinition::form(
        SEGMENT_TITLE,
        vec![DetailField::toggle(
            "enabled",
            "启用执行者登记与内省口",
            "登记抢占判定者行，并把 Actor 表暴露为只读查询路由（审计 / 调试用）",
            d.enabled,
        )],
    )
}

/// 判定者登记方 —— 本插件就是它在 root 层的唯一登记者。
struct DeciderSource;

impl ActorSource for DeciderSource {
    fn layer(&self) -> ActorScope {
        // 本插件只装机在 root 树（不在 ASSEMBLY_SUB_AGENT_PLUGINS 里，
        // 故子树不会装配它）。若某天子树也要，装配方给不同的实现即可——
        // `layer` 由装配方赋予，不是自我声明（A5）。
        ActorScope::Root
    }

    fn specs(&self) -> Vec<ActorSpec> {
        vec![ActorSpec::new(
            ACTOR_NAME_SESSION_DECIDER,
            "", // 占位：判定者也是"运行期才知道是谁在问"
            ActorPattern::Decider,
            DECIDER_BUDGET_MS,
            ActorScope::Root,
        )]
    }
}

/// actor 插件。
pub struct ActorPlugin {
    config: ActorConfig,
    config_file: PluginConfigFile,
}

impl ActorPlugin {
    pub fn build(ctx: Arc<dyn PluginInvokeRequest>) -> Arc<dyn Plugin> {
        let dir = plugin_dir_from_ctx(&*ctx, PLUGIN_ID_ACTOR);
        let config: ActorConfig = match dir.load::<ActorConfig>() {
            Ok(Some(c)) => c,
            Ok(None) => ActorConfig::default(),
            Err(e) => {
                crate::plugin_warn!("actor", "读取自身配置失败，改用默认值：{e}");
                ActorConfig::default()
            }
        };
        Arc::new(Self::new(dir, config)) as Arc<dyn Plugin>
    }

    pub fn new(dir: PluginDir, config: ActorConfig) -> Self {
        let plugin = Self {
            config,
            config_file: PluginConfigFile::new(dir, SEGMENT_TITLE, config_definition()),
        };
        plugin.register_if_enabled();
        plugin
    }

    /// 登记判定者行（仅 `enabled` 时）。失败只记日志——表是扩展点，
    /// 少一行不该让插件装配失败（与 session 侧同款处置）。
    fn register_if_enabled(&self) {
        if !self.config.enabled {
            return;
        }
        if let Err(e) = DeciderSource.register_all() {
            crate::plugin_warn!("actor", "判定者行登记失败: {e}");
        }
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new(PLUGIN_ID_ACTOR, SEGMENT_TITLE)
            .with_description("执行者登记与内省：登记判定者行，列出全部 Actor 行（只读）。")
            .with_version("0.1.0")
            .with_order(14)
            .with_icon(PLUGIN_ID_ACTOR)
            .with_hidden(true)
    }
}

#[async_trait]
impl Plugin for ActorPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    /// 一条只读内省路由：`list` → 全部 Actor 行（含四字段 + name）。
    ///
    /// [`ActorConfig::enabled`] = false 时不可达（平凡值，J2）：
    /// 关掉内省口不影响 Actor 机制，只影响"这个窗口"。
    async fn route(
        self: Arc<Self>,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let path = ctx.get(PATH).unwrap_or_default();
        let path = path.strip_prefix('/').unwrap_or(&path);

        if !self.config.enabled {
            return Err(PluginError::NotFound(format!(
                "[actor] 内省口已停用（enabled=false），路由 {path} 不可达"
            )));
        }

        match path {
            "list" => {
                let rows: Vec<serde_json::Value> =
                    actor_list().iter().map(|s| s.to_wire()).collect();
                Ok(PluginPayload::new(&json!({ "actors": rows })))
            }
            _ => Err(PluginError::NotFound(format!(
                "[actor] 无路由子命令: {path}"
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

crate::submit_object_creator!(PLUGIN_ID_ACTOR, ActorPlugin::build, dyn Plugin);

#[cfg(test)]
#[path = "plugin.test.rs"]
mod tests;
