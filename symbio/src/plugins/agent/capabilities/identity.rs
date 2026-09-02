//! `agent_identity` 身份能力——人格以"工具说明"形式送达 LLM
//!
//! ## 重构背景
//!
//! 重构前，agent 插件在 `agent/chat` 路由里把身份 / 规则 / 策略 / 预算状态整段
//! 塞进 `model_chat::Request.system_prompt`。这意味着：
//! - agent 插件独占 chat 路由（特权节点），session 必须选出一个 agent 才能对话；
//! - 人格内容走了与其它插件完全不同的注入通道。
//!
//! 重构后，**会话编排归 session，agent 与其它插件一样只通过 `traverse` 贡献工具**。
//! 人格因此改由本能力的 `description` 承载：
//! - 工具定义每轮请求都会随 `tools` 一并送达 LLM ⇒ 人格与系统提示词等价地"始终可见"；
//! - 被预算截断的认知、或需要按主题深挖时，LLM 调用本工具取回（`execute`）。
//!
//! ## 与 `agent_cognition` 的分工
//!
//! - 本工具：**读人格**（我是谁 / 我遵循什么 / 我擅长什么）
//! - `agent_cognition`：**读写认知**（记忆存取、推理、反思、整理）
//!
//! ## 重要行为变化
//!
//! 每轮自动注入的 `<active_memory>`（语义记忆片段）已移除。工作记忆不再自动灌入
//! 上下文，LLM 需主动调用 `agent_cognition` 的 `memory.retrieve` 回忆——
//! 这正是"自我进化"的方向：由 LLM 自主决定何时回忆、何时固化，
//! 而不是由系统在每轮替它做主。本工具说明中会明确提示这一约定。

use crate::plugins::agent::core::{AgentStore, FilterExpr, PageRequest, PromptBudget};
use crate::plugins::agent::handlers::system_prompt;
use crate::plugins::agent::plugin::AgentPlugin;
use crate::symbio_core::CAPABILITY_AGENT_IDENTITY;
use crate::symbio_core::{
    Capability, CapabilityMeta, InvokeRequest, InvokeRequestExt, InvokeResponse, PluginError,
    PluginPayload,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

/// 语义检索默认条数
const DEFAULT_RETRIEVE_LIMIT: usize = 8;
/// `execute` 无 query 时重渲染人格所用的预算放大倍数。
///
/// 说明里那份人格受 `prompt_budget_tokens` 约束（默认 3500）；调用本工具说明
/// "我要看全"，就给一个更宽松的预算，让被截断的部分浮出来。
const FULL_BUDGET_MULTIPLIER: usize = 4;

pub(crate) struct AgentIdentityTool {
    plugin: Arc<AgentPlugin>,
    agent_id: String,
    /// `traverse` 阶段预渲染的人格文本（受预算约束）
    persona: String,
}

impl AgentIdentityTool {
    pub fn new(plugin: Arc<AgentPlugin>, agent_id: String, persona: String) -> Self {
        Self {
            plugin,
            agent_id,
            persona,
        }
    }

    /// 从 ctx 解析 workdir（供 mindscape 解析用）
    fn workdir_of(ctx: &Arc<dyn InvokeRequest>) -> Option<String> {
        ctx.get(crate::symbio_core::WORKDIR).filter(|w| !w.is_empty())
    }

    async fn mindscape(
        &self,
        workdir: Option<String>,
    ) -> Result<Arc<dyn AgentStore>, PluginError> {
        self.plugin
            .get_mindscape(workdir.as_deref(), &self.agent_id)
            .await
            .ok_or_else(|| {
                PluginError::NotFound(format!(
                    "Agent '{}' not found (workdir={:?})",
                    self.agent_id, workdir
                ))
            })
    }

    /// 放大预算重渲染人格（让被截断的认知浮出）
    async fn full_persona(&self, mindscape: &dyn AgentStore) -> String {
        let cfg = self.plugin.config.read().await;
        let budget = PromptBudget::new(
            cfg.prompt_budget_tokens.saturating_mul(FULL_BUDGET_MULTIPLIER),
            cfg.prompt_overhead_tokens,
        );
        drop(cfg);

        system_prompt::build_persona(mindscape, &budget, None)
            .await
            .prompt
    }
}

#[derive(Debug, Clone, Deserialize)]
struct IdentityRequest {
    /// 语义检索词：按主题检索认知单元（不受人格预算截断）
    #[serde(default)]
    query: Option<String>,
    /// 检索条数上限
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
struct IdentityResponse {
    agent_id: String,
    /// 放大预算后的人格全文
    persona: String,
    /// `query` 命中时的语义检索结果
    #[serde(skip_serializing_if = "Option::is_none")]
    recalled: Option<Vec<RecalledUnit>>,
}

#[derive(Debug, Clone, Serialize)]
struct RecalledUnit {
    id: String,
    name: Option<String>,
    description: Option<String>,
    content: Option<String>,
    score: Option<f32>,
}

#[async_trait]
impl Capability for AgentIdentityTool {
    fn meta(&self) -> CapabilityMeta {
        let agent_id = &self.agent_id;

        let description = format!(
            "## 智能体身份：`{agent_id}`\n\n\
             以下身份锚定、行为准则与可用策略构成你的认知基线，**随每次请求自动送达**，\n\
             无需调用本工具即已生效。\n\n\
             {persona}\n\n\
             ## 使用约定\n\n\
             - 上面是**预算内摘要**：受提示词预算约束，部分认知可能被截断（见上方预算状态段）。\n\
             - 需要被截断的部分、或要按主题深挖时：调用本工具并传 `query` 做语义检索。\n\
             - **工作记忆不会自动注入**：需要回忆过往认知时，主动调用 `agent_cognition`\n\
               并以 `memory.retrieve` 操作按语义检索（传 `filter:{{\"semantic\":\"...\"}}`）。\n\
             - 学到新知识 / 新规则后，用 `agent_cognition` 的 `memory.save` 固化——\n\
               这是你自我进化的唯一途径；不写下来的经验对你而言不会留存。",
            agent_id = agent_id,
            persona = self.persona,
        );

        CapabilityMeta {
            name: "agent_identity".to_string(),
            description,
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "可选。语义检索词：按该主题检索认知单元，不受人格预算截断。\
                                        例：`用户的编码规范`、`上次那个部署故障`。"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "可选。语义检索返回条数上限，默认 8。"
                    }
                }
            }),
            category: Some(crate::symbio_core::CapabilityCategory::Metacognition),
            examples: Some(vec![
                "query='用户的编码规范'".to_string(),
                "query='上次部署故障的教训'".to_string(),
                "（无参数：取回放大预算后的完整人格）".to_string(),
            ]),
            ..Default::default()
        }
    }

    async fn execute(&self, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload> {
        let req: IdentityRequest = ctx.payload().unwrap_or(IdentityRequest {
            query: None,
            limit: None,
        });

        let workdir = Self::workdir_of(&ctx);
        let mindscape = self.mindscape(workdir).await?;

        let persona = self.full_persona(mindscape.as_ref()).await;

        let recalled = match req.query.as_deref().map(str::trim).filter(|q| !q.is_empty()) {
            None => None,
            Some(query) => {
                let limit = req.limit.unwrap_or(DEFAULT_RETRIEVE_LIMIT);
                let filter = FilterExpr::Semantic {
                    query: query.to_string(),
                    min_score: 0.0,
                };
                match mindscape
                    .query(&filter, &PageRequest::first(limit))
                    .await
                {
                    Ok(page) => {
                        let scores = page.scores.unwrap_or_default();
                        let items = page
                            .items
                            .iter()
                            .enumerate()
                            .map(|(i, cu)| RecalledUnit {
                                id: cu.id().to_string(),
                                name: cu.name().map(|s| s.to_string()),
                                description: cu.description().map(|s| s.to_string()),
                                content: cu
                                    .content()
                                    .map(|s| crate::plugins::agent::core::truncate_chars(s, 400)),
                                score: scores.get(i).copied(),
                            })
                            .collect::<Vec<_>>();
                        // 记录访问（与 chat handler 的 active_memory 语义一致）
                        let ids: Vec<&str> = page.items.iter().map(|u| u.id()).collect();
                        if !ids.is_empty() {
                            mindscape.record_access(&ids).await;
                        }
                        Some(items)
                    }
                    Err(e) => {
                        return Err(PluginError::InternalError(format!(
                            "语义检索失败: {e}"
                        )))
                    }
                }
            }
        };

        Ok(PluginPayload::new(&IdentityResponse {
            agent_id: self.agent_id.clone(),
            persona,
            recalled,
        }))
    }
}

// ── 工厂 + 自注册 ──
//
// 工厂签名遵循 `submit_object_creator!` 的统一协议：
// `fn(Arc<dyn InvokeRequest>) -> Arc<dyn Capability>`
// `agent_id` / `persona` 由 `plugin.rs::traverse` 在异步阶段渲染后注入
// `AGENT_CAPABILITY_CONTEXT`；本工厂只负责取出并构造。
pub fn build_identity(ctx: Arc<dyn InvokeRequest>) -> Arc<dyn Capability> {
    let cap_ctx = crate::plugins::agent::capabilities::get_capability_context(ctx.as_ref());
    Arc::new(AgentIdentityTool::new(
        cap_ctx.plugin.clone(),
        cap_ctx.agent_id.clone().unwrap_or_default(),
        cap_ctx.persona.clone().unwrap_or_default(),
    )) as Arc<dyn Capability>
}

crate::submit_object_creator!(CAPABILITY_AGENT_IDENTITY, build_identity, dyn Capability);
