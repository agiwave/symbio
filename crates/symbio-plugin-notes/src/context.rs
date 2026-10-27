//! 上下文贡献：把本会话最近记下的便签渲染成一段。
//!
//! provider 是**纯函数**（D-215）：入参是 `ContextRequest` 里的值（本轮事实切片），
//! 出参是一段文本；它不写事实、不改本轮状态，也不参与「欠什么 / 何时停」的判断。
//!
//! 段名 `notes_context` 由**声明方自带**（`slots::KNOWN` 只登记宿主当前的三个名字，
//! 新名字走同一条开放集合，见 `symbio_plugin_api::context`）。拼接按 owner 序，
//! 因此这个段的位置由 `"notes"` 与兄弟 provider 的字典序决定，与装配次序无关。

use symbio_plugin_api::{ContextProvider, ContextRequest, ContextSection};

use super::decl::NOTES_CREATED;

/// 本段最多回显几条便签：上下文是每轮都付的成本，最近的几条足够提示模型。
const MAX_NOTES: usize = 5;

/// 段身份（`ChatNode.meta.kind`）。
pub(crate) const SLOT: &str = "notes_context";

pub(crate) struct NotesProvider;

impl ContextProvider for NotesProvider {
    fn owner(&self) -> &'static str {
        "notes"
    }

    fn provide(&self, req: &ContextRequest<'_>) -> Vec<ContextSection> {
        let notes: Vec<&str> = req
            .facts
            .iter()
            .filter(|e| e.kind == NOTES_CREATED)
            .filter_map(|e| e.payload.get("text").and_then(|v| v.as_str()))
            .filter(|text| !text.trim().is_empty())
            .collect();
        if notes.is_empty() {
            return Vec::new();
        }
        let tail = &notes[notes.len().saturating_sub(MAX_NOTES)..];
        let mut text = String::from("最近记下的便签：");
        for note in tail {
            text.push_str("\n- ");
            text.push_str(note.trim());
        }
        vec![ContextSection { slot: SLOT, text }]
    }
}

symbio_plugin_api::submit_context_provider!(NotesProvider);

#[cfg(test)]
#[path = "context.test.rs"]
mod tests;
