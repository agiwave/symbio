//! 把 OAB bundle 贡献的能力包装为 symbio `Capability`。
//!
//! ## 唯一的能力：身份/提示词锚定
//!
//! bundle 对模型的贡献只有两类（规范 §5）：
//!
//! - **提示词**：`prompts/` + `skills/` 装配出的片段 → 汇总注册为 **一个**
//!   [`BundleIdentityCapability`]（`agent_identity` 工具）；
//! - **工具**：来自 `mcps/` 的 MCP server，由**宿主已有的 MCP 客户端**启动并注册，
//!   本插件不包壳、不实现任何 OAB 专有执行器（早期草案的 `oab.echo` 已移除）。
//!
//! ## 为什么人格是工具而不是系统提示词通道
//!
//! symbio 的会话编排（session orchestrator）只组装与智能体无关的基础提示词，
//! 人格由「agent_identity 工具说明」承载（老 agent 插件同款语义）。bundle 的
//! 提示词片段经装配汇总后在此注册为**一个**身份工具：模型在每轮工具列表里看到
//! 它的描述（身份锚定），调用即取回全文。

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
