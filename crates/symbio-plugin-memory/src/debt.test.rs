//! 巩固债的判据（D-268 订正 / D-387 Q3）：**何时不申报**、**同一活集同 id**。
//!
//! 这些是生产判据：`verify/consolidation_fidelity.rs` 只证合并模型，不覆盖「债该不该
//! 欠、欠几次」。

use super::*;
use symbio_plugin_api::debt::DebtFacts;

const ENCODED: &str = EVENT_MEMORY_ENCODED;
const CONSOLIDATED: &str = EVENT_MEMORY_CONSOLIDATED;

fn request() -> DriveRequest {
    DriveRequest {
        session_id: "s1".to_string(),
        provider: Some("test".to_string()),
        principal: "user".to_string(),
    }
}

/// 记一条事实的身份 + 载荷（宿主给申报方的就是这两样）。
fn facts(entries: &[(&str, &str, &str)]) -> DebtFacts {
    let mut facts = DebtFacts::default();
    for (kind, id, tag) in entries {
        facts.note_fact(kind, id);
        facts.note_fact_payload(kind, id, json!({ "tag": tag }));
    }
    facts
}

fn rows_of(entries: &[(&str, &str, &str)]) -> Vec<DebtSpec> {
    MemoryDebts.rows(&request(), &facts(entries))
}

/// 活集不足 4 条 ⇒ **返回空**：不申报一笔注定 NoDebt 的债（不制造每轮留痕的噪音）。
#[test]
fn three_live_entries_do_not_report_a_debt() {
    let rows = rows_of(&[
        (ENCODED, "m1", "经验"),
        (ENCODED, "m2", "经验"),
        (ENCODED, "m3", "经验"),
    ]);
    assert!(rows.is_empty());
}

/// 满 4 条 ⇒ 欠一笔，欠本插件自己的 `memory` 端口，载荷带标签。
#[test]
fn four_live_entries_report_one_debt_on_the_memory_port() {
    let rows = rows_of(&[
        (ENCODED, "m1", "经验"),
        (ENCODED, "m2", "经验"),
        (ENCODED, "m3", "经验"),
        (ENCODED, "m4", "经验"),
    ]);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, ROW);
    assert_eq!(rows[0].port, MEMORY_PORT);
    assert_eq!(rows[0].payload.get("tag"), Some(&json!("经验")));
}

/// 幂等：同一份活集重复申报 ⇒ 同 id（环按 `(port, id, attempt)` 吸住）。
#[test]
fn the_same_active_set_reports_the_same_id() {
    let entries = [
        (ENCODED, "m1", "经验"),
        (ENCODED, "m2", "经验"),
        (ENCODED, "m3", "经验"),
        (ENCODED, "m4", "经验"),
    ];
    assert_eq!(rows_of(&entries)[0].id, rows_of(&entries)[0].id);
}

/// 活集一变（新编码）⇒ id 变，才会产生新的一笔。
#[test]
fn a_new_encoding_changes_the_id() {
    let before = rows_of(&[
        (ENCODED, "m1", "经验"),
        (ENCODED, "m2", "经验"),
        (ENCODED, "m3", "经验"),
        (ENCODED, "m4", "经验"),
    ]);
    let after = rows_of(&[
        (ENCODED, "m1", "经验"),
        (ENCODED, "m2", "经验"),
        (ENCODED, "m3", "经验"),
        (ENCODED, "m4", "经验"),
        (ENCODED, "m5", "经验"),
    ]);
    assert_ne!(before[0].id, after[0].id);
}

/// 活集条数按「编码 − 巩固」算：一次巩固抵消两条编码，4 条编码 + 1 条产物 ⇒ 3 条，
/// 不再欠（这条钉的是申报方与活集折叠同一笔账）。
#[test]
fn a_consolidation_and_two_forgotten_entries_settle_the_debt() {
    let rows = rows_of(&[
        (ENCODED, "m1", "经验"),
        (ENCODED, "m2", "经验"),
        (ENCODED, "m3", "经验"),
        (ENCODED, "m4", "经验"),
        (CONSOLIDATED, "c1", "经验"),
    ]);
    assert!(rows.is_empty());
}

/// 5 条编码 + 1 条产物 ⇒ 活集 4 条，仍欠；id 随最新产物变化，所以下一笔能再欠。
#[test]
fn a_consolidation_that_leaves_four_live_entries_still_owes() {
    let plain = rows_of(&[
        (ENCODED, "m1", "经验"),
        (ENCODED, "m2", "经验"),
        (ENCODED, "m3", "经验"),
        (ENCODED, "m4", "经验"),
        (ENCODED, "m5", "经验"),
    ]);
    let settled = rows_of(&[
        (ENCODED, "m1", "经验"),
        (ENCODED, "m2", "经验"),
        (ENCODED, "m3", "经验"),
        (ENCODED, "m4", "经验"),
        (ENCODED, "m5", "经验"),
        (CONSOLIDATED, "c1", "经验"),
    ]);
    assert_eq!(settled.len(), 1, "活集 4 条，应当再欠一笔");
    assert_ne!(
        plain[0].id, settled[0].id,
        "巩固后 id 必须变，否则这笔会被吸掉"
    );
}

/// 别的标签不参与：非巩固域的 4 条编码不构成债。
#[test]
fn other_tags_do_not_report_a_debt() {
    let rows = rows_of(&[
        (ENCODED, "s1", "技能"),
        (ENCODED, "s2", "技能"),
        (ENCODED, "s3", "技能"),
        (ENCODED, "s4", "技能"),
    ]);
    assert!(rows.is_empty());
}

/// 两个巩固域各算各的：判断满 4 条时只欠判断那一笔。
#[test]
fn each_tag_owes_its_own_debt() {
    let rows = rows_of(&[
        (ENCODED, "p1", "判断"),
        (ENCODED, "p2", "判断"),
        (ENCODED, "p3", "判断"),
        (ENCODED, "p4", "判断"),
    ]);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].payload.get("tag"), Some(&json!("判断")));
}
