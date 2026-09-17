//! SessionPlugin 的 invoke 处理方法集合（按 `schemas/session/*` 请求类型分发）。
//!
//! 路由层在 `plugin.rs`（`Plugin::route`），本文件只承载各 invoke 的实现体：
//! 消息增删改查、会话删除/清空、
//! metadata 合并与统一删除路径 `delete_session_internal` 等。

use super::chat_session::{ChatSession, ChatSessionHandle, PersistentChatSession};
use super::plugin::SessionPlugin;
use crate::symbio_core::schemas::session::{
    chat_message as cm, session_append, session_clear, session_clear_messages,
    session_delete_message, session_get_messages, session_open, session_update,
    session_update_message,
};
use crate::symbio_core::{InvokeRequest, InvokeRequestExt, PluginPayload};
use crate::symbio_core::{InvokeResponse, PluginError};
use serde_json::{json, Value};
use std::sync::Arc;

impl SessionPlugin {
    pub async fn invoke_get_messages(&self, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<Value> {
        let req: session_get_messages::Request = ctx.payload()?;
        let chat_session = self.open_chat_session(&req.session_id).await?;
        let messages = chat_session.get_messages().await?;

        Ok(serde_json::to_value(session_get_messages::Response { messages }).unwrap_or_default())
    }

    pub async fn invoke_append(&self, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<Value> {
        let req: session_append::Request = ctx.payload()?;
        let chat_session = self.open_chat_session(&req.session_id).await?;
        let message_count = chat_session.append_messages(req.messages).await?;

        Ok(serde_json::to_value(session_append::Response { message_count }).unwrap_or_default())
    }

    pub async fn invoke_clear(&self, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<Value> {
        let req: session_clear::Request = ctx.payload()?;
        self.delete_session_internal(&req.session_id).await?;
        Ok(serde_json::to_value("会话已删除".to_string())?)
    }

    /// 删除会话的统一内部实现（abort 活跃任务 → 清活跃条目 → 存储删除）。
    ///
    /// 两个消费方：`invoke_clear`（旧 session/clear 路由）与
    /// `VdfsProvider::delete`（VDFS 删除与此共用同一份实现）。
    pub(crate) async fn delete_session_internal(
        &self,
        session_id: &str,
    ) -> Result<(), PluginError> {
        // 删除前先 abort 该会话的活跃任务
        let state = self.active_mgr.get_or_create(session_id).await;
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
        self.active_mgr.sessions.write().await.remove(session_id);

        let store = self.get_store().await?;
        store.delete_session(session_id).await?;

        // VDFS 实时链路（provider 侧变更广播 → watch 的 sink → 总线 kind="vdfs"）：
        // 前端据此把该会话从清单移除。session/clear 与 VDFS 删除两条删除路径共用此处。
        // 作用域按**路径前缀**分流（子会话落在 `<sid>/子会话/…` 之下），因此这里
        // 不再需要实体时代的 `parent_id` 载荷——也不必为发事件多读一次盘。
        self.notify_change(session_id, crate::symbio_core::vdfs::VDFS_CHANGE_DELETED);

        Ok(())
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
        // VDFS 视图同步（前端已在本地收敛，故只发变更、不发前端帧）
        self.emit_transcript_cleared(&req.session_id);
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
        // VDFS 视图同步：逐条发 `deleted`（与前端本地移除的 id 列表严格一致）
        for id in &deleted_ids {
            self.emit_message_deleted(&req.session_id, id);
        }
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

        let updated = existing.clone();
        chat_session.replace_messages(messages).await?;
        // VDFS 视图同步：把合并后的**完整消息**作为载荷发出（消费者零回读）
        self.emit_message_updated(&req.session_id, &updated);
        Ok(serde_json::to_value(session_update_message::Response {
            updated: true,
        })?)
    }

    /// 合并写入会话 metadata（workdir / title / agent_id 等）。
    pub async fn invoke_update(&self, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<Value> {
        let req: session_update::Request = ctx.payload()?;

        let mut session = self.get_or_create_session(&req.session_id).await?;

        // 新建判定：get_or_create 未命中已存会话时返回全新空会话
        // （无消息、metadata 为空对象）。用于区分 created / updated 生命周期事件。
        let is_new = session.messages.is_empty()
            && session
                .metadata
                .as_object()
                .map(|o| o.is_empty())
                .unwrap_or(true);

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

        session.updated_at = crate::symbio_core::now_ms();
        self.save_session(&session).await?;

        // VDFS 实时链路（provider 侧变更广播 → watch 的 sink → 总线 kind="vdfs"）：
        // 新建 → created（粗粒度，消费方重拉清单）；其余 → **带节点视图的 updated**
        // （标题 / 元数据等就地收敛，消费方零回读）。VDFS 会话清单因此无需轮询。
        if is_new {
            self.notify_change(
                &req.session_id,
                crate::symbio_core::vdfs::VDFS_CHANGE_CREATED,
            );
        } else {
            self.notify_session_state(&req.session_id).await;
        }

        Ok(serde_json::to_value(session_update::Response {
            success: true,
            session: serde_json::to_value(session)?,
        })
        .unwrap_or_default())
    }

    /// 按 session_id 构造会话引擎实例（唯一构造实现）。
    ///
    /// - `_t_` 前缀 → 内存 ephemeral 会话；
    /// - 非空 id → 持久会话；
    /// - None/空 → 内存 ephemeral 会话（固定 id `"ephemeral"`）。
    ///
    /// 两类会话共用 [`PersistentChatSession`]，差异只在存储后端（审计 B1）；
    /// ephemeral 会话的配置取当前值的快照（内存会话不随 `session/config` 变更而变）。
    ///
    /// 消费方：session/open 路由（对外 API），以及会话编排器向 chat_ctx
    /// 交付会话句柄（SESSION_HANDLE，交付失败时 model 侧兜底内存会话）。
    pub async fn open_session_handle(
        &self,
        session_id: Option<String>,
    ) -> Result<Arc<dyn ChatSession>, PluginError> {
        let snapshot = {
            let cfg = self.config.read().await;
            cfg.clone()
        };

        let session: Arc<dyn ChatSession> = match session_id {
            Some(sid) if !sid.is_empty() && !sid.starts_with("_t_") => {
                let store = self.get_store().await?;
                Arc::new(PersistentChatSession::new(sid, self.config.clone(), store))
            }
            // `_t_` 前缀与空/缺省 id：内存临时会话，配置取当前值快照。
            // 固定 id "ephemeral"（审计 B2）：随机 id 会让压缩前的 transcript
            // 转存落到永不复现的目录名下，成为无法关联的孤儿存档。
            Some(sid) => Arc::new(PersistentChatSession::detached(sid, snapshot)),
            None => Arc::new(PersistentChatSession::detached("ephemeral", snapshot)),
        };

        Ok(session)
    }

    pub async fn invoke_open(&self, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload> {
        let req: session_open::Request = ctx.payload()?;
        let session = self.open_session_handle(req.session_id).await?;

        Ok(PluginPayload::Native(Arc::new(ChatSessionHandle::new(
            session,
        ))))
    }
}
