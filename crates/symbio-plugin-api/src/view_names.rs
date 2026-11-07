//! 跨插件可读视图名 —— **由 `scripts/facts/views-rs.mjs` 生成，请勿手改**。
//!
//! 事实源：各插件自己目录里的 `PLUGIN.yml` 的 `shared_views:` 行。名字、版本、
//! 扫描成本与求值函数仍住在 owner 自己的 `ViewDecl` 里；本表只回答「哪几条视图公开
//! 给别的插件按名字读」。读口见 `symbio_plugin_api::views`（DE-025 D-221）。
//!
//! 手改本文件会被 `gen-facts --check` 判红。

/// 一条公开给别的插件按名字读的视图：名字 + owner。
///
/// `ver` / `cost` / 求值函数不在这里：它们随 owner 的 `ViewDecl` 走，读口在
/// 冻结声明表里查——本表多抄一份就会与 owner 漂移。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SharedViewDecl {
    pub name: &'static str,
    pub owner: &'static str,
}

/// 每条公开视图一个名字常量：消费方 import 它，不写第二遍字面量（D-364）。
pub const VIEW_CAUSAL_LINKS: &str = "causal.links";
pub const VIEW_CONATION_AUTONOMY: &str = "conation.autonomy";
pub const VIEW_CONATION_INTENTS: &str = "conation.intents";
pub const VIEW_COST_LEDGER: &str = "cost.ledger";
pub const VIEW_EXPLAIN_RATIONALES: &str = "explain.rationales";
pub const VIEW_FORECAST_REMAINING: &str = "forecast.remaining";
pub const VIEW_IDEMPOTENCY_LEDGER: &str = "idempotency.ledger";
pub const VIEW_MEMORY_LIVE: &str = "memory.live";
pub const VIEW_NOTES_DIGEST: &str = "notes.digest";
pub const VIEW_NOTES_INSIGHTS: &str = "notes.insights";
pub const VIEW_NOTES_RECENT: &str = "notes.recent";
pub const VIEW_REPUTATION_LEDGER: &str = "reputation.ledger";
pub const VIEW_SKILL_CALIBRATION: &str = "skill.calibration";
pub const VIEW_SKILL_RETREAT: &str = "skill.retreat";
pub const VIEW_SLO_FALLBACK: &str = "slo.fallback";
pub const VIEW_SLO_LATENCY: &str = "slo.latency";
pub const VIEW_SLO_SPAN: &str = "slo.span";
pub const VIEW_TASK_READYSET: &str = "task.readyset";

/// 跨插件可读视图（按名字排序）。
#[rustfmt::skip]
pub const SHARED_VIEWS: &[SharedViewDecl] = &[
    SharedViewDecl { name: VIEW_CAUSAL_LINKS, owner: "causal" },
    SharedViewDecl { name: VIEW_CONATION_AUTONOMY, owner: "conation" },
    SharedViewDecl { name: VIEW_CONATION_INTENTS, owner: "conation" },
    SharedViewDecl { name: VIEW_COST_LEDGER, owner: "cost" },
    SharedViewDecl { name: VIEW_EXPLAIN_RATIONALES, owner: "explain" },
    SharedViewDecl { name: VIEW_FORECAST_REMAINING, owner: "forecast" },
    SharedViewDecl { name: VIEW_IDEMPOTENCY_LEDGER, owner: "idempotency" },
    SharedViewDecl { name: VIEW_MEMORY_LIVE, owner: "memory" },
    SharedViewDecl { name: VIEW_NOTES_DIGEST, owner: "notes" },
    SharedViewDecl { name: VIEW_NOTES_INSIGHTS, owner: "notes" },
    SharedViewDecl { name: VIEW_NOTES_RECENT, owner: "notes" },
    SharedViewDecl { name: VIEW_REPUTATION_LEDGER, owner: "reputation" },
    SharedViewDecl { name: VIEW_SKILL_CALIBRATION, owner: "skill" },
    SharedViewDecl { name: VIEW_SKILL_RETREAT, owner: "skill" },
    SharedViewDecl { name: VIEW_SLO_FALLBACK, owner: "slo" },
    SharedViewDecl { name: VIEW_SLO_LATENCY, owner: "slo" },
    SharedViewDecl { name: VIEW_SLO_SPAN, owner: "slo" },
    SharedViewDecl { name: VIEW_TASK_READYSET, owner: "task" },
];
