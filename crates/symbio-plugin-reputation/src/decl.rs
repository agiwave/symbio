//! 声誉台账能力包的那一条视图（名册 #22/#28，DE-038 §2/§3 的「声誉 = 一个投影」）。
//!
//! ## 折出来的是什么
//!
//! [`REPUTATION_LEDGER`] 把一份事实切片按**立约方**累计成一本账：
//!
//! ```text
//! { "by_principal": { "<主体>": { principal, offered, kept, broken, score }, … 按主体字典序 } }
//! ```
//!
//! `offered` 是 as-of 内立下的承诺数，`kept` / `broken` 是其中已了结的两半，`score` 是
//! 打分。声誉记在**承诺方**头上（`commitment.opened` 的载荷 `from`），不是记在事件写出方
//! （`actor`）头上——委托由父主体发起、由子主体执行，账要落在「谁许的诺」。
//!
//! ## 三条口径（与搬出前逐字相同）
//!
//! - **配对靠载荷 `id`**：收束格（`released` / `broken`）经 `id` 找回立约方，`from` 是
//!   立约时的事实、收束时不重抄；认不出的了结**跳过**而不是猜「大概履行了吧」——切片被
//!   截断时猜一次就往 `kept` 里掺一次假账（与 `commitment_records` 同一条 fail-closed）；
//! - **as-of**：`ts > now` 的立约与收束都不参与，「截至何时」由调用方给（与 `recall`
//!   同口径，未来的收束不影响当前声誉）；
//! - **排序**：`BTreeMap` 按主体字典序 ⇒ 同一份切片永远折出同一串字节（N1 双跑一致）。
//!
//! ## 打分是平凡值，不是架构
//!
//! `score = 守约 − 违约`（[`plain_score`]）：换算法改那一处，不加机制。本包交的是**数**，
//! 判决住在读它的那一侧（本包的淘汰贡献者问「低到该收回派活权没有」，阈值是它的词汇，
//! `DE-043`）。
//!
//! ## 自报不进账
//!
//! `commitment.asserted`（对等主体自己宣告的履约）与两条收束格长得像，但它**不是**一次
//! 了结：主体可以宣告自己守约，却没法宣告一次「立约」发生了。折叠只认 `opened` 起的那
//! 一笔配对，所以自报永远改变不了账——声誉的定义就是「可被别人观测的那一份」（DE-038 §2）。
//!
//! ## 为什么 `reads` 这四对落在断言格通道
//!
//! `commitment.*` 有 `EventDecl`（owner = `loop`，它同时是会话统计的读方）但**没有**
//! `FactDecl`——那一族格子不属于义务环，给它补一份事实声明等于把词汇塞回控制面
//! （`EX-007` 判「碰过机制面」的那一侧）。所以这四对按 `D-282` 的第二种读法登记：
//! 名字由 `EventDecl` 保证拼对，字段无从校验；装载期进 `Frozen::assertion_reads()`
//! 的名单，不是静默跳过。

use std::collections::BTreeMap;

use serde_json::{json, Value};

use symbio_plugin_api::events::{
    EVENT_COMMITMENT_BROKEN, EVENT_COMMITMENT_OPENED, EVENT_COMMITMENT_RELEASED,
};
use symbio_plugin_api::reports::{ReportCtx, ReportDecl};
use symbio_plugin_api::{Entity, Event, EventDecl, PluginDeclarations, Verb, ViewDecl};

/// 淘汰裁决的名字（本包自己写、自己声明；单消费方，不进共享表）。
///
/// 这里是**写方**的声明，所以名字随本包走，不放 `plugin_api::events`。
pub(crate) const EVENT_VERDICT_ASSERTED: &str = "verdict.asserted";

/// 裁决的归属主体：仲裁者（`principal_*` 注册表里已登记的行，`assert.verification`）。
///
/// 这是 `EventDecl::owner` 的第二态——「作者主体 ≠ 会话主体」：写裁决的那一轮可能是
/// 任一省话，但裁决的签名者恒为仲裁者。装配期会拿它查主体注册表，查不到即拒绝装配
/// （D-339）；`plugin-api` 门面没有导出 `PRINCIPAL_ARBITER`，本包又不依赖 SDK
/// （E-009），故取登记表里同一个字面量。
const VERDICT_AUTHOR: &str = "session/arbiter";

/// 本包交出的事件格：裁决落 `verdict × asserted`，载荷由 [`crate::elimination`] 给。
pub(crate) const EVENTS: &[EventDecl] =
    &[
        EventDecl::new(EVENT_VERDICT_ASSERTED, (Entity::Verdict, Verb::Asserted))
            .owned(VERDICT_AUTHOR),
    ];

/// 对外可读的那条视图。名字只在本 crate 出现一次（`SV-001`）；公开动作是下面的
/// 报表登记（`D-387`），读方按报表的 `views` 取，不依赖本 crate（`D-221`）。
pub(crate) const REPUTATION_LEDGER: &str = "reputation.ledger";

/// owner —— 与目录名同形（`crates/symbio-plugin-reputation`），装配表按它排序与报缺口。
pub(crate) const OWNER: &str = "crates/symbio-plugin-reputation";

/// 扫描成本上界（事件条数）。它是环内窗口求值的预算；跨插件读口不查它——读侧按
/// 调用方给的切片全量折（`D-284`）。
///
/// 声明的上界必须与全仓**同一个值**（`seal` 的 `ViewCostMismatch`：切点只有一个），
/// 所以这里不按「承诺通常没几条」调小——调小会改整张表的切点。
const COST: u32 = 4096;

pub(crate) const VIEWS: &[ViewDecl] = &[ViewDecl {
    name: REPUTATION_LEDGER,
    ver: 1,
    // 立约读两列（`id` 是配对的键、`from` 是记账的主体），两条收束格各读 `id`。
    reads: &[
        (EVENT_COMMITMENT_OPENED, "id"),
        (EVENT_COMMITMENT_OPENED, "from"),
        (EVENT_COMMITMENT_RELEASED, "id"),
        (EVENT_COMMITMENT_BROKEN, "id"),
    ],
    // 台账不替任何事实关门：它是读数，不是证据。
    evidence: &[],
    cost: COST,
    eval: ledger,
    agg: None,
}];

/// 平凡打分：守约 − 违约。
///
/// 它是闭包里唯一的「算法」，也是本包刻意交得**很少**的那一半——`offered` 现在不进
/// 算式（立过多少约不改变「这个人靠不靠得住」），留着它是账目透明：`score` 的两个
/// 操作数就摆在同一行里，读的人不必回头猜折法。
fn plain_score(kept: u64, broken: u64) -> i64 {
    kept as i64 - broken as i64
}

/// 按主体累计承诺的守约与违约。纯函数 ⇒ 同一切片永远同一本账（N1）。
///
/// 认输入只认**这三条名字**，不认格子：`Commitment × Closed` 上挤着 `released` 与
/// `broken` 两个名字，按格子认就得再问一次名字，而 `asserted` 也在同一实体上——
/// 「守约」与「自报守约」的差别正是本视图的全部口径（见模块文档「自报不进账」）。
pub(crate) fn ledger(events: &[Event], now: i64) -> Value {
    let mut offered_of: BTreeMap<String, u64> = BTreeMap::new();
    // commitment id → 立约方（收束格经它找回 `from`）。
    let mut open: BTreeMap<String, String> = BTreeMap::new();
    let mut kept_of: BTreeMap<String, u64> = BTreeMap::new();
    let mut broken_of: BTreeMap<String, u64> = BTreeMap::new();
    for e in events {
        if e.ts > now {
            continue;
        }
        let id = e
            .payload
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if e.kind == EVENT_COMMITMENT_OPENED {
            let from = e
                .payload
                .get("from")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            *offered_of.entry(from.clone()).or_insert(0) += 1;
            open.insert(id, from);
        } else if e.kind == EVENT_COMMITMENT_RELEASED || e.kind == EVENT_COMMITMENT_BROKEN {
            // 认不出的了结：跳过，不猜。
            let Some(from) = open.get(&id) else { continue };
            let from = from.clone();
            if e.kind == EVENT_COMMITMENT_BROKEN {
                *broken_of.entry(from).or_insert(0) += 1;
            } else {
                *kept_of.entry(from).or_insert(0) += 1;
            }
        }
    }
    let by_principal: serde_json::Map<String, Value> = offered_of
        .into_iter()
        .map(|(principal, offered)| {
            let kept = *kept_of.get(&principal).unwrap_or(&0);
            let broken = *broken_of.get(&principal).unwrap_or(&0);
            (
                principal.clone(),
                json!({
                    "principal": principal,
                    "offered": offered,
                    "kept": kept,
                    "broken": broken,
                    "score": plain_score(kept, broken),
                }),
            )
        })
        .collect();
    json!({ "by_principal": Value::Object(by_principal) })
}

// 报表登记（`D-387`）：`session/stats` 的 `reputation` 顶层键。`views` 是 owner 自己的
// 常量；台账本体与 `own` 行都在 [`report`] 里排，读方不再替 owner 猜形状。
symbio_plugin_api::submit_reports!([ReportDecl {
    owner: "reputation",
    slot: "reputation",
    order: 50,
    views: &[REPUTATION_LEDGER],
    fleet: None,
    render: report,
    fleet_render: None,
    panel: None,
}]);

/// 声誉列的排版：台账原样交出，另附目标会话主体那一行。
///
/// 本函数不加总、不打分——「守约 − 违约」的算式在 [`ledger`] 里。它只做两件事：按
/// 主体取一行，以及没立过约时补一行零。那一行零不是重算，是让出口恒有一格 `own`
/// 可读（`t75` 判的正是它的 `principal` 必须是**目标会话**的主体，不是请求方的——
/// 跨会话查数时两者不同）。
///
/// 无读权限 ⇒ 整列为空对象：零值等于替一个你本来就看不得的主体报数。
fn report(views: &[Value], ctx: &ReportCtx<'_>) -> Value {
    if !ctx.readable {
        return json!({});
    }
    let ledger = match views.first() {
        Some(value) if !value.is_null() => value.clone(),
        _ => json!({ "by_principal": {} }),
    };
    let rows = ledger
        .get("by_principal")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let own = rows.get(ctx.principal).cloned().unwrap_or_else(|| {
        json!({
            "principal": ctx.principal,
            "offered": 0,
            "kept": 0,
            "broken": 0,
            "score": 0,
        })
    });
    json!({ "own": own, "by_principal": rows })
}

// 装配口的提交：宿主链接本 crate 即生效。本插件不认识也不依赖任何兄弟插件。
symbio_plugin_api::submit_declarations!(OWNER, submitted_declarations);

fn submitted_declarations() -> PluginDeclarations<'static> {
    PluginDeclarations {
        events: EVENTS,
        facts: &[],
        views: VIEWS,
        ports: &[],
        actors: Vec::new(),
        rows: &[],
    }
}

#[cfg(test)]
#[path = "decl.test.rs"]
mod tests;
