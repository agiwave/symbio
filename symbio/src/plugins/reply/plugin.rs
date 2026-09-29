//! Reply 插件 —— 对话面的**措辞**能力
//!
//! ## 它是什么
//!
//! 「首响 / 答话 / 汇报」——**只输出文本，不做判决**。它执行上游判决
//! （`schemas::dialog::Verdict`）：`Answered` 说一句答话、`Escalate` 说一句首响、
//! `Report` 说一句进度。判决归 [`crate::plugins::triage`]。
//!
//! ## 为什么是一个独立插件（而不是 session 里的一个函数）
//!
//! 三条判据（缺一不可，见 `docs/plan/09-对话面插件拆分实施方案.md` §2.4）：
//!
//! 1. **一个能力一个插件**：插件名 = 能力名（措辞）；
//! 2. **可卸载**：没有它，系统**完整运行**——没有对话面产出的文本，而 worker 的正文
//!    照旧（= 卸载前的行为），走的是装配期（不挂载 ⇒ 路由 `NotFound`）；
//! 3. **零相关**：不 import、不持有任何兄弟插件——**包括 `triage`**。两者都由 `session`
//!    在 `ctx` 里喂输入，两条边都从 session 出发，插件之间没有边。
//!
//! 「无工具」是**结构保证**：本插件不注册任何 `Capability`（同 `triage`）。
//!
//! ## 本批（S1）的形态：骨架 + 契约，恒空文本
//!
//! 本批结束时**行为必须与今天逐字一致**：`compose` 一律返回空串，即
//! 「没有对话面文本」——那正是今天的行为。模板表与生成在下一批（S3）替换这个
//! 平凡实现，本批只证明插件边界真实存在。

use crate::symbio_core::schemas::dialog::ComposeRequest;
use crate::symbio_core::{
    Plugin, PluginError, PluginInvokeRequest, PluginInvokeRequestExt, PluginInvokeResponse,
    PluginMeta, PluginPayload, PATH, PLUGIN_ID_REPLY,
};
use async_trait::async_trait;
use std::sync::Arc;

/// Reply 插件（无状态、无副作用、不持有任何地址）
pub struct ReplyPlugin;

impl ReplyPlugin {
    /// 工厂方法（满足 `submit_object_creator!` 协议）
    pub fn build(_ctx: Arc<dyn PluginInvokeRequest>) -> Arc<dyn Plugin> {
        Arc::new(ReplyPlugin) as Arc<dyn Plugin>
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new(PLUGIN_ID_REPLY, "对话措辞")
            .with_description("首响 / 答话 / 汇报的措辞；只输出文本，不做判决")
            .with_version("0.1.0")
    }
}

#[async_trait]
impl Plugin for ReplyPlugin {
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
            "compose" => {
                // 请求必须能解析：契约的形状由这一行保证，而不是由注释保证。
                // 本批**不使用**请求内容（平凡实现），但解析本身是可往返的证明。
                let _req: ComposeRequest = ctx.payload()?;
                Ok(PluginPayload::new(&String::new()))
            }
            _ => Err(PluginError::NotFound(format!("[reply] 未知子命令: {path}"))),
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

crate::submit_object_creator!(PLUGIN_ID_REPLY, ReplyPlugin::build, dyn Plugin);

#[cfg(test)]
#[path = "plugin.test.rs"]
mod tests;
