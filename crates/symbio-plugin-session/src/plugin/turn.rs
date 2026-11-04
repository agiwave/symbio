//! 一轮的两半——**会话侧**的写方与投影。
//!
//! - 收件箱队首 → 一条 `user.message` 事实（开轮）。收件箱归本插件，会话身份也归
//!   本插件，因此这条事实的写方在这里，不在宿主。
//! - 终答事实 → 转写（收束）。`chat.assistant.final` 是本轮已经落下的事实，
//!   本插件把它投影成消息节点；**宿主不写任何业务事实**。
//!
//! 本插件**不调** `drive`（那会变成插件调插件）：它只写事实 / 读事实，宿主看到
//! 新事实再单次驱动那一轮。两半之间的唯一接口是**事实源上的 scope**。

use super::*;
use crate::chat_session::Carrier;
use crate::convergence::SessionStateChange;
use crate::declarations::{CHAT_VOCAB, MESSAGE_VOCAB};
use crate::transcript::publish::append_and_publish;
use symbio_kernel::event::{Entity, Verb};
use symbio_plugin_api::events::{EVENT_CHAT_ASSISTANT_FINAL, EVENT_USER_MESSAGE};
use symbio_plugin_api::fact_source::FactSource;
use symbio_plugin_api::FactDraft;
use symbio_sdk::authz::{PRINCIPAL_MAIN, PRINCIPAL_USER};
use symbio_sdk::chat::chat_node as cm;

impl SessionPlugin {
    /// 收件箱队首 → `user.message` 事实；返回这一轮的 `scope`（无消息 ⇒ `None`）。
    ///
    /// `scope` 取「下一条事实的号」：它与这条 `user.message` 落格后拿到的 seq 相同，
    /// 因此每轮唯一且单调。**不能**用会话哈希：事实源里 scope 是**轮的同一性**
    /// （同 scope 二次开轮会被 I1 拒绝），而一个会话有很多轮。
    pub(crate) async fn consume_inbox_turn(
        &self,
        session_id: &str,
        source: &dyn FactSource,
    ) -> Result<Option<u64>, PluginError> {
        let state = self.active_mgr.get_or_create(session_id).await;
        let Some(item) = self.take_inbox_front(&state).await else {
            return Ok(None);
        };

        let scope = source.head();
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
        source
            .append(scope, PRINCIPAL_USER.to_string(), draft)
            .map_err(|e| PluginError::InternalError(format!("user.message 落格失败: {e}")))?;

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
        Ok(Some(scope))
    }

    /// 该 `scope` 的终答事实 → 转写节点，并收束本轮；没有终答 ⇒ `false`（不投影）。
    ///
    /// 节点由 `rebuild` 从**同一条事实**折出（读路径与写路径同一个折叠）。幂等：
    /// 同一条事实再投影一次时，`apply` 见到节点已存在会跳过（不重复追加）。
    pub(crate) async fn project_final_turn(
        &self,
        session_id: &str,
        scope: u64,
        source: &dyn FactSource,
    ) -> Result<bool, PluginError> {
        let events = source.range(0);
        let Some(final_event) = events
            .iter()
            .find(|e| e.scope == scope && e.kind == EVENT_CHAT_ASSISTANT_FINAL)
            .cloned()
        else {
            return Ok(false);
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

    /// 路由 `session/consume`：把队首落成事实，返回本轮 scope。
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
        let scope = self.consume_inbox_turn(&sid, &*source).await?;
        Ok(PluginPayload::new(&json!({ "scope": scope })))
    }

    /// 路由 `session/project`：把该 scope 的终答事实投影回转写并收束。
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
        let scope = payload
            .get("scope")
            .and_then(Value::as_u64)
            .ok_or_else(|| PluginError::ValidationError("session/project 需要 scope".into()))?;
        let source = self.fact_source_for(&ctx, &sid).await?;
        let projected = self.project_final_turn(&sid, scope, &*source).await?;
        Ok(PluginPayload::new(&json!({ "projected": projected })))
    }
}
