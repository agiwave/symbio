//! `session/grant` —— 审批卡第三档「永久放行」的唯一写入口（D-170）。
//!
//! ## 为什么是控制路由，不是第二个数据面
//!
//! 它改的是本插件的**全局配置文档**（`<homedir>/session/PLUGIN.yml` 的
//! `approval_grants`），而配置文档的写路径只有 `PluginConfigFile::apply`
//! （校验 → 落内存 → 落盘 → 广播）。在路由上另写一遍「读文件、改一行、写文件」
//! 会让校验与广播各漏一半；本文件只负责**把这一格抬上去**，落盘交给唯一路径。
//!
//! ## 只抬不降
//!
//! 与「本会话放行」同纪律：一次批准不该顺手收紧已有的授权。撤销走配置文档本身
//! （用户在设置页删掉一行），因此没有第二套「撤销动作」。
//!
//! ## 入参
//!
//! `{ tool, risk }`：`tool` 是**能力名**（canonical name），`risk` 是这次调用的
//! 风险档（`low` / `medium` / `high`）。审批卡的 `prompt.tool_name` 就是能力名
//! （`approval_gate` 从 `CapabilityMeta.name` 产出），前端原样回传即可。

use super::plugin::SessionPlugin;
use crate::symbio_core::{
    CapabilityRiskLevel, PluginError, PluginInvokeRequest, PluginInvokeRequestExt,
    PluginInvokeResponse, PluginPayload, VdfsContent,
};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Clone, Deserialize)]
struct GrantRequest {
    tool: String,
    risk: String,
}

impl SessionPlugin {
    pub(crate) async fn handle_grant(
        &self,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let req: GrantRequest = ctx.payload()?;
        let tool = req.tool.trim();
        if tool.is_empty() {
            return Err(PluginError::ValidationError(
                "session/grant 需要 tool（能力名）".into(),
            ));
        }
        // 写清单是**严格**解析：写错一个词不能静默折成 Medium 后写进「永久放行」。
        let risk = CapabilityRiskLevel::parse(req.risk.trim())
            .ok_or_else(|| PluginError::ValidationError(format!("未知风险档：{}", req.risk)))?;

        // 读-改-写串行：两张审批卡同时永久放行时，后写的不能丢掉先写的那一格。
        let _serial = self.grant_lock.lock().await;
        let mut next = self.config.read().await.clone();
        next.approval_grants.raise(tool, risk);
        let text = serde_json::to_string(&next)
            .map_err(|e| PluginError::InternalError(format!("会话配置序列化失败：{e}")))?;
        // 唯一写路径：校验 → 落内存 → 落盘 → 广播。
        self.config_file
            .apply(&self.config, &VdfsContent::text(text))
            .await?;

        Ok(PluginPayload::new(
            &self.config.read().await.approval_grants,
        ))
    }
}

#[cfg(test)]
#[path = "grants.test.rs"]
mod tests;
