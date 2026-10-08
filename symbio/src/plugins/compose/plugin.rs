//! Compose 插件 —— 对话面的**措辞**能力
//!
//! ## 它是什么
//!
//! 「首响 / 答话 / 汇报」——**只输出文本，不做判决**。它执行上游判决
//! （`schemas::dialog::Verdict`）：`Answered` 说一句答话、`Escalate` 说一句首响、
//! `Report` 说一句进度。判决的产出方见该枚举的变体表（`Answered` / `Escalate` 归
//! [`crate::plugins::classify`]，`Report` 由 `session` 自己判出）——本插件两种都执行，
//! 不区分来源。
//!
//! ## 为什么是一个独立插件（而不是 session 里的一个函数）
//!
//! 三条判据（缺一不可，见 `docs/plan/09-对话面插件拆分实施方案.md` §2.4）：
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
//! | `Answered { reason: from_context }` | **生成**（[`compose`]） | 1 次**静默** LLM 往返 |
//! | 其余 `Answered` / `Escalate` | **模板**（[`templates`]） | 0 次 LLM 往返 |
//! | `Report` | **填表**（[`templates::progress_text`]） | 0 次 LLM 往返（事实随 `RunSnapshot` 带来） |
//!
//! 分派顺序是「先生成、生成不了落模板」：`from_context` 在模板表里**没有行**
//! （有一条用例钉着，见 `templates.test.rs`），因此生成失败时它落到**变体兜底**，
//! 而不是落到某句与问题无关的模板。
//!
//! ## 出参是 `String`，**空串 = 没有对话面文本**（契约上的平凡值）
//!
//! 三条产线各自都有兜底 ⇒ 本实现**恒有文本**。空串因此不是本插件的产出形态，
//! 而是契约留给**其它产出方**的形态（网关把外部客户端的 `path` 原样转发给容器，
//! `compose/compose` 可能由仓外程序调用）：调用方（`session`）见到空串即**不写节点**，
//! 与"没有这个插件"走同一条降级路径。
//!
//! ## 本插件**不写转写**
//!
//! 文本由 `session` 落库，转写只有一个写入者（ADR-020）。本插件只返回字符串。

use crate::symbio_core::{
    plugin_dir_from_ctx, Plugin, PluginConfigFile, PluginDir, PluginError, PluginInvokeRequest,
    PluginInvokeRequestExt, PluginInvokeResponse, PluginMeta, PluginPayload, PATH,
    PLUGIN_ID_COMPOSE,
};
use crate::symbio_core::{ComposeRequest, Verdict, REASON_FROM_CONTEXT};
use crate::symbio_core::{DetailDefinition, DetailField};
use async_trait::async_trait;
use std::sync::Arc;

use super::config::ComposeConfig;
use super::templates::{progress_text, template_for};
use super::wording::generate;

/// Compose 插件（无状态、无副作用、不持有任何地址）
///
/// 持有自己的配置（`<本插件目录>/PLUGIN.yml`）：**用哪个模型**、**指令段怎么写**
/// ——两件真的有分歧的事（见 `config.rs` 的模块文档）。它不持有会话状态，也不持有
/// 任何兄弟插件。
pub struct ComposePlugin {
    config: ComposeConfig,
    config_file: PluginConfigFile,
}

impl ComposePlugin {
    /// 工厂方法（满足 `submit_object_creator!` 协议）
    ///
    /// **不读"要不要用本插件"**：那归调用方（`SessionConfig::reply_enabled`）。
    /// 这里读的是本插件**自己**的两件事：模型与指令段。
    pub fn build(ctx: Arc<dyn PluginInvokeRequest>) -> Arc<dyn Plugin> {
        let dir = plugin_dir_from_ctx(&*ctx, PLUGIN_ID_COMPOSE);
        let config: ComposeConfig = match dir.load::<ComposeConfig>() {
            Ok(Some(c)) => c,
            Ok(None) => ComposeConfig::default(),
            Err(e) => {
                crate::plugin_warn!("compose", "读取自身配置失败，改用默认值：{e}");
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

    /// 三条产线的分派：**填表**（`Report`）/ **先生成、生成不了落模板**（其余）。
    ///
    /// 恒有返回：三条产线各自都有兜底（见模块文档）。
    async fn compose(&self, ctx: &Arc<dyn PluginInvokeRequest>, req: &ComposeRequest) -> String {
        // `Report` 走**填表**产线：事实在 `snapshot` 里，措辞是固定的。它与下面的
        // 模板表并列而不混入表——表里每一行都是"与上下文无关的固定措辞"，
        // 而汇报的正文随现状变（见 `templates::progress_text`）。
        if matches!(req.verdict, Verdict::Report) {
            return progress_text(&req.snapshot);
        }
        if requires_generation(&req.verdict) {
            if let Some(text) = generate(ctx, req, &self.config).await {
                return text;
            }
            // 生成失败**不返回空串**：继续往下走模板产线。`from_context` 在模板表里
            // 没有行 ⇒ 落到变体兜底（"好的。"）——一句通用话，但**不是空白**。
        }
        // `template_for` 只对 `Report` 返回 `None`，而它在上面已经分派走了。
        template_for(&req.verdict).unwrap_or_default()
    }
}

/// 这个判决是否需要**组织语言**（而不是查表）。
///
/// 判据只有一个理由码：`from_context` —— "答案已在对话里"，那段文本只能从对话线
/// 组织出来。其余理由码的**内容**是固定的（"不客气""收到""好，我来处理"），
/// 模型只会把它们写长。
///
/// 它是**函数而不是表里的一列**：`from_context` 的存在与否不是措辞的属性，
/// 而是"这条产线要不要花一次 LLM 往返"的编排判断。
fn requires_generation(verdict: &Verdict) -> bool {
    matches!(verdict, Verdict::Answered { reason } if reason == REASON_FROM_CONTEXT)
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
            "wording" => {
                // 请求必须能解析：契约的形状由这一行保证，而不是由注释保证。
                let req: ComposeRequest = ctx.payload()?;
                let text = self.compose(&ctx, &req).await;
                Ok(PluginPayload::new(&text))
            }
            _ => Err(PluginError::NotFound(format!(
                "[compose] 未知子命令: {path}"
            ))),
        }
    }

    /// 不参与**能力**收集：本插件不注册 `Capability`，也没有挂载视图
    /// （它无状态、不持有地址，因此不实现 `VdfsProvider`）。
    ///
    /// 它参与的是**另一条通道**（`ConfigurableVisitor`）：声明「本插件有一份配置
    /// 文档」，设置页据此列出并指路 `<根>/compose/PLUGIN.yml`——「可 A/B」要能操作，
    /// 靠的就是这一条。
    async fn traverse(
        self: Arc<Self>,
        _path: String,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        crate::symbio_core::capability_announce_configurable(&ctx, &self.config_file).await;
        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

crate::submit_object_creator!(PLUGIN_ID_COMPOSE, ComposePlugin::build, dyn Plugin);

// ==================== 配置文档（`<根>/compose/PLUGIN.yml`） ====================

/// 措辞配置的定义 —— **定义由配置的拥有者产出**（与 `classify` / `session` 同一条纪律）。
///
/// 两个可空字段没有"默认值"可写——它们的缺省就是**留空**，故给 `placeholder` 说明
/// 留空意味着什么，而不是编一个看起来像默认值的字面量。
fn config_definition() -> DetailDefinition {
    DetailDefinition::form(
        "对话措辞设置",
        vec![
            DetailField {
                placeholder: Some("留空 = 用会话选定的模型".into()),
                ..DetailField::text(
                    "model",
                    "措辞模型",
                    "本插件自己的 provider id（`<根>/model/<id>`）。答话是用户逐字读的\
                     那段文本，值得用比判决更好的模型；温度不必在这里写——它是 provider\
                     条目自己的参数，换一个条目就是换温度",
                )
            },
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
