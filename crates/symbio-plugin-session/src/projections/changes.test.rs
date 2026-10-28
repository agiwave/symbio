//! 变更账的折叠判据（[D-169]）：收哪些、谁说话、界外怎么办。
//!
//! 折叠的**呈现**（地址、文案）由会话出口自己钉（`plugins/session/changes.test.rs`）——
//! 本文件只判折叠这一半：从事实到 `{path, op, turn, changes}`。
//!
//! 判据的口径是**字段**而不是工具名：`file_op` 由写族工具声明的行为交出
//! （DE-025 D-207），所以本文件同时钉住「折叠不认识任何工具名」这件事——
//! 一个从没见过的工具名带着 `file_op` 照样进账，一个 `vdfs_write` 形状但没有
//! 那个字段的产物**不**进账。

use super::workdir_changes;
use crate::declarations::MESSAGE_VOCAB as MESSAGE;
use symbio_core::{Entity, Event, Verb};
use symbio_plugin_api::effects::FileOp;
use symbio_plugin_api::events::EVENT_ARTIFACT_ADDED;

/// 一条成功的写（`args` 按生产形状给 JSON 文本）。
fn ok(id: &str, op: &str, args: serde_json::Value, turn: u64) -> Event {
    artifact(id, "vdfs_write", Some(op), args, true, turn, "{}")
}

/// 一条工具产物；`text` 是结果 JSON 文本（`edit` 的 `replaced` 从这里读）。
///
/// `op` 是**富化字段**（`file_op`）：`None` = 这个工具没有声明 `file_change`
/// 行为，产物格上就没有这个字段——与「没改过」同形（见模块头）。
fn artifact(
    id: &str,
    tool: &str,
    op: Option<&str>,
    args: serde_json::Value,
    ok: bool,
    turn: u64,
    text: &str,
) -> Event {
    let mut payload = serde_json::json!({
        "tool": tool,
        "text": text,
        "args": args.to_string(),
        "success": ok,
    });
    if let Some(op) = op {
        payload["file_op"] = serde_json::json!(op);
    }
    Event::pending(
        id,
        EVENT_ARTIFACT_ADDED,
        Entity::Artifact,
        Verb::Asserted,
        turn,
        "agent",
    )
    .with_payload(payload)
}

fn fold(events: &[Event]) -> Vec<(String, FileOp, u64, u64)> {
    workdir_changes(MESSAGE)
        .read_all(events)
        .value
        .into_iter()
        .map(|c| (c.path, c.op, c.turn, c.changes))
        .collect()
}

/// 只收**成功的、带动作的**产物：失败调用、读族（没有 `file_op`）都不是变更。
#[test]
fn only_successful_annotated_writes_are_changes() {
    let failed = artifact(
        "a-1",
        "vdfs_write",
        Some("write"),
        serde_json::json!({"path": "a.txt"}),
        false,
        1,
        "boom",
    );
    let events = [
        ok("a-0", "write", serde_json::json!({"path": "a.txt"}), 1),
        failed,
        // 读族没有声明 `file_change` ⇒ 产物上没有 `file_op` ⇒ 不是变更。
        artifact(
            "a-2",
            "vdfs_read",
            None,
            serde_json::json!({"path": "a.txt"}),
            true,
            1,
            "{}",
        ),
        artifact(
            "a-3",
            "vdfs_search",
            None,
            serde_json::json!({"path": "a.txt"}),
            true,
            1,
            "{}",
        ),
    ];
    assert_eq!(fold(&events), vec![("a.txt".into(), FileOp::Write, 1, 1)]);
}

/// **core 不认识工具名**：一个从没见过的工具只要带上 `file_op`，就按那个动作进账。
///
/// 反向自检：把 `change_of` 改回按 `payload.tool` 认表 ⇒ 本条红（`mcp__fs__put`
/// 不在表里）。这正是「加一个写工具不必改 core」的可执行形式。
#[test]
fn any_tool_that_declares_an_op_is_counted() {
    let events = [
        artifact(
            "a-0",
            "mcp__fs__put",
            Some("write"),
            serde_json::json!({"path": "remote.txt"}),
            true,
            1,
            "{}",
        ),
        artifact(
            "a-1",
            "shell",
            Some("delete"),
            serde_json::json!({"path": "tmp/x"}),
            true,
            2,
            "{}",
        ),
    ];
    assert_eq!(
        fold(&events),
        vec![
            ("tmp/x".into(), FileOp::Delete, 2, 1),
            ("remote.txt".into(), FileOp::Write, 1, 1),
        ]
    );
}

/// 反向：**有写族工具名、但没有 `file_op`** 的产物不计——读侧不再按名字猜。
#[test]
fn a_write_shaped_tool_without_the_field_is_not_counted() {
    let legacy = artifact(
        "a-0",
        "vdfs_write",
        None,
        serde_json::json!({"path": "old.txt"}),
        true,
        1,
        "{}",
    );
    assert!(
        fold(&[legacy]).is_empty(),
        "没有动作字段 ⇒ 无从判断，宁可不记"
    );
}

/// 同一路径多次改：**最新一条说了算**（动作 / 轮号），次数累加。
#[test]
fn the_last_change_wins_and_changes_are_counted() {
    let events = [
        ok("a-0", "write", serde_json::json!({"path": "src/x.rs"}), 2),
        ok("a-1", "edit", serde_json::json!({"path": "src/x.rs"}), 3),
        ok("a-2", "delete", serde_json::json!({"path": "src/x.rs"}), 4),
    ];
    assert_eq!(
        fold(&events),
        vec![("src/x.rs".into(), FileOp::Delete, 4, 3)]
    );
}

/// 最近改过的在前——顺序取「最后一次出现的位置」，不是字典序。
#[test]
fn most_recently_changed_first() {
    let events = [
        ok("a-0", "write", serde_json::json!({"path": "z.txt"}), 1),
        ok("a-1", "write", serde_json::json!({"path": "a.txt"}), 1),
        // 再写 z.txt：它回到最前，且次数变 2。
        ok("a-2", "write", serde_json::json!({"path": "z.txt"}), 2),
    ];
    let rows = fold(&events);
    assert_eq!(
        rows,
        vec![
            ("z.txt".into(), FileOp::Write, 2, 2),
            ("a.txt".into(), FileOp::Write, 1, 1),
        ]
    );
}

/// 无操作的编辑不算变更：`replaced: 0` 跳过，`replaced: 1` 计入；
/// 结果里**没有** `replaced` 的旧事件按变更计入（不知道 ≠ 没改）。
#[test]
fn noop_edit_is_not_a_change() {
    let args = serde_json::json!({"path": "n.txt", "old_string": "a", "new_string": "a"});
    let noop = artifact(
        "a-0",
        "vdfs_edit",
        Some("edit"),
        args.clone(),
        true,
        1,
        r#"{"success":true,"replaced":0}"#,
    );
    let real = artifact(
        "a-1",
        "vdfs_edit",
        Some("edit"),
        args.clone(),
        true,
        2,
        r#"{"success":true,"replaced":1}"#,
    );
    let legacy = artifact(
        "a-2",
        "vdfs_edit",
        Some("edit"),
        args.clone(),
        true,
        3,
        r#"{"success":true}"#,
    );
    let events = [noop, real.clone(), legacy];
    assert_eq!(fold(&events), vec![("n.txt".into(), FileOp::Edit, 3, 2)]);
    assert_eq!(fold(&[real]), vec![("n.txt".into(), FileOp::Edit, 2, 1)]);
    assert!(fold(&[artifact(
        "a-0",
        "vdfs_edit",
        Some("edit"),
        args,
        true,
        1,
        r#"{"success":true,"replaced":0}"#
    )])
    .is_empty());
}

/// 工作目录外 / 虚拟根 / 形状不对的路径一律不计——它们不是「工作目录的变更」。
#[test]
#[ignore = "跨 crate 拆分：依赖 vdfs 插件的静态声明，不在 core 的测试二进制里"]
fn paths_outside_the_workdir_are_skipped() {
    let outside = [
        "",
        "/etc/passwd",
        "C:/Windows/win.ini",
        "..",
        "../escape.txt",
        "src/../../escape.txt",
        "./",
    ];
    let mut events: Vec<Event> = outside
        .iter()
        .enumerate()
        .map(|(i, p)| {
            ok(
                &format!("a-{i}"),
                "write",
                serde_json::json!({"path": p}),
                1,
            )
        })
        .collect();
    // 虚拟根的名字由 vdfs 插件声明，测试**问声明**而不是写死拼法。
    let root = symbio_core::declared_addr_root().expect("vdfs 插件应已声明根名");
    events.push(ok(
        "a-virtual",
        "write",
        serde_json::json!({ "path": format!("{root}/session/s1/inbox/0.md") }),
        1,
    ));
    assert!(fold(&events).is_empty());
}

/// 合法路径归一：反斜杠与 `./` 归到同一形状，读方不必再认识平台写法。
#[test]
fn paths_are_normalized_inside_the_workdir() {
    let events = [
        ok(
            "a-0",
            "write",
            serde_json::json!({"path": "src\\lib\\main.rs"}),
            1,
        ),
        ok(
            "a-1",
            "write",
            serde_json::json!({"path": "./src/lib/main.rs"}),
            2,
        ),
    ];
    assert_eq!(
        fold(&events),
        vec![("src/lib/main.rs".into(), FileOp::Write, 2, 2)]
    );
}

/// 参数形状容错：`args` 是对象也认，坏 JSON / 缺 `path` 跳过，不炸。
#[test]
fn argument_shapes_are_tolerated() {
    let mut object_args = ok("a-0", "write", serde_json::json!({"path": "o.txt"}), 1);
    object_args.payload["args"] = serde_json::json!({"path": "o.txt"});
    let mut bad_json = artifact(
        "a-1",
        "vdfs_write",
        Some("write"),
        serde_json::json!({}),
        true,
        1,
        "{}",
    );
    bad_json.payload["args"] = serde_json::json!("not json");
    let missing_path = ok("a-2", "write", serde_json::json!({"text": "x"}), 1);
    assert_eq!(
        fold(&[object_args, bad_json, missing_path]),
        vec![("o.txt".into(), FileOp::Write, 1, 1)]
    );
}

/// as-of：`ts > now` 的事件不在账上（与其余投影同口径）。
#[test]
fn future_events_are_skipped() {
    let mut e = ok("a-0", "write", serde_json::json!({"path": "t.txt"}), 1);
    e = e.with_ts(100);
    assert!(workdir_changes(MESSAGE)
        .apply(&[e.clone()], 50, symbio_core::Budget::generous())
        .value
        .is_empty());
    assert_eq!(fold(&[e]), vec![("t.txt".into(), FileOp::Write, 1, 1)]);
}
