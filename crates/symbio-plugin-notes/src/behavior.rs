//! 效果行为：一次成功的 `notes_add` ⇒ 一条 `notes.created` 草稿。
//!
//! 实现只交 [`FactDraft`]，**不碰事实源**：轮末的唯一写口（`loop` 的 `record_derived`）
//! 补上轮锚与主体再落格（D-213）。行为与工具通过 `CapabilityMeta.behaviors` 的**名字**
//! 相连——宿主两侧都不认识 `notes_add` 这个工具名。
//!
//! 幂等键用 `EffectCall::anchor`（轮锚）+ 调用 id：同一轮重放不会把一条便签记两次，
//! 而跨轮同名调用也不会互相吞掉。

use symbio_core::{Entity, Verb};
use symbio_plugin_api::{behaviors, EffectCall, FactDraft, ToolBehavior};

use super::decl::NOTES_CREATED;

/// 本行为管辖的名字。它是**宿主词汇**（`symbio_plugin_api::behaviors`），
/// 工具在自己的 `CapabilityMeta.behaviors` 里声明同一个常量。
pub(crate) const NOTE_CAPTURE: &str = behaviors::NOTE_CAPTURE;

/// 一条成功的调用 = 一条便签。失败的调用没有效果（`success == false` ⇒ 空）。
pub(crate) struct NoteCapture;

impl ToolBehavior for NoteCapture {
    fn owner(&self) -> &'static str {
        "notes"
    }

    fn behavior(&self) -> &'static str {
        NOTE_CAPTURE
    }

    fn after(&self, call: &EffectCall<'_>) -> Vec<FactDraft> {
        if !call.success {
            return Vec::new();
        }
        let Some(text) = call.args.get("text").and_then(|v| v.as_str()) else {
            return Vec::new();
        };
        let text = text.trim();
        if text.is_empty() {
            return Vec::new();
        }
        vec![FactDraft {
            key: format!("n-{}-{}", call.anchor(), call.call_id),
            kind: NOTES_CREATED.to_string(),
            entity: Entity::Domain,
            verb: Verb::Opened,
            payload: serde_json::json!({ "text": text }),
            produced_by: Some(call.user_seq),
        }]
    }
}

// 链接期提交：本实现随本 crate 进入二进制，宿主按声明选它。
symbio_plugin_api::submit_behavior!(NoteCapture);

#[cfg(test)]
#[path = "behavior.test.rs"]
mod tests;
