//! 轮边界汇报（`chat.assistant.reported`）：会话的债行 + 落格行。
//!
//! ## 为什么是会话自己的债行
//!
//! 「什么时候该打断用户」是**编排**的判断（静默多久、跑了几轮、说过几次），
//! 而这些量都在会话状态里；措辞只归 compose（`progress_text` 把 [`RunSnapshot`]
//! 说成人话，零往返）。环里没有「轮边界」这个钩子——计划一次算全、按批派发，
//! 任何执行体都不许在发送前再查一次事实（D-342）。所以汇报做成一条独立事实与
//! 一笔独立债：
//!
//! ```text
//! session.progress（plugin_row，债 id 由边界事实派生）
//!   ├─ when   `tool.calls.done:<tool.requested id>`（走完一整轮工具才欠一句）
//!   ├─ unless `chat.assistant.reported:<同一边界>`（同一边界至多一次）
//!   └─ 执行体经 route 调 `compose/wording` 取措辞 → 落 `chat.assistant.reported`
//! ```
//!
//! **不写「第几轮」**：边界身份就是 tool 端口落的 `tool.calls.done` 事实 id，
//! 它本身由那一轮的 `tool.requested` 派生；轮次序数只是读事实时数出来的位置，
//! 不是代码里的字面量。停等批次不落 `tool.calls.done`，因此「完成」是真的完成。
//!
//! ## 为什么不是收束贡献者 / 独立插件
//!
//! 汇报要在**末轮正文之前**到达（它是给正在等的用户看的），而收束贡献者问在
//! `project_final_turn`（收束之后）——放进那个缝时刻就反了。独立插件则没有
//! `chat.assistant.reported` 的写口：转写只归会话写（D-020），搬出去只是把策略
//! 搬到远处。`Verdict::Report` 的产出方本来就是 session（见 `symbio-kernel`
//! `dialog` 的「判决归谁」一表）。
//!
//! ## 策略数字住在配置里，不住在本文件
//!
//! 租约、尝试上界、静默阈值、最少轮次、每轮上限都从**当轮** `session.config`
//! 读（`SessionConfig` 是这些数的唯一 owner）；本文件只把它们转交 `DebtSpec` /
//! 行载荷，不写字面量。`progress_interval_ms` 是唯一不能进 `when` / `unless`
//! 的门——`DebtFacts` 不带时间；按首响行的先例「不满足就不落」，判定在行里用
//! `now_ms` 与事实 `ts` 做。

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::{json, Value};

use symbio_plugin_api::debt::{session_config_id, DebtFacts, DebtSource, DebtSpec, DriveRequest};
use symbio_plugin_api::events::{
    EVENT_CHAT_ASSISTANT_FINAL, EVENT_SESSION_CONFIG, EVENT_TOOL_REQUESTED, EVENT_USER_MESSAGE,
};
use symbio_plugin_api::rows::{RowHandler, RowRequest, RowStep, PLUGIN_ROW};
use symbio_plugin_api::{
    Entity, ExecEventSink, FactDraft, PluginInvokeRequest, RunSnapshot, Verb, Verdict,
};
use symbio_sdk::turn::facts::ReportDraft;

use crate::config::{
    default_progress_enabled, default_progress_interval_ms, default_progress_lease_ms,
    default_progress_max_attempts, default_progress_max_per_turn, default_progress_min_rounds,
};
use crate::declarations::{CHAT_VOCAB, EVENT_CHAT_ASSISTANT_REPORTED, OWNER};
use crate::wording::compose_wording;

/// 汇报行的行名 / 债名前缀。
pub(crate) const SESSION_PROGRESS: &str = "session.progress";

/// 工具端口落的「一整轮已完成」事实：名字由 owner（tool）的清单生成，
/// 本插件 import 同一常量（D-364）。
use symbio_plugin_api::events::EVENT_TOOL_CALLS_DONE as TOOL_CALLS_DONE;

/// 汇报的落格行：取措辞 → 落 `chat.assistant.reported`；取不到就**不落**。
pub(crate) struct ProgressRow;

impl RowHandler for ProgressRow {
    fn owner(&self) -> &'static str {
        OWNER
    }

    fn row(&self) -> &'static str {
        SESSION_PROGRESS
    }

    fn writes(&self) -> &'static [&'static str] {
        &[EVENT_CHAT_ASSISTANT_REPORTED]
    }

    fn run(&self, _req: &RowRequest<'_>) -> RowStep {
        // 本行要经 route 取措辞，同步入口只满足 trait 的契约位；派发永远走 `run_ctx`。
        RowStep::Facts(Vec::new())
    }

    fn run_ctx<'a>(
        &'a self,
        req: &'a RowRequest<'_>,
        ctx: Option<Arc<dyn PluginInvokeRequest>>,
    ) -> Pin<Box<dyn Future<Output = RowStep> + Send + 'a>> {
        Box::pin(async move { progress_step(req, ctx).await })
    }
}

/// 取措辞 → 落汇报事实；取不到就**不落**（绝不自己编一句）。
async fn progress_step(req: &RowRequest<'_>, ctx: Option<Arc<dyn PluginInvokeRequest>>) -> RowStep {
    let Some(boundary) = req.key_of("boundary") else {
        return RowStep::Facts(Vec::new());
    };
    let rounds = req
        .key_of("rounds")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);
    let interval_ms = req
        .key_of("interval_ms")
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(0);
    // 静默起点：对话线上用户看得见的上一次动静（用户发言 / 助手说话）。
    // `req.facts` 只含本轮切片，所以这条读数天然是「本轮之内」的。
    let quiet_ms = req
        .facts
        .iter()
        .filter(|e| e.kind == EVENT_USER_MESSAGE || CHAT_VOCAB.is_spoken(&e.kind))
        .map(|e| e.ts)
        .max()
        .map(|ts| (req.now_ms - ts).max(0))
        .unwrap_or(0);
    if quiet_ms < interval_ms {
        return RowStep::Facts(Vec::new());
    }
    let Some(ctx) = ctx else {
        return RowStep::Facts(Vec::new());
    };
    let verdict = Verdict::Report;
    let snapshot = RunSnapshot {
        tool_rounds: rounds,
        quiet_ms,
    };
    let Some(reply) = compose_wording(&ctx, req.facts, verdict, snapshot).await else {
        return RowStep::Facts(Vec::new());
    };
    let text = reply.text;
    if text.trim().is_empty() {
        return RowStep::Facts(Vec::new());
    }
    // 实时下发：汇报是说给**正在等**的用户听的，不能等轮末的统一投影。节点形状只
    // 出自 SDK 的唯一构造器（[`ReportDraft::to_message`]），本行只递参数；`id` 与
    // 落格载荷同值，轮末重建会按同一 id 覆盖权威副本，实时面因此不多出第二张卡。
    let mut node = ReportDraft {
        text: text.clone(),
        tool_rounds: rounds,
        quiet_ms,
    }
    .to_message(&symbio_kernel::chat::principal::Principal::from_legacy(
        req.principal,
    ));
    node.id = format!("report:{boundary}");
    ExecEventSink::of(ctx.as_ref()).emit(node).await;
    RowStep::Facts(vec![FactDraft {
        // 节点身份与事件身份分开：键是本事实的幂等身份（`unless` 按它判），
        // `id` 是投影读的节点 id。
        key: format!("{EVENT_CHAT_ASSISTANT_REPORTED}:{boundary}"),
        kind: EVENT_CHAT_ASSISTANT_REPORTED.to_string(),
        entity: Entity::Scope,
        verb: Verb::Asserted,
        payload: json!({
            "id": format!("report:{boundary}"),
            "text": text,
            "tool_rounds": rounds,
            "quiet_ms": quiet_ms,
        }),
        produced_by: Some(req.user_seq),
    }])
}

/// 汇报的**运行时债表**（无状态：每轮从事实视图现算）。
///
/// 一「轮边界」一笔债：边界身份是 `tool.calls.done` 的事实 id（由那一轮的
/// `tool.requested` 派生），不是第几轮的数字。开关、最少轮次与每轮上限读
/// **当轮** `session.config`（上一轮的配置不算这一轮的账）；租约与尝试上界同理
/// ——它们是策略，由配置持有，本行不写字面量。
pub(crate) struct ProgressDebts;

impl DebtSource for ProgressDebts {
    fn rows(&self, _request: &DriveRequest, facts: &DebtFacts) -> Vec<DebtSpec> {
        let Some(user_id) = facts.latest_fact_id(EVENT_USER_MESSAGE) else {
            return Vec::new();
        };
        let config = facts.fact_payload(EVENT_SESSION_CONFIG, &session_config_id(user_id));
        let enabled = config
            .and_then(|c| c.get("progress_enabled"))
            .and_then(Value::as_bool)
            .unwrap_or_else(default_progress_enabled);
        if !enabled {
            return Vec::new();
        }
        // 本轮已经答完：这之后再说「还在继续」是假的，一句都不欠。
        let final_id = format!("{EVENT_CHAT_ASSISTANT_FINAL}:{user_id}");
        if facts
            .fact_payload(EVENT_CHAT_ASSISTANT_FINAL, &final_id)
            .is_some()
        {
            return Vec::new();
        }
        let min_rounds = config_u64(
            config,
            "progress_min_rounds",
            default_progress_min_rounds() as u64,
        );
        let max_per_turn = config_u64(
            config,
            "progress_max_per_turn",
            default_progress_max_per_turn() as u64,
        );
        let lease_ms = config_u64(config, "progress_lease_ms", default_progress_lease_ms());
        let max_attempts = config_u64(
            config,
            "progress_max_attempts",
            default_progress_max_attempts() as u64,
        ) as u32;
        let interval_ms = config_u64(
            config,
            "progress_interval_ms",
            default_progress_interval_ms(),
        );

        let mut rounds = 0u64;
        let mut reported = 0u64;
        let mut specs = Vec::new();
        // `facts_of` 按记入顺序给出——「最早」是位置，不是计数。
        for (done_id, done) in facts.facts_of(TOOL_CALLS_DONE) {
            let Some(requested) = done.get("requested").and_then(Value::as_str) else {
                continue;
            };
            // 这一轮属于哪一轮，看工具请求事实的 `root_id`：它与本轮 `user.message`
            // 同一身份，因此历史轮的边界不会被算进本轮的配额。
            let Some(requested_payload) = facts.fact_payload(EVENT_TOOL_REQUESTED, requested)
            else {
                continue;
            };
            if requested_payload.get("root_id").and_then(Value::as_str) != Some(user_id) {
                continue;
            }
            rounds += 1;
            let report_id = format!("{EVENT_CHAT_ASSISTANT_REPORTED}:{done_id}");
            if facts
                .fact_payload(EVENT_CHAT_ASSISTANT_REPORTED, &report_id)
                .is_some()
            {
                // 这个边界说过了：占一个配额位，但不再欠。
                reported += 1;
                continue;
            }
            if rounds < min_rounds || reported >= max_per_turn {
                continue;
            }
            reported += 1;
            specs.push(
                DebtSpec::new(
                    SESSION_PROGRESS,
                    PLUGIN_ROW,
                    format!("{SESSION_PROGRESS}:boundary={done_id}"),
                    json!({
                        "row": SESSION_PROGRESS,
                        "key": [
                            ["boundary", done_id],
                            ["root_id", user_id],
                            ["rounds", rounds.to_string()],
                            ["interval_ms", interval_ms.to_string()],
                        ],
                    }),
                )
                .when_id(TOOL_CALLS_DONE, done_id)
                .unless_id(EVENT_CHAT_ASSISTANT_REPORTED, report_id)
                .lease(lease_ms, max_attempts),
            );
        }
        specs
    }
}

/// 配置事实里的一个非负整数；缺键时用配置默认（`SessionConfig` 是唯一的数值 owner）。
fn config_u64(config: Option<&Value>, field: &str, fallback: u64) -> u64 {
    config
        .and_then(|c| c.get(field))
        .and_then(Value::as_u64)
        .unwrap_or(fallback)
}

// 链接期提交：本实现随本 crate 进入二进制，宿主按行名选它。
symbio_plugin_api::submit_row!(ProgressRow);
