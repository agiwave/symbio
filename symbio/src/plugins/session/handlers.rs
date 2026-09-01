use super::chat_session::{EphemeralChatSession, PersistentChatSession};
use super::plugin::SessionPlugin;
use crate::symbio_core::schemas::session::session_config::SessionConfig;
use crate::symbio_core::schemas::{
    common,
    session::{
        chat_message as cm, session_append, session_clear, session_clear_messages,
        session_compress, session_delete_message, session_get_messages, session_list, session_open,
        session_update, session_update_message,
    },
};
use crate::symbio_core::{ChatSessionHandle, InvokeRequest, InvokeRequestExt, PluginPayload};
use crate::symbio_core::{InvokeResponse, PluginError};
use serde_json::{json, Value};
use std::sync::Arc;
use time::OffsetDateTime;

impl SessionPlugin {
    pub async fn invoke_get_messages(&self, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<Value> {
        let req: session_get_messages::Request = ctx.payload()?;
        let chat_session = self.open_chat_session(&req.session_id).await?;
        let messages = chat_session.get_messages().await?;

        Ok(serde_json::to_value(session_get_messages::Response { messages }).unwrap_or_default())
    }

    pub async fn invoke_compress(&self, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<Value> {
        let req: session_compress::Request = ctx.payload()?;

        let store = self.get_store().await?;
        let session_dir = store
            .session_dir(&req.session_id)
            .ok_or_else(|| PluginError::InternalError("该存储后端不支持消息存档".to_string()))?;

        // 压缩路径下的 display path 由 SessionPlugin::session_storage_dir() 派生，
        // 仅作 UI 展示。
        let cfg = self.config.read().await;
        let display_session_path = SessionPlugin::session_storage_dir()
            .join(req.session_id.replace(['/', '\\', ':'], "_"));

        let mut compressed_messages = Vec::new();
        for chat_msg in req.messages {
            let ts = (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000) as i64;
            let archive_filename = format!("{}/m{:x}.txt", super::compress::MESSAGES_SUBDIR, ts);
            let archive_display_path = display_session_path
                .join(&archive_filename)
                .to_string_lossy()
                .replace("\\", "/");

            let compressed = super::compress::compress_message(
                &session_dir,
                &chat_msg,
                cfg.compress_line_threshold,
                &archive_filename,
                &archive_display_path,
            )
            .await;

            match compressed {
                Ok(Some(c)) => compressed_messages.push(c),
                Ok(None) => compressed_messages.push(chat_msg),
                Err(e) => {
                    crate::plugin_error!("session", "主动压缩消息失败: {}", e);
                    compressed_messages.push(chat_msg);
                }
            }
        }

        Ok(serde_json::to_value(session_compress::Response {
            messages: compressed_messages,
        })
        .unwrap_or_default())
    }

    pub async fn invoke_append(&self, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<Value> {
        let req: session_append::Request = ctx.payload()?;
        let chat_session = self.open_chat_session(&req.session_id).await?;
        let message_count = chat_session.append_messages(req.messages).await?;

        Ok(serde_json::to_value(session_append::Response { message_count }).unwrap_or_default())
    }

    pub async fn invoke_clear(&self, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<Value> {
        let req: session_clear::Request = ctx.payload()?;

        // 删除前先 abort 该会话的活跃任务
        let state = self.active_mgr.get_or_create(&req.session_id).await;
        {
            let mut inner = state.inner.write().await;
            if let Some(tx) = inner.ai_control_tx.take() {
                let _ = tx
                    .send(crate::symbio_core::PluginFrame::Data(json!({
                        "type": "abort",
                    })))
                    .await;
            }
        }
        // 清理活跃条目
        self.active_mgr
            .sessions
            .write()
            .await
            .remove(&req.session_id);

        self.get_store()
            .await?
            .delete_session(&req.session_id)
            .await?;

        Ok(serde_json::to_value("会话已删除".to_string())?)
    }

    /// 清空会话消息（保留 metadata / 工作目录 / 标题等）。
    ///
    /// 与 `invoke_clear`（删除整个会话文件）不同：这里只把 `session.messages`
    /// 整体替换为空，会话本体继续存在。UI 的"清空历史"按钮走此路径。
    pub async fn invoke_clear_messages(
        &self,
        ctx: Arc<dyn InvokeRequest>,
    ) -> InvokeResponse<Value> {
        let req: session_clear_messages::Request = ctx.payload()?;
        let chat_session = self.open_chat_session(&req.session_id).await?;
        chat_session.replace_messages(Vec::new()).await?;
        Ok(serde_json::to_value(session_clear_messages::Response {
            cleared: true,
        })?)
    }

    /// 删除单条消息（连同其后续所有消息一并删除）。
    ///
    /// 消息列表本身已按时间/顺序排好序，因此只需按列表顺序定位到目标消息，
    /// 然后把"它及其之后的所有消息"整段 `drain` 掉即可——无需任何 parent_id 级联逻辑。
    /// 这样既能保证会话消息的连续性（不会出现孤立的后半截助手回复），
    /// 又足够简单直接。
    pub async fn invoke_delete_message(
        &self,
        ctx: Arc<dyn InvokeRequest>,
    ) -> InvokeResponse<Value> {
        let req: session_delete_message::Request = ctx.payload()?;
        let chat_session = self.open_chat_session(&req.session_id).await?;
        let mut messages = chat_session.get_messages().await?;

        // 在已排序的列表中定位目标消息，删除"它及其之后的全部消息"。
        let idx = messages.iter().position(|m| m.id == req.message_id);
        let deleted_ids: Vec<String> = match idx {
            Some(i) => {
                let removed: Vec<String> = messages[i..].iter().map(|m| m.id.clone()).collect();
                messages.drain(i..);
                removed
            }
            None => Vec::new(),
        };

        chat_session.replace_messages(messages).await?;
        Ok(serde_json::to_value(session_delete_message::Response {
            deleted: deleted_ids.len(),
            deleted_ids,
        })?)
    }

    /// 更新单条消息（手工编辑 / 标错重试等场景）。
    ///
    /// 按 `message.id` 定位，仅覆盖请求中提供的字段
    /// （content / status / error / meta 等），未提供的字段保持不变。
    pub async fn invoke_update_message(
        &self,
        ctx: Arc<dyn InvokeRequest>,
    ) -> InvokeResponse<Value> {
        let req: session_update_message::Request = ctx.payload()?;
        let patch = &req.message;
        if patch.id.is_empty() {
            return Err(PluginError::ValidationError(
                "message.id 不能为空".to_string(),
            ));
        }
        let chat_session = self.open_chat_session(&req.session_id).await?;
        let mut messages = chat_session.get_messages().await?;

        let Some(existing) = messages.iter_mut().find(|m| m.id == patch.id) else {
            return Err(PluginError::NotFound(format!("消息不存在: {}", patch.id)));
        };

        if let Some(role) = &patch.role {
            existing.role = Some(role.clone());
        }
        if let Some(t) = &patch.msg_type {
            existing.msg_type = Some(t.clone());
        }
        if let Some(n) = &patch.name {
            existing.name = Some(n.clone());
        }
        if let Some(p) = &patch.parent_id {
            existing.parent_id = Some(p.clone());
        }
        if let Some(c) = &patch.content {
            existing.content = Some(c.clone());
        }
        if let Some(s) = &patch.status {
            existing.status = Some(s.clone());
        }
        if let Some(e) = &patch.error {
            existing.error = Some(e.clone());
        } else if patch
            .status
            .as_ref()
            .map(|s| *s != cm::MessageStatus::Failed)
            .unwrap_or(false)
        {
            // 状态不再是 Failed 时，顺带清掉旧的 error，避免残留误导。
            existing.error = None;
        }
        if let Some(ts) = patch.timestamp {
            existing.timestamp = Some(ts);
        }
        if let Some(rid) = &patch.response_id {
            existing.response_id = Some(rid.clone());
        }
        if let Some(new_meta) = &patch.meta {
            match &mut existing.meta {
                Some(existing_meta) => {
                    if let (Some(a), Some(b)) =
                        (existing_meta.as_object_mut(), new_meta.as_object())
                    {
                        for (k, v) in b {
                            a.insert(k.clone(), v.clone());
                        }
                    } else {
                        existing.meta = Some(new_meta.clone());
                    }
                }
                None => {
                    existing.meta = Some(new_meta.clone());
                }
            }
        }

        chat_session.replace_messages(messages).await?;
        Ok(serde_json::to_value(session_update_message::Response {
            updated: true,
        })?)
    }

    pub async fn invoke_list(&self) -> InvokeResponse<Value> {
        let sessions = self.list_sessions().await?;
        let active = self.active_mgr.sessions.read().await;
        let session_items = sessions
            .iter()
            .map(|s| {
                let is_working = active
                    .get(&s.id)
                    .map(|st| st.inner.try_read().map(|i| i.is_working).unwrap_or(false))
                    .unwrap_or(false);
                session_list::SessionListItem {
                    id: s.id.clone(),
                    message_count: s.messages.len(),
                    updated_at: s.updated_at,
                    is_working,
                    metadata: s.metadata.clone(),
                }
            })
            .collect::<Vec<_>>();

        Ok(serde_json::to_value(session_list::Response {
            sessions: session_items,
        })
        .unwrap_or_default())
    }

    /// 合并写入会话 metadata（workdir / title / agent_id 等）。
    pub async fn invoke_update(&self, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<Value> {
        let req: session_update::Request = ctx.payload()?;

        let mut session = self.get_or_create_session(&req.session_id).await?;

        // 合并 metadata（浅合并）
        if let Some(existing_obj) = session.metadata.as_object_mut() {
            if let Some(new_obj) = req.metadata.as_object() {
                for (k, v) in new_obj {
                    existing_obj.insert(k.clone(), v.clone());
                }
            } else {
                session.metadata = req.metadata.clone();
            }
        } else {
            session.metadata = req.metadata.clone();
        }

        // 单独处理 title 字段
        if let Some(title) = &req.title {
            if let Some(obj) = session.metadata.as_object_mut() {
                obj.insert("title".to_string(), Value::String(title.clone()));
            }
        }

        session.updated_at = (OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64;
        self.save_session(&session).await?;

        Ok(serde_json::to_value(session_update::Response {
            success: true,
            session: serde_json::to_value(session)?,
        })
        .unwrap_or_default())
    }

    pub async fn invoke_config_get(&self) -> InvokeResponse<Value> {
        let cfg = self.config.read().await;
        Ok(serde_json::to_value(&*cfg)?)
    }

    pub async fn invoke_config_set(&self, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<Value> {
        let new_cfg: SessionConfig = ctx.payload()?;

        {
            let mut cfg = self.config.write().await;
            *cfg = new_cfg;
        }

        if let Some(p) = self.get_parent() {
            let save_ctx = ctx.fork();
            save_ctx.set(crate::symbio_core::PATH, "save_config".to_string());
            let _ = p.route(save_ctx).await;
        }

        Ok(serde_json::to_value(common::SuccessResponse::default())?)
    }

    pub async fn invoke_open(&self, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload> {
        let req: session_open::Request = ctx.payload()?;
        let cfg = self.config.read().await;

        let session: Arc<dyn crate::symbio_core::ChatSession> = match req.session_id {
            Some(sid) if !sid.is_empty() => {
                if sid.starts_with("_t_") {
                    let ephemeral = EphemeralChatSession::new(&cfg);
                    drop(cfg);
                    Arc::new(ephemeral)
                } else {
                    let store = self.get_store().await?;
                    drop(cfg);
                    Arc::new(PersistentChatSession::new(sid, self.config.clone(), store))
                }
            }
            _ => {
                let ephemeral = EphemeralChatSession::new(&cfg);
                drop(cfg);
                Arc::new(ephemeral)
            }
        };

        Ok(PluginPayload::Native(Arc::new(ChatSessionHandle::new(
            session,
        ))))
    }

    pub async fn invoke_config_schema(&self) -> InvokeResponse<Value> {
        Ok(json!({ "schema": Self::config_schema() }))
    }
}
