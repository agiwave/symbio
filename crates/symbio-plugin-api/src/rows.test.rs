//! 行执行体的判据：按行名查实现、写入声明来自实现、纯值入口不碰宿主。

use super::*;
use symbio_core::{Entity, Verb};

/// 一个只为测试存在的实现：把债键与事实条数记成一条草稿。
struct Recorder;

impl RowHandler for Recorder {
    fn owner(&self) -> &'static str {
        "tests/recorder"
    }

    fn row(&self) -> &'static str {
        "test.row"
    }

    fn writes(&self) -> &'static [&'static str] {
        &["test.indexed"]
    }

    fn run(&self, req: &RowRequest<'_>) -> RowStep {
        RowStep::Facts(vec![FactDraft {
            key: format!("r-{}-{}", req.anchor(), req.key_of("id").unwrap_or("-")),
            kind: "test.indexed".to_string(),
            entity: Entity::Domain,
            verb: Verb::Asserted,
            payload: serde_json::json!({ "facts": req.facts.len() }),
            produced_by: Some(req.user_seq),
        }])
    }
}

// 判据用**进程内**的提交：本 crate 的单测里没有别的插件，`inventory` 里只有这一条。
crate::submit_row!(Recorder);

#[test]
fn handler_is_found_by_its_row() {
    let handler = handler_for("test.row").expect("按行名查得到");
    assert_eq!(handler.owner(), "tests/recorder");
    assert!(handler_for("test.absent").is_none());
}

#[test]
fn writes_come_from_the_implementation() {
    assert_eq!(writes_of("test.row"), Some(&["test.indexed"][..]));
    assert_eq!(writes_of("test.absent"), None);
    assert!(declared_writes().contains(&"test.indexed"));
}

#[test]
fn request_is_pure_values() {
    let key = vec![("id".to_string(), "call-1".to_string())];
    let req = RowRequest {
        row: "test.row",
        key: &key,
        turn: 3,
        now_ms: 7,
        principal: "session/a",
        user_seq: 42,
        facts: &[],
    };
    assert_eq!(req.key_of("id"), Some("call-1"));
    assert_eq!(req.key_of("absent"), None);
    assert_eq!(req.anchor(), "t3");
    let RowStep::Facts(drafts) = handler_for("test.row").unwrap().run(&req) else {
        panic!("Recorder 不请求往返");
    };
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0].key, "r-t3-call-1");
    assert_eq!(drafts[0].produced_by, Some(42));
}

/// 解析口的默认实现是空：不请求往返的实现不该被迫回答「回执怎么办」。
#[test]
fn parse_defaults_to_no_facts() {
    let req = RowRequest {
        row: "test.row",
        key: &[],
        turn: 1,
        now_ms: 0,
        principal: "session/a",
        user_seq: 1,
        facts: &[],
    };
    let drafts = handler_for("test.row")
        .unwrap()
        .parse(&req, ModelReply::Failed("boom"));
    assert!(drafts.is_empty());
}
