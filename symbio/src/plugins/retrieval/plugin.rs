//! retrieval 插件本体 —— `Plugin` 实现 + **检索行登记** + 一条只读召回路由。
//!
//! ## 它登记什么
//!
//! S06 第 3 节的"检索者"一行：
//!
//! ```text
//! ActorSpec { name: "session.retrieval", pattern: Translator,
//!             budget_ms: 500, scope: Root, principal: "" }
//! ```
//!
//! `budget_ms = 500` 来自 S06 §3 的增量清单（"检索者 `{ budget_ms: 500 }`"）。
//! `pattern = Translator` 的语义是"把查询翻译成检索/召回动作"——与 `chat_loop`
//! 的 `Reasoner` 相对。
//!
//! ## 它跑什么
//!
//! `list` 路由：从磁盘派生事实 → 调 `memory.recall` 投影 → 返回召回视图。
//! **一条路由，一个动作**——与 `fact_log` / `projection` 的"恰好一条"同款。
//!
//! ## 平凡值（J2）
//!
//! - `config.enabled = false` ⇒ 不登记检索行、路由不可达；**会话照常**；
//! - `memory.recall` 投影未登记 ⇒ 路由返回 `degraded: true`（投影缺席 = 能力未接入），
//!   而不是报错——这是"平凡值必须可区分"的落点。

use super::derive::{
    derive_facts, read_memory_lines, read_messages, session_ids, sort_inputs, SessionFactsInput,
};
use crate::symbio_core::schemas::detail::{DetailDefinition, DetailField};
use crate::symbio_core::PATH;
use crate::symbio_core::{
    actor_list, plugin_dir_from_ctx, projection_has, projection_run, ActorPattern, ActorScope,
    ActorSource, ActorSpec, Fact, Plugin, PluginConfigFile, PluginDir, PluginError,
    PluginInvokeRequest, PluginInvokeRequestExt, PluginInvokeResponse, PluginMeta, PluginPayload,
    ProjectionInput, PLUGIN_ID_RETRIEVAL,
};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const SEGMENT_TITLE: &str = "检索";

/// 检索者行的名字。**字面量副本**（不 `use` session）——本插件不认识 session。
const ACTOR_NAME_SESSION_RETRIEVAL: &str = "session.retrieval";

/// S06 §3 的检索者预算：500ms（一次召回的分时上限）。
const RETRIEVAL_BUDGET_MS: u64 = 500;

/// 会话插件目录名（= 其 VDFS 挂载点 / 工厂 id）。**字面量而非 import**。
const PLUGIN_ID_SESSION_DIR: &str = "session";

/// 本插件消费的投影名 —— B2 的登记表按名寻址，本插件**不认识**产生它的插件。
const PROJECTION_RECALL: &str = "memory.recall";

/// 本插件自身配置 —— 只有一个开关（与 `fact_log` / `projection` / `actor` 同形）。
#[derive(Debug, Clone, Deserialize)]
pub struct RetrievalConfig {
    /// 是否登记检索行 + 开放召回路由。关掉 = 行不登记、路由不可达（平凡值），
    /// 但**不影响任何机制**——会话照常运行。
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

impl Default for RetrievalConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

fn config_definition() -> DetailDefinition {
    let d = RetrievalConfig::default();
    DetailDefinition::form(
        SEGMENT_TITLE,
        vec![DetailField::toggle(
            "enabled",
            "启用检索者",
            "登记检索 Actor 行，并开放一条只读召回路由（跑 memory.recall 投影）",
            d.enabled,
        )],
    )
}

/// 检索者登记方 —— 本插件是它在 root 层的唯一登记者。
struct RetrievalSource;

impl ActorSource for RetrievalSource {
    fn layer(&self) -> ActorScope {
        // 只装机在 root 树（不在 ASSEMBLY_SUB_AGENT_PLUGINS 里）。
        // `layer` 由装配方赋予，不是自我声明（A5）。
        ActorScope::Root
    }

    fn specs(&self) -> Vec<ActorSpec> {
        vec![ActorSpec::new(
            ACTOR_NAME_SESSION_RETRIEVAL,
            "", // 占位：检索者也是"运行期才知道是谁在问"
            ActorPattern::Translator,
            RETRIEVAL_BUDGET_MS,
            ActorScope::Root,
        )]
    }
}

/// retrieval 插件。
pub struct RetrievalPlugin {
    config: RetrievalConfig,
    config_file: PluginConfigFile,
    /// 事实派生所需的**读取根**（`<homedir>`）——同 `fact_log` 的取法。
    root: PathBuf,
}

impl RetrievalPlugin {
    pub fn build(ctx: Arc<dyn PluginInvokeRequest>) -> Arc<dyn Plugin> {
        let dir = plugin_dir_from_ctx(&*ctx, PLUGIN_ID_RETRIEVAL);
        let config: RetrievalConfig = match dir.load::<RetrievalConfig>() {
            Ok(Some(c)) => c,
            Ok(None) => RetrievalConfig::default(),
            Err(e) => {
                crate::plugin_warn!("retrieval", "读取自身配置失败，改用默认值：{e}");
                RetrievalConfig::default()
            }
        };
        Arc::new(Self::new(dir, config)) as Arc<dyn Plugin>
    }

    pub fn new(dir: PluginDir, config: RetrievalConfig) -> Self {
        // 本插件目录 = `<root>/retrieval`，父目录即系统根。
        let root = dir
            .dir()
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| dir.dir().to_path_buf());
        let plugin = Self {
            config,
            config_file: PluginConfigFile::new(dir, SEGMENT_TITLE, config_definition()),
            root,
        };
        plugin.register_if_enabled();
        plugin
    }

    /// 登记检索行（仅 `enabled` 时）。失败只记日志——表是扩展点。
    fn register_if_enabled(&self) {
        if !self.config.enabled {
            return;
        }
        if let Err(e) = RetrievalSource.register_all() {
            crate::plugin_warn!("retrieval", "检索者行登记失败: {e}");
        }
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new(PLUGIN_ID_RETRIEVAL, SEGMENT_TITLE)
            .with_description("检索者：登记检索 Actor 行，跑 memory.recall 投影做召回（只读）。")
            .with_version("0.1.0")
            .with_order(15)
            .with_icon(PLUGIN_ID_RETRIEVAL)
            .with_hidden(true)
    }

    /// 从磁盘派生全部可召回事实（含 `memory.*` 格子）。
    fn derive_all(&self) -> Vec<Fact> {
        let session_root = self.root.join(PLUGIN_ID_SESSION_DIR);
        let mut ids = session_ids(&session_root);
        ids.sort();

        let owned: Vec<(
            String,
            Vec<crate::symbio_core::schemas::session::chat_message::ChatMessage>,
            Vec<String>,
        )> = ids
            .into_iter()
            .filter_map(|id| {
                let msgs = read_messages(&session_root, &id)?;
                let mem = read_memory_lines(&session_root, &id);
                Some((id, msgs, mem))
            })
            .collect();

        let mut inputs: Vec<SessionFactsInput<'_>> = owned
            .iter()
            .map(|(id, msgs, mem)| SessionFactsInput {
                session_id: id.as_str(),
                session_ordinal: 0,
                messages: msgs.as_slice(),
                memory_lines: mem.as_slice(),
            })
            .collect();
        sort_inputs(&mut inputs);
        derive_facts(&inputs, 0)
    }
}

#[async_trait]
impl Plugin for RetrievalPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    /// **一条只读召回路由** —— `list`：
    /// 派生事实 → 跑 `memory.recall` 投影 → 回 `{ facts, recall, actor }`。
    ///
    /// 为什么把三项放在一起返回：它们正是 B4 想展示的**一条链**
    /// （B1 事实 → B2 投影 → B3 Actor 行），一次调用即可核对三者是否都在跑。
    ///
    /// **平凡值**：投影未登记 ⇒ `degraded: true` 且 `recall` 为 `null`，
    /// **不是错误**（能力未接入 ≠ 出错，与 B2 的"未登记按平凡值处理"一致）。
    async fn route(
        self: Arc<Self>,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let path = ctx.get(PATH).unwrap_or_default();
        let path = path.strip_prefix('/').unwrap_or(&path);

        if !self.config.enabled {
            return Err(PluginError::NotFound(format!(
                "[retrieval] 检索者已停用（enabled=false），路由 {path} 不可达"
            )));
        }

        match path {
            "list" => {
                let facts = self.derive_all();
                // 跑投影（B2）；未登记 ⇒ 平凡值（能力未接入）。
                let (recall, degraded) = if projection_has(PROJECTION_RECALL) {
                    let input = ProjectionInput::new(&facts, 0);
                    match projection_run(PROJECTION_RECALL, &input) {
                        Ok(view) => (serde_json::to_value(&view).ok(), false),
                        // 转到实际错误：登记了却跑不出来——如实报错，不假装平凡
                        Err(e) => {
                            return Err(PluginError::NotFound(format!(
                                "[retrieval] 投影运行失败: {e}"
                            )))
                        }
                    }
                } else {
                    (None, true) // 平凡值：投影未接入 ⇒ 退化，可区分
                };

                // 本行（B3）：检索者行此刻在不在表里。
                let actor = actor_list()
                    .into_iter()
                    .find(|s| s.name == ACTOR_NAME_SESSION_RETRIEVAL)
                    .map(|s| s.to_wire());

                Ok(PluginPayload::new(&json!({
                    "facts": facts,
                    "recall": recall,
                    "degraded": degraded,
                    "actor": actor,
                })))
            }
            _ => Err(PluginError::NotFound(format!(
                "[retrieval] 无路由子命令: {path}"
            ))),
        }
    }

    /// 本插件**没有能力面**：它是纯只读召回口，不贡献工具 / 选项。
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

crate::submit_object_creator!(PLUGIN_ID_RETRIEVAL, RetrievalPlugin::build, dyn Plugin);

#[cfg(test)]
#[path = "plugin.test.rs"]
mod tests;
