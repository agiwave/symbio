//! Compose 插件 —— 对话面的**措辞**能力
//!
//! ## 它是什么
//!
//! 「首响 / 答话 / 汇报」——**只输出文本，不做判决**。它执行上游判决
//! （`Verdict`，`crates/symbio-kernel/src/dialog/`）：`Answered` 说一句答话、`Escalate` 说一句首响、
//! `Report` 说一句进度。判决的产出方见该枚举的变体表（`Answered` / `Escalate` 归
//! [`symbio_plugin_classify`]，`Report` 由 `session` 自己判出）——本插件两种都执行，
//! 不区分来源。
//!
//! ## 为什么是一个独立插件（而不是 session 里的一个函数）
//!
//! 三条判据（缺一不可，见本插件 README 的「契约」节）：
//!
//! 1. **一个能力一个插件**：插件名 = 能力名（措辞）；
//! 2. **可卸载**：没有它，系统**完整运行**——没有对话面产出的文本，而 worker 的正文
//!    照旧（= 卸载前的行为），走的是装配期（不挂载 ⇒ 路由 `NotFound`）；
//! 3. **零相关**：不 import、不持有任何兄弟插件——**包括 `classify`**。两者都由 `session`
//!    在 `ctx` 里喂输入，两条边都从 session 出发，插件之间没有边。
//!
//! 「无工具」是**结构保证**：本插件不注册任何 `Capability`（同 `classify`）。
//!
//! ## 三条产线
//!
//! | 判决 | 产线 | 代价 |
//! |---|---|---|
//! | `Answered { reason: from_context }` | **行债**（[`crate::debt`]） | 1 次**静默** LLM 往返（派发与三格归环） |
//! | 其余 `Answered` / `Escalate` | **模板**（[`compose`]） | 0 次 LLM 往返 |
//! | `Report` | **填表**（[`compose`]） | 0 次 LLM 往返（事实随 `RunSnapshot` 带来） |
//!
//! 生成**不在路由里**：`from_context` 由本插件申报的 `model.assist` 口债驱动，
//! 回执由 `session` 读走并落终答（D-020）。仓外调用方直呼 `compose/wording` 时
//! `from_context` 落到**变体兜底**（`from_context` 在模板表里没有行，见 `templates.test.rs`），
//! 不会在那里再打一次模型——那正是这条债要收掉的账。
//!
//! ## 出参是 `String`，**空串 = 没有对话面文本**（契约上的平凡值）
//!
//! 三条产线各自都有兜底 ⇒ 本实现**恒有文本**。空串因此不是本插件的产出形态，
//! 而是契约留给**其它产出方**的形态（网关把外部客户端的 `path` 原样转发给容器，
//! `compose/wording` 可能由仓外程序调用）：调用方（`session`）见到空串即**不写节点**，
//! 与"没有这个插件"走同一条降级路径。
//!
//! ## 本插件**不写转写**
//!
//! 文本由 `session` 落库，转写只有一个写入者（D-020）。本插件只返回字符串。

use async_trait::async_trait;
use std::sync::Arc;
use symbio_plugin_api::PLUGIN_ID_COMPOSE;
use symbio_plugin_api::{
    plugin_dir_from_ctx, Plugin, PluginConfigFile, PluginDir, PluginError, PluginInvokeRequest,
    PluginInvokeRequestExt, PluginInvokeResponse, PluginMeta, PluginPayload, CAPABILITY_VISITOR,
    PATH,
};
use symbio_plugin_api::{ComposeRequest, Verdict, WordingReply};
use symbio_plugin_api::{DetailDefinition, DetailField};

use super::config::ComposeConfig;
use super::debt::compose_debt_rows;
use super::templates::{progress_text, template_for};

/// Compose 插件（无状态、无副作用、不持有任何地址）
///
/// 持有自己的配置（`<本插件目录>/PLUGIN.yml`）：**指令段怎么写**——一件真有分歧
/// 的事（见 `config.rs` 的模块文档）。它不持有会话状态，也不持有任何兄弟插件。
pub struct ComposePlugin {
    config: ComposeConfig,
    config_file: PluginConfigFile,
}

impl ComposePlugin {
    /// 工厂方法（满足 `submit_object_creator!` 协议）
    ///
    /// **不读"要不要用本插件"**：那归调用方（会话侧的措辞策略）。
    /// 这里读的是本插件**自己**的两件事：模型与指令段。
    pub fn build(ctx: Arc<dyn PluginInvokeRequest>) -> Arc<dyn Plugin> {
        let dir = plugin_dir_from_ctx(&*ctx, PLUGIN_ID_COMPOSE);
        let config: ComposeConfig = match dir.load::<ComposeConfig>() {
            Ok(Some(c)) => c,
            Ok(None) => ComposeConfig::default(),
            Err(e) => {
                symbio_plugin_api::plugin_warn!("compose", "读取自身配置失败，改用默认值：{e}");
                ComposeConfig::default()
            }
        };
        Arc::new(ComposePlugin::new(config, dir)) as Arc<dyn Plugin>
    }

    pub fn new(config: ComposeConfig, dir: PluginDir) -> Self {
        Self {
            config,
            config_file: PluginConfigFile::new(dir, "对话措辞设置", config_definition()),
        }
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new(PLUGIN_ID_COMPOSE, "对话措辞")
            .with_description("首响 / 答话 / 汇报的措辞；只输出文本，不做判决")
            .with_version("0.4.0")
    }

    /// 路由侧的产线：**填表**（`Report`）/ **模板**（其余有固定措辞的判决）。
    ///
    /// 生成产线不在这里（见模块文档）：它由 [`crate::debt`] 申报的 `model.assist`
    /// 口债驱动，因此本函数全同步、不接触 provider。
    /// 思考事实只有生成产线有（模板零模型往返 ⇒ `None`）。
    fn compose(&self, req: &ComposeRequest) -> WordingReply {
        // `Report` 走**填表**产线：事实在 `snapshot` 里，措辞是固定的。它与下面的
        // 模板表并列而不混入表——表里每一行都是"与上下文无关的固定措辞"，
        // 而汇报的正文随现状变（见 `templates::progress_text`）。
        if matches!(req.verdict, Verdict::Report) {
            return WordingReply {
                text: progress_text(&req.snapshot),
                reasoning: None,
                reasoning_id: None,
                cost_ms: 0,
            };
        }
        // `template_for` 只对 `Report` 返回 `None`，而它在上面已经分派走了。
        // 对 `from_context`（模板表里没有行）落到**变体兜底**，不是空串。
        WordingReply {
            text: template_for(&req.verdict).unwrap_or_default(),
            reasoning: None,
            reasoning_id: None,
            cost_ms: 0,
        }
    }
}

#[async_trait]
impl Plugin for ComposePlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    async fn route(
        self: Arc<Self>,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let path = ctx.get(PATH).unwrap_or_default();
        let path = path.strip_prefix('/').unwrap_or(&path);

        match path {
            // 按上游判决组织一段面向用户的文本。
            "wording" => {
                // 请求必须能解析：契约的形状由这一行保证，而不是由注释保证。
                let req: ComposeRequest = ctx.payload()?;
                let reply = self.compose(&req);
                Ok(PluginPayload::new(&reply))
            }
            _ => Err(PluginError::NotFound(format!(
                "[compose] 未知子命令: {path}"
            ))),
        }
    }

    /// 不参与**能力**收集：本插件不注册 `Capability`，也没有挂载视图
    /// （它无状态、不持有地址，因此不实现 `VdfsProvider`）。
    ///
    /// 它参与的是**另一条通道**（`CONFIGURABLE_VISITOR`）：声明「本插件有一份配置
    /// 文档」，设置页据此列出并指路 `<根>/compose/PLUGIN.yml`——「可 A/B」要能操作，
    /// 靠的就是这一条。
    async fn traverse(
        self: Arc<Self>,
        _path: String,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        // 本轮配置快照贡献（D-427）：真源是本插件自己的 PLUGIN.yml，值由本插件交。
        // 挂载即贡献，卸载即缺席；会话不再按本插件的名字去债表算「有效开关」。
        symbio_plugin_api::capability_announce_config_snapshot(
            &ctx,
            "compose_enabled",
            serde_json::json!(self.config.compose_enabled),
        )
        .await;
        symbio_plugin_api::capability_announce_configurable(&ctx, &self.config_file).await;
        // 措辞生成的行债（`compose-wording-model`）：装载期只挂「这一族归谁申报」，
        // 每轮 `rows()` 现算是否欠一次 `model.assist` 往返（分类判决与当轮配置为准）。
        if let Some(visitor) = ctx.get(CAPABILITY_VISITOR) {
            for row in compose_debt_rows(&self.config) {
                visitor.put(row).await;
            }
        }
        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

symbio_plugin_api::submit_object_creator!(PLUGIN_ID_COMPOSE, ComposePlugin::build, dyn Plugin);

// ==================== 配置文档（`<根>/compose/PLUGIN.yml`） ====================

/// 措辞配置的定义 —— **定义由配置的拥有者产出**（与 `classify` / `session` 同一条纪律）。
///
/// 可空字段没有"默认值"可写——它的缺省就是**留空**，故给 `placeholder` 说明
/// 留空意味着什么，而不是编一个看起来像默认值的字面量。
fn config_definition() -> DetailDefinition {
    let d = ComposeConfig::default();
    DetailDefinition::form(
        "对话措辞设置",
        vec![
            DetailField::toggle(
                "compose_enabled",
                "启用对话措辞",
                "关掉后首响 / 答话都不生成；卸载本插件同样让这一格缺席，\
                 读方按出厂值收敛（D-427）",
                d.compose_enabled,
            ),
            DetailField {
                rows: Some(6),
                full_width: true,
                placeholder: Some("留空 = 用内置的指令段".into()),
                ..DetailField {
                    widget: "textarea".to_string(),
                    ..DetailField::text(
                        "instruction",
                        "生成指令段",
                        "覆盖内置的生成指令段（拼在人格与记忆之后）。只覆盖指令段，\
                     不覆盖人格与记忆——同一个人在两处口吻不同，比一句措辞不理想糟得多",
                    )
                }
            },
        ],
    )
}

#[cfg(test)]
#[path = "plugin.test.rs"]
mod tests;
