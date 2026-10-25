//! `session/changes` —— 本会话「改过哪些文件」的只读出口。
//!
//! ## 它是一条投影的出口，不是新事实族（[D-169]）
//!
//! 变更账由 `symbio_core::workdir_changes` 从 `artifact.added` 折出（写侧为什么
//! 不该另立 `file.*` 见该投影的模块文档）。本文件只做两件事：**取数**（本会话快照）
//! 与**排版**（相对路径 → VDFS 地址 + 判别词 + 文案）。它不解析工具参数、不判
//! 「哪些工具算写」——那些口径只在 core 的投影里。
//!
//! ## 为什么行是呈现就绪的
//!
//! 前端需要的三件事在这里一次给全：`address`（`<sid>/workdir/<path>`，直接喂
//! `vdfs/read`）、`label`（展示文案）、`op`（判别词，给样式 / 测试用，**不是**
//! 让前端自己翻译成文案）。让前端认识 `vdfs_write` / `vdfs_edit` 这些名字，
//! 等于把「哪些工具算写」在两端各放一份判据，迟早分叉——前端只认这份描述。
//!
//! ## 边界
//!
//! 面只覆盖智能体写族工具的**成功**调用。用户编辑器保存、shell / MCP / 其它进程
//! 的写入没有可溯源的轮归属，留在实时面（`FsWatcher` → `VdfsChange`），不入这条账。
//! 清单也不承诺重建旧内容：打开的是**当前**文件。
//!
//! ## 读侧闸
//!
//! 与 `session/stats` 同一道：声明了读方身份且看不到本会话内容 ⇒ **空清单**
//! （`has_wal: true` + 空行 = 有源但不给你看，不是「没改过」）。
//!
//! [D-169]: ../../../../docs/003-design/001-核心架构.md "DE-001-核心架构"

use std::sync::Arc;

use serde::Serialize;

use crate::symbio_core::{
    content_visible, workdir_changes, Event, FileOp, PluginError, PluginInvokeRequest,
    PluginInvokeRequestExt, PluginInvokeResponse, PluginPayload, SESSION_ID,
};

use super::plugin::SessionPlugin;
use super::stats_reading::{content_scope_of, ViewerRequest};
use super::workdir::SEG_WORKDIR;

/// 一行变更：**呈现就绪**（前端只排版，不判读）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct ChangeRow {
    /// 工作目录内的相对路径（`/` 分隔）。
    pub path: String,
    /// 可直接交给 `vdfs/read` 的会话地址。
    pub address: String,
    /// 判别词（`write` / `edit` / `delete` / `mkdir`）。
    pub op: &'static str,
    /// 展示文案（动作的人话，前端不认 [`Self::op`]）。
    pub label: &'static str,
    /// 最后一次改它的轮号。
    pub turn: u64,
    /// 本会话对它成功写入的次数。
    pub changes: u64,
}

/// 会话变更清单：最近改过的在前。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct SessionChanges {
    pub session_id: String,
    /// 事实源在不在；`false` = 这个会话还没跑过一轮（有据的空，不是错误）。
    pub has_wal: bool,
    pub rows: Vec<ChangeRow>,
}

/// 读一份会话事实源，出变更清单（纯函数：不碰存储、不拼路径）。
pub(crate) fn read_changes(
    session_id: &str,
    snapshot: Option<Vec<Event>>,
    viewer: Option<&str>,
) -> SessionChanges {
    let has_wal = snapshot.is_some();
    let snapshot = snapshot.unwrap_or_default();
    let may_read = match viewer {
        None => true,
        Some(principal) => content_visible(content_scope_of(&snapshot), principal),
    };
    let rows = if may_read {
        workdir_changes()
            .read_all(&snapshot)
            .value
            .into_iter()
            .map(|c| ChangeRow {
                address: format!("{session_id}/{SEG_WORKDIR}/{}", c.path),
                path: c.path,
                op: c.op.as_str(),
                label: op_label(c.op),
                turn: c.turn,
                changes: c.changes,
            })
            .collect()
    } else {
        Vec::new()
    };
    SessionChanges {
        session_id: session_id.to_string(),
        has_wal,
        rows,
    }
}

/// 动作的展示文案——业务词汇只在这一处。
fn op_label(op: FileOp) -> &'static str {
    match op {
        FileOp::Write => "写入",
        FileOp::Edit => "编辑",
        FileOp::Delete => "删除",
        FileOp::Mkdir => "新建目录",
    }
}

impl SessionPlugin {
    pub(crate) async fn handle_changes(
        &self,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let sid = ctx.get(SESSION_ID).unwrap_or_default();
        if sid.is_empty() {
            return Err(PluginError::ValidationError(
                "session/changes 需要 metadata.session_id".into(),
            ));
        }
        // 载荷 = 读方身份声明（可选）；带了却解析不出 ⇒ 拒绝（fail-closed），
        // 与 `session/stats` 同判据。
        let viewer: Option<String> = match ctx.payload::<serde_json::Value>()? {
            serde_json::Value::Null => None,
            v => {
                serde_json::from_value::<ViewerRequest>(v)
                    .map_err(|e| {
                        PluginError::ValidationError(format!("session/changes 载荷不合法：{e}"))
                    })?
                    .principal
            }
        };
        let store = self.get_store().await?;
        let changes = read_changes(&sid, store.snapshot(&sid), viewer.as_deref());
        Ok(PluginPayload::new(&changes))
    }
}

#[cfg(test)]
#[path = "changes.test.rs"]
mod tests;
