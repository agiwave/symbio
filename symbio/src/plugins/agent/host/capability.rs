//! 把 OAB bundle 贡献的能力包装为 symbio `Capability`。
//!
//! ## 唯一的能力：身份全文取回
//!
//! bundle 对模型的贡献只有两类（规范 §5）：
//!
//! - **提示词**：`prompts/` + `skills/` 装配出的人格；
//! - **工具**：来自 `mcps/` 的 MCP server，由**宿主已有的 MCP 客户端**启动并注册，
//!   本插件不包壳、不实现任何 OAB 专有执行器。
//!
//! 人格本体走**系统提示词片段**通道（见 [`super::prompt`]）——每轮随提示词注入，
//! 模型一开始就知道自己是谁。本模块的 [`BundleIdentityCapability`] 是它的**配套**：
//! 片段只带注入预算内的部分，超出的正文由 `agent_identity` 工具取回。
//!
//! ## 为什么两个通道都要
//!
//! 只要工具：模型得先「决定去调用它」才看得到人格，人格就不是「我是谁」而是
//! 「一个可调用的东西」。
//! 只要片段：注入预算一旦被截断，模型就再也拿不到剩下的部分——人格缺一块且无声。
//!
//! 片段保证**在场**，工具保证**完整**。

use crate::symbio_core::{
    Capability, CapabilityCategory, CapabilityMeta, InvokeRequest, InvokeResponse, PluginPayload,
};
use std::sync::Arc;

/// bundle 身份能力：把装配出的提示词片段挂成一个 `agent_identity` 工具。
pub struct BundleIdentityCapability {
    /// 按 priority 升序拼接的完整提示词文本
    identity_text: String,
    /// 来源 bundle（日志与展示）
    bundle_id: String,
}

impl BundleIdentityCapability {
    pub fn new(identity_text: String, bundle_id: String) -> Arc<Self> {
        Arc::new(Self {
            identity_text,
            bundle_id,
        })
    }
}

#[async_trait::async_trait]
impl Capability for BundleIdentityCapability {
    fn meta(&self) -> CapabilityMeta {
        // 描述截断：身份锚定靠第一段（priority 最小），全文经工具调用取回
        let brief: String = self.identity_text.chars().take(160).collect();
        CapabilityMeta {
            name: "agent_identity".to_string(),
            context_retention: None,
            description: format!(
                "[bundle:{}] 当前智能体的身份与人格（调用可取回全文）：{brief}",
                self.bundle_id
            ),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
            }),
            keywords: Vec::new(),
            category: Some(CapabilityCategory::Skill),
            examples: None,
        }
    }

    async fn execute(&self, _ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload> {
        Ok(PluginPayload::new(&serde_json::json!({
            "content": self.identity_text,
        })))
    }
}
