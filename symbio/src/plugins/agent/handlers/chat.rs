use crate::plugins::agent::core::default_tool_manager::DefaultToolManager;
use crate::plugins::agent::core::types::cu_fields;
use crate::plugins::agent::core::{truncate_chars, AgentStore, FilterExpr, PageRequest};
use crate::plugins::agent::handlers::system_prompt;
use crate::plugins::agent::plugin::AgentPlugin;
use crate::symbio_core::schemas::model::model_chat;
use crate::symbio_core::{
    CapabilityManager, InvokeRequest, InvokeRequestExt, InvokeResponse, PluginError, PluginPayload,
    MODEL_CHAT,
};
use std::sync::Arc;

// ── 消息上下文构建器 ──
// 原为独立模块 engine/context_builder.rs，现内聚到 chat handler 中。
// 每轮对话到达时，从多个维度构建上下文提示词，注入为用户消息的前缀（msg.prompt）。

const DEFAULT_MIN_SCORE: f64 = 0.7;
const DEFAULT_LIMIT: usize = 5;
const DEFAULT_MIN_QUERY_LEN: usize = 4;
const TASK_CONTEXT_LIMIT: usize = 3;

struct ContextBuilder {
    min_score: f64,
    limit: usize,
    min_query_len: usize,
}

impl Default for ContextBuilder {
    fn default() -> Self {
        Self {
            min_score: DEFAULT_MIN_SCORE,
            limit: DEFAULT_LIMIT,
            min_query_len: DEFAULT_MIN_QUERY_LEN,
        }
    }
}

impl ContextBuilder {
    async fn build(
        &self,
        mindscape: &dyn AgentStore,
        user_text: &str,
        workdir: Option<&str>,
    ) -> String {
        let mut sections: Vec<String> = Vec::new();

        if user_text.trim().len() >= self.min_query_len {
            let memory = self.build_active_memory(mindscape, user_text).await;
            if !memory.is_empty() {
                sections.push(memory);
            }
        }

        sections.push(build_temporal_context(workdir));

        let task_ctx = self.build_task_context(mindscape).await;
        if !task_ctx.is_empty() {
            sections.push(task_ctx);
        }

        if sections.is_empty() {
            return String::new();
        }

        // I-059 优化：临时上下文元说明（3 行内）
        // 让 LLM 立即明白下面的标签是"每轮注入的临时上下文"，与系统提示词中的"长期规则"区分开
        // 用 `` 代码包裹避免与实际 XML 标签冲突（测试和解析都能区分）
        let header = String::from(
            "## 临时上下文（每轮注入，区别于系统提示词的长期规则）\n\
- 块 `active_memory`：与本轮用户消息相关的记忆片段。\n\
- 块 `context`：当前时间、工作区等环境信息。\n\
- 块 `task_context`：本会话可用的策略与技能。\n\n",
        );
        format!("{}{}", header, sections.join("\n"))
    }

    async fn build_active_memory(&self, mindscape: &dyn AgentStore, user_text: &str) -> String {
        let semantic_filter = crate::plugins::agent::core::FilterExpr::Semantic {
            query: user_text.to_string(),
            min_score: self.min_score as f32,
        };
        let page = match mindscape
            .query(
                &semantic_filter,
                &crate::plugins::agent::core::PageRequest::first(self.limit),
            )
            .await
        {
            Ok(page) => {
                let ids: Vec<&str> = page.items.iter().map(|r| r.id()).collect();
                if !ids.is_empty() {
                    mindscape.record_access(&ids).await;
                }
                page
            }
            Err(_) => return String::new(),
        };
        if page.items.is_empty() {
            return String::new();
        }

        let scores = page.scores.unwrap_or_default();
        let mut lines: Vec<String> = Vec::new();
        for (i, unit) in page.items.iter().enumerate() {
            let id = unit.id();
            if id == "identity" {
                continue;
            }
            let is_rule = unit.is_type("rule");
            if is_rule {
                continue;
            }

            let text = unit
                .get(cu_fields::CONTENT)
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .or_else(|| unit.description())
                .unwrap_or("");
            if !text.is_empty() {
                let score = scores.get(i).copied().unwrap_or(0.0) as f64;
                lines.push(format!("- [{:.0}%] {}", score * 100.0, text));
            }
        }

        if lines.is_empty() {
            return String::new();
        }

        let mut prompt =
            String::from("<active_memory>\n基于语义记忆，以下是与当前对话相关的认知：\n");
        for line in &lines {
            prompt.push_str(line);
            prompt.push('\n');
        }
        prompt.push_str("</active_memory>\n");
        prompt
    }

    async fn build_task_context(&self, mindscape: &dyn AgentStore) -> String {
        let page = PageRequest::first(TASK_CONTEXT_LIMIT);
        let strategies = mindscape.query(&FilterExpr::is_a("strategy"), &page).await;
        let skills = mindscape.query(&FilterExpr::is_a("skill"), &page).await;

        let mut all: Vec<&crate::plugins::agent::core::CognitiveUnit> = Vec::new();
        if let Ok(ref p) = strategies {
            all.extend(p.items.iter());
        }
        if let Ok(ref p) = skills {
            all.extend(p.items.iter());
        }

        let mut lines: Vec<String> = Vec::new();
        for unit in &all {
            let id = unit.id();
            if id == "identity" {
                continue;
            }
            let text = unit
                .get(cu_fields::CONTENT)
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .or_else(|| unit.description())
                .unwrap_or("");
            if !text.is_empty() {
                let name = unit.name().unwrap_or("");
                if name.is_empty() {
                    lines.push(format!("- {}", truncate_chars(text, 120)));
                } else {
                    lines.push(format!("- {}: {}", name, truncate_chars(text, 100)));
                }
            }
        }

        if lines.is_empty() {
            return String::new();
        }

        let mut prompt = String::from("<task_context>\n当前可用的策略与技能：\n");
        for line in &lines {
            prompt.push_str(line);
            prompt.push('\n');
        }
        prompt.push_str("</task_context>\n");
        prompt
    }
}

fn build_temporal_context(workdir: Option<&str>) -> String {
    use time::{format_description, OffsetDateTime};

    let now = OffsetDateTime::now_utc();
    // time 0.3.55 起 parse 被 deprecated，parse_borrowed::<2> 为等价替代
    let fmt = format_description::parse_borrowed::<2>("[year]-[month]-[day] [hour]:[minute]")
        .unwrap_or_default();
    let time_str = now.format(&fmt).unwrap_or_else(|_| "unknown".to_string());

    let weekday = match now.weekday() {
        time::Weekday::Monday => "星期一",
        time::Weekday::Tuesday => "星期二",
        time::Weekday::Wednesday => "星期三",
        time::Weekday::Thursday => "星期四",
        time::Weekday::Friday => "星期五",
        time::Weekday::Saturday => "星期六",
        time::Weekday::Sunday => "星期日",
    };

    let mut line = format!("<context>当前时间: {} {}", time_str, weekday);
    if let Some(wd) = workdir {
        line.push_str(&format!(" | 工作区: {}", wd));
    }
    line.push_str("</context>\n");
    line
}

// ── Chat Handler ──

pub async fn handle(
    plugin: &AgentPlugin,
    ctx: Arc<dyn InvokeRequest>,
    workdir_opt: Option<&str>,
) -> InvokeResponse<PluginPayload> {
    let mut req: model_chat::Request = ctx.payload()?;

    let agent_id = ctx
        .get(crate::symbio_core::AGENT_ID)
        .ok_or_else(|| PluginError::ValidationError("Missing agent_id".to_string()))?;

    let mindscape_opt = plugin.get_mindscape(workdir_opt, &agent_id).await;

    // agent 解析失败（mindscape 为 None）必须显式报错，
    // 绝不能静默发出空 system prompt——否则模型失去身份约束会凭空臆造
    // （表现为「agent_id 没生效 / 路由到默认 agent」）。
    let mindscape = mindscape_opt.ok_or_else(|| {
        PluginError::NotFound(format!(
            "Agent '{agent_id}' not found (workdir={workdir_opt:?}); 无法在不解析到具体智能体的情况下开始对话。"
        ))
    })?;

    // ── 获取工具列表（先于系统提示词构建） ──
    let tool_manager: Arc<dyn CapabilityManager> = Arc::new(DefaultToolManager::new());
    let _ = plugin
        .fetch_tools_with_manager(
            workdir_opt.map(|s| s.to_string()),
            &agent_id,
            tool_manager.clone(),
        )
        .await;

    // ── 提取用户消息文本（供 system_prompt 评分使用，需要先于提示词构建） ──
    //
    // **三层目标映射（第 1 层 动态构建）**：
    // 把用户消息作为 `relevance_query` 传给 `system_prompt::build`，
    // 让 CU 的"相关性"维度能根据当前 query 动态评分。
    // 这要求 build 在 message context 之前调用——打破原来的调用顺序。
    let user_text_for_prompt: Option<String> = req
        .single_message
        .as_ref()
        .filter(|m| {
            m.role == Some(crate::symbio_core::schemas::session::chat_message::MessageRole::User)
        })
        .and_then(|m| m.content.as_ref().map(|c| c.to_text()))
        .filter(|s| !s.is_empty());

    // ── 构建系统提示词 ──
    if req.system_prompt.is_none() {
        let mut prompt_buf = String::new();

        if let Some(system_agents) = plugin.load_system_agents_md().await {
            if !system_agents.trim().is_empty() {
                prompt_buf.push_str("## 全局指令\n");
                prompt_buf.push_str(&system_agents);
                prompt_buf.push_str("\n\n");
            }
        }

        if let Some(workspace_agents) = plugin.load_workspace_agents_md(workdir_opt).await {
            if !workspace_agents.trim().is_empty() {
                prompt_buf.push_str("## 工作区指令\n");
                prompt_buf.push_str(&workspace_agents);
                prompt_buf.push_str("\n\n");
            }
        }

        // 从 config 读取预算（I-065 第 1/3 层）
        let cfg = plugin.config.read().await;
        let budget = crate::plugins::agent::core::PromptBudget::new(
            cfg.prompt_budget_tokens,
            cfg.prompt_overhead_tokens,
        );
        drop(cfg);

        let result =
            system_prompt::build(mindscape.as_ref(), &budget, user_text_for_prompt.as_deref())
                .await;
        if !result.prompt.trim().is_empty() {
            prompt_buf.push_str(&result.prompt);
        }

        if !prompt_buf.trim().is_empty() {
            req.system_prompt = Some(prompt_buf);
        }
    }

    ctx.set(crate::symbio_core::CAPABILITY_MANAGER, tool_manager);

    // ── 消息上下文构建（激活记忆 + 时间上下文） ──
    if let Some(msg) = &mut req.single_message {
        if msg.role == Some(crate::symbio_core::schemas::session::chat_message::MessageRole::User) {
            // 复用 Step 5 中已提取的 user_text（避免重复提取）
            let user_text = user_text_for_prompt.clone().unwrap_or_default();

            let context_builder = ContextBuilder::default();
            let ctx_prompt = context_builder
                .build(mindscape.as_ref(), &user_text, workdir_opt)
                .await;
            if !ctx_prompt.is_empty() {
                msg.prompt = Some(ctx_prompt);
            }
        }
    }

    // ── 路由到 MODEL 服务 ──
    if let Some(parent) = plugin.get_parent().await {
        let final_ctx = ctx.fork();
        final_ctx.set(crate::symbio_core::PATH, MODEL_CHAT.to_string());
        // 显式 set WORKDIR 给下游 model/chat 路由链
        // （fork 虽会继承父 ctx 的 WORKDIR，但显式 set 能让"子 agent 工作目录覆盖父 agent"语义无歧义，
        //  同时保护子智能体内部所有 local 工具的沙箱边界）
        if let Some(wd) = workdir_opt {
            final_ctx.set(crate::symbio_core::WORKDIR, wd.to_string());
        }
        final_ctx.set_payload(req)?;
        return parent.route(final_ctx).await;
    }
    Err(PluginError::InternalError("父插件未设置".to_string()))
}

#[cfg(test)]
#[path = "chat_tests.rs"]
mod tests;
