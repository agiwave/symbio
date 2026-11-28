//! `crate::sdk::sandbox` 的判据单测：旧查询句柄缺席、发布唯一性、开放事实词表。

use super::*;
use crate::kernel::plugin::{PluginInvokeRequest, PluginSimpleRequest};

struct AllowAll;
impl SandboxFace for AllowAll {
    fn permits(&self, _facts: &[SandboxFact]) -> bool {
        true
    }
    fn intercept(&self, _tool_name: &str, _args: &Value) -> Option<Value> {
        None
    }
}

struct DenyAll;
impl SandboxFace for DenyAll {
    fn permits(&self, _facts: &[SandboxFact]) -> bool {
        false
    }
    fn intercept(&self, _tool_name: &str, _args: &Value) -> Option<Value> {
        Some(serde_json::json!({ "blocked": true }))
    }
}

fn ctx() -> Arc<dyn PluginInvokeRequest> {
    Arc::new(PluginSimpleRequest::new(None, None))
}

#[test]
fn absent_slot_has_no_legacy_face() {
    let c = ctx();
    assert!(
        sandbox_of(c.as_ref()).is_none(),
        "旧查询句柄缺席；新审批执行闸门另行失败关闭"
    );
}

#[test]
fn published_face_is_visible_to_consumers() {
    let c = ctx();
    sandbox_init_slot(&c);
    assert!(sandbox_publish(&c, Arc::new(DenyAll)));
    let face = sandbox_of(c.as_ref()).expect("发布后消费方应读到句柄");
    assert!(!face.permits(&[SandboxFact::new("shell_command", "rm -rf /")]));
    assert!(face.intercept("any_tool", &serde_json::json!({})).is_some());
}

#[test]
fn a_second_publish_is_rejected_not_silently_overwritten() {
    let c = ctx();
    assert!(sandbox_publish(&c, Arc::new(AllowAll)));
    assert!(
        !sandbox_publish(&c, Arc::new(DenyAll)),
        "第二次发布必须被拒，不静默顶掉"
    );
    // 先发布者仍在：句柄不被后来者改写。
    let face = sandbox_of(c.as_ref()).expect("句柄仍在");
    assert!(face.permits(&[]), "先发布的 AllowAll 未被覆盖");
}

#[test]
fn default_decision_combines_existing_queries() {
    let face = DenyAll;
    let decision = face.decide(&[], "any_tool", &serde_json::json!({}));
    assert!(!decision.permitted);
    assert_eq!(
        decision.intercept,
        Some(serde_json::json!({"blocked": true}))
    );
    assert!(decision.reason.is_some());
}

#[test]
fn facts_are_open_strings_with_no_vocabulary_gate() {
    // kind 是开放词表：任何新种类都能进，沙箱按 kind 配规则，SDK 不认识具体值。
    let f = SandboxFact::new("brand_new_kind_from_a_future_tool", "opaque-value");
    assert_eq!(f.kind, "brand_new_kind_from_a_future_tool");
    assert_eq!(f.value, "opaque-value");
}
