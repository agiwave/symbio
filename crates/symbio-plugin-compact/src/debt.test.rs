//! 压缩债的**切片判据**（D-401）：摘要输入永远装得下。
//!
//! 这里只钉申报面能看到的东西：两行载荷里的切片端点 / `keep_from` / `split`，
//! 以及「按预算累计」本身。执行体怎么把切片装成请求由 `model.assist` 那边的判据钉，
//! 端到端由 small-window e2e 钉——三处各司其职，避免一份判据跑三遍。

use serde_json::{json, Value};
use symbio_plugin_api::debt::{Closing, DebtFacts, DebtSource, DriveRequest};
use symbio_plugin_api::events::{
    EVENT_CHAT_ASSISTANT_FINAL, EVENT_MESSAGE_REMOVED, EVENT_USER_MESSAGE,
};
use symbio_plugin_api::text::{default_tokenizer, Tokenizer};

use super::{CompactDebts, SESSION_CONSOLIDATED, SUMMARIZE_INSTRUCTION, WRITE_ROW};

const LIMIT: u64 = 1_000;

fn drive() -> DriveRequest {
    DriveRequest {
        session_id: "session/test".into(),
        provider: None,
        principal: "session/main".into(),
    }
}

/// 造 `rounds` 轮 user/final 事实；最后一轮的水位收到 `limit`（越过 0.7 阈值）。
fn facts_with(rounds: usize, user_len: usize, final_len: usize) -> (DebtFacts, Vec<String>) {
    let mut facts = DebtFacts::default();
    let mut users = Vec::new();
    for index in 0..rounds {
        let user_id = format!("u-{index}");
        let final_id = format!("{EVENT_CHAT_ASSISTANT_FINAL}:{user_id}");
        facts.note_fact(EVENT_USER_MESSAGE, &user_id);
        facts.note_fact_payload(
            EVENT_USER_MESSAGE,
            &user_id,
            json!({ "text": "问".repeat(user_len) }),
        );
        facts.note_fact(EVENT_CHAT_ASSISTANT_FINAL, &final_id);
        facts.note_fact_payload(
            EVENT_CHAT_ASSISTANT_FINAL,
            &final_id,
            json!({ "text": "答".repeat(final_len) }),
        );
        users.push(user_id);
    }
    let current = users.last().expect("至少一轮");
    facts.note_closing(
        &format!("model:{current}"),
        Closing::Ok,
        json!({ "context": { "request_tokens": LIMIT, "limit_tokens": LIMIT } }),
    );
    (facts, users)
}

/// 两行：第一行带切片，第二行带 `keep_from`。
fn rows_of(facts: &DebtFacts) -> (Value, Value) {
    let rows = CompactDebts.rows(&drive(), facts);
    assert_eq!(rows.len(), 2, "越过阈值且装得下 ⇒ 两行都欠");
    (rows[0].payload.clone(), rows[1].payload.clone())
}

/// 转写远大于窗口时，摘要只取**最旧的前缀**，且累计不超过预算。
#[test]
fn slice_stops_at_budget_instead_of_full_transcript() {
    // 10 轮 × 各 100 token：逐条约 113 token（含消息信封）；预算约 392 ⇒ 约 3 轮。
    let (facts, users) = facts_with(10, 100, 100);
    let (summarize, write) = rows_of(&facts);
    let slice = &summarize["slice"];

    // 预算边界：用同一口径算「最多装得下几条」，再钉端点。
    let tokenizer = default_tokenizer();
    let k_budget = (LIMIT as f64 * 0.5) as usize;
    let slice_budget = k_budget.saturating_sub(tokenizer.count(SUMMARIZE_INSTRUCTION) + 48);
    let node_cost = tokenizer.count(&"问".repeat(100)) + 12;
    let fit = slice_budget / node_cost;
    let mut ids = Vec::new();
    for user in &users {
        ids.push(user.clone());
        ids.push(format!("{EVENT_CHAT_ASSISTANT_FINAL}:{user}"));
    }
    assert!(fit < ids.len(), "用例的转写必须远大于切片预算");

    assert_eq!(slice["from"], json!(ids[0]), "从最旧一条开始");
    assert_eq!(
        slice["to"],
        json!(ids[fit - 1]),
        "累计到预算边界就停（按消息边界）"
    );
    assert!(
        slice["truncate"].as_object().is_some_and(|m| m.is_empty()),
        "没有单条超长 ⇒ 不切"
    );
    assert_eq!(
        write["keep_from"],
        json!(ids[fit]),
        "首条未压事实交给读面接着折"
    );
    assert_eq!(write["split"], json!(false));

    // 预算判据：切片正文 + 指令 + 信封 ≤ k × limit。
    let used = tokenizer.count(SUMMARIZE_INSTRUCTION) + fit * node_cost;
    assert!(
        used <= k_budget,
        "摘要输入 {used} 必须 ≤ k × limit {k_budget}"
    );
}

/// 最旧一条自己就超过预算 ⇒ 头尾截断，并在载荷里留下 `split`。
#[test]
fn an_oversized_oldest_node_is_split_and_recorded() {
    let (facts, users) = facts_with(2, 5_000, 100);
    let (summarize, write) = rows_of(&facts);
    let spec = &summarize["slice"]["truncate"][users[0].as_str()];
    assert!(
        spec["head_chars"].as_u64().unwrap_or(0) > 0
            && spec["tail_chars"].as_u64().unwrap_or(0) > 0,
        "超大单条按头尾预算切"
    );
    assert_eq!(summarize["slice"]["from"], json!(users[0]));
    assert_eq!(
        summarize["slice"]["to"],
        json!(users[0]),
        "只压得下这一条（截断后）"
    );
    assert_eq!(write["split"], json!(true), "事实里记下「被切开过」");
    assert_eq!(
        write["keep_from"],
        json!(format!("{EVENT_CHAT_ASSISTANT_FINAL}:{}", users[0])),
        "下一条未压事实正好是同一轮的终答"
    );
}

/// 没有可信水位 ⇒ 一行都不欠（不拿猜测去压）。
#[test]
fn no_watermark_means_no_debt() {
    let (mut facts, _) = facts_with(3, 100, 100);
    let mut bare = DebtFacts::default();
    for (kind, id) in [
        (EVENT_USER_MESSAGE, "u-0"),
        (EVENT_USER_MESSAGE, "u-1"),
        (EVENT_CHAT_ASSISTANT_FINAL, "chat.assistant.final:u-0"),
    ] {
        bare.note_fact(kind, id);
    }
    assert!(CompactDebts.rows(&drive(), &bare).is_empty());
    // 对照：有水位时同一份事实是欠的（排除「测试本身没构造对」）。
    facts.note_closing(
        "model:u-1",
        Closing::Ok,
        json!({ "context": { "request_tokens": 900, "limit_tokens": 1_000 } }),
    );
    assert_eq!(CompactDebts.rows(&drive(), &facts).len(), 2);
}

/// 失败收束的载荷（形状与 `port.rs::summary_failed` 一致）。
fn failed_payload(final_of: &str) -> Value {
    let fingerprint = format!("u-0|{LIMIT}|");
    json!({
        "root_id": final_of,
        "final_id": format!("{EVENT_CHAT_ASSISTANT_FINAL}:{final_of}"),
        "compression_id": format!("compression:{fingerprint}"),
        "failure_class": "non_retryable_provider",
        "detail": "boom",
        "from": "u-0",
        "to": format!("{EVENT_CHAT_ASSISTANT_FINAL}:u-0"),
        "truncate": {},
        "keep_from": null,
        "split": false,
        "fingerprint": fingerprint,
    })
}

// ==================== D-402：跨轮抑制与重试 ====================

/// 同一前缀收束为 Failed ⇒ 下一轮不再欠（不每轮重付注定失败的往返）。
#[test]
fn a_failed_prefix_is_not_owed_again_on_the_next_round() {
    let (mut facts, _) = facts_with(2, 100, 100);
    // 收束键 = 最旧未压事实 | 上限 | 代（无成功压缩 ⇒ 代为空）。
    let base = format!("u-0|{LIMIT}|");
    facts.note_closing(
        &format!("{WRITE_ROW}:{base}"),
        Closing::Failed,
        failed_payload("u-1"),
    );
    assert!(
        CompactDebts.rows(&drive(), &facts).is_empty(),
        "同一前缀已判死 ⇒ 跨轮抑制"
    );
}

/// 失败产物被删过一次（`message.removed`）⇒ 换一把指纹，按失败时那段切片重跑；
/// 且**不欠水位**也重跑（显式重试不是自动压缩）。
#[test]
fn removing_the_failed_node_reopens_the_debt_with_a_new_epoch() {
    let (mut facts, _) = facts_with(2, 100, 100);
    // 水位压到阈值以下：自动压缩不欠，重试仍要跑。
    facts.note_closing(
        "model:u-1",
        Closing::Ok,
        json!({ "context": { "request_tokens": 100, "limit_tokens": LIMIT } }),
    );
    let base = format!("u-0|{LIMIT}|");
    facts.note_closing(
        &format!("{WRITE_ROW}:{base}"),
        Closing::Failed,
        failed_payload("u-1"),
    );

    facts.note_fact(EVENT_MESSAGE_REMOVED, "removed-1");
    facts.note_fact_payload(
        EVENT_MESSAGE_REMOVED,
        "removed-1",
        json!({ "ids": [format!("compression:{base}")] }),
    );

    let rows = CompactDebts.rows(&drive(), &facts);
    assert_eq!(rows.len(), 2, "显式重试 ⇒ 两行重新欠");
    let summarize = &rows[0].payload;
    let write = &rows[1].payload;
    assert_eq!(
        summarize["slice"]["from"],
        json!("u-0"),
        "沿用失败时那段切片的起点，不拿此刻的转写重规划"
    );
    assert_eq!(
        summarize["slice"]["to"],
        json!("chat.assistant.final:u-0"),
        "沿用失败时那段切片的终点"
    );
    assert_eq!(summarize["final_id"], json!("chat.assistant.final:u-1"));
    let fingerprint = write["fingerprint"].as_str().expect("带指纹");
    assert!(
        fingerprint.starts_with(&base) && fingerprint.contains("|retry:removed-1"),
        "指纹换到重试纪元：{fingerprint}"
    );
}

/// 重试本身也失败、节点再被删一次 ⇒ 换**下一个**纪元（链走到最新那一笔）。
#[test]
fn a_second_retry_moves_to_the_next_epoch() {
    let (mut facts, _) = facts_with(2, 100, 100);
    let base = format!("u-0|{LIMIT}|");
    let first = format!("{WRITE_ROW}:{base}");
    facts.note_closing(&first, Closing::Failed, failed_payload("u-1"));
    facts.note_fact(EVENT_MESSAGE_REMOVED, "removed-1");
    facts.note_fact_payload(
        EVENT_MESSAGE_REMOVED,
        "removed-1",
        json!({ "ids": [format!("compression:{base}")] }),
    );

    // 第一次重试也判死：它的键是 `base|retry:removed-1`，节点身份带同一纪元。
    let retry_key = format!("{base}|retry:removed-1");
    let mut retried = failed_payload("u-1");
    retried["fingerprint"] = json!(retry_key);
    retried["compression_id"] = json!(format!("compression:{retry_key}"));
    facts.note_closing(
        &format!("{WRITE_ROW}:{retry_key}"),
        Closing::Failed,
        retried,
    );
    // 没有第二次移除 ⇒ 抑制（不把同一把键重付）。
    assert!(
        CompactDebts.rows(&drive(), &facts).is_empty(),
        "第二次失败尚未被显式重试 ⇒ 抑制"
    );

    facts.note_fact(EVENT_MESSAGE_REMOVED, "removed-2");
    facts.note_fact_payload(
        EVENT_MESSAGE_REMOVED,
        "removed-2",
        json!({ "ids": [format!("compression:{retry_key}")] }),
    );
    let rows = CompactDebts.rows(&drive(), &facts);
    assert_eq!(rows.len(), 2, "第二次显式重试 ⇒ 两行重新欠");
    let fingerprint = rows[1].payload["fingerprint"].as_str().expect("带指纹");
    assert_eq!(
        fingerprint,
        format!("{base}|retry:removed-2"),
        "纪元换到最后一次移除，而不是停在第一次"
    );
}

/// **本轮**刚落成的压缩不算代：否则同一轮立刻换一把键，把同一段重压一遍。
#[test]
fn this_rounds_consolidation_does_not_move_the_generation() {
    let (mut facts, users) = facts_with(2, 100, 100);
    let (_, write) = rows_of(&facts);
    let fingerprint = write["fingerprint"].clone();

    // 本轮的压缩落成（root_id = 当前用户）：代不动，键不变。
    facts.note_fact(SESSION_CONSOLIDATED, "consolidated-now");
    facts.note_fact_payload(
        SESSION_CONSOLIDATED,
        "consolidated-now",
        json!({ "root_id": users.last() }),
    );
    let (_, same) = rows_of(&facts);
    assert_eq!(same["fingerprint"], fingerprint, "本轮压缩不进代");

    // 收 `Ok` ⇒ 同一把键不再欠（成功路径的抑制）。
    facts.note_closing(
        &format!("{WRITE_ROW}:{}", fingerprint.as_str().expect("带指纹")),
        Closing::Ok,
        json!({}),
    );
    assert!(
        CompactDebts.rows(&drive(), &facts).is_empty(),
        "这一代已压过 ⇒ 不重付"
    );

    // 上一轮的压缩才进代：下一轮有资格再压，键随代前进。
    facts.note_fact(SESSION_CONSOLIDATED, "consolidated-before");
    facts.note_fact_payload(
        SESSION_CONSOLIDATED,
        "consolidated-before",
        json!({ "root_id": users[users.len() - 2] }),
    );
    let rows = CompactDebts.rows(&drive(), &facts);
    assert_eq!(rows.len(), 2, "换代 ⇒ 新一轮重新欠");
    let next = rows[1].payload["fingerprint"].as_str().expect("带指纹");
    assert!(
        next.ends_with("consolidated-before"),
        "代取上一轮的压缩：{next}"
    );
}
