//! 上下文贡献：把本会话已定义的术语渲染成一段。
//!
//! provider 是**纯函数**：入参是本轮事实切片，出参是一段文本；它不写事实、不改本轮
//! 状态，也不参与「欠什么 / 何时停」的判断。
//!
//! 同一个术语定义过多次时只渲染**最新**那条释义——术语表的语义是「现在的说法」，
//! 历史释义仍在事实源里，但不再占每轮的上下文成本。
//!
//! 段名 `glossary_context` 由声明方自带；拼接按 owner 序，位置与装配次序无关。

use symbio_plugin_api::{ContextProvider, ContextRequest, ContextSection};

use super::decl::{GLOSSARY_DEFINED, MAX_DEFINITION_CHARS, MAX_TERM_CHARS};

/// 本段最多回显几条术语：上下文是每轮都付的成本。
const MAX_TERMS: usize = 20;

/// 段身份（`ChatNode.meta.kind`）。
pub(crate) const SLOT: &str = "glossary_context";

/// 段优先级：术语表是**参考**，排在三类宿主段之后、便签之前。
const PRIO: u8 = 50;

fn term_of(value: &serde_json::Value) -> Option<&str> {
    let term = value.get("term")?.as_str()?.trim();
    (!term.is_empty() && term.chars().count() <= MAX_TERM_CHARS).then_some(term)
}

fn definition_of(value: &serde_json::Value) -> Option<&str> {
    let definition = value.get("definition")?.as_str()?.trim();
    (!definition.is_empty() && definition.chars().count() <= MAX_DEFINITION_CHARS)
        .then_some(definition)
}

pub(crate) struct GlossaryProvider;

impl ContextProvider for GlossaryProvider {
    fn owner(&self) -> &'static str {
        super::decl::OWNER
    }

    fn provide(&self, req: &ContextRequest<'_>) -> Vec<ContextSection> {
        // 术语按**首次定义**的顺序排列；再次定义只更新释义，不改变位置。
        let mut terms: Vec<(String, String)> = Vec::new();
        for event in req.facts.iter().filter(|e| e.kind == GLOSSARY_DEFINED) {
            let (Some(term), Some(definition)) =
                (term_of(&event.payload), definition_of(&event.payload))
            else {
                continue;
            };
            match terms.iter_mut().find(|(known, _)| known.as_str() == term) {
                Some(slot) => slot.1 = definition.to_string(),
                None => terms.push((term.to_string(), definition.to_string())),
            }
        }
        if terms.is_empty() {
            return Vec::new();
        }
        if terms.len() > MAX_TERMS {
            terms.drain(..terms.len() - MAX_TERMS);
        }
        let mut text = String::from("项目术语表（本会话已定义）：");
        for (term, definition) in &terms {
            text.push_str("\n- ");
            text.push_str(term);
            text.push('：');
            text.push_str(definition);
        }
        vec![ContextSection::new(SLOT, text, PRIO)]
    }
}

symbio_plugin_api::submit_context_provider!(GlossaryProvider);

#[cfg(test)]
#[path = "context.test.rs"]
mod tests;
