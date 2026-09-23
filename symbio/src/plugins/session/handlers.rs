//! SessionPlugin 里**仅剩的一个** invoke 路由的实现体，加上两个非路由的内部函数。
//!
//! 路由层在 `plugin.rs`（`Plugin::route`）。本文件已经收缩到只剩四件事，
//! 因为会话与消息的增删改查**全部**经 VDFS 地址完成了：
//!
//! | 函数 | 性质 | 为什么还在这里 |
//! |---|---|---|
//! | `invoke_update` | 路由 `update` | 仅 CLI：它需要**客户端指定会话 id**，而 VDFS 新建是 provider 生成 id——见同文 §3.5 |
//! | `delete_session_internal` | **非路由** | `VdfsProvider::delete` 的内部实现（唯一消费方） |
//! | `open_session_handle` | **非路由** | 编排器构造会话引擎句柄（`SESSION_HANDLE`）用 |
//!
//! ## 已退役的（各自都有 VDFS 侧等价入口，留着就是第二份实现）
//!
//! - `append` → 消息追加的唯一入口是聊天协议，编排自身的落库走引擎直连
//!   （`orchestrator/entry.rs` 的 `open_chat_session` + `append_messages`）。
//! - `open` → 它返回的是**进程内句柄**，而句柄交付早已改由编排器直接塞进
//!   `chat_ctx`，不走路由。
//! - `clear`（会话）→ `delete(<根>/session/<id>)`。
//! - `chat/clear_messages` → `action(<id>/消息, "clear")`。
//! - `chat/delete_message` → `action(<id>/消息/<mid>, "truncate")`。
//! - `chat/update_message` → `write(<id>/消息/<mid>)`。
//! - `get_messages` → 子会话**存在性校验**改走进程内 VDFS 纯接口
//!   （`Plugin::get_vfs_provider()` + `stat(<挂载名>/<sid>)`，见同文 §3.4.1）
//!   ——它当时也不是「会话的读接口」，读历史一直是 `vdfs/read`。
//!
//! 后三者的实现搬到了 `plugin/vdfs_provider.rs`（`patch_message` /
//! `truncate_messages` / `clear_messages`）——**搬移不是重写**：同一个操作只有
//! 一份实现，正是本文件收缩的全部意义。

use super::chat_session::{ChatSession, PersistentChatSession};
use super::plugin::SessionPlugin;
use crate::symbio_core::schemas::session::session_update;
use crate::symbio_core::{InvokeRequest, InvokeRequestExt};
use crate::symbio_core::{InvokeResponse, PluginError};
use serde_json::{json, Value};
use std::sync::Arc;

impl SessionPlugin {
    /// 删除会话的统一内部实现（abort 活跃任务 → 清活跃条目 → 存储删除）。
    ///
    /// 唯一消费方：`VdfsProvider::delete`（`delete(<根>/session/<id>)`）。
    /// 曾经的 `session/clear` 路由是它的第二个消费方，已退役——两个入口对同一件事
    /// 就是两条会各自漂移的实现，VDFS 侧本来就已经完整具备这个能力。
    pub(crate) async fn delete_session_internal(
        &self,
        session_id: &str,
    ) -> Result<(), PluginError> {
        // 删除前先 abort 该会话的活跃任务
        let state = self.active_mgr.get_or_create(session_id).await;
        {
            // 置位即中止（无帧、无 await）：与 `handle_abort` 走同一个原语。
            let mut inner = state.inner.write().await;
            if let Some(signal) = inner.abort_signal.take() {
                signal.abort();
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

    /// 合并写入会话 metadata（workdir / title / agent_id 等）。
    ///
    /// **仅 CLI 使用**。前端走 `VdfsProvider::write`（`vdfs/write(<根>/session/<id>)`）
    /// ——两条路径共用 `Session::merge_metadata_object`，语义不可能分叉。
    /// 本路由保留的原因：CLI 需要**客户端指定会话 id**（`cli/src/client.rs` 自己
    /// `gen_id` 后 upsert），而 VDFS 新建会话是 provider 生成 id。
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

        // 合并 metadata + title —— 与 VDFS `write` 同一份实现（见该方法文档）
        session.merge_metadata_object(&json!({
            "metadata": req.metadata,
            "title": req.title,
        }));

        session.updated_at = crate::symbio_core::now_ms();
        self.save_session(&session).await?;

        // VDFS 实时链路（provider 侧变更广播 → watch 的 sink → 总线 kind="vdfs"）：
        // 会话叶子上的**资源**变更一律走粗粒度信号，消费方重拉清单收敛。
        // 运行态不在这里——它走转写流（见 `plugin::notify_change` 的分工表）。
        if is_new {
            self.notify_change(
                &req.session_id,
                crate::symbio_core::vdfs::VDFS_CHANGE_CREATED,
            );
        } else {
            self.notify_change(
                &req.session_id,
                crate::symbio_core::vdfs::VDFS_CHANGE_UPDATED,
            );
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
    /// 消费方**只有一处**：会话编排器向 chat_ctx 交付会话句柄
    /// （`orchestrator/entry.rs` 的 `SESSION_HANDLE`，交付失败时 model 侧兜底内存会话）。
    /// 曾经还有一个 `session/open` 路由消费它（对外返回进程内句柄），已退役——
    /// 进程内句柄不该有对外路由。
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
}

#[cfg(test)]
#[path = "handlers.test.rs"]
mod tests;
