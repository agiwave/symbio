//! `cli/src/render.rs` 的单元测试 —— 与实现同级分文件（约定：`X.rs` + `X.test.rs`）。
//!
//! 只钉住一条判据：正文该不该进 stdout，依据是节点上的 `exclude_from_context`，
//! **不是** `surface`——答话节点也带 `surface = "reply"`，按 surface 判会把答案吞掉。

use super::*;
use serde_json::json;

#[test]
fn opening_node_is_progress() {
    // 写侧形状：`chat_loop/compose.rs` 的开场白节点（重建侧 `opening_node` 同形）
    let meta = json!({"surface": "reply", "reason": "unclassified", "exclude_from_context": true});
    assert!(is_progress_surface(Some(&meta)));
}

#[test]
fn final_answer_node_is_not_progress() {
    // `Answered` 答话节点：surface 相同，只有排除标记相反
    let meta = json!({"surface": "reply", "reason": "clarify", "exclude_from_context": false});
    assert!(!is_progress_surface(Some(&meta)));
}

#[test]
fn node_without_meta_is_not_progress() {
    assert!(!is_progress_surface(None));
    assert!(!is_progress_surface(Some(&json!({"surface": "reply"}))));
}
