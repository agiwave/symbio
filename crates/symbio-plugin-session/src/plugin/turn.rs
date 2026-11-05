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
use crate::declarations::{CHAT_VOCAB, MESSAGE_VOCAB};
use crate::transcript::publish::append_and_publish;
use symbio_kernel::event::{Entity, Verb};
use symbio_plugin_api::events::{
    EVENT_ARTIFACT_ADDED, EVENT_CHAT_ASSISTANT_FINAL, EVENT_SESSION_CONFIG, EVENT_USER_MESSAGE,
};
use symbio_plugin_api::fact_source::FactSource;
use symbio_plugin_api::FactDraft;
use symbio_sdk::authz::{PRINCIPAL_MAIN, PRINCIPAL_USER};
use symbio_sdk::chat::chat_node as cm;

impl SessionPlugin {
    /// 收件箱队首 → `user.message` 事实；返回这条事实的 `seq`（无消息 ⇒ `None`）。
    ///
    /// `seq` 由事实源在入库时分配，它就是**开轮事实的身份**：`session.config` 的
    /// 幂等键与溯源都指向它，投影终答时也用它找回这条 `user.message`。
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
        let cfg = self.config.read().await.clone();
        let config_draft = FactDraft {
            key: format!("session-config-{seq}"),
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

    /// 该开轮事实的终答事实 → 转写节点，并收束本轮；没有终答 ⇒ `false`（不投影）。
    ///
    /// 配对靠**事实身份**：由 `user.message` 的 `seq` 取回它的 `event_id`，
    /// 再找载荷 `turn_id` 指向它的那条终答——不再拿轮号当键。
    /// 节点由 `rebuild` 从**同一条事实**折出（读路径与写路径同一个折叠）。幂等：
    /// 同一条事实再投影一次时，`apply` 见到节点已存在会跳过（不重复追加）。
    pub(crate) async fn project_final_turn(
        &self,
        session_id: &str,
        user_seq: u64,
        source: &dyn FactSource,
    ) -> Result<bool, PluginError> {
        let events = source.range(0);
        let Some(user) = events.iter().find(|e| {
            e.kind == EVENT_USER_MESSAGE && e.seq.map(|seq| seq.value()) == Some(user_seq)
        }) else {
            return Ok(false);
        };
        let Some(final_event) = events
            .iter()
            .rev()
            .find(|e| {
                e.kind == EVENT_CHAT_ASSISTANT_FINAL
                    && e.payload.get("turn_id").and_then(Value::as_str) == Some(&user.event_id)
            })
            .cloned()
        else {
            // 没有终答：若本轮落了「等用户动作」的产物，本轮就停在卡片上。
            // 这是 interactive 档的正常收束形态（`ask_user` / 审批卡）——运行器
            // 不再推进工具循环，CLI 也据此以退出码 0 结束。归本轮靠 seq：只看
            // 开轮事实之后的这一段，前一轮没答复的卡不会误判。
            let waiting_events: Vec<_> = events
                .iter()
                .filter(|e| e.seq.is_some_and(|seq| seq.value() > user_seq))
                .cloned()
                .collect();
            let waiting_on_user = waiting_events.iter().any(|e| {
                e.kind == EVENT_ARTIFACT_ADDED
                    && e.payload.get("needs_user_action").and_then(Value::as_bool) == Some(true)
            });
            if !waiting_on_user {
                return Ok(false);
            }
            // 卡片节点与终答走同一条折叠 + 发布（读路径与写路径同一份规则）：
            // 本轮所有节点（正文 / 工具调用 / 卡片）一并投影，`apply` 对已存在的
            // 节点幂等跳过。否则实时面只看得到环的协议事实、看不到卡片。
            let nodes: Vec<cm::ChatNode> = symbio_sdk::projection::rebuild::rebuild(
                &waiting_events,
                &CHAT_VOCAB,
                &MESSAGE_VOCAB,
            )
            .into_iter()
            .map(|mut node| {
                node.seq = None;
                node
            })
            .collect();
            if !nodes.is_empty() {
                let session = self.open_chat_session(session_id).await?;
                append_and_publish(
                    &session,
                    nodes,
                    Carrier::Runner,
                    &self.active_mgr,
                    session_id,
                    PRINCIPAL_MAIN,
                )
                .await?;
            }
            let state = self.active_mgr.get_or_create(session_id).await;
            self.convergence
                .emit_session_state(
                    &state,
                    SessionStateChange::Finished {
                        outcome: OUTCOME_COMPLETED,
                        error: None,
                    },
                )
                .await;
            return Ok(true);
        };

        let nodes: Vec<cm::ChatNode> = symbio_sdk::projection::rebuild::rebuild(
            std::slice::from_ref(&final_event),
            &CHAT_VOCAB,
            &MESSAGE_VOCAB,
        )
        .into_iter()
        .map(|mut node| {
            // seq 由存储在临界区内权威分配；折叠给的是「从这份切片起」的局部号。
            node.seq = None;
            node
        })
        .collect();
        if !nodes.is_empty() {
            let session = self.open_chat_session(session_id).await?;
            append_and_publish(
                &session,
                nodes,
                Carrier::Runner,
                &self.active_mgr,
                session_id,
                PRINCIPAL_MAIN,
            )
            .await?;
        }

        let state = self.active_mgr.get_or_create(session_id).await;
        self.convergence
            .emit_session_state(
                &state,
                SessionStateChange::Finished {
                    outcome: OUTCOME_COMPLETED,
                    error: None,
                },
            )
            .await;
        Ok(true)
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
        let projected = self.project_final_turn(&sid, seq, &*source).await?;
        Ok(PluginPayload::new(&json!({ "projected": projected })))
    }
}
