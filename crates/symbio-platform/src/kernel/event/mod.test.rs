//! 事件信封的**当前词面**：只写当前词，不复活已退场的字段。

use super::{Entity, Event};

/// 实体词只有 `Scope` 一种写法；信封写侧不再有 `scope` 或旧的 `turn` 字段
/// （会话身份改由事实源在打开时绑定， /  B7）。
#[test]
fn envelope_writes_only_current_entity_word() {
    let fresh_json = r#"{"event_id":"e-1","kind":"chat.assistant.final","entity":"Scope","verb":"Closed","actor":"main","produced_by":null,"cost_ms":0,"ts":0,"payload":null,"seq":null}"#;
    let event: Event = serde_json::from_str(fresh_json).expect("当前实体词应可读");
    assert_eq!(event.entity, Entity::Scope);
    assert_eq!(event.event_id, "e-1");

    let fresh = serde_json::to_value(&event).expect("写侧应可序列化");
    assert_eq!(fresh.get("entity"), Some(&serde_json::json!("Scope")));
}
