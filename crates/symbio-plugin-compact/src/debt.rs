//! 压缩债：**两行**（D-336），身份全部从事实派生（禁计数），永不读数值载荷。
//!
//! | 行 | 端口 | 执行体在哪 | 做什么 |
//! |---|---|---|---|
//! | `compact-summarize` | `model.assist`（model 插件） | model | 一次问答：把本轮转写压成摘要 |
//! | `compact-write` | `compact`（本插件） | 本插件 | 读回摘要，落 `session.consolidated` |
//!
//! ## 为什么必须两行
//!
//! 执行体归**端口提供方**：`model.assist` 的执行体不认识 `session.consolidated`，
//! 本插件的执行体不调模型。一行做完等于让模型插件写会话世界的事实（D-336 裁决 2）。
//!
//! ## 四谓词
//!
//! 两行各自的身份都由**这一轮**的用户消息派生：
//! - `when`：本轮有终答这一**种类**；
//! - `when_id`：钉死**本轮**那条终答（上一轮的终答不算）；
//! - `unless_id`：钉死「已压缩」（同一轮只压一次）；
//! - `after`：`compact-summarize` 等 `model:<uid>` 收束；`compact-write` 等
//!   `compact-summarize:<uid>` 收束——**按债键钉住**，不靠尝试号猜。
//!
//! 超没超水位在这里判（**欠不欠**），但读的是 `model` 那笔收束的载荷：`context`
//! 的 `request_tokens` / `limit_tokens` 由模型执行体按 provider 实测写入。没超 ⇒
//! 一行都不欠，`model.assist` 不会被派发——阈值住插件内，且不拿每轮一次白问去换。
//!
//! 行的载荷只带身份与**切片边界**（`root_id` / `final_id` / 切片两端事实 id /
//! `keep_from`）：摘要正文仍从事实读，世界语义仍归本插件（D-336 / D-401）。

use std::sync::Arc;

use serde_json::{json, Value};
use symbio_plugin_api::debt::{debt_row, DebtFacts, DebtSource, DebtSpec, DriveRequest};
use symbio_plugin_api::events::{EVENT_CHAT_ASSISTANT_FINAL, EVENT_USER_MESSAGE};
use symbio_plugin_api::text::{default_tokenizer, Tokenizer};
use symbio_plugin_api::CapabilityRow;

use crate::port::{COMPACT_PORT, SESSION_CONSOLIDATED};

/// 触发阈值（水位占比）。**住插件内**：不下发 `session.config`，不进契约。
const COMPACT_AT: f64 = 0.7;

/// 摘要输入占窗口的比例（k）。
///
/// 取 **0.5**：另一半留给摘要输出与协议余量——多数 provider 把输出也算进同一个
/// 窗口，摘要自己还可能很长；而 system 指令与消息包装从这一半里再先扣除（见
/// [`slice_plan`]）。k 不随用例调：它是「输入最多占一半」这条策略本身。
const SUMMARY_INPUT_RATIO: f64 = 0.5;

/// 整个请求的信封预留（model 名 / `stream` 等协议字段、JSON 包装）。
const REQUEST_ENVELOPE_TOKENS: usize = 48;

/// 单条消息的协议开销（role / 分隔符 / JSON 包装）的保守估计。
const MESSAGE_ENVELOPE_TOKENS: usize = 12;

/// 截断标记（`……（中间省略）……`）自身的开销。
const ELISION_MARKER_TOKENS: usize = 8;

/// 窗口小到连指令都放不下时的最小切片：宁可送一小段并留痕，
/// 也不送整份注定失败的转写（真实 provider 会直接按超窗拒绝）。
const MIN_SLICE_TOKENS: usize = 16;

/// 一行债的名字（环的读数与日志用它，不是契约）。
const SUMMARIZE_ROW: &str = "compact-summarize";
/// 另一行债的名字。
const WRITE_ROW: &str = "compact-write";

/// 一次性问答端口的名字（数据）：名字住 `plugin-api` 的词汇面（D-364），
/// 提供者 `model` 与本消费方 import 同一常量。
pub(crate) use symbio_plugin_api::vocab::PORT_MODEL_ASSIST as ASSIST_PORT;

/// 摘要指令（**本插件的世界知识**，随行载荷交给 `model.assist` 的执行体）。
///
/// 执行体只按这份 system 发一次问答；它不认识「压缩」——压缩策略归本插件。
pub(crate) const SUMMARIZE_INSTRUCTION: &str =
    "你是上下文压缩器。把下面的对话压缩成一段简洁的中文摘要，\
保留关键事实、决定、未完成事项与用户偏好；不要编造。只输出摘要正文。";

pub(crate) struct CompactDebts;

impl DebtSource for CompactDebts {
    fn rows(&self, _request: &DriveRequest, facts: &DebtFacts) -> Vec<DebtSpec> {
        let Some(user_id) = facts.latest_fact_id(EVENT_USER_MESSAGE) else {
            return Vec::new();
        };
        let Some((used, limit)) = watermark(facts, user_id) else {
            return Vec::new();
        };
        if (used as f64) < limit as f64 * COMPACT_AT {
            return Vec::new();
        }
        // 装不下就没有可压的段：不派发注定失败的往返（正文见 `slice_plan`）。
        let Some(plan) = slice_plan(facts, limit) else {
            return Vec::new();
        };
        let final_id = format!("{EVENT_CHAT_ASSISTANT_FINAL}:{user_id}");
        let consolidated_id = format!("{SESSION_CONSOLIDATED}:{user_id}");
        let summarize_id = format!("{SUMMARIZE_ROW}:{user_id}");
        vec![
            // 拿摘要：欠 model 插件的端口。载荷只带身份 + 切片计划（都是本轮的
            // 事实 id 与预算参数，不是世界语义）；正文由执行体从事实读。
            DebtSpec::new(
                SUMMARIZE_ROW,
                ASSIST_PORT,
                summarize_id.clone(),
                json!({
                    "root_id": user_id,
                    "final_id": final_id,
                    "system": SUMMARIZE_INSTRUCTION,
                    "user": "",
                    "slice": {
                        "from": plan.from,
                        "to": plan.to,
                        "truncate": plan.truncate,
                    },
                }),
            )
            .when(EVENT_CHAT_ASSISTANT_FINAL)
            .when_id(EVENT_CHAT_ASSISTANT_FINAL, final_id.clone())
            .unless_id(SESSION_CONSOLIDATED, consolidated_id.clone())
            .after(format!("model:{user_id}")),
            // 落压缩事实：必须等**本轮那笔 summarize** 收束；`after` 写的是它的债键，
            // 不是尝试号派生出来的键（失败后重试的尝试号与本行的判据无关）。
            DebtSpec::new(
                WRITE_ROW,
                COMPACT_PORT,
                format!("{WRITE_ROW}:{user_id}"),
                // 载荷只带身份 + 切片边界：摘要正文从 summarize 的收束事实读，
                // 水位从终答事实读；`keep_from` 告诉读面从哪条事实起接着折。
                json!({
                    "root_id": user_id,
                    "final_id": final_id,
                    "after_port": ASSIST_PORT,
                    "keep_from": plan.keep_from,
                    "split": plan.split,
                }),
            )
            .when(EVENT_CHAT_ASSISTANT_FINAL)
            .when_id(EVENT_CHAT_ASSISTANT_FINAL, final_id)
            .unless_id(SESSION_CONSOLIDATED, consolidated_id)
            .after(summarize_id),
        ]
    }
}

/// 一次摘要的切片计划：**只压最旧的前缀**，其余原样留给读面。
///
/// `from` / `to` 是事实 id（含两端）；`truncate` 是「这条事实只取头尾」的
/// 字符预算；`keep_from` 是首条**未压**事实 id（`None` = 整份都压掉了）。
struct SlicePlan {
    from: String,
    to: String,
    truncate: serde_json::Map<String, Value>,
    keep_from: Option<String>,
    split: bool,
}

/// 按预算从最旧的事实取切片。
///
/// 预算 = `k × limit` 先减去摘要指令与请求/消息信封——执行体据此装请求，摘要
/// 请求的输入因此不会超过 `k × limit`。按**消息边界**停：下一条装不下就留给读面；
/// 只有「最旧一条自己就超过预算」才头尾截断（头 3/5、尾 2/5），并记 `split`。
/// 事实读面里没有可压的对话线（没有 user/final 正文）⇒ 不欠这一笔。
fn slice_plan(facts: &DebtFacts, limit: u64) -> Option<SlicePlan> {
    let nodes = transcript_of(facts);
    if nodes.is_empty() {
        return None;
    }
    let tokenizer = default_tokenizer();
    let budget = ((limit as f64) * SUMMARY_INPUT_RATIO).floor() as usize;
    let slice_budget =
        budget.saturating_sub(tokenizer.count(SUMMARIZE_INSTRUCTION) + REQUEST_ENVELOPE_TOKENS);

    let mut used = 0usize;
    let mut included: Vec<(String, Option<(usize, usize)>)> = Vec::new();
    let mut split = false;
    let mut excluded_at = None;
    for (index, (id, text)) in nodes.iter().enumerate() {
        let cost = tokenizer.count(text) + MESSAGE_ENVELOPE_TOKENS;
        if used + cost <= slice_budget {
            used += cost;
            included.push((id.clone(), None));
            continue;
        }
        if included.is_empty() {
            // 最旧一条自己就超预算：头尾截断（3:5 / 2:5），并留下「被切开过」。
            // 字符数取 token 预算本身：本仓估算器里「1 字符 ≤ 1 token」（CJK 最高
            // 1 token/字），因此按字符截是硬上限，不会把预算再放大一倍。
            // 窗口小到连指令都放不下时仍取最小切片：一份非空的短切片比整份
            // 注定超窗的转写更有机会拿到摘要。
            let available = slice_budget
                .saturating_sub(MESSAGE_ENVELOPE_TOKENS + ELISION_MARKER_TOKENS)
                .max(MIN_SLICE_TOKENS);
            let head = available * 3 / 5;
            let tail = available - head;
            included.push((id.clone(), Some((head, tail))));
            split = true;
            excluded_at = Some(index + 1);
        } else {
            excluded_at = Some(index);
        }
        break;
    }
    let (from, _) = included.first()?;
    let (to, _) = included.last()?;
    let mut truncate = serde_json::Map::new();
    for (id, split_budget) in &included {
        if let Some((head, tail)) = split_budget {
            truncate.insert(
                id.clone(),
                json!({ "head_chars": head, "tail_chars": tail }),
            );
        }
    }
    Some(SlicePlan {
        from: from.clone(),
        to: to.clone(),
        truncate,
        keep_from: excluded_at
            .filter(|index| *index < nodes.len())
            .map(|index| nodes[index].0.clone()),
        split,
    })
}

/// 摘要输入的对话线：用户发言 / 助手终答各一条，按事实顺序（与
/// `model.assist` 执行体的角色投影同一口径）。
fn transcript_of(facts: &DebtFacts) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (user_id, payload) in facts.facts_of(EVENT_USER_MESSAGE) {
        if let Some(text) = text_field(payload, &["text"]) {
            out.push((user_id.to_string(), text));
        }
        let final_id = format!("{EVENT_CHAT_ASSISTANT_FINAL}:{user_id}");
        if let Some(payload) = facts.fact_payload(EVENT_CHAT_ASSISTANT_FINAL, &final_id) {
            if let Some(text) = text_field(payload, &["text", "answer"]) {
                out.push((final_id, text));
            }
        }
    }
    out
}

fn text_field(payload: &Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        let Some(text) = payload.get(*key).and_then(Value::as_str) else {
            continue;
        };
        let text = text.trim();
        if !text.is_empty() {
            return Some(text.to_string());
        }
    }
    None
}

/// 本插件申报的债行（`debt` 族的登记）。
pub(crate) fn compact_debt_rows() -> Vec<CapabilityRow> {
    vec![debt_row("compact", Arc::new(CompactDebts))]
}

/// 本轮水位：`(request_tokens, limit_tokens)`。数据来自 `model:<uid>` 的
/// **收束载荷**（provider 实测），缺任一项 ⇒ 没有可信水位（宁可少压，不拿猜测去压）。
///
/// **已知项（D-336）**：`context.request_tokens` / `limit_tokens` 由 model 执行体写，
/// 不是 SDK 共享类型；当前唯一消费者是本插件。第二个插件需要同一字段时必须先把它
/// 提升为 SDK 共享类型，不许各自猜载荷形状。
fn watermark(facts: &DebtFacts, user_id: &str) -> Option<(u64, u64)> {
    let (_, payload) = facts.closing(&format!("model:{user_id}"))?;
    let context = payload.get("context")?;
    let used = context.get("request_tokens").and_then(Value::as_u64)?;
    let limit = context.get("limit_tokens").and_then(Value::as_u64)?;
    (limit > 0).then_some((used, limit))
}

#[cfg(test)]
#[path = "debt.test.rs"]
mod tests;
