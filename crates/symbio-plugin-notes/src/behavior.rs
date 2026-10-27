//! 效果行为：一次成功的 `notes_add` ⇒ 一条 `notes.created` 草稿。
//!
//! 实现只交 [`FactDraft`]，**不碰事实源**：轮末的唯一写口（`loop` 的 `record_derived`）
//! 补上轮锚与主体再落格（D-213）。行为与工具通过 `CapabilityMeta.behaviors` 的**名字**
//! 相连——宿主两侧都不认识 `notes_add` 这个工具名。
//!
//! 幂等键用 `EffectCall::anchor`（轮锚）+ 调用 id：同一轮重放不会把一条便签记两次，
//! 而跨轮同名调用也不会互相吞掉。
//!
//! 前置判断（[`NoteCapture::before`]）是同一份声明的另一半：重复的便签在**执行前**
//! 就被拒，模型看到的是「已经记过」而不是一条被幂等键悄悄吞掉的成功结果。

use symbio_core::{Entity, Verb};
use symbio_plugin_api::{
    behaviors, BeforeDecision, BeforeRefusal, BeforeRequest, EffectCall, FactDraft, ToolBehavior,
};

use super::decl::NOTES_CREATED;

/// 本行为管辖的名字。它是**宿主词汇**（`symbio_plugin_api::behaviors`），
/// 工具在自己的 `CapabilityMeta.behaviors` 里声明同一个常量。
pub(crate) const NOTE_CAPTURE: &str = behaviors::NOTE_CAPTURE;

/// 参数 / 事实载荷里的正文：去空白后的非空 `text`。写方与判方共用同一口径。
fn note_text(value: &serde_json::Value) -> Option<&str> {
    let text = value.get("text")?.as_str()?.trim();
    (!text.is_empty()).then_some(text)
}

/// 一条成功的调用 = 一条便签。失败的调用没有效果（`success == false` ⇒ 空）。
pub(crate) struct NoteCapture;

impl ToolBehavior for NoteCapture {
    fn owner(&self) -> &'static str {
        "notes"
    }

    fn behavior(&self) -> &'static str {
        NOTE_CAPTURE
    }

    /// 同一会话里已经记过同一条便签 ⇒ 拒绝这次写入。
    ///
    /// 判据只看 `req.facts` 里的 `notes.created`（本插件自己的事件名）：既不需要知道
    /// 工具名，也不需要任何跨插件约定——换上另一个写便签的工具，这条判断照样成立。
    fn before(&self, req: &BeforeRequest<'_>) -> BeforeDecision {
        let Some(text) = note_text(req.args) else {
            return BeforeDecision::Allow;
        };
        if !req
            .facts
            .iter()
            .any(|e| e.kind == NOTES_CREATED && note_text(&e.payload) == Some(text))
        {
            return BeforeDecision::Allow;
        }
        BeforeDecision::Refuse(BeforeRefusal::new(
            "便签",
            format!("这条便签已经记过（同一会话内重复）：{text}"),
            symbio_core::failure_kind::ERROR,
        ))
    }

    fn after(&self, call: &EffectCall<'_>) -> Vec<FactDraft> {
        if !call.success {
            return Vec::new();
        }
        let Some(text) = note_text(call.args) else {
            return Vec::new();
        };
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
