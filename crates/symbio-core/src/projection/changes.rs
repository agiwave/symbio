//! 工作目录的变更账：从 `artifact.added` 折出「本会话改过哪些文件」。
//!
//! ## 为什么它是纯投影而不是一件新事实（[D-169]）
//!
//! 工作目录的真实内容由文件系统持有。再写一条 `file.*` 事实，只是给本体加一份影子：
//! 影子与本体之间没有机制强制一致，而不一致时两边都自证正确。变更的**来源**本来就是
//! 一次工具调用，`artifact.added` 已经记下 `tool` / 定稿参数 `args`（含 `path`）/
//! `success` / `produced_by`——清单折得出来，就不该有第二个写点。
//!
//! 写侧也不在 VDFS 手上：agent 的 `vdfs_*` 工具走 `UnifiedFs → PhysicalFs`，
//! shell / MCP / 操作系统的写入完全不经过 VDFS，`FsWatcher` 只在界面订阅时启动且
//! 没有轮归属。竖在这条路径上的写点只能记下「经由它自己的变更」，却会被读成
//! 「工作目录的变更」。
//!
//! ## 口径
//!
//! - 只认 `success: true` 的写族工具：`vdfs_write` / `vdfs_edit` / `vdfs_delete` /
//!   `vdfs_mkdir`（读、搜索、统计不是变更）；
//! - 路径取**定稿参数** `args.path`——工具收到什么就记什么，不去反推结果文本；
//! - 路径必须落在工作目录内：绝对路径、`..` 逃逸与**虚拟根**（`AddrRootDecl` 声明的
//!   挂载名，那是资源树不是工作目录）下的路径不计入；
//! - 同一路径**最新一条胜出**，`changes` 数它被成功改过几次；
//! - `vdfs_edit` 的 `replaced: 0`（内容已是最新）不算变更：从结果文本里读，
//!   没有该字段的旧事件按变更计入——「不知道」不等于「没改」。
//!
//! ## 边界
//!
//! 用户编辑器保存、shell / MCP / 其它进程的写入没有可溯源的轮归属，留在实时面
//! （`FsWatcher` → `VdfsChange`），不入这条账。本账**不承诺重建旧内容**：它记的是
//! 「谁在哪一轮改过这个路径」，不是写入前的全文。
//!
//! [D-169]: ../../../../../docs/003-design/001-核心架构.md "DE-001-核心架构"

use super::super::event::Event;
use super::Projection;
use crate::MessageVocab;

use std::collections::HashMap;

/// 写族工具的四种动作（判别值走 [`FileOp::as_str`]，呈现文案由出口给）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileOp {
    Write,
    Edit,
    Delete,
    Mkdir,
}

impl FileOp {
    /// 工具短名 → 动作。只有写族有动作；其余工具不是变更。
    fn of_tool(tool: &str) -> Option<Self> {
        match tool {
            "vdfs_write" => Some(Self::Write),
            "vdfs_edit" => Some(Self::Edit),
            "vdfs_delete" => Some(Self::Delete),
            "vdfs_mkdir" => Some(Self::Mkdir),
            _ => None,
        }
    }

    /// 线格式的判别词——与 `#[serde(rename_all = "snake_case")]` 逐字一致。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Write => "write",
            Self::Edit => "edit",
            Self::Delete => "delete",
            Self::Mkdir => "mkdir",
        }
    }
}

/// 一个被改过的文件（每路径一条）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    /// 工作目录内相对路径，`/` 分隔（跨平台同一形状）。
    pub path: String,
    /// 最后一次成功的动作。
    pub op: FileOp,
    /// 最后一次改它的轮号。
    pub turn: u64,
    /// 本会话对它成功写入的次数。
    pub changes: u64,
}

/// 变更账投影：最近改过的在前。
///
/// 顺序取「最后一次出现的位置」而不是 `path` 字典序：清单的用途是「刚改了什么」，
/// 而字典序只回答「字母表里谁在前」。
pub fn workdir_changes(v: MessageVocab) -> Projection<Vec<FileChange>> {
    Projection::new(move |events: &[Event], now| {
        let mut by_path: HashMap<String, (usize, FileChange)> = HashMap::new();
        for (idx, e) in events.iter().enumerate() {
            if e.ts > now || e.kind != v.artifact_added {
                continue;
            }
            let Some((op, path)) = change_of(e) else {
                continue;
            };
            let entry = by_path.entry(path.clone()).or_insert_with(|| {
                (
                    idx,
                    FileChange {
                        path,
                        op,
                        turn: e.turn,
                        changes: 0,
                    },
                )
            });
            entry.0 = idx;
            entry.1.op = op;
            entry.1.turn = e.turn;
            entry.1.changes += 1;
        }
        let mut rows: Vec<(usize, FileChange)> = by_path.into_values().collect();
        rows.sort_by(|a, b| b.0.cmp(&a.0));
        rows.into_iter().map(|(_, row)| row).collect()
    })
}

/// 一条产物事实 → 一次工作目录内的变更（不是变更 ⇒ `None`）。
fn change_of(e: &Event) -> Option<(FileOp, String)> {
    let payload = &e.payload;
    if payload.get("success").and_then(|v| v.as_bool()) != Some(true) {
        return None;
    }
    let op = FileOp::of_tool(payload.get("tool").and_then(|v| v.as_str())?)?;
    // 定稿参数：生产侧是 JSON 文本（`args` 字符串），存量/测试可能直接给对象。
    let args = payload.get("args")?;
    let args = match args {
        serde_json::Value::String(s) => serde_json::from_str::<serde_json::Value>(s).ok()?,
        v => v.clone(),
    };
    let raw = args.get("path").and_then(|v| v.as_str())?;
    let path = inside_workdir(raw)?;
    if op == FileOp::Edit && edit_was_noop(payload) {
        return None;
    }
    Some((op, path))
}

/// `vdfs_edit` 的结果是否「没有替换任何东西」。
///
/// 字段来自 `vdfs_edit` 的结果 JSON（`{ success, path, replaced, message }`）。
/// 缺席 = 该事件写在补充字段之前 ⇒ 按变更计入（见模块头「不知道 ≠ 没改」）。
fn edit_was_noop(payload: &serde_json::Value) -> bool {
    payload
        .get("text")
        .and_then(|v| v.as_str())
        .and_then(|text| serde_json::from_str::<serde_json::Value>(text).ok())
        .and_then(|result| result.get("replaced").and_then(|v| v.as_u64()))
        == Some(0)
}

/// 归一成工作目录内的相对路径；界外 / 虚拟根 / 空路径 ⇒ `None`。
///
/// 虚拟根的名字由 vdfs 插件经 [`AddrRootDecl`] 声明，core 只持有**声明**、
/// 不持有字面量——所以这里问声明，不认某个拼法。
fn inside_workdir(raw: &str) -> Option<String> {
    let normalized = raw.trim().replace('\\', "/");
    if normalized.is_empty() || normalized.starts_with('/') {
        return None;
    }
    // Windows 盘符（`C:/…`）也是绝对路径。
    if normalized.as_bytes().get(1) == Some(&b':') {
        return None;
    }
    let mut segments: Vec<&str> = Vec::new();
    for seg in normalized.split('/') {
        match seg {
            "" | "." => continue,
            ".." => return None,
            s => segments.push(s),
        }
    }
    if segments.is_empty() {
        return None;
    }
    if crate::vdfs::declared_addr_root() == Some(segments[0]) {
        return None;
    }
    Some(segments.join("/"))
}

#[cfg(test)]
#[path = "changes.test.rs"]
mod tests;
