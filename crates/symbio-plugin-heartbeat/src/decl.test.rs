//! `#48` 的最小能力面：**名字 + 格位 + 载荷字段 + 归宿行**。
//!
//! 这一批只交声明面，所以用例钉的也正是这四样：写方不在时，任何一样缺失都会让装配期
//! 的 `seal` 判红（`OrphanFact` / `UnknownSource`），而不是等运行期表现为「这条能力
//! 什么都没发生」。同时钉住「不进共享表」——没有第二个消费方的名字进 `shared_events`
//! 就是一条没人读的登记（`CS-007`）。

use super::*;

#[test]
fn cadence_ships_a_name_cell_field_and_row_without_a_writer() {
    let event = EVENTS
        .iter()
        .find(|e| e.name == EVENT_HEARTBEAT_CADENCE)
        .expect("节拍事实必须在 EVENTS 里");
    assert_eq!(event.cell, (Entity::Control, Verb::Asserted));
    assert!(!event.host, "它不是宿主写口：写方接上之后由能力包自己落格");

    let fact = FACTS
        .iter()
        .find(|f| f.name == EVENT_HEARTBEAT_CADENCE)
        .expect("节拍事实必须在 FACTS 里");
    assert_eq!(fact.fields, &["reason"]);

    let row = ROWS
        .iter()
        .find(|r| r.name == EVENT_HEARTBEAT_CADENCE)
        .expect("归宿行缺失会被 seal 判成 OrphanFact");
    assert!(row.by.is_none(), "写方还没接：行不假装有人跑它");
    assert_eq!(row.owes.source, EVENT_HEARTBEAT_CADENCE);
    assert!(
        row.until
            .iter()
            .any(|p| p.source == EVENT_HEARTBEAT_CADENCE),
        "自读即闭合：出口必须与 owes 同源"
    );

    assert!(
        !symbio_plugin_api::events::SHARED_EVENTS
            .iter()
            .any(|e| e.name == EVENT_HEARTBEAT_CADENCE),
        "单消费方的名字随 owner 走，不进共享表（CS-007）"
    );
}
