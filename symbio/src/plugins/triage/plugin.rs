//! Triage 插件 —— 对话面的**判决**能力
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
//! ## 本批（S1）的形态：骨架 + 契约，恒 `Escalate`
//!
//! 本批结束时**行为必须与今天逐字一致**：`decide` 一律返回 `Escalate`，即
//! 「全部输入进工具循环」——那正是今天的行为。规则短路与快速档分类在下一批
//! （S2）替换这个平凡实现，本批只证明插件边界真实存在。

use crate::symbio_core::schemas::dialog::{DecideRequest, Verdict};
use crate::symbio_core::{
    Plugin, PluginError, PluginInvokeRequest, PluginInvokeRequestExt, PluginInvokeResponse,
    PluginMeta, PluginPayload, PATH, PLUGIN_ID_TRIAGE,
};
use async_trait::async_trait;
use std::sync::Arc;

/// 平凡实现的理由码：本插件尚未接入判决逻辑（S1 的骨架形态）
///
/// 是**数据**（理由码），不是机制——S2 起它被规则表 / 快速档分类给出的码取代。
const REASON_UNWIRED: &str = "unwired";

/// Triage 插件（无状态、无副作用、不持有任何地址）
pub struct TriagePlugin;

impl TriagePlugin {
    /// 工厂方法（满足 `submit_object_creator!` 协议）
    pub fn build(_ctx: Arc<dyn PluginInvokeRequest>) -> Arc<dyn Plugin> {
        Arc::new(TriagePlugin) as Arc<dyn Plugin>
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new(PLUGIN_ID_TRIAGE, "意图判决")
            .with_description(
                "判决这一轮该直接回答还是派给工具循环；只输出枚举，不产出面向用户的文本",
            )
            .with_version("0.1.0")
    }
}

#[async_trait]
impl Plugin for TriagePlugin {
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
                // 本批**不使用**请求内容（平凡实现），但解析本身是可往返的证明。
                let _req: DecideRequest = ctx.payload()?;
                Ok(PluginPayload::new(&Verdict::Escalate {
                    reason: REASON_UNWIRED.to_string(),
                }))
            }
            _ => Err(PluginError::NotFound(format!(
                "[triage] 未知子命令: {path}"
            ))),
        }
    }

    /// 不参与任何收集：本插件不注册 `Capability`，也没有挂载视图
    /// （它无状态、不持有地址，因此不实现 `VdfsProvider`）。
    async fn traverse(
        self: Arc<Self>,
        _path: String,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

crate::submit_object_creator!(PLUGIN_ID_TRIAGE, TriagePlugin::build, dyn Plugin);

#[cfg(test)]
#[path = "plugin.test.rs"]
mod tests;
