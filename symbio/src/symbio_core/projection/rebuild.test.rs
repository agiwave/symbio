//! `rebuild` 的判据（轨 第一步）。
//!
//! 这个模块的失效形态是**静默的形状漂移**：重建出的树与落库树长得像但不一样
//! （父节点接错、`msg_type` 差一档、工具结果挂到 Turn 根而不是 ToolCall 上）。
//! 它不报错、不告警，前端只是把工具卡片渲染成一条普通正文。所以这里逐条钉
//! **结构**而不是内容：
//!
//! 1. 每条事实落到**该落的位置**（id / parent_id / type / role 四件套）；
//! 2. 同一批调用在两个载荷里各记一次时**不重复建卡**；
//! 3. 同输入 ⇒ 逐字同输出（纯函数，N1）；
//! 4. 载荷没记身份的格子**合成 id 必须稳定**（否则每次重开都多一批新节点）。

use super::rebuild;
use crate::symbio_core::chat_node::{MessageContent, NodeKind, NodeRole, NodeStatus};
use crate::symbio_core::{Entity, Event, Verb};
use crate::symbio_core::{
 EVENT_ARTIFACT_ADDED, EVENT_ASSISTANT_FALLBACK, EVENT_ASSISTANT_FINAL,
 EVENT_ASSISTANT_REPORTED, EVENT_TURN_SUPPLEMENTED, EVENT_USER_MESSAGE,
};

/// 一条事件的**可断言摘要**——`ChatNode` 没有 `PartialEq`（它是增量帧的形状，
/// 逐字段比较才有意义），所以判据比的是这四件套 + 正文 + 状态。
///
/// 类型这一格比的是**语义形状**（[`NodeForm`]），不是 `msg_type` 变体名：形状由
/// 「形态 + `meta.style_tag`」两半共同决定，只比变体名会把「骨架没带 tag」这种
/// 半截迁移判成通过。
#[derive(Debug, PartialEq)]
struct Shape {
 id: String,
 parent: Option<String>,
 ty: NodeForm,
 role: Option<NodeRole>,
 text: String,
 status: Option<NodeStatus>,
}

/// 节点形状的可断言标签：判据全部走 [`ChatNode`] 谓词（两种 wire 形态都认）。
#[derive(Debug, PartialEq, Eq)]
enum NodeForm {
 /// 正文
 Body,
 /// 推理正文
 Reasoning,
 /// 骨架：一整轮响应
 Run,
 /// 骨架：一次工具调用
 Tool,
 /// 等待用户回应
 Interaction,
 /// 系统提示（压缩记录）
 Notice,
 /// 谓词都不命中（含未标类型）
 Other,
}

fn form_of(m: &crate::symbio_core::chat_node::ChatNode) -> NodeForm {
 if m.is_reasoning {
 NodeForm::Reasoning
 } else if m.is_text {
 NodeForm::Body
 } else if m.is_run {
 NodeForm::Run
 } else if m.is_tool_call {
 NodeForm::Tool
 } else if m.is_interaction {
 NodeForm::Interaction
 } else if m.is_compression {
 NodeForm::Notice
 } else {
 NodeForm::Other
 }
}

fn shape(m: &crate::symbio_core::chat_node::ChatNode) -> Shape {
 Shape {
 id: m.id.clone,
 parent: m.parent_id.clone,
 ty: form_of(m),
 role: m.role.clone,
 text: m.content.as_ref.map(|c| c.to_text).unwrap_or_default,
 status: m.status.clone,
 }
}

fn shapes(msgs: &[crate::symbio_core::chat_node::ChatNode]) -> Vec<Shape> {
 msgs.iter.map(shape).collect
}

/// 用户格（载荷带节点 id）。
fn user_event(id: &str, text: &str) -> Event {
 Event::pending(
 "u-1",
 EVENT_USER_MESSAGE,
 Entity::Turn,
 Verb::Opened,
 1,
 "user",
 )
 .with_ts(1_000)
 .with_payload(serde_json::json!({ "text": text, "tier": "full", "id": id }))
}

/// 收束格（产出形状见 `turn_runner::produced_payload`）。
fn final_event(payload: serde_json::Value) -> Event {
 Event::pending(
 "f-1",
 EVENT_ASSISTANT_FINAL,
 Entity::Turn,
 Verb::Closed,
 1,
 "session/main",
 )
 .with_ts(2_000)
 .with_payload(payload)
}

// ── ① 一轮对话的树 ────────────────────────────────────────────────────

#[test]
fn final_builds_turn_root_with_reasoning_and_text_children {
 let events = vec![
 user_event("u-node", "问 A"),
 final_event(serde_json::json!({
 "text": "答 A",
 "model": "m",
 "turn_id": "T1",
 "text_id": "T1-text",
 "reasoning_id": "T1-reason",
 "reasoning": "想了一下",
 "tool_calls": [],
 "rounds": [],
 "status": "completed",
 })),
 ];

 let got = shapes(&rebuild(&events));
 assert_eq!(
 got,
 vec![
 Shape {
 id: "u-node".into,
 parent: None,
 ty: NodeForm::Body,
 role: Some(NodeRole::User),
 text: "问 A".into,
 // 落库的用户消息**没有** status（调用方 `..Default::default`）——
 // 重建不得补（补了就是两个身份，前端按 status 参与合并键）。
 status: None,
 },
 Shape {
 id: "T1".into,
 parent: None,
 ty: NodeForm::Run,
 role: Some(NodeRole::Assistant),
 text: String::new,
 status: Some(NodeStatus::Completed),
 },
 Shape {
 id: "T1-reason".into,
 parent: Some("T1".into),
 ty: NodeForm::Reasoning,
 role: Some(NodeRole::Assistant),
 text: "想了一下".into,
 status: Some(NodeStatus::Completed),
 },
 Shape {
 id: "T1-text".into,
 parent: Some("T1".into),
 ty: NodeForm::Body,
 role: Some(NodeRole::Assistant),
 text: "答 A".into,
 status: Some(NodeStatus::Completed),
 },
 ]
 );
}

/// reasoning-only：思考与正文同一段内容且无独立文本回复 ⇒ 只出一个 Text 子节点，
/// **不另立 Reasoning 子**（与落库侧 `llm_build_assistant_messages` 同一判据）。
#[test]
fn reasoning_only_does_not_duplicate_the_paragraph {
 let events = vec![
 user_event("u-node", "问"),
 final_event(serde_json::json!({
 "text": "同一段",
 "turn_id": "T1",
 "text_id": "T1-text",
 "reasoning_id": "T1-reason",
 "reasoning": "同一段",
 "status": "completed",
 })),
 ];
 let got = shapes(&rebuild(&events));
 assert_eq!(got.len, 3, "user + Turn + 一条 Text，实得 {got:?}");
 assert_eq!(got[2].id, "T1-reason", "reasoning-only 时复用推理子节点 id");
 assert_eq!(got[2].ty, NodeForm::Body);
}

/// 兜底格的 `why`（失败原因）必须折回 Turn 节点的 `error`。
///
/// 存在的理由是一条实测事故：一轮跑了 18 个工具轮后模型吐了空正文，整轮落兜底格，
/// 重建把 `rounds[]` 的 18 段思考摊成子节点，却**没人把「为什么失败」带出来**——
/// 界面上是「一串连续思考，然后什么都没有」，与「模型什么也没说」无法区分。
/// 失败原因是网格里的事实（`payload.why`），它就该到界面（组级交代条读节点 `error`）。
#[test]
fn fallback_carries_the_failure_reason_onto_the_turn_node {
 let events = vec![
 user_event("u-node", "问"),
 Event::pending(
 "fb-1",
 EVENT_ASSISTANT_FALLBACK,
 Entity::Turn,
 Verb::Closed,
 1,
 "session/main",
 )
 .with_ts(2_000)
 .with_payload(serde_json::json!({
 "text": "",
 "model": "m",
 "turn_id": "T1",
 "text_id": null,
 "reasoning_id": null,
 "reasoning": null,
 "tool_calls": [],
 "rounds": [
 { "text_id": null, "text": null, "reasoning_id": "R0", "reasoning": "第一轮在想" },
 { "text_id": null, "text": null, "reasoning_id": "R1", "reasoning": "第二轮在想" },
 ],
 "status": "failed",
 "reason": "fallback",
 "detail": "model returned empty text",
 })),
 ];

 let got = rebuild(&events);
 let turn = got
 .iter
 .find(|m| m.id == "T1")
 .expect("Turn 根在重建结果里");
 assert_eq!(form_of(turn), NodeForm::Run);
 assert_eq!(turn.status, Some(NodeStatus::Failed));
 assert_eq!(
 turn.error.as_deref,
 Some("model returned empty text"),
 "失败原因进节点 error——否则交代条只有一句话都没有"
 );

 let shapes = shapes(&got);
 let reasoning: Vec<&Shape> = shapes
 .iter
 .filter(|s| s.ty == NodeForm::Reasoning)
 .collect;
 assert_eq!(reasoning.len, 2, "两轮中途思考各一个节点：{shapes:?}");
 assert!(
 !shapes
 .iter
 .any(|s| s.ty == NodeForm::Body && s.id != "u-node"),
 "空正文不该长出正文节点：{shapes:?}"
 );
}

/// 一轮的中途产出**随该轮的产物格入格**，于是重建把它排在自己的工具卡片之前。
///
/// 判据是**位置**而不是存在：这些思考本来就记在收束格的 `rounds[]` 里，一个字段都不少。
/// 但重建的顺序锚是格的发射顺序，收束格排在全轮末尾 ⇒ 历史面上十几个思考块连排在
/// **所有工具卡片之后**（实测会话 fixverify01：实时面是「思考→卡片→思考→卡片」，
/// 重开一次就变成用户报的「大模型连续的思考过程」）。同 id 只认首次出现的那一份 ⇒
/// 产物格先建，收束格 `rounds[]` 里的那份副本不再建第二张卡。
#[test]
fn round_output_rides_the_artifact_cell_and_lands_before_its_tool_card {
 let round = serde_json::json!({
 "text_id": null, "text": null, "reasoning_id": "R0", "reasoning": "先读 a.md 再说",
 });
 let artifact = |no: u64, call: &str, res: &str, path: &str, body: &str, round: _| {
 Event::pending(
 format!("a-1-{no}"),
 EVENT_ARTIFACT_ADDED,
 Entity::Artifact,
 Verb::Asserted,
 1,
 "session/main",
 )
 .with_ts(1_500)
 .with_payload(serde_json::json!({
 "tool": "read", "text": body, "tool_call_id": call, "wire_id": call,
 "args": format!("{{\"path\":\"{path}\"}}"), "result_id": res,
 "success": true, "turn_id": "T1", "round": round,
 }))
 };
 let events = vec![
 user_event("u-node", "读两个文件"),
 artifact(0, "c0", "r0", "a.md", "a 的内容", round.clone),
 artifact(1, "c1", "r1", "b.md", "b 的内容", serde_json::Value::Null),
 final_event(serde_json::json!({
 "text": "都读完了", "model": "m", "turn_id": "T1", "text_id": "T1-text",
 "reasoning_id": null, "reasoning": null, "tool_calls": [],
 "rounds": [round], "status": "completed",
 })),
 ];

 let got = shapes(&rebuild(&events));
 let at = |id: &str| got.iter.position(|s| s.id == id);
 let reasoning = at("R0").expect("中途思考随产物格建出节点");
 let card = at("c0").expect("工具卡在");
 assert!(
 reasoning < card,
 "思考排在它那一轮的工具卡之前（实测它被收束格挪到全轮末尾）：{got:?}"
 );
 assert_eq!(
 got.iter.filter(|s| s.id == "R0").count,
 1,
 "收束格 `rounds[]` 的同 id 副本不建第二份"
 );
 assert_eq!(got[reasoning].parent, Some("T1".into));
 assert_eq!(got[reasoning].ty, NodeForm::Reasoning);
 // 第二格没有 `round`（同一轮只挂第一格）⇒ 一个节点都不多发。
 let c1 = at("c1").expect("第二张卡在");
 assert_eq!(
 at("r1").expect("第二条结果在"),
 c1 + 1,
 "第二格只建调用与结果两张卡"
 );
}

// ── ② 工具卡：调用与结果是两张卡，结果挂在调用下 ────────────────────────
#[test]
fn artifact_builds_call_and_result_under_the_call {
 let events = vec![
 user_event("u-node", "读文件"),
 Event::pending(
 "a-1-0",
 EVENT_ARTIFACT_ADDED,
 Entity::Artifact,
 Verb::Asserted,
 1,
 "session/main",
 )
 .with_ts(1_500)
 .with_payload(serde_json::json!({
 "tool": "read",
 "text": "文件内容",
 "tool_call_id": "",
 "wire_id": "call_wire_1",
 "args": "{\"path\":\"a.md\"}",
 "result_id": "C1-res",
 "success": true,
 "turn_id": "T1",
 })),
 ];

 let got = shapes(&rebuild(&events));
 assert_eq!(got.len, 3);
 // 调用卡：ToolCall，挂在 Turn 根下，正文是**已定稿的参数文本**
 assert_eq!(got[1].id, "");
 assert_eq!(got[1].parent, Some("T1".into));
 assert_eq!(got[1].ty, NodeForm::Tool);
 assert_eq!(got[1].role, Some(NodeRole::Assistant));
 assert_eq!(got[1].text, "{\"path\":\"a.md\"}");
 // 结果卡：role=Tool，挂在**调用**下（不是 Turn 根）
 assert_eq!(got[2].id, "C1-res");
 assert_eq!(got[2].parent, Some("".into));
 assert_eq!(got[2].role, Some(NodeRole::Tool));
 assert_eq!(got[2].text, "文件内容");
 assert_eq!(got[2].status, Some(NodeStatus::Completed));
}

/// 收束载荷的 `tool_calls[]` 是**引用**（结果正文归 `artifact.added`）——
/// 同 id 不得重复建卡，否则前端会看到两张一样的工具卡。
#[test]
fn tool_calls_reference_does_not_duplicate_the_card {
 let events = vec![
 Event::pending(
 "a-1-0",
 EVENT_ARTIFACT_ADDED,
 Entity::Artifact,
 Verb::Asserted,
 1,
 "session/main",
 )
 .with_payload(serde_json::json!({
 "tool": "read", "text": "内容", "tool_call_id": "", "wire_id": "w1",
 "args": "{}", "result_id": "C1-res", "success": true, "turn_id": "T1",
 })),
 final_event(serde_json::json!({
 "text": "读完了",
 "turn_id": "T1",
 "text_id": "T1-text",
 "tool_calls": [{ "id": "", "name": "read", "args": "{}", "wire_id": "w1", "result_id": "C1-res" }],
 "status": "completed",
 })),
 ];
 let got = shapes(&rebuild(&events));
 let calls = got.iter.filter(|s| s.id == "").count;
 assert_eq!(calls, 1, "同一张工具卡只能有一份，实得 {calls}");
}

// ── ③ 纯函数：同输入 ⇒ 同输出 ─────────────────────────────────────────

#[test]
fn rebuild_is_deterministic {
 let events = vec![
 user_event("u-node", "问"),
 final_event(serde_json::json!({
 "text": "答", "turn_id": "T1", "text_id": "t", "status": "completed",
 })),
 ];
 assert_eq!(shapes(&rebuild(&events)), shapes(&rebuild(&events)));
}

// ── ④ 载荷没记身份 ⇒ 合成 id 必须稳定 ─────────────────────────────────

#[test]
fn supplement_gets_a_stable_synthetic_id {
 let ev = Event::pending(
 "s-1-0",
 EVENT_TURN_SUPPLEMENTED,
 Entity::Turn,
 Verb::Asserted,
 1,
 "user",
 )
 .with_ts(1_200)
 .with_payload(serde_json::json!({ "text": "插一句", "count": 1 }));

 let a = shapes(&rebuild(std::slice::from_ref(&ev)));
 let b = shapes(&rebuild(&[ev]));
 assert_eq!(a, b, "同一条事件每次重建必须得同一个 id");
 assert_eq!(a[0].id, "s-1-0", "载荷缺 id 时合成 id 取事件 id");
 assert_eq!(a[0].role, Some(NodeRole::User));
 assert_eq!(a[0].text, "插一句");
}

/// 补充格的形状必须**逐字复刻落库那一份**（`turn_exec::supplement_into_request`）：
/// 无 `msg_type`、无 `status`、带 `meta.supplement*` 三键。任一格不同，同一条补充
/// 在实时面与重建面就是两个节点。
#[test]
fn supplement_mirrors_the_persisted_shape {
 let ev = Event::pending(
 "s-1-0",
 EVENT_TURN_SUPPLEMENTED,
 Entity::Turn,
 Verb::Asserted,
 1,
 "user",
 )
 .with_ts(1_200)
 .with_payload(serde_json::json!({
 "text": "插一句", "count": 2, "id": "inbox-a", "ids": ["inbox-a", "inbox-b"],
 }));

 let msgs = rebuild(std::slice::from_ref(&ev));
 let m = &msgs[0];
 assert_eq!(m.id, "inbox-a", "载荷带 id ⇒ 复刻该身份（不合成）");
 assert_eq!(m.msg_type, None, "落库的补充消息没有 msg_type");
 assert_eq!(m.status, None, "落库的补充消息没有 status");
 let meta = m.meta.as_ref.expect("补充必须带 meta");
 assert_eq!(meta["supplement"], serde_json::json!(true));
 assert_eq!(meta["supplement_count"], serde_json::json!(2));
 assert_eq!(
 meta["supplement_ids"],
 serde_json::json!(["inbox-a", "inbox-b"])
 );
}

/// 答话轮收束格（带 `dialog` 载荷）+ 思考事实：Reason 节点是答话的**子节点**。
///
/// 生成产线（`from_context`）的模型推理经 `WordingReply` 带回、随收束格入格
/// ——重建必须复刻它，否则普通答话轮的 Reason 节点重启后消失（AMD MiMo
/// 实测 2026-10-13 的原始缺口）。
#[test]
fn final_with_dialog_rebuilds_reasoning_child_from_the_payload {
 let ev = final_event(serde_json::json!({
 "text": "我们刚才在聊 README 的事。",
 "model": "",
 "turn_id": "",
 "text_id": null,
 "reasoning_id": "ans-reason",
 "reasoning": "想一想：直接从上下文总结。",
 "tool_calls": [],
 "rounds": [],
 "status": "completed",
 "dialog": { "id": "ans-node", "reason": "from_context", "exclude_from_context": false },
 }));

 let msgs = rebuild(std::slice::from_ref(&ev));
 assert_eq!(shapes(&msgs).len, 2, "答话节点 + 思考子节点");
 let reply = shape(&msgs[0]);
 assert_eq!(reply.id, "ans-node");
 assert_eq!(reply.ty, NodeForm::Body);
 assert_eq!(reply.parent, None, "答话挂根级（对话线）");
 let reason = shape(&msgs[1]);
 assert_eq!(reason.id, "ans-reason", "id 由载荷给（与流式期同源）");
 assert_eq!(reason.parent.as_deref, Some("ans-node"));
 assert_eq!(reason.ty, NodeForm::Reasoning);
 assert_eq!(reason.text, "想一想：直接从上下文总结。");
 assert_eq!(reason.status, Some(NodeStatus::Completed));
}

/// 无思考事实（旧事件 / 模板产线 / 生成失败退模板）⇒ 恰一个节点，**不多发**。
#[test]
fn final_with_dialog_without_reasoning_stays_single_node {
 let ev = final_event(serde_json::json!({
 "text": "不客气。",
 "model": "",
 "turn_id": "",
 "reasoning_id": null,
 "reasoning": null,
 "tool_calls": [],
 "rounds": [],
 "status": "completed",
 "dialog": { "id": "ans-node", "reason": "thanks", "exclude_from_context": false },
 }));

 let msgs = rebuild(std::slice::from_ref(&ev));
 assert_eq!(shapes(&msgs).len, 1, "没有思考事实就只有答话节点");
 assert_eq!(shape(&msgs[0]).ty, NodeForm::Body);
}

/// 汇报格的形状复用写入侧的 `ReportDraft::to_message`（`dialog_node(text, "progress", true)`）
/// ——`surface` / `reason` / `exclude_from_context` 三键缺一不可，`exclude_from_context`
/// 更是轮次窗口「不进模型请求包」的判别位。
#[test]
fn reported_mirrors_the_persisted_shape {
 let ev = Event::pending(
 "rp-1-0",
 EVENT_ASSISTANT_REPORTED,
 Entity::Turn,
 Verb::Asserted,
 1,
 "session/main",
 )
 .with_ts(3_000)
 .with_payload(serde_json::json!({
 "text": "还在跑", "tool_rounds": 2, "quiet_ms": 5_000,
 }));

 let msgs = rebuild(std::slice::from_ref(&ev));
 let m = &msgs[0];
 assert_eq!(m.id, "rp-1-0", "载荷缺 id ⇒ 合成 id 取事件 id");
 assert_eq!(m.role, Some(NodeRole::Assistant));
 assert_eq!(form_of(m), NodeForm::Body);
 assert_eq!(m.status, Some(NodeStatus::Completed));
 assert_eq!(m.timestamp, Some(3_000), "时间戳覆写为事件自身的 ts");
 let meta = m.meta.as_ref.expect("汇报必须带 meta");
 assert_eq!(meta["surface"], serde_json::json!("reply"));
 assert_eq!(meta["reason"], serde_json::json!("progress"));
 assert_eq!(meta["exclude_from_context"], serde_json::json!(true));
}

// ── ⑤ 状态是产出事实的状态，落到根与正文上 ────────────────────────────

#[test]
fn aborted_status_lands_on_the_turn_and_its_text {
 let events = vec![
 user_event("u-node", "问"),
 final_event(serde_json::json!({
 "text": "说到一半", "turn_id": "T1", "text_id": "t",
 "status": "aborted",
 })),
 ];
 let got = shapes(&rebuild(&events));
 assert_eq!(got[1].status, Some(NodeStatus::Aborted));
 assert_eq!(got[2].status, Some(NodeStatus::Aborted));
 assert_eq!(got[2].text, "说到一半", "半截正文照入格（D-052）");
}

/// 正文用 `MessageContent::Text` 承载（不是 `Parts`）——重建给的是纯文本形态，
/// 与落库侧一致。
#[test]
fn text_content_is_plain_text {
 let events = vec![user_event("u-node", "问")];
 let msgs = rebuild(&events);
 assert!(matches!(msgs[0].content, Some(MessageContent::Text(_))));
}

// ── ⑥ 变更格：编辑 / 删除（S2.7-b1，D-053）──────────────────────────

/// 一条 `message.edited`：载荷由**写侧同一条序列化路径**造出来
/// （`serde_json::to_value(&msg)` 再包一层 `{"message": …}`，见
/// `message_events::edited`）。手写 JSON 会让判据钉住我抄的那份，而不是写侧真的落的东西。
fn edited_event(msg: crate::symbio_core::chat_node::ChatNode) -> Event {
 Event::pending(
 "m-edit-0",
 crate::symbio_core::EVENT_MESSAGE_EDITED,
 Entity::Message,
 Verb::Asserted,
 0,
 "user",
 )
 .with_payload(serde_json::json!({
 "message": serde_json::to_value(&msg).expect("ChatNode 必然可序列化"),
 }))
}

/// 一条 `message.removed`（载荷 = 被删 id 列表）。
fn removed_event(ids: &[&str]) -> Event {
 Event::pending(
 "m-del-0",
 crate::symbio_core::EVENT_MESSAGE_REMOVED,
 Entity::Message,
 Verb::Closed,
 0,
 "user",
 )
 .with_payload(serde_json::json!({ "ids": ids }))
}

/// 编辑：**换内容，不动位置**（`seq` 原样保留）。
///
/// 「不动位置」是硬要求：`seq` 是前端的排序键、轮次窗口按位置切片。换号等于把这条
/// 消息挪到别处——历史次序当场错乱，而错乱的表现是「模型把后说的话当成先说的」。
#[test]
fn an_edit_replaces_the_node_in_place {
 let events = vec![
 user_event("u-node", "原始问题"),
 final_event(serde_json::json!({
 "text": "答", "turn_id": "T1", "text_id": "T1-text", "status": "completed",
 })),
 edited_event(crate::symbio_core::chat_node::ChatNode {
 id: "u-node".into,
 role: Some(NodeRole::User),
 msg_type: Some(NodeKind::Body),
 content: Some(MessageContent::Text("改过的问题".into)),
 timestamp: Some(1_500),
 ..Default::default
 }),
 ];

 let msgs = rebuild(&events);
 let got = shapes(&msgs);
 assert_eq!(got.len, 3, "编辑**不新增节点**：{got:?}");
 assert_eq!(got[0].id, "u-node", "位置不变（还是第一条）");
 assert_eq!(got[0].text, "改过的问题", "内容换成新的");
 assert_eq!(got[0].role, Some(NodeRole::User));
 // 位置锚：编辑前后 `seq` 逐字相同（号不重发）。
 assert_eq!(
 msgs[0].seq,
 Some(1),
 "编辑保留原顺序锚——重发号会让这条在前后两次重建里换个位置"
 );
 assert_eq!(msgs[2].seq, Some(3), "它之后的节点号不受影响");
}

/// 删除：被删节点**消失**，其余保持原序；`seq` 留缺口（顺序锚不是条数）。
#[test]
fn a_removal_drops_the_nodes_and_leaves_seq_gaps {
 let events = vec![
 user_event("u-node", "第一问"),
 final_event(serde_json::json!({
 "text": "第一答", "turn_id": "T1", "text_id": "T1-text", "status": "completed",
 })),
 // 从 Turn 根起截断（起点 + 其子节点，与 `truncate_messages` 的区间同形）
 removed_event(&["T1", "T1-text"]),
 ];

 let msgs = rebuild(&events);
 let got = shapes(&msgs);
 assert_eq!(got.len, 1, "被删的两个节点都要没了：{got:?}");
 assert_eq!(got[0].id, "u-node");
 assert_eq!(
 msgs[0].seq,
 Some(1),
 "留下的那条号不变（不重编号——号是顺序锚，不是条数）"
 );
}

/// 整段截断：删掉**全部**节点 ⇒ 空列表（不是「删了但还剩几条」）。
#[test]
fn truncating_everything_yields_an_empty_list {
 let events = vec![
 user_event("u-node", "问"),
 final_event(serde_json::json!({
 "text": "答", "turn_id": "T1", "text_id": "T1-text", "status": "completed",
 })),
 removed_event(&["u-node", "T1", "T1-text"]),
 ];
 assert!(rebuild(&events).is_empty, "全删 ⇒ 空");
}

/// 编辑一个**不在切片里**的节点是空操作——不硬插到末尾。
///
/// 「硬插」会让一段历史在错误的时刻冒出一句话，比不显示更糟。
#[test]
fn an_edit_for_an_unknown_node_adds_nothing {
 let events = vec![
 user_event("u-node", "问"),
 edited_event(crate::symbio_core::chat_node::ChatNode {
 id: "ghost".into,
 role: Some(NodeRole::User),
 content: Some(MessageContent::Text("谁都不是".into)),
 ..Default::default
 }),
 ];
 let got = shapes(&rebuild(&events));
 assert_eq!(got.len, 1);
 assert_eq!(got[0].id, "u-node");
}

/// 折叠之后**仍是纯函数**（N1）：同输入 ⇒ 逐字同输出。
///
/// 这条不是形式：折叠引入了两张表（索引 / 判死集合），用 `HashMap` 的迭代顺序
/// 写实现就会让它变成**不确定**的——而表现是「同一份 WAL 两次重建次序不同」，
/// 前端按 id 合并时当场错乱。
#[test]
fn rebuild_with_mutations_is_deterministic {
 let events = vec![
 user_event("u-node", "问"),
 final_event(serde_json::json!({
 "text": "答", "turn_id": "T1", "text_id": "T1-text", "status": "completed",
 })),
 edited_event(crate::symbio_core::chat_node::ChatNode {
 id: "u-node".into,
 role: Some(NodeRole::User),
 content: Some(MessageContent::Text("改过".into)),
 ..Default::default
 }),
 removed_event(&["T1-text"]),
 ];
 assert_eq!(shapes(&rebuild(&events)), shapes(&rebuild(&events)));
}

/// 坏载荷（不是合法 `ChatNode`）**跳过**，不让整次重建失败。
///
/// 读侧必须对任意历史事件容错：手工改过的 WAL / 未来版本写的载荷都不该让会话打不开。
#[test]
fn a_malformed_edit_payload_is_skipped {
 let events = vec![
 user_event("u-node", "问"),
 Event::pending(
 "m-edit-9",
 crate::symbio_core::EVENT_MESSAGE_EDITED,
 Entity::Message,
 Verb::Asserted,
 0,
 "user",
 )
 .with_payload(serde_json::json!({ "message": "我不是一条消息" })),
 ];
 let got = shapes(&rebuild(&events));
 assert_eq!(got.len, 1);
 assert_eq!(got[0].text, "问");
}

// ── ⑦ 压缩快照：**取代**前缀（S2.7-b2）────────────────────────────────

/// 一条 `session.consolidated`：载荷 = 压缩后的整份消息列表（写侧把
/// `context.messages` 逐条序列化进去，与落库那一份同源）。
fn snapshot_event(msgs: Vec<crate::symbio_core::chat_node::ChatNode>) -> Event {
 Event::pending(
 "snap-0",
 crate::symbio_core::EVENT_SESSION_CONSOLIDATED,
 Entity::Message,
 Verb::Progressed,
 0,
 "session/main",
 )
 .with_ts(3_000)
 .with_payload(serde_json::json!({
 "messages": msgs
 .iter
 .map(|m| serde_json::to_value(m).expect("ChatNode 必然可序列化"))
 .collect::<Vec<_>>,
 }))
}

/// 快照列表里的一条普通文本节点。
fn text_msg(id: &str, role: NodeRole, text: &str) -> crate::symbio_core::chat_node::ChatNode {
 crate::symbio_core::chat_node::ChatNode {
 id: id.into,
 role: Some(role),
 msg_type: Some(NodeKind::Body),
 content: Some(MessageContent::Text(text.into)),
 ..Default::default
 }
}

/// 快照**取代**前缀：它之前的事实一律不参与，重建从载荷那份列表开始。
///
/// 少了这一段，一次压缩在重建面上等于**没发生过**——被压掉的消息全部复活，
/// 而压缩的全部意义就是让它们不再进入上下文。
#[test]
fn a_snapshot_rebases_the_prefix {
 let events = vec![
 user_event("u-old", "被压掉的问题"),
 final_event(serde_json::json!({
 "text": "被压掉的答复", "turn_id": "T-old", "text_id": "T-old-text",
 "status": "completed",
 })),
 snapshot_event(vec![
 text_msg("snap-root", NodeRole::User, "[CONTEXT SNAPSHOT] 摘要"),
 text_msg("u-keep", NodeRole::User, "保留的近轮问题"),
 text_msg("T-keep-text", NodeRole::Assistant, "保留的近轮答复"),
 ]),
 user_event("u-new", "压缩之后的问题"),
 ];
 let got = shapes(&rebuild(&events));
 let ids: Vec<&str> = got.iter.map(|s| s.id.as_str).collect;
 assert_eq!(
 ids,
 vec!["snap-root", "u-keep", "T-keep-text", "u-new"],
 "重建从快照载荷起，之前的事实不参与"
 );
 assert!(
 got.iter
 .all(|s| s.text != "被压掉的问题" && s.text != "被压掉的答复"),
 "被压掉的不复活"
 );
}

/// **只认最近一次**：更早的快照连它自己的列表都已被后来的快照取代。
#[test]
fn only_the_latest_snapshot_rebases {
 let events = vec![
 user_event("u-old", "更早的问题"),
 snapshot_event(vec![text_msg("snap-a", NodeRole::User, "第一份摘要")]),
 user_event("u-mid", "中间的问题"),
 snapshot_event(vec![
 text_msg("snap-b", NodeRole::User, "第二份摘要"),
 text_msg("u-keep", NodeRole::User, "保留的近轮"),
 ]),
 ];
 let got = shapes(&rebuild(&events));
 let ids: Vec<&str> = got.iter.map(|s| s.id.as_str).collect;
 assert_eq!(ids, vec!["snap-b", "u-keep"], "取最近一份快照的列表");
}

/// 载荷不是合法列表（`messages` 不是数组）⇒ **不是快照**，整段历史照常折。
///
/// 容错方向必须是「少折」而不是「多抹」：把一条坏载荷当快照，后果是**整段历史
/// 被抹掉**——而那是静默的（列表短了没人会红）。
#[test]
fn a_malformed_snapshot_payload_is_not_a_snapshot {
 let broken = Event::pending(
 "snap-x",
 crate::symbio_core::EVENT_SESSION_CONSOLIDATED,
 Entity::Message,
 Verb::Progressed,
 0,
 "session/main",
 )
 .with_payload(serde_json::json!({ "messages": "我不是数组" }));
 let events = vec![user_event("u-node", "问"), broken];
 let got = shapes(&rebuild(&events));
 assert_eq!(got.len, 1);
 assert_eq!(got[0].id, "u-node");
}

/// 空列表同样**不是**快照：压缩恒产出「至少那条摘要消息」，空数组若被当成有效
/// 快照，整段历史当场消失。
#[test]
fn an_empty_snapshot_list_is_not_a_snapshot {
 let events = vec![user_event("u-node", "问"), snapshot_event(Vec::new)];
 let got = shapes(&rebuild(&events));
 assert_eq!(got.len, 1, "空列表不得抹掉历史");
}

/// 快照**之后**的变更格照常折叠：被删的从快照列表里剔除，被编辑的换成新读法。
///
/// 这条钉的是「换基不是终点」——快照之后还有一整段 live 事实要接着折。
#[test]
fn mutations_after_the_snapshot_still_fold {
 let events = vec![
 user_event("u-old", "被压掉的问题"),
 snapshot_event(vec![
 text_msg("snap-root", NodeRole::User, "摘要"),
 text_msg("u-keep", NodeRole::User, "保留的问题"),
 text_msg("u-drop", NodeRole::User, "保留但随后被删"),
 ]),
 edited_event(crate::symbio_core::chat_node::ChatNode {
 id: "u-keep".into,
 role: Some(NodeRole::User),
 content: Some(MessageContent::Text("保留的问题（改过）".into)),
 ..Default::default
 }),
 removed_event(&["u-drop"]),
 ];
 let got = shapes(&rebuild(&events));
 let ids: Vec<&str> = got.iter.map(|s| s.id.as_str).collect;
 assert_eq!(ids, vec!["snap-root", "u-keep"], "被删的从快照列表里剔除");
 assert_eq!(got[1].text, "保留的问题（改过）", "编辑照常生效");
}

/// 快照列表的号由**重建自己的发号器**重发，且与后继事件**同一套**（单调）。
///
/// 载荷里那份号是**存储层**的槽位号，与重建的号不是同一套。混用会让
/// 「快照列表内部单调、与后继事件不单调」——而前端的排序键只有一个。
#[test]
fn seq_is_renumbered_monotonically_across_the_snapshot_boundary {
 let mut snap_msg = text_msg("snap-root", NodeRole::User, "摘要");
 snap_msg.seq = Some(9_999); // 存储层的号：重建必须无视它
 let events = vec![
 user_event("u-old", "被压掉的问题"),
 snapshot_event(vec![snap_msg, text_msg("u-keep", NodeRole::User, "保留")]),
 user_event("u-new", "压缩之后"),
 ];
 let got = rebuild(&events);
 let seqs: Vec<i64> = got.iter.filter_map(|m| m.seq).collect;
 assert_eq!(seqs.len, got.len, "每条都有号");
 assert_eq!(seqs, vec![1, 2, 3], "从 1 起重发，跨快照边界仍单调");
}

// ── ⑦ 诞生格：`message.added`（S2.7-b3 前置，D-054）─────────────────

/// 一条 `message.added`：载荷 = 完整 `ChatNode` 列表（写侧 `message_events::added`
/// 直给，同一条序列化路径——手写 JSON 会钉住我抄的那份）。
fn added_event(msgs: Vec<crate::symbio_core::chat_node::ChatNode>) -> Event {
 Event::pending(
 "m-add-0",
 crate::symbio_core::EVENT_MESSAGE_ADDED,
 Entity::Message,
 Verb::Opened,
 0,
 "user",
 )
 .with_payload(serde_json::json!({
 "messages": msgs
 .iter
 .map(|m| serde_json::to_value(m).expect("ChatNode 必然可序列化"))
 .collect::<Vec<_>>,
 }))
}

/// 诞生格：载荷里每条消息**逐条出生**、形状逐字保留（不补 `status`、不改内容），
/// 按发射顺序排在已有节点之后。
#[test]
fn an_added_event_births_the_payload_nodes_verbatim {
 let record = crate::symbio_core::chat_node::ChatNode {
 id: "compact-record".into,
 role: Some(NodeRole::Assistant),
 msg_type: Some(NodeKind::Body),
 content: Some(MessageContent::Text("〔本轮已压缩〕".into)),
 // 故意**不带** status：重建不得替它补（补了就是两个身份）。
 ..Default::default
 };
 let events = vec![user_event("u-node", "问"), added_event(vec![record])];

 let msgs = rebuild(&events);
 let got = shapes(&msgs);
 assert_eq!(got.len, 2, "诞生格产节点（与变更格相反）：{got:?}");
 assert_eq!(got[1].id, "compact-record");
 assert_eq!(got[1].text, "〔本轮已压缩〕");
 assert_eq!(got[1].status, None, "载荷没有的状态不得由重建补");
 assert_eq!(msgs[1].seq, Some(2), "按发射顺序排在既有节点之后");
}

/// 同一 id 二次诞生 ⇒ **只认首次**（`Carrier` 文档声称的那一半无害：
/// 该 `Runner` 写成 `Here` 只是多一条被去重的事实，不是多一个节点）。
#[test]
fn a_second_birth_for_a_known_id_does_not_duplicate {
 let record = crate::symbio_core::chat_node::ChatNode {
 id: "u-node".into,
 role: Some(NodeRole::User),
 msg_type: Some(NodeKind::Body),
 content: Some(MessageContent::Text("重复出生".into)),
 ..Default::default
 };
 let events = vec![user_event("u-node", "首次出生"), added_event(vec![record])];

 let got = shapes(&rebuild(&events));
 assert_eq!(got.len, 1, "同 id 只认首次：{got:?}");
 assert_eq!(got[0].text, "首次出生", "留下的是先到的那条");
}
