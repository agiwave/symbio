//! `live` 单测：落格即投影的**形状**判据。
//!
//! 与实现同级分文件（`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）。这里只钉
//! 「哪条事实折出哪些节点、什么状态、什么身份」——渲染链（`rebuild`）自己的用例
//! 已覆盖折叠细节，本文件不重复。

use super::*;
use symbio_kernel::chat::chat_node::{NodeKind, NodeStatus, CHAT_STYLE_TAG_RUN};
use symbio_kernel::event::{Entity, Verb, EVENT_FAILED, EVENT_REQUEST, EVENT_RESPONSE};
use symbio_plugin_api::events::{
    EVENT_ARTIFACT_ADDED, EVENT_CHAT_ASSISTANT_FINAL, EVENT_TOOL_REQUESTED, EVENT_TURN_DISPATCH,
};

use crate::declarations::{EVENT_SESSION_SETTLED, EVENT_SESSION_TERMINATED};

fn event(id: &str, kind: &str, entity: Entity, verb: Verb, payload: serde_json::Value) -> Event {
    Event::pending(id, kind, entity, verb, "session/main").with_payload(payload)
}

/// 派发清单：本轮一次调用（wire id 与节点身份**不同**）。
fn request() -> Event {
    event(
        "e-request",
        EVENT_TOOL_REQUESTED,
        Entity::Scope,
        Verb::Asserted,
        serde_json::json!({
            "root_id": "u1",
            "calls": [{ "id": "wire-1", "node_id": "card-1", "name": "read_file" }],
        }),
    )
}

/// 工具产物：结果落在派发清单给出的那张卡下。
fn artifact() -> Event {
    event(
        "e-artifact",
        EVENT_ARTIFACT_ADDED,
        Entity::Artifact,
        Verb::Asserted,
        serde_json::json!({
            "tool": "read_file",
            "text": "done",
            "tool_call_id": "wire-1",
            "args": "{\"path\":\"a\"}",
            "success": true,
        }),
    )
}

/// 一次对读者说的模型往返（收束轮）。
fn response() -> Event {
    event(
        "e-response",
        EVENT_RESPONSE,
        Entity::Scope,
        Verb::Asserted,
        serde_json::json!({
            "root_id": "u1",
            "text": "answer",
            "text_id": "t1",
            "reasoning": "think",
            "reasoning_id": "r1",
            "answer": "answer",
            "answer_id": "t1",
            "port": "model",
            "stamp": { "role": "reader" },
        }),
    )
}

/// 首响：对话面开场白，没有配对事实（`D-424`）；父（轮根）来自载荷 `root_id`（`D-426`）。
fn opening() -> Event {
    event(
        "e-opening",
        CHAT_VOCAB.opening,
        Entity::Scope,
        Verb::Asserted,
        serde_json::json!({
            "root_id": "u1",
            "id": "opening:u1",
            "reason": "needs_work",
            "text": "我来处理",
            "exclude_from_context": true,
        }),
    )
}

/// 拒言事实：本轮那句答复没获准进入对话（`D-390`）。
fn denial() -> Event {
    event(
        "e-denied",
        "model.speech.denied",
        Entity::Scope,
        Verb::Asserted,
        serde_json::json!({
            "key": EVENT_CHAT_ASSISTANT_FINAL,
            "root_id": "u1",
            "reason": "denied",
        }),
    )
}

/// 面向读者的模型派发（`root_id` 非空 = 这一轮对读者说话）；`None` = 内部往返。
fn model_request(id: &str, root: Option<&str>) -> Event {
    let mut payload = serde_json::json!({
        "provider": "LMStudio",
        "principal": "session/main",
    });
    if let Some(root) = root {
        payload["root_id"] = serde_json::json!(root);
    }
    event(id, EVENT_REQUEST, Entity::Scope, Verb::Asserted, payload)
}

fn wal() -> (tempfile::TempDir, crate::store::EventWalStore) {
    let dir = tempfile::tempdir().unwrap();
    let wal = crate::store::EventWalStore::open(dir.path().join("e.jsonl")).unwrap();
    (dir, wal)
}

fn by_id<'a>(nodes: &'a [cm::ChatNode], id: &str) -> &'a cm::ChatNode {
    nodes
        .iter()
        .find(|n| n.id == id)
        .unwrap_or_else(|| panic!("没有节点 {id}：{nodes:?}"))
}

/// 工具产物 → 卡片终态 + 结果子节点，且身份与流式面同源（卡用节点身份，不用 wire id）。
#[test]
fn artifact_projects_completed_card_and_result() {
    assert!(projects(&artifact()));
    let nodes = frames(&artifact(), vec![request()]);
    assert_eq!(nodes.len(), 2, "{nodes:?}");

    let card = by_id(&nodes, "card-1");
    assert_eq!(card.parent_id.as_deref(), Some("u1"));
    assert_eq!(card.status, Some(NodeStatus::Completed));
    assert_eq!(card.tool_call_id.as_deref(), Some("wire-1"));
    assert_eq!(card.tool_name(), Some("read_file"));

    let result = by_id(&nodes, "wire-1-result");
    assert_eq!(result.parent_id.as_deref(), Some("card-1"));
    assert_eq!(result.status, Some(NodeStatus::Completed));
    assert_eq!(result.msg_type, Some(NodeKind::Body));

    // 在途号归转写唯一写入点：投影器自己的号不落实时面。
    assert!(nodes.iter().all(|n| n.seq.is_none()), "{nodes:?}");
}

/// 没有派发清单就没有卡可挂：不凭空造节点（与 `rebuild` 同一纪律）。
#[test]
fn artifact_without_dispatch_has_no_partner() {
    let mut index = LiveIndex::new();
    let dir = tempfile::tempdir().unwrap();
    let wal = crate::store::EventWalStore::open(dir.path().join("e.jsonl")).unwrap();
    assert!(index.partners(&artifact(), &wal).is_empty());
    assert!(frames(&artifact(), Vec::new()).is_empty());
}

/// 索引先答：派发清单收进索引后，产物不必回读事实源。
#[test]
fn index_answers_partner_before_fallback() {
    let mut index = LiveIndex::new();
    index.observe(&request());
    let dir = tempfile::tempdir().unwrap();
    let wal = crate::store::EventWalStore::open(dir.path().join("e.jsonl")).unwrap();
    let partners = index.partners(&artifact(), &wal);
    assert_eq!(partners.len(), 1);
    assert_eq!(partners[0].kind, EVENT_TOOL_REQUESTED);
}

/// 未命中回读兜底：事实早于本写口落格（换事实源句柄）时仍能配对。
#[test]
fn fallback_reads_partner_from_fact_source() {
    let dir = tempfile::tempdir().unwrap();
    let wal = crate::store::EventWalStore::open(dir.path().join("e.jsonl")).unwrap();
    Store::append(&wal, request().with_produced_by(0)).unwrap();

    let mut index = LiveIndex::new();
    let partners = index.partners(&artifact(), &wal);
    assert_eq!(partners.len(), 1, "回读没找到派发清单");
    // 收进索引：第二次不再回读。
    assert_eq!(index.partners(&artifact(), &wal).len(), 1);
}

/// 首响没有配对，`frames_for` 的「轮根前置」不得把它吞掉；父仍取载荷的 `root_id`。
#[test]
fn opening_projects_through_frames_for() {
    let (_dir, wal) = wal();
    let mut index = LiveIndex::new();
    let nodes = index.frames_for(&opening(), &wal);
    assert_eq!(nodes.len(), 1, "{nodes:?}");
    assert_eq!(nodes[0].id, "opening:u1");
    assert_eq!(nodes[0].parent_id.as_deref(), Some("u1"));
}

/// 首响落格即下发：开场白是正在等的用户的第一句话，不能等到轮收尾。
#[test]
fn opening_projects_immediately() {
    assert!(projects(&opening()));
    let nodes = frames(&opening(), Vec::new());
    assert_eq!(nodes.len(), 1, "{nodes:?}");
    let node = &nodes[0];
    assert_eq!(node.id, "opening:u1");
    assert_eq!(
        node.parent_id.as_deref(),
        Some("u1"),
        "首响挂轮根（用户消息节点）下，与工具 / 正文同级（D-426）"
    );
    assert_eq!(node.msg_type, Some(NodeKind::Body));
    assert_eq!(node.status, Some(NodeStatus::Completed));
    assert_eq!(
        node.meta.as_ref().and_then(|m| m.get("reason")),
        Some(&serde_json::json!("needs_work"))
    );
}

/// 模型往返 → 思考 / 正文终态节点；状态取载荷 `status`，缺省 `Completed`。
#[test]
fn response_projects_completed_text_and_reasoning() {
    assert!(projects(&response()));
    let nodes = frames(&response(), Vec::new());
    assert_eq!(nodes.len(), 2, "{nodes:?}");
    assert_eq!(by_id(&nodes, "r1").status, Some(NodeStatus::Completed));
    assert_eq!(by_id(&nodes, "r1").msg_type, Some(NodeKind::Notice));
    let text = by_id(&nodes, "t1");
    assert_eq!(text.status, Some(NodeStatus::Completed));
    assert_eq!(text.parent_id.as_deref(), Some("u1"));
    assert!(nodes.iter().all(|n| n.seq.is_none()));
}

/// 中止：半截正文照样出节点，状态是 `aborted` 而不是「跑完了」。
#[test]
fn failed_aborted_projects_aborted_nodes() {
    let failed = event(
        "e-failed",
        EVENT_FAILED,
        Entity::Scope,
        Verb::Asserted,
        serde_json::json!({
            "root_id": "u1",
            "text": "half",
            "text_id": "t1",
            "reasoning": "think",
            "reasoning_id": "r1",
            "answer": "half",
            "answer_id": "t1",
            "status": "aborted",
            "port": "model",
            "stamp": { "role": "reader" },
        }),
    );
    let nodes = frames(&failed, Vec::new());
    assert_eq!(by_id(&nodes, "t1").status, Some(NodeStatus::Aborted));
    assert_eq!(by_id(&nodes, "r1").status, Some(NodeStatus::Aborted));
}

/// 发言被拒：拒言事实进切片 ⇒ 答复那一次一个节点都不出（fail-closed）。
#[test]
fn denied_response_projects_nothing() {
    let mut index = LiveIndex::new();
    index.observe(&denial());
    let dir = tempfile::tempdir().unwrap();
    let wal = crate::store::EventWalStore::open(dir.path().join("e.jsonl")).unwrap();
    let partners = index.partners(&response(), &wal);
    assert_eq!(partners.len(), 1);
    assert!(frames(&response(), partners).is_empty());
}

/// 内部往返（角色的写侧判据不是 `reader`）不是这一轮对读者说的话 ⇒ 不产节点。
#[test]
fn internal_response_is_not_projected() {
    let mut internal = response();
    internal.payload["stamp"]["role"] = serde_json::json!("internal");
    assert!(!projects(&internal));
    assert!(frames(&internal, Vec::new()).is_empty());
}

/// 工具结果的协议事实（`response{port=tool}`）与产物格无关：不产节点。
#[test]
fn tool_port_response_is_not_projected() {
    let mut tool = response();
    tool.payload["stamp"]["role"] = serde_json::json!("internal");
    tool.payload["port"] = serde_json::json!("tool");
    assert!(!projects(&tool));
}

/// 面向读者的派发 → 一个临时轮骨架（`D-425`）：父 = 轮根、形状 = 整轮骨架。
#[test]
fn reader_request_projects_round_placeholder() {
    let request = model_request("e-round", Some("u1"));
    assert!(watches(&request));
    let (_dir, wal) = wal();
    let mut index = LiveIndex::new();
    let nodes = index.frames_for(&request, &wal);
    assert_eq!(nodes.len(), 1, "{nodes:?}");
    let node = &nodes[0];
    assert_eq!(node.id, "round:e-round");
    assert_eq!(node.parent_id.as_deref(), Some("u1"));
    assert_eq!(node.msg_type, Some(NodeKind::Skeleton));
    assert_eq!(node.status, Some(NodeStatus::Streaming));
    assert_eq!(node.style_tag(), Some(CHAT_STYLE_TAG_RUN));
    assert!(is_round_placeholder(node));
    // 运行中秒数的锚点（前端读 `meta.started_at`，与工具卡同一条读法）。
    assert_eq!(
        node.meta.as_ref().and_then(|meta| meta.get("started_at")),
        Some(&serde_json::json!(request.ts))
    );
    // 在途号归转写唯一写入点（与终态投影同一条纪律）。
    assert_eq!(node.seq, None);
}

/// 内部往返与工具端口的请求没有轮根：不开轮、不产占位。
#[test]
fn requests_without_root_have_no_placeholder() {
    let (_dir, wal) = wal();
    let mut index = LiveIndex::new();
    let internal = model_request("e-internal", None);
    assert!(!watches(&internal));
    assert!(index.frames_for(&internal, &wal).is_empty());

    let tool_port = event(
        "e-tool-port",
        EVENT_REQUEST,
        Entity::Scope,
        Verb::Asserted,
        serde_json::json!({
            "call_id": "wire-1",
            "request_id": "tool.requested:model:model:u1:1",
        }),
    );
    assert!(!watches(&tool_port));
    assert!(index.frames_for(&tool_port, &wal).is_empty());
}

/// 产物格没有 `root_id`：轮占位逻辑不参与，终态投影照常出。
#[test]
fn artifact_projects_through_frames_for() {
    let (_dir, wal) = wal();
    let mut index = LiveIndex::new();
    index.observe(&request());
    let nodes = index.frames_for(&artifact(), &wal);
    assert_eq!(nodes.len(), 2, "{nodes:?}");
    assert_eq!(by_id(&nodes, "card-1").status, Some(NodeStatus::Completed));
    assert_eq!(
        by_id(&nodes, "wire-1-result").parent_id.as_deref(),
        Some("card-1")
    );
}

/// 开轮派发即出骨架：用户消息之后的第一条回合事实，不等 reader 请求（`D-425`）。
#[test]
fn dispatch_projects_placeholder_immediately() {
    let (_dir, wal) = wal();
    let mut index = LiveIndex::new();
    let dispatch = event(
        "e-dispatch",
        EVENT_TURN_DISPATCH,
        Entity::Scope,
        Verb::Asserted,
        serde_json::json!({
            "root_id": "u1",
            "tier": "deep",
            "budget_ms": 600000,
            "principal": "session/main",
        }),
    );
    assert!(watches(&dispatch));
    let nodes = index.frames_for(&dispatch, &wal);
    assert_eq!(nodes.len(), 1, "{nodes:?}");
    let node = &nodes[0];
    assert_eq!(node.id, "round:e-dispatch");
    assert_eq!(node.parent_id.as_deref(), Some("u1"));
    assert!(is_round_placeholder(node));
    assert_eq!(node.status, Some(NodeStatus::Streaming));
}

/// 不经过模型流的轮次：轮收场事实（终答 / 收束 / 终止）也撤占位。
#[test]
fn closing_facts_retire_placeholder() {
    for kind in [
        EVENT_CHAT_ASSISTANT_FINAL,
        EVENT_SESSION_SETTLED,
        EVENT_SESSION_TERMINATED,
    ] {
        let (_dir, wal) = wal();
        let mut index = LiveIndex::new();
        let dispatch = event(
            "e-dispatch",
            EVENT_TURN_DISPATCH,
            Entity::Scope,
            Verb::Asserted,
            serde_json::json!({ "root_id": "u1", "principal": "session/main" }),
        );
        index.frames_for(&dispatch, &wal);

        let end = event(
            "e-end",
            kind,
            Entity::Scope,
            Verb::Asserted,
            serde_json::json!({ "root_id": "u1" }),
        );
        assert!(watches(&end), "{kind} 应参与实时投影");
        let nodes = index.frames_for(&end, &wal);
        assert_eq!(nodes.len(), 1, "{kind}: {nodes:?}");
        assert_eq!(nodes[0].id, "round:e-dispatch");
        assert_eq!(nodes[0].status, Some(NodeStatus::Removed));
    }
}

/// 本轮输出事实落格 → 占位退出（不吐流式帧的提供方 / 中止 / 拒言的兜底）。
#[test]
fn output_fact_retires_placeholder() {
    let (_dir, wal) = wal();
    let mut index = LiveIndex::new();
    index.frames_for(&model_request("e-round", Some("u1")), &wal);

    let nodes = index.frames_for(&response(), &wal);
    assert_eq!(nodes[0].id, "round:e-round");
    assert_eq!(nodes[0].status, Some(NodeStatus::Removed));
    // 收束事实自己的终态投影照常出（思考 + 正文）。
    assert_eq!(by_id(&nodes, "r1").status, Some(NodeStatus::Completed));

    // 同一轮根的第二次收束不重复撤（登记已清）。
    let again = index.frames_for(&response(), &wal);
    assert!(again.iter().all(|n| n.id != "round:e-round"), "{again:?}");
}

/// 派发清单（工具清单事实）同样收轮：占位在工具卡到达之前就该让位。
#[test]
fn dispatch_fact_retires_placeholder() {
    let (_dir, wal) = wal();
    let mut index = LiveIndex::new();
    index.frames_for(&model_request("e-round", Some("u1")), &wal);
    let nodes = index.frames_for(&request(), &wal);
    assert_eq!(nodes.len(), 1, "{nodes:?}");
    assert_eq!(nodes[0].id, "round:e-round");
    assert_eq!(nodes[0].status, Some(NodeStatus::Removed));
}

/// 同一轮根的下一段派发：先撤旧占位，再发新的——同时最多一个。
#[test]
fn next_round_request_replaces_placeholder() {
    let (_dir, wal) = wal();
    let mut index = LiveIndex::new();
    index.frames_for(&model_request("e-round-1", Some("u1")), &wal);
    let nodes = index.frames_for(&model_request("e-round-2", Some("u1")), &wal);
    assert_eq!(nodes.len(), 2, "{nodes:?}");
    assert_eq!(nodes[0].id, "round:e-round-1");
    assert_eq!(nodes[0].status, Some(NodeStatus::Removed));
    assert_eq!(nodes[1].id, "round:e-round-2");
    assert_eq!(nodes[1].status, Some(NodeStatus::Streaming));
}
