//! Classify 插件 —— 对话面的**判决**能力
//!
//! ## 它是什么
//!
//! 「这一轮该直接回答、还是派给工具循环」——**只输出枚举，不产出面向用户的文本**。
//! 判决与措辞分开的理由：编排层要能**执行**判决（收尾 / 进工具循环 / 只说一句），
//! 而执行一段自由文本只能靠字符串匹配（见 `schemas::dialog` 的模块文档）。
//!
//! ## 为什么是一个独立插件（而不是 session 里的一个函数）
//!
//! 三条判据（缺一不可，见 `docs/plan/09-对话面插件拆分实施方案.md` §2.4）：
//!
//! 1. **一个能力一个插件**：插件名 = 能力名（判决）；
//! 2. **可卸载**：没有它，系统**完整运行**——所有输入直接进工具循环（= 卸载前的行为），
//!    走的是装配期（不挂载 ⇒ 路由 `NotFound`），不是运行期读一个 `enabled` 字段；
//! 3. **零相关**：不 import、不持有任何兄弟插件（`plugin-entry-audit` 的 E-007 / E-009）。
//!
//! 「无工具」是**结构保证**：本插件不注册任何 `Capability`，而模型能调用的工具来自
//! `traverse(TRAVERSE_AVAILABLE_TOOLS)` → `CapabilityVisitor::register`——因此工具集里
//! **在结构上不可能**出现本插件。不需要一条 CI 断言来补偿。
//!
//! ## 两条产线（S2）
//!
//! ```text
//! decide(utterance)
//!   ├─ 规则表命中（问候 / 致谢 / 确认 / 空输入）→ Answered{reason}   0 次 LLM 往返
//!   ├─ 未命中 → 快速档分类（一次静默 LLM 往返，四选一）→ Answered / Escalate
//!   └─ 判不出来（无模型服务 / 响应不可解析）→ Escalate{unclassified}（= 今天的行为）
//! ```
//!
//! 规则表由本插件自己的配置开关（`ClassifyConfig::rule_shortcut`）管辖——它是本插件的
//! 内部策略，不该出现在调用方的配置面里（见 `config.rs` 的模块文档）。
//!
//! ## 它不做什么
//!
//! - **不产出面向用户的文本**：措辞归 `compose`；
//! - **不写转写**：转写只有 `session` 一个写入者（ADR-020）；
//! - **不注册工具**：见上「结构保证」；
//! - **不持有会话状态**：入参带 `session_id` 与对话线投影，它每次现算。

use std::sync::Arc;

use crate::symbio_core::schemas::detail::{DetailDefinition, DetailField};
use crate::symbio_core::schemas::dialog::{DecideRequest, Verdict};
use crate::symbio_core::{
    plugin_dir_from_ctx, Plugin, PluginConfigFile, PluginDir, PluginError, PluginInvokeRequest,
    PluginInvokeRequestExt, PluginInvokeResponse, PluginMeta, PluginPayload, PATH,
    PLUGIN_ID_CLASSIFY,
};
use async_trait::async_trait;

use super::config::ClassifyConfig;
use super::decide::decide;
use super::reasons::REASON_UNCLASSIFIED;
use super::rules::classify_by_rule;

/// Classify 插件（无状态、无副作用、不持有任何地址）
///
/// 唯一持有的东西是自己的配置（`<本插件目录>/PLUGIN.yml`）——装配期读一次，
/// 与 `session` / `skill` 同款（插件从**自己的目录**里读自己的配置）。
///
/// 同时持有那份配置的**文档**（[`PluginConfigFile`]）：设置页要能改它，
/// 「可 A/B」才不是一句空话——提示词的措辞直接决定四选一的准确率，调它不该
/// 需要改代码重编译。
pub struct ClassifyPlugin {
    config: ClassifyConfig,
    config_file: PluginConfigFile,
}

impl ClassifyPlugin {
    pub fn new(config: ClassifyConfig, dir: PluginDir) -> Self {
        Self {
            config,
            config_file: PluginConfigFile::new(dir, "意图判决设置", config_definition()),
        }
    }

    /// 静态工厂：从 `PluginInvokeRequest` 构造 Plugin 实例
    ///
    /// 自己的目录由容器经 `PLUGIN_DIR` 告知；配置就存在那里的 `PLUGIN.yml`
    /// （`#[serde(default)]` ⇒ 装配期刚补出身份键、还没有业务键的存量文件也能读）。
    pub fn build(ctx: Arc<dyn PluginInvokeRequest>) -> Arc<dyn Plugin> {
        let dir = plugin_dir_from_ctx(&*ctx, PLUGIN_ID_CLASSIFY);
        let config: ClassifyConfig = match dir.load::<ClassifyConfig>() {
            Ok(Some(c)) => c,
            Ok(None) => ClassifyConfig::default(),
            Err(e) => {
                crate::plugin_warn!("classify", "读取自身配置失败，改用默认值：{e}");
                ClassifyConfig::default()
            }
        };
        Arc::new(ClassifyPlugin::new(config, dir)) as Arc<dyn Plugin>
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new(PLUGIN_ID_CLASSIFY, "意图判决")
            .with_description(
                "判决这一轮该直接回答还是派给工具循环；只输出枚举，不产出面向用户的文本",
            )
            .with_version("0.3.0")
    }

    /// 判决（本插件能力的全部）。
    ///
    /// 顺序是判据的一部分：**先规则、后分类**。反过来的话，规则短路省下的那次
    /// 往返会被分类请求吃掉——「简单问题反而更慢」。
    async fn decide(&self, ctx: &Arc<dyn PluginInvokeRequest>, req: &DecideRequest) -> Verdict {
        let Some(utterance) = req.utterance.as_deref() else {
            // 没有用户发言（后台触发的判定）：无事可判。兜底方向与判不出来一致——
            // `Escalate` = 今天的行为，绝不返回 `Answered`（那会让用户看到沉默）。
            return escalate_unclassified();
        };

        if self.config.rule_shortcut {
            if let Some(reason) = classify_by_rule(utterance) {
                crate::plugin_info!(
                    "classify",
                    "[Classify] 规则短路命中（session={}, reason={}）",
                    req.session_id,
                    reason
                );
                return Verdict::Answered {
                    reason: reason.to_string(),
                };
            }
        }

        match decide(ctx, utterance, &req.context, &self.config).await {
            Some(verdict) => verdict,
            None => escalate_unclassified(),
        }
    }
}

/// 「判不出来」的判决：`Escalate` + 理由码 `unclassified`。
fn escalate_unclassified() -> Verdict {
    Verdict::Escalate {
        reason: REASON_UNCLASSIFIED.to_string(),
    }
}

#[async_trait]
impl Plugin for ClassifyPlugin {
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
            "decide" => {
                // 请求必须能解析：契约的形状由这一行保证，而不是由注释保证。
                let req: DecideRequest = ctx.payload()?;
                Ok(PluginPayload::new(&self.decide(&ctx, &req).await))
            }
            _ => Err(PluginError::NotFound(format!(
                "[classify] 未知子命令: {path}"
            ))),
        }
    }

    /// 不参与**能力**收集：本插件不注册 `Capability`，也没有挂载视图
    /// （它无状态、不持有地址，因此不实现 `VdfsProvider`）。
    ///
    /// ⚠️ 「不注册 `Capability`」与「不调用模型」是**两件事**：本插件确实会发起一次
    /// 静默的模型请求（快速档分类），但那次请求不向模型暴露任何工具，本插件也从不
    /// 出现在模型的工具清单里——工具集来自 `CapabilityVisitor::register`，而这里
    /// 一个也没注册。
    ///
    /// 它参与的是**另一条通道**（`ConfigurableVisitor`）：声明「本插件有一份配置
    /// 文档」，设置页据此列出并指路 `<根>/classify/PLUGIN.yml`。缺了它插件照常工作，
    /// 只是那份配置在界面上改不到。
    async fn traverse(
        self: Arc<Self>,
        _path: String,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        crate::symbio_core::capability_announce_configurable(&ctx, &self.config_file).await;
        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

crate::submit_object_creator!(PLUGIN_ID_CLASSIFY, ClassifyPlugin::build, dyn Plugin);

// ==================== 配置文档（`<根>/classify/PLUGIN.yml`） ====================

/// 判决配置的定义 —— **定义由配置的拥有者产出**（与 `session` 同一条纪律）。
///
/// 默认值一律从 [`ClassifyConfig::default()`] 读出，不写第二份字面量：schema 与 serde
/// 各写一份默认值，就会出现"面板显示的值与实际行为不符"的漂移（`session` 那边为此
/// 吃过一次 `max_tool_rounds` 的亏，见其 `config_definition` 的说明）。
///
/// 两个可空字段没有"默认值"可写——它们的缺省就是**留空**，故给 `placeholder` 说明
/// 留空意味着什么，而不是编一个看起来像默认值的字面量。
fn config_definition() -> DetailDefinition {
    let d = ClassifyConfig::default();
    DetailDefinition::form(
        "意图判决设置",
        vec![
            DetailField::toggle(
                "rule_shortcut",
                "规则短路",
                "问候 / 致谢 / 确认 / 空输入由规则表直接判决，省掉一次分类请求；\
                 关闭后全部落快速档（慢但正确）",
                d.rule_shortcut,
            ),
            DetailField {
                placeholder: Some("留空 = 用会话选定的模型".into()),
                ..DetailField::text(
                    "model",
                    "判决模型",
                    "本插件自己的 provider id（`<根>/model/<id>`）。判决是一次四选一分类，\
                     用最便宜最快的模型即可；温度不必在这里写——它是 provider 条目自己的\
                     参数，换一个条目就是换温度",
                )
            },
            DetailField {
                rows: Some(6),
                full_width: true,
                placeholder: Some("留空 = 用内置的分类提示词".into()),
                ..DetailField {
                    widget: "textarea".to_string(),
                    ..DetailField::text(
                        "system_prompt",
                        "分类提示词",
                        "覆盖内置的分类提示词。必须只让模型输出一个词\
                         （direct / clarify / refuse / work）——词表本身在代码里，\
                         改提示词不会改行为映射",
                    )
                }
            },
        ],
    )
}

#[cfg(test)]
#[path = "plugin.test.rs"]
mod tests;
