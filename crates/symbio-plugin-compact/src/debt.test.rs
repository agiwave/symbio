//! 压缩债的**切片判据**（D-401）：摘要输入永远装得下。
//!
//! 这里只钉申报面能看到的东西：两行载荷里的切片端点 / `keep_from` / `split`，
//! 以及「按预算累计」本身。执行体怎么把切片装成请求由 `model.assist` 那边的判据钉，
//! 端到端由 small-window e2e 钉——三处各司其职，避免一份判据跑三遍。

use serde_json::{json, Value};
use symbio_plugin_api::debt::{Closing, DebtFacts, DebtSource, DriveRequest};
use symbio_plugin_api::events::{EVENT_CHAT_ASSISTANT_FINAL, EVENT_USER_MESSAGE};
use symbio_plugin_api::text::{default_tokenizer, Tokenizer};

use super::{CompactDebts, SUMMARIZE_INSTRUCTION};

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
