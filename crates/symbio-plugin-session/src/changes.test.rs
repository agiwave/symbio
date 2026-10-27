//! `session/changes` 出口的判据：行是不是**呈现就绪**、读侧闸是否与 stats 同一道。
//!
//! 折叠口径（收哪些工具、谁说话、界外怎么办）由 core 的
//! `projection/changes.test.rs` 钉死；本文件只判出口这一半——地址 / 文案 / 空账。

use super::read_changes;
use symbio_core::{Entity, Event, Verb, PRINCIPAL_MAIN};
use symbio_plugin_api::events::EVENT_ARTIFACT_ADDED;

fn write(id: &str, path: &str, turn: u64) -> Event {
    Event::pending(
        id,
        EVENT_ARTIFACT_ADDED,
        Entity::Artifact,
        Verb::Asserted,
        turn,
        "agent",
    )
    .with_payload(serde_json::json!({
        "tool": "vdfs_write",
        "text": "{}",
        "args": serde_json::json!({ "path": path }).to_string(),
        "success": true,
    }))
}

/// 行是呈现就绪的：地址、判别词、文案、轮号、次数一次给全。
#[test]
fn rows_are_presentation_ready() {
    let got = read_changes(
        "s1",
        Some(vec![
            write("a-0", "src/lib.rs", 2),
            write("a-1", "docs/a.md", 3),
        ]),
        None,
    );
    assert!(got.has_wal && got.readable);
    assert_eq!(got.rows.len(), 2);
    // 最近改过的在前；地址相对会话容器，拼上会话地址即可读。
    assert_eq!(got.rows[0].path, "docs/a.md");
    assert_eq!(got.rows[0].address, "workdir/docs/a.md");
    assert_eq!(got.rows[0].op, "write");
    assert_eq!(got.rows[0].label, "写入");
    assert_eq!(got.rows[0].turn, 3);
    assert_eq!(got.rows[0].changes, 1);
    assert_eq!(got.rows[1].address, "workdir/src/lib.rs");
}

/// 没有事实源与「有源但没改过」是两件事：`has_wal` 分辨它们。
#[test]
fn no_facts_is_an_evidenced_empty() {
    let missing = read_changes("s1", None, None);
    assert!(!missing.has_wal && missing.readable && missing.rows.is_empty());

    let untouched = read_changes("s1", Some(Vec::new()), None);
    assert!(untouched.has_wal && untouched.readable && untouched.rows.is_empty());
}

/// 读侧闸与 `session/stats` 同一道：看不到内容 ⇒ 空清单，不是「没改过」。
#[test]
fn reader_without_access_gets_an_empty_ledger() {
    let got = read_changes(
        "s1",
        Some(vec![write("a-0", "secret.txt", 1)]),
        Some(PRINCIPAL_MAIN),
    );
    assert!(got.has_wal, "有源这一格照报");
    assert!(!got.readable, "不给看要自己说出来，不能只给一张空表");
    assert!(got.rows.is_empty(), "不给看 ⇒ 一条都不给");
}
