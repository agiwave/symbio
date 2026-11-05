//! 一轮的两半——**会话侧**的写方与投影。
//!
//! - 收件箱队首 → 一条 `user.message` 事实（开轮）。收件箱归本插件，会话身份也归
//!   本插件，因此这条事实的写方在这里，不在宿主。
//! - 终答事实 → 转写（收束）。`chat.assistant.final` 是本轮已经落下的事实，
//!   本插件把它投影成消息节点；**宿主不写任何业务事实**。
//!
//! 本插件**不调** `drive`（那会变成插件调插件）：它只写事实 / 读事实，宿主看到
//! 新事实再单次驱动那一轮。两半之间的唯一接口是**事实源 + 开轮事实的身份**：
//! 事实源在打开时就绑定会话，`user.message` 的 `seq` / `event_id` 就是这一轮的名字。

use super::*;
use crate::chat_session::Carrier;
use crate::convergence::SessionStateChange;
use crate::transcript::publish::append_and_publish;
use symbio_kernel::event::{Entity, Verb};
use symbio_plugin_api::capability_take_errors;
use symbio_plugin_api::debt::session_config_id;
use symbio_plugin_api::events::{EVENT_SESSION_CONFIG, EVENT_USER_MESSAGE};
use symbio_plugin_api::fact_source::FactSource;
use symbio_plugin_api::FactDraft;
use symbio_sdk::authz::{PRINCIPAL_MAIN, PRINCIPAL_USER};

impl SessionPlugin {
    /// 收件箱队首 → `user.message` 事实；返回这条事实的 `seq`（无消息 ⇒ `None`）。
    ///
    /// `seq` 由事实源在入库时分配，它就是**开轮事实的身份**：`session.config` 的
    /// 溯源指向它；`session.config` 的幂等键则由这条 `user.message` 的入库 id 派生
    /// （`session_config_id`），让模型债行的 `when_id` 能只认**这一轮**的 config。
    pub(crate) async fn consume_inbox_turn(
        &self,
        ctx: &Arc<dyn PluginInvokeRequest>,
        session_id: &str,
        source: &dyn FactSource,
    ) -> Result<Option<u64>, PluginError> {
        let state = self.active_mgr.get_or_create(session_id).await;
        let Some(item) = self.take_inbox_front(&state).await else {
            return Ok(None);
        };

        let text = item
            .message
            .content
            .as_ref()
            .map(|c| c.to_text())
            .unwrap_or_default();
        let draft = FactDraft {
            key: item.id.clone(),
            kind: EVENT_USER_MESSAGE.to_string(),
            entity: Entity::Scope,
            verb: Verb::Opened,
            payload: json!({ "text": text, "id": item.id }),
            produced_by: None,
        };
        let seq = source
            .append(PRINCIPAL_USER.to_string(), draft)
            .map_err(|e| PluginError::InternalError(format!("user.message 落格失败: {e}")))?;

        // 轮首装配：能力收集 → `session.config`（系统提示词 + 工具表 + 本轮生效设置）。
        //
        // 写方是本插件（会话编排权归 session）：模型端口只读「最近一条 session.config」，
        // 读不到就退化为空提示词。提示词里的 VDFS 地址由收集期的 `descend_addr` 归一
        // （根名取自 vdfs 插件的装配期声明，不是本插件里的字面量），**不是**在这里把
        // 绝对路径再相对化。
        let manager =
            crate::capabilities::collect_capabilities(self.get_parent().as_ref(), ctx).await;
        let system_prompt = symbio_sdk::capability::capability_prompts(&*manager)
            .await
            .into_iter()
            .map(|(_, prompt)| prompt)
            .filter(|prompt| !prompt.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
        let tools = manager
            .list(symbio_kernel::capability::family::TOOL)
            .await
            .into_iter()
            .map(|row| row.meta())
            .collect::<Vec<_>>();
        // 收集过程的失败信号（traverse 侧记在 ctx 的桶里）：能力表为空/有错都要落 config，
        // 只是把原因写进载荷。**不造新事件类型**——模型行依赖的是 `session.config` 这条
        // 输入本身，它必须在每一轮都出现，否则模型行会永远等不到输入。
        let capability_errors = capability_take_errors(ctx).await;
        let tools_empty_reason = if tools.is_empty() {
            Some(if capability_errors.is_empty() {
                "no_tool_rows"
            } else {
                "capability_error"
            })
        } else {
            None
        };
        let cfg = self.config.read().await.clone();
        let config_draft = FactDraft {
            key: session_config_id(&item.id),
            kind: EVENT_SESSION_CONFIG.to_string(),
            entity: Entity::Scope,
            verb: Verb::Asserted,
            payload: json!({
                "system_prompt": system_prompt,
                "tools": tools,
                "classify_enabled": cfg.classify_enabled,
                "progress_enabled": cfg.progress_enabled,
                "progress_interval_ms": cfg.progress_interval_ms,
                "progress_max_per_turn": cfg.progress_max_per_turn,
                "readyset_cap": cfg.task_cap,
                "skill_compile_enabled": cfg.skill_compile_enabled,
                // 兜底表达：空表原因 / 收集错误明细（均为既有 config 载荷的字段，
                // 不是新事件类型）。非空表时前者为 null，后者为空数组。
                "tools_empty_reason": tools_empty_reason,
                "capability_errors": capability_errors
                    .iter()
                    .map(|e| json!({ "plugin": e.plugin, "message": e.message }))
                    .collect::<Vec<_>>(),
            }),
            // 溯源指同一轮刚落的 `user.message`（seq 就是它的号）。
            produced_by: Some(seq),
        };
        source
            .append(PRINCIPAL_MAIN.to_string(), config_draft)
            .map_err(|e| PluginError::InternalError(format!("session.config 落格失败: {e}")))?;

        let mut message = item.message;
        message.seq = None;
        let session = self.open_chat_session(session_id).await?;
        append_and_publish(
            &session,
            vec![message],
            Carrier::Runner,
            &self.active_mgr,
            session_id,
            PRINCIPAL_MAIN,
        )
        .await?;

        self.convergence
            .emit_session_state(&state, SessionStateChange::Working)
            .await;
        Ok(Some(seq))
    }

    /// 路由 `session/consume`：把队首落成事实，返回开轮事实的 `seq`。
    pub(crate) async fn handle_consume(
        &self,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let sid = ctx.get(SESSION_ID).unwrap_or_default();
        if sid.is_empty() {
            return Err(PluginError::ValidationError(
                "session/consume 需要 metadata.session_id".into(),
            ));
        }
        let source = self.fact_source_for(&ctx, &sid).await?;
        let seq = self.consume_inbox_turn(&ctx, &sid, &*source).await?;
        Ok(PluginPayload::new(&json!({ "seq": seq })))
    }

    /// 路由 `session/project`：把该开轮事实的终答投影回转写并收束。
    pub(crate) async fn handle_project(
        &self,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let sid = ctx.get(SESSION_ID).unwrap_or_default();
        if sid.is_empty() {
            return Err(PluginError::ValidationError(
                "session/project 需要 metadata.session_id".into(),
            ));
        }
        let payload = ctx.payload::<Value>()?;
        let seq = payload
            .get("seq")
            .and_then(Value::as_u64)
            .ok_or_else(|| PluginError::ValidationError("session/project 需要 seq".into()))?;
        let source = self.fact_source_for(&ctx, &sid).await?;
        let projected = self
            .convergence
            .project_final_turn(&sid, seq, &*source)
            .await?;
        Ok(PluginPayload::new(&json!({ "projected": projected })))
    }
}
