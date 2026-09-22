//! `entry.rs` 的单元测试（`SessionSnapshot` 的读取语义）。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）：测试跟着被测试的实现走。
//!
//! ## 这里钉的是"共享快照"的判据，不是它的 I/O
//!
//! `SessionSnapshot` 存在的意义是让 `provider_id` / `workdir` / `agent_id` /
//! 「有无标题」四个回退从**同一份**会话读出来，而不是各读一遍（详见
//! `entry.rs` 里该结构体的文档）。I/O 次数由端到端实测，这里钉的是判据本身：
//! 字段缺失 / 读取失败 / 空白值三种情况的区分——两处回退对它们的处置不同
//! （`workdir` 报错收场、`agent_id` 只降级），判错了就是用户可见的文案错位。

use super::*;
use serde_json::json;

/// 造一份带指定 metadata 的快照（读取成功分支）。
fn snapshot_with(meta: serde_json::Value) -> SessionSnapshot {
    let mut s = Session::new("s1");
    s.metadata = meta;
    SessionSnapshot {
        provider_id: None,
        session: Ok(s),
    }
}

/// 造一份读取失败的快照。
fn snapshot_failed() -> SessionSnapshot {
    SessionSnapshot {
        provider_id: None,
        session: Err(PluginError::NotFound("存储不可读".into())),
    }
}

/// 字段存在 → 原样取出。
#[test]
fn meta_str_reads_a_present_field() {
    let s = snapshot_with(json!({"workdir": "D:/ws", "agent_id": "a1"}));
    assert_eq!(s.meta_str("workdir").as_deref(), Some("D:/ws"));
    assert_eq!(s.meta_str("agent_id").as_deref(), Some("a1"));
}

/// 字段缺失与字段不是字符串，都给 `None`——两者对回退而言等价（"没配"）。
#[test]
fn meta_str_gives_none_for_missing_and_non_string() {
    let s = snapshot_with(json!({"workdir": 42, "mode": null}));
    assert_eq!(s.meta_str("workdir"), None, "非字符串按缺失处理");
    assert_eq!(s.meta_str("mode"), None);
    assert_eq!(s.meta_str("never_set"), None);
}

/// 读取失败时一切回退都不可用（`None`）——与"字段缺失"在**取值**上同形，
/// 差别只在 `workdir` 那条路径的报错文案（它区分两者，见 `entry.rs` 第 5 步）。
#[test]
fn a_failed_read_makes_every_fallback_unavailable() {
    let s = snapshot_failed();
    assert_eq!(s.meta_str("workdir"), None);
    assert_eq!(s.meta_str("agent_id"), None);
    assert!(!s.has_title(), "读取失败 ⇒ 不得据此断定已有标题");
}

/// `has_title`：有标题 / 空标题 / 纯空白标题 / 无该字段。
///
/// 判据必须与 `ensure_auto_title` 里的提前返回**逐字一致**（`!s.trim().is_empty()`），
/// 否则会出现"快照说已有标题 → 跳过 → 实际从未命名"这种静默丢命名。
#[test]
fn has_title_matches_the_ensure_auto_title_criterion() {
    assert!(snapshot_with(json!({"title": "我的会话"})).has_title());
    assert!(
        !snapshot_with(json!({"title": ""})).has_title(),
        "空标题 = 没有标题"
    );
    assert!(
        !snapshot_with(json!({"title": "   "})).has_title(),
        "纯空白 = 没有标题"
    );
    assert!(!snapshot_with(json!({})).has_title(), "字段缺失 = 没有标题");
    assert!(
        !snapshot_with(json!({"title": 42})).has_title(),
        "非字符串 = 没有标题"
    );
}
