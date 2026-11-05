//! 事件信封的**词面兼容**：旧词仍要读得回来，但不再复活已退场的字段。

use super::{Entity, Event};

/// 旧词 `turn` / `Turn` 都落到 `Scope`；旧信封字段 `turn` 被忽略（信封不再有该字段）。
///
/// 这条守的是**迁移动词的可回放性**：删变体之后，WAL 里已经写成旧词的事实不能变成
/// 「读不回来」的数据——那会把一次语法改写变成一次历史销毁。`scope` 已从信封退场
/// （会话身份改由事实源在打开时绑定），旧记录里的 `turn` 不再用于分域。
#[test]
fn legacy_turn_words_deserialize_into_scope() {
    let lower: Entity = serde_json::from_str("\"turn\"").expect("旧小写实体词应可读");
    let variant: Entity = serde_json::from_str("\"Turn\"").expect("旧变体名应可读");
    assert_eq!(lower, Entity::Scope);
    assert_eq!(variant, Entity::Scope);

    let legacy = r#"{"event_id":"e-1","kind":"chat.assistant.final","entity":"Turn","verb":"Closed","turn":7,"actor":"main","produced_by":null,"cost_ms":0,"ts":0,"payload":null,"seq":null}"#;
    let event: Event = serde_json::from_str(legacy).expect("旧信封字段应可读");
    assert_eq!(event.entity, Entity::Scope);
    assert_eq!(event.event_id, "e-1");

    let fresh = serde_json::to_value(&event).expect("写侧应可序列化");
    assert!(fresh.get("scope").is_none(), "写侧不再写 scope");
    assert!(fresh.get("turn").is_none(), "写侧不再写旧字段名");
}
