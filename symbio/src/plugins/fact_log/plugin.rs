//! fact_log 插件本体 —— `Plugin` 实现 + **事实源登记**。
//!
//! 本次交付只做**一件事**：在 `traverse` 里把自己的事实源登记进访问者
//! （`CapabilityVisitor::register_fact_source`）。消费方（未来的检索者 / 审计者）
//! 经 `list_fact_sources` 取用——**谁也不必认识谁**。
//!
//! ## 为什么不注入系统提示词、不给 LLM 工具
//!
//! 本插件当前的价值在**机制面**（让事实可被程序消费），不在**模型面**。
//! 按"一个功能一个插件且可选"的原则，先只交付它**唯一真正需要的**那一样东西。
//! 需要让模型查询事实时，再单独加工具（那是一次加法，不改本插件的机制）。
//!
//! ## 平凡值（J2）
//!
//! `config.enabled = false` 时**不登记事实源**。消费方取不到 → 按"无事实"处理，
//! 系统其余部分完好。这让"停掉事实日志"是一个无损操作。

use super::config::FactLogConfig;
use super::derive::{derive_facts, sort_inputs, SessionFactsInput};
use crate::symbio_core::schemas::detail::{DetailDefinition, DetailField};
use crate::symbio_core::schemas::session::chat_message::ChatMessage;
use crate::symbio_core::{
    capability_announce_configurable, plugin_dir_from_ctx, Fact, FactError, FactSource, Plugin,
    PluginConfigFile, PluginDir, PluginError, PluginInvokeRequest, PluginInvokeRequestExt,
    PluginInvokeResponse, PluginMeta, PluginPayload, CAPABILITY_VISITOR, FACT_NONE_SEQ, PATH,
    PLUGIN_ID_FACT_LOG, TRAVERSE_AVAILABLE_TOOLS,
};
use async_trait::async_trait;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;

const SEGMENT_TITLE: &str = "事实日志";

/// 配置表单定义 —— 从 [`FactLogConfig::default`] 读出默认值，不写第二份字面量。
fn config_definition() -> DetailDefinition {
    let d = FactLogConfig::default();
    DetailDefinition::form(
        SEGMENT_TITLE,
        vec![
            DetailField::toggle(
                "enabled",
                "启用事实日志",
                "把各域的可观测事实聚合为一条只读序列，供新能力查询",
                d.enabled,
            ),
            DetailField::number(
                "max_facts",
                "单次查询上限（条）",
                "一次派生最多返回的事实条数，超出部分截断",
                1.0,
                1_048_576.0,
                json!(d.max_facts),
            ),
        ],
    )
}

/// 事实日志插件。
#[derive(Clone)]
pub struct FactLogPlugin {
    config: Arc<RwLock<FactLogConfig>>,
    config_file: PluginConfigFile,
    /// 事实派生所需的**读取根** —— 系统根（`<homedir>`）。
    ///
    /// 为什么是系统根而不是本插件目录：事实来自会话（`<homedir>/session/...`），
    /// 那是**会话插件的目录**。本插件只读磁盘上的公开布局，因此拿到的必须是根。
    /// `plugin_dir_from_ctx` 给出的是本插件自己的目录（`<homedir>/fact_log`），
    /// 取它的父目录即根——这是一条**纯路径运算**，不依赖任何插件。
    root: PathBuf,
}

impl FactLogPlugin {
    /// 静态工厂。
    pub fn build(ctx: Arc<dyn PluginInvokeRequest>) -> Arc<dyn Plugin> {
        let dir = plugin_dir_from_ctx(&*ctx, PLUGIN_ID_FACT_LOG);
        let config: FactLogConfig = match dir.load::<FactLogConfig>() {
            Ok(Some(c)) => c,
            Ok(None) => FactLogConfig::default(),
            Err(e) => {
                crate::plugin_warn!("fact_log", "读取自身配置失败，改用默认值：{e}");
                FactLogConfig::default()
            }
        };
        Arc::new(Self::new(dir, config)) as Arc<dyn Plugin>
    }

    pub fn new(dir: PluginDir, config: FactLogConfig) -> Self {
        // 本插件目录 = `<root>/fact_log`，其父目录即系统根（见字段文档）。
        let root = dir
            .dir()
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| dir.dir().to_path_buf());
        Self {
            config: Arc::new(RwLock::new(config)),
            config_file: PluginConfigFile::new(dir, SEGMENT_TITLE, config_definition()),
            root,
        }
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new(PLUGIN_ID_FACT_LOG, SEGMENT_TITLE)
            .with_description("事实日志：把各域的可观测事实聚合为一条只读序列。")
            .with_version("0.1.0")
            .with_order(12)
            .with_icon(PLUGIN_ID_FACT_LOG)
            // 隐藏：它没有自己的资源树（只读事实，不落文件），故有挂载点但不显示
            .with_hidden(true)
    }

    /// 读一个会话目录的消息列表。
    ///
    /// 布局是**公开约定**（`docs/design/vdfs.md`）：`<session_root>/<id>/messages.json`，
    /// 内容是 `{"messages": [ChatMessage, …]}`——**带一层包装**（大字段与元数据分行，
    /// 见 `session::store` 的 `MessagesFile`）。本函数只依赖这份约定，**不 import
    /// `session`**：包装结构在此**就地重声明**（一个只读 `messages` 键），
    /// 而不是引用别人的类型——插件独立原则。
    fn read_messages(session_root: &Path, session_id: &str) -> Option<Vec<ChatMessage>> {
        let path = session_root.join(session_id).join("messages.json");
        let text = std::fs::read_to_string(path).ok()?;
        let file: MessagesFile = serde_json::from_str(&text).ok()?;
        Some(file.messages)
    }

    /// 枚举会话根下的全部会话 id（一层目录）。
    ///
    /// 只看目录名，不解析内容——**确定性**来自后续的字典序排序，不是枚举顺序。
    fn session_ids(session_root: &Path) -> Vec<String> {
        let mut ids = Vec::new();
        let Ok(entries) = std::fs::read_dir(session_root) else {
            return ids;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                // 会话存储用 `messages.json` 作为存在判据（与 store 布局一致）
                if path.join("messages.json").is_file() {
                    ids.push(name.to_string());
                }
            }
        }
        ids
    }

    /// 全量派生（本阶只支持全量；`from` 被忽略但语义正确——返回的仍是全集）。
    fn derive_all(&self, config: &FactLogConfig) -> Vec<Fact> {
        let session_root = self.root.join(PLUGIN_ID_SESSION_DIR);
        let mut ids = Self::session_ids(&session_root);
        ids.sort(); // 确定性：不依赖目录枚举顺序

        // 先把消息读出来（生命周期：inputs 借用 owned 的 messages）
        let owned: Vec<(String, Vec<ChatMessage>)> = ids
            .into_iter()
            .filter_map(|id| Self::read_messages(&session_root, &id).map(|msgs| (id, msgs)))
            .collect();

        let mut inputs: Vec<SessionFactsInput<'_>> = owned
            .iter()
            .map(|(id, msgs)| SessionFactsInput {
                session_id: id.as_str(),
                session_ordinal: 0, // 由 sort_inputs 就地分配
                messages: msgs.as_slice(),
            })
            .collect();
        sort_inputs(&mut inputs);
        derive_facts(&inputs, config.effective_max_facts())
    }
}

/// 会话插件目录名（= 其 VDFS 挂载点 / 工厂 id）。**字面量而非 import**：
/// 本插件不 import `session`，只依赖"会话数据住在这个目录下"这条公开约定。
const PLUGIN_ID_SESSION_DIR: &str = "session";

/// `messages.json` 的落盘形状 —— 只声明本插件要读的那一个键。
///
/// `session::store` 里的同名结构是它的**拥有者**；本插件**就地重声明**（而非 import）
/// 是插件独立原则的体现：只依赖「这个键叫什么」这条公开约定。
/// `#[serde(default)]` 让空文件 / 缺键退化为空列表而非解析失败。
#[derive(serde::Deserialize)]
struct MessagesFile {
    #[serde(default)]
    messages: Vec<ChatMessage>,
}

#[async_trait]
impl Plugin for FactLogPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    /// 分形路由入口：**一条只读内省路由** —— `list` 返回当前派生出的事实序列。
    ///
    /// 为什么只给这一条：本插件的机制面是「事实源登记」（`traverse`），那对模型不可见。
    /// 但**审计者 / 测试 / 调试者**需要一个能看见派生结果的窗口，否则「事实日志在跑」
    /// 与「事实日志没在跑」在系统外部无法区分。因此给**恰好一条**只读路由——多一条都
    /// 是范围外的机制。它不是 LLM 工具（不进 `available_tools`），只是 HTTP/VDFS 边界
    /// 上的内省口。
    async fn route(
        self: Arc<Self>,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let path = ctx.get(PATH).unwrap_or_default();
        let path = path.strip_prefix('/').unwrap_or(&path);

        match path {
            "list" => {
                let config = self.config.read().await;
                let facts = self.derive_all(&config);
                Ok(PluginPayload::new(&facts))
            }
            _ => Err(PluginError::NotFound(format!(
                "[fact_log] 无路由子命令: {path}"
            ))),
        }
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let sub = ctx.get(PATH).unwrap_or_default();
        if sub != TRAVERSE_AVAILABLE_TOOLS {
            return Err(PluginError::NotFound(format!("未知遍历路径: {sub}")));
        }

        // ① 事实源登记（仅当启用）——平凡值：不启用则什么都不做
        let enabled = self.config.read().await.enabled;
        if let Some(visitor) = ctx.get(CAPABILITY_VISITOR) {
            if enabled {
                let me: Arc<dyn FactSource> = self.clone();
                visitor.register_fact_source(PLUGIN_ID_FACT_LOG, me).await;
            }
        }

        // ② 声明本插件有配置文档（设置页据此列出）
        capability_announce_configurable(&ctx, &self.config_file).await;

        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

/// [`FactSource`] 实现 —— 事实日志的**唯一出口**。
///
/// `from` 语义：本阶只支持全量派生（`from` 被忽略）。返回的 `seq` 仍是全集且严格
/// 递增，因此调用方按 `seq >= from` 过滤即可得到增量——**增量是调用方的过滤，
/// 不是实现方的职责**（这符合 `FactSource` 的契约：增量是优化，不是契约）。
#[async_trait]
impl FactSource for FactLogPlugin {
    async fn facts(&self, _from: u64) -> Result<Vec<Fact>, FactError> {
        let config = self.config.read().await;
        Ok(self.derive_all(&config))
    }

    fn head(&self) -> u64 {
        // `head` 是同步方法，不能 await 配置锁；用磁盘当前状态估算上界。
        // 语义：返回"已知的最大 seq"（0 = 空）。全量派生后再取最大即可，
        // 但为避免在同步上下文里做重活，这里直接返回 FACT_NONE_SEQ（0=未知/空），
        // 由调用方以 `facts()` 的实际结果为准——契约允许 head 保守。
        FACT_NONE_SEQ
    }
}

crate::submit_object_creator!(PLUGIN_ID_FACT_LOG, FactLogPlugin::build, dyn Plugin);

#[cfg(test)]
#[path = "plugin.test.rs"]
mod tests;
