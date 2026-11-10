//! `crates/symbio-kernel/src/capability/row.rs` 的单元测试。
//!
//! 钉两件事：**行的名字与 meta 出自同一次自述**（调用方没有机会填错，两份
//! 表示结构上不分叉）、**meta 是入表时的快照**（行携带事实，行上没有执行对象——
//! `D-440`：工具即端口，执行面在端口收集表上）。

use super::*;
use crate::capability::{CapabilityMeta, CapabilityRiskLevel};
use serde_json::{json, Value};

/// 最小自述桩：名字与风险档可指定。
fn meta_of(name: &str) -> CapabilityMeta {
    CapabilityMeta {
        name: name.to_string(),
        description: "桩".to_string(),
        input_schema: json!({"type": "object"}),
        risk: Some(CapabilityRiskLevel::Medium),
        ..Default::default()
    }
}

/// 行的名字 = 自述的 name（含挂载点前缀的全名），不是调用方另填的。
#[test]
fn a_tool_row_takes_its_name_from_the_meta_self_report() {
    let row = CapabilityRow::tool(meta_of("local/shell"));
    assert_eq!(row.name(), "local/shell");
    assert_eq!(row.family(), family::TOOL);
    assert_eq!(row.meta().name, "local/shell");
}

/// meta 是入表时的**快照**：改后面的自述对象不影响行——行携带事实，
/// 不持有对任何执行对象的反向引用。
#[test]
fn a_tool_row_carries_a_snapshot_of_the_meta() {
    let mut m = meta_of("local/read");
    m.description = "入表时的说明".to_string();
    let row = CapabilityRow::tool(m.clone());
    m.name = "renamed".to_string();
    m.description = "入表后的改动".to_string();
    assert_eq!(row.meta().name, "local/read");
    assert_eq!(row.meta().description, "入表时的说明");
    assert_eq!(row.meta().risk, Some(CapabilityRiskLevel::Medium));
    assert_eq!(row.meta().input_schema, meta_of("local/read").input_schema);
    let _ = Value::Null; // 保持 serde_json 导入的形状稳定（schema 断言已用 json!）
}

/// 新族在**声明方**定义：一个 [`CapabilityFamily`] 常量 + 自己的载荷类型。
///
/// 行词汇只按族过滤、按具体类型下转型——内核没有它的枚举臂，这就是「加一族不改
/// 内核」的最小证据。
#[test]
fn a_family_declared_outside_core_round_trips_its_payload() {
    struct NotePayload {
        text: String,
    }
    const NOTE: CapabilityFamily = CapabilityFamily::multi("test_note");

    let row = CapabilityRow::new(
        NOTE,
        "hello",
        NotePayload {
            text: "正文".to_string(),
        },
        false,
    );

    assert_eq!(row.family(), NOTE);
    assert_eq!(row.family().name(), "test_note");
    assert!(!row.family().is_single());
    assert_eq!(
        row.payload::<NotePayload>().map(|p| p.text.as_str()),
        Some("正文")
    );
    // 读错类型 ⇒ 诚实的 None（不 panic，也不误认成别的族）
    assert!(row.payload::<String>().is_none());
}

/// 模型行的名字 = `provider_id`（表键）；生效位是行的一部分。
///
/// 行里**没有执行对象**（`D-437`）：构造它不需要先有一个能跑的 provider，读它的
/// 人也拿不到一次调用能力——`payload::<T>()` 按族取不到别族的东西。
#[test]
fn a_model_row_carries_id_and_slot() {
    let row = CapabilityRow::model("openai", true);
    assert_eq!(row.name(), "openai");
    assert_eq!(row.family(), family::MODEL);
    assert!(row.active(), "带位的行才占生效槽");
    assert!(
        row.payload::<String>().is_none(),
        "模型行不带执行对象，也不冒充别的族的载荷"
    );
}

/// `demote()` 给的是**退位副本**：位清零、名字与载荷不变，原行不动——表里的行是
/// `Arc`，单槽移交靠换副本入表，不靠就地改位。
#[test]
fn demote_returns_a_copy_with_the_slot_cleared() {
    let row = CapabilityRow::model("openai", true);

    let demoted = row.demote();
    assert!(!demoted.active());
    assert_eq!(demoted.name(), "openai");
    assert_eq!(demoted.family(), family::MODEL, "退位只清位，不换族");
    assert!(row.active(), "原行不受影响");
    // 无槽的族：退位副本就是它自己（工具行没有位可清）
    let tool_row = CapabilityRow::tool(meta_of("local/read"));
    assert!(!tool_row.active());
    assert_eq!(tool_row.demote().name(), "local/read");
}
