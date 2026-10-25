//! 变更账的折叠判据（[D-169]）：收哪些、谁说话、界外怎么办。
//!
//! 折叠的**呈现**（地址、文案）由会话出口自己钉（`plugins/session/changes.test.rs`）——
//! 本文件只判 core 这一半：从事实到 `{path, op, turn, changes}`。

use super::{workdir_changes, FileOp};
use crate::symbio_core::event::{Entity, Event, Verb, EVENT_ARTIFACT_ADDED};

/// 一条成功的工具产物（`args` 按生产形状给 JSON 文本）。
fn ok(id: &str, tool: &str, args: serde_json::Value, turn: u64) -> Event {
    artifact(id, tool, args, true, turn, "{}")
}

/// 一条工具产物；`text` 是结果 JSON 文本（`vdfs_edit` 的 `replaced` 从这里读）。
fn artifact(
    id: &str,
    tool: &str,
    args: serde_json::Value,
    ok: bool,
    turn: u64,
    text: &str,
) -> Event {
    Event::pending(
        id,
        EVENT_ARTIFACT_ADDED,
        Entity::Artifact,
        Verb::Asserted,
        turn,
        "agent",
    )
    .with_payload(serde_json::json!({
        "tool": tool,
        "text": text,
        "args": args.to_string(),
        "success": ok,
    }))
}

fn fold(events: &[Event]) -> Vec<(String, FileOp, u64, u64)> {
    workdir_changes()
        .read_all(events)
        .value
        .into_iter()
        .map(|c| (c.path, c.op, c.turn, c.changes))
        .collect()
}

/// 只收**成功的写族**：失败调用、读族、未知工具都不是变更。
#[test]
fn only_successful_write_tools_are_changes() {
    let failed = artifact(
        "a-1",
        "vdfs_write",
        serde_json::json!({"path": "a.txt"}),
        false,
        1,
        "boom",
    );
    let events = [
        ok("a-0", "vdfs_write", serde_json::json!({"path": "a.txt"}), 1),
        failed,
        ok("a-2", "vdfs_read", serde_json::json!({"path": "a.txt"}), 1),
        ok(
            "a-3",
            "vdfs_search",
            serde_json::json!({"path": "a.txt"}),
            1,
        ),
        ok("a-4", "vdfs_stat", serde_json::json!({"path": "a.txt"}), 1),
    ];
    assert_eq!(fold(&events), vec![("a.txt".into(), FileOp::Write, 1, 1)]);
}

/// 同一路径多次改：**最新一条说了算**（动作 / 轮号），次数累加。
#[test]
fn the_last_change_wins_and_changes_are_counted() {
    let events = [
        ok(
            "a-0",
            "vdfs_write",
            serde_json::json!({"path": "src/x.rs"}),
            2,
        ),
        ok(
            "a-1",
            "vdfs_edit",
            serde_json::json!({"path": "src/x.rs"}),
            3,
        ),
        ok(
            "a-2",
            "vdfs_delete",
            serde_json::json!({"path": "src/x.rs"}),
            4,
        ),
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
        ok("a-0", "vdfs_write", serde_json::json!({"path": "z.txt"}), 1),
        ok("a-1", "vdfs_write", serde_json::json!({"path": "a.txt"}), 1),
        // 再写 z.txt：它回到最前，且次数变 2。
        ok("a-2", "vdfs_write", serde_json::json!({"path": "z.txt"}), 2),
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
        args.clone(),
        true,
        1,
        r#"{"success":true,"replaced":0}"#,
    );
    let real = artifact(
        "a-1",
        "vdfs_edit",
        args.clone(),
        true,
        2,
        r#"{"success":true,"replaced":1}"#,
    );
    let legacy = artifact(
        "a-2",
        "vdfs_edit",
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
        args,
        true,
        1,
        r#"{"success":true,"replaced":0}"#
    )])
    .is_empty());
}

/// 工作目录外 / 虚拟根 / 形状不对的路径一律不计——它们不是「工作目录的变更」。
#[test]
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
                "vdfs_write",
                serde_json::json!({"path": p}),
                1,
            )
        })
        .collect();
    // 虚拟根的名字由 vdfs 插件声明，测试**问声明**而不是写死拼法。
    let root = crate::symbio_core::vdfs::declared_addr_root().expect("vdfs 插件应已声明根名");
    events.push(ok(
        "a-virtual",
        "vdfs_write",
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
            "vdfs_write",
            serde_json::json!({"path": "src\\lib\\main.rs"}),
            1,
        ),
        ok(
            "a-1",
            "vdfs_write",
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
    let mut object_args = ok("a-0", "vdfs_write", serde_json::json!({"path": "o.txt"}), 1);
    object_args.payload["args"] = serde_json::json!({"path": "o.txt"});
    let mut bad_json = artifact("a-1", "vdfs_write", serde_json::json!({}), true, 1, "{}");
    bad_json.payload["args"] = serde_json::json!("not json");
    let missing_path = ok("a-2", "vdfs_write", serde_json::json!({"text": "x"}), 1);
    assert_eq!(
        fold(&[object_args, bad_json, missing_path]),
        vec![("o.txt".into(), FileOp::Write, 1, 1)]
    );
}

/// as-of：`ts > now` 的事件不在账上（与其余投影同口径）。
#[test]
fn future_events_are_skipped() {
    let mut e = ok("a-0", "vdfs_write", serde_json::json!({"path": "t.txt"}), 1);
    e = e.with_ts(100);
    assert!(workdir_changes()
        .apply(&[e.clone()], 50, crate::symbio_core::Budget::generous())
        .value
        .is_empty());
    assert_eq!(fold(&[e]), vec![("t.txt".into(), FileOp::Write, 1, 1)]);
}
