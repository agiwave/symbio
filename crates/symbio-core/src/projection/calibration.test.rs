//! `calibration` 投影的回归测试
//!
//! ## 它防的是哪种失效
//!
//! 校准读侧曾**完全不按格子过滤**：扫全部事件，靠「载荷带 `skill_id` + `fallback`」
//! 认自己的输入。那样写的时候，读侧的正确性依赖一个巧合——别的格子恰好不带这两个
//! 字段。巧合可以被任何一次新增打破，而打破之后**没有任何东西会红**：
//! 一个新格子带了同名字段，置信度就凭空被算歪。
//!
//! 所以这里钉三件事：
//! ① 只有 `memory.calibrated` 的事件进统计（别的格子带同名字段也**不算**）；
//! ② 格子对了但字段缺 ⇒ 不参与统计（不是「未回退」）；
//! ③ 回退率与置信度按已知动作钉死期望值。

use super::calibration;
use super::CalibrationView;
use crate::event::EVENT_MEMORY_CALIBRATED;
use crate::view::Budget;
use crate::{Entity, Event, Verb};

/// 一条 `memory × asserted` 事件（校准观测就落在这一族）。
fn ev(id: i64, kind: &str, payload: serde_json::Value) -> Event {
    Event::pending(
        format!("e-{id}"),
        kind,
        Entity::Memory,
        Verb::Asserted,
        1,
        "session/main",
    )
    .with_ts(id)
    .with_payload(payload)
}

fn view_of(events: &[Event]) -> CalibrationView {
    calibration()
        .apply(events, i64::MAX, Budget::generous())
        .value
}

#[test]
fn 只统计校准格_同名字段的别的格子不算() {
    let events = vec![
        ev(
            1,
            EVENT_MEMORY_CALIBRATED,
            serde_json::json!({"skill_id":"sk-1","fallback":false}),
        ),
        // 两个「长得像」的别的格子：一个带同名字段，一个不带
        // `memory.recalled` / `memory.consolidated` 是 loop 插件声明的事件名；
        // core 的夹具按线上名字造，不从 owner 借常量（跨插件零依赖）。
        ev(
            2,
            "memory.recalled",
            serde_json::json!({"skill_id":"sk-2","fallback":true}),
        ),
        ev(
            3,
            "memory.consolidated",
            serde_json::json!({"skill_id":"sk-3"}),
        ),
    ];
    let v = view_of(&events);
    assert_eq!(
        v.by_skill.keys().cloned().collect::<Vec<_>>(),
        vec!["sk-1".to_string()],
        "只有 `memory.calibrated` 进统计；带同名字段的 `memory.recalled` 不算"
    );
    assert_eq!(v.of("sk-1").uses, 1);
    assert_eq!(v.of("sk-1").fallbacks, 0);
}

#[test]
fn 格子对了字段缺_不参与统计而不是记成未回退() {
    let events = vec![
        ev(
            1,
            EVENT_MEMORY_CALIBRATED,
            serde_json::json!({"skill_id":"sk-1","fallback":false}),
        ),
        ev(
            2,
            EVENT_MEMORY_CALIBRATED,
            serde_json::json!({"skill_id":"sk-1"}),
        ),
    ];
    let v = view_of(&events);
    // 缺字段的那条**不计入**：记成「未回退」会让 uses 涨、fallbacks 不涨，
    // 凭空造出高置信度——那正是回退机制失效的形态。
    assert_eq!(v.of("sk-1").uses, 1, "缺字段的事件不应让 uses 增长");
    assert_eq!(v.of("sk-1").fallbacks, 0);
}

#[test]
fn 回退计数与置信度按已知动作钉死() {
    let events = vec![
        ev(
            1,
            EVENT_MEMORY_CALIBRATED,
            serde_json::json!({"skill_id":"sk-1","fallback":false}),
        ),
        ev(
            2,
            EVENT_MEMORY_CALIBRATED,
            serde_json::json!({"skill_id":"sk-1","fallback":true}),
        ),
        ev(
            3,
            EVENT_MEMORY_CALIBRATED,
            serde_json::json!({"skill_id":"sk-1","fallback":true}),
        ),
    ];
    let v = view_of(&events);
    assert_eq!(v.of("sk-1").uses, 3);
    assert_eq!(v.of("sk-1").fallbacks, 2);
    assert!((v.of("sk-1").confidence() - (1.0 - 2.0 / 3.0)).abs() < 1e-12);
    // 零使用 ⇒ 1.0（不给新技能判死刑）
    assert_eq!(v.of("never-used").confidence(), 1.0);
}

#[test]
fn as_of_之前不可见() {
    let events = vec![ev(
        5,
        EVENT_MEMORY_CALIBRATED,
        serde_json::json!({"skill_id":"sk-1","fallback":true}),
    )];
    let v = calibration().apply(&events, 4, Budget::new(0, 0)).value;
    assert!(v.by_skill.is_empty(), "as-of 之前的观测不得计入");
}
