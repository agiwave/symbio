//! 效果行为：一次成功的 `glossary_define` ⇒ 一条 `glossary.defined` 草稿。
//!
//! 实现只交 [`FactDraft`]，**不碰事实源**：唯一的写口补上宿主面再落格。
//! 行为与工具通过 `CapabilityMeta.behaviors` 的**名字**相连——宿主两侧都不认识
//! `glossary_define` 这个工具名。
//!
//! 幂等键用 `EffectCall::anchor`（宿主给的不透明词干）+ 调用 id：同一次重放不会把
//! 一条定义记两次，而跨请求同名调用也不会互相吞掉。
//!
//! 前置判断（[`GlossaryRecord::before`]）是同一份声明的另一半：同一个术语用**同样的
//! 释义**再定义一次，在**执行前**就被拒，模型看到的是「已经定义过」而不是一条被幂等键
//! 悄悄吞掉的成功结果。换一条释义则放行——术语表允许覆盖。

use symbio_plugin_api::behaviors::Kind;
use symbio_plugin_api::{
    BeforeDecision, BeforeRefusal, BeforeRequest, EffectCall, FactDraft, ToolBehavior,
};
use symbio_plugin_api::{Entity, Verb};

use super::decl::{GLOSSARY_DEFINED, MAX_DEFINITION_CHARS, MAX_TERM_CHARS};

/// 本行为管辖的名字。由本 crate 自报：工具在自己的 `CapabilityMeta.behaviors` 里
/// 声明同一个常量，宿主只做收集。
pub(crate) const GLOSSARY_RECORD: &str = "glossary_record";

/// 参数 / 事实载荷里的术语名：去空白后的非空串，长度在声明上界内。
fn term_of(value: &serde_json::Value) -> Option<&str> {
    let term = value.get("term")?.as_str()?.trim();
    (!term.is_empty() && term.chars().count() <= MAX_TERM_CHARS).then_some(term)
}

/// 释义：与术语同一口径。
fn definition_of(value: &serde_json::Value) -> Option<&str> {
    let definition = value.get("definition")?.as_str()?.trim();
    (!definition.is_empty() && definition.chars().count() <= MAX_DEFINITION_CHARS)
        .then_some(definition)
}

/// 一条成功的调用 = 一条定义。失败的调用没有效果。
pub(crate) struct GlossaryRecord;

impl ToolBehavior for GlossaryRecord {
    fn owner(&self) -> &'static str {
        super::decl::OWNER
    }

    fn behavior(&self) -> &'static str {
        GLOSSARY_RECORD
    }

    fn kind(&self) -> Kind {
        Kind::LandsFacts
    }

    /// 同一会话里已经用同样的释义定义过同一个术语 ⇒ 拒绝这次写入。
    ///
    /// 判据只看 `req.facts` 里的 `glossary.defined`（本插件自己的事件名）：既不需要
    /// 知道工具名，也不需要任何跨插件约定。
    fn before(&self, req: &BeforeRequest<'_>) -> BeforeDecision {
        let (Some(term), Some(definition)) = (term_of(req.args), definition_of(req.args)) else {
            return BeforeDecision::Allow;
        };
        let already = req.facts.iter().any(|e| {
            e.kind == GLOSSARY_DEFINED
                && term_of(&e.payload) == Some(term)
                && definition_of(&e.payload) == Some(definition)
        });
        if !already {
            return BeforeDecision::Allow;
        }
        BeforeDecision::Refuse(BeforeRefusal::new(
            "术语",
            format!("这个术语已经用同样的释义定义过：{term}"),
            symbio_plugin_api::failure_kind::ERROR,
        ))
    }

    fn after(&self, call: &EffectCall<'_>) -> Vec<FactDraft> {
        if !call.success {
            return Vec::new();
        }
        let (Some(term), Some(definition)) = (term_of(call.args), definition_of(call.args)) else {
            return Vec::new();
        };
        vec![FactDraft {
            key: format!("g-{}-{}", call.anchor, call.call_id),
            kind: GLOSSARY_DEFINED.to_string(),
            entity: Entity::Domain,
            verb: Verb::Opened,
            payload: serde_json::json!({ "term": term, "definition": definition }),
            produced_by: Some(call.user_seq),
        }]
    }
}

// 链接期提交：本实现随本 crate 进入二进制，宿主按声明选它。
symbio_plugin_api::submit_behavior!(GlossaryRecord);

#[cfg(test)]
#[path = "behavior.test.rs"]
mod tests;
