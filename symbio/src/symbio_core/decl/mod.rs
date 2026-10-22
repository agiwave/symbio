//! 事实与视图的**声明层**——让「字段都在声明里」第一次可查（PL-016 §3.2 / §3.4）。
//!
//! ## 它解决什么
//!
//! 目标形状里每一行是数据：
//!
//! ```text
//! 行  dispatch-model   owes: request(port=model)   until: sent(port=model) | failed(port=model)
//! ```
//!
//! `Pattern` 匹配的是**字段**（`port=model`），而事实的载荷是 `serde_json::Value`——
//! 没有字段清单。于是拼错一个字段名时，运行期**静默不命中**，装载期也无从检查。
//! 本模块把两个缺失的声明补上：
//!
//! - [`FactDecl`]：一个事实名有哪些载荷字段，以及它是否携带端口、按什么键幂等、
//!   按什么字段展开；
//! - [`ViewDecl`]：一个视图读哪些 `(事实, 字段)`、口径是第几版、扫描成本上界、
//!   以及求值函数。
//!
//! [`Builder::seal`] 于是能回答三件事：事实名不重名、事实与视图不撞名、
//! **视图读的每个字段都出现在某个 `FactDecl.fields` 里**。
//!
//! ## 与 `event::grid` 的分工
//!
//! `CELLS` 管「名字落在哪个格子」，`FactDecl` 管「名字有哪些字段」。两者并存，
//! 不互相推导：格子是实体 × 动词，字段是载荷形状，合并会逼其中一方说谎。
//!
//! ## `port` 是载荷字段，不是信封字段
//!
//! 只有协议族的事实有端口，放信封等于让「每条事实都有端口」变成假话。
//! `FactDecl.port` 声明的是**载荷里承载端口的字段名**（协议族为 `Some("port")`），
//! 因此它也必须出现在 `fields` 里——声明不能自相矛盾。
//!
//! ## 视图 `eval` 的签名就是隔离
//!
//! `fn(&[Event], i64) -> Value`：拿不到写口，也拿不到别的视图；`now` 是入参而不是
//! 视图自己读时钟，同一份日志重放两次必须得到同一结果（ADR-115 决策 1）。
//! PL-016 判据 3（视图互不可见）因此落在类型上，不落在纪律上。
//!
//! ## 首批声明
//!
//! 16 个事实：协议五名（`request` / `sent` / `response` / `failed` / `retry`）、助手文本六格
//! （`chat.assistant.final` / `fallback` / `reported` / `opening` / `progress`）、
//! `turn.stage`、`user.message`、`turn.supplemented`、`session.config`、
//! `turn.dispatch`、`authz.denied`、`authz.granted`；7 个视图：`in_flight` / `stale_sent` / `turn_head` /
//! `turn_stages` / `turn_closed` / `report_due` / `reported_anchors`。
//!
//! 视图点名决定了事实的**下限**：`in_flight` 读三个协议名（`sent` / `response` /
//! `failed` / `retry`），`stale_sent` 读 `sent` 的租约两字段（`lease_ms` /
//! `attempt_cap`）与三个出口，`turn_head` 读 `user.message` / `turn.stage` /
//! `session.config`，`turn_stages` 与 `turn_closed` 读本幕锚之后的阶段 / 收束格，
//! `report_due` 读助手文本六格 / `turn.supplemented` / `turn.stage`（恢复幕）/
//! `session.config` + `now`。
//!
//! ## 生产接线
//!
//! T-221 只做声明层；T-222b 起控制面（`plugins::loop::loop_rows`）经 `owed` 读它，
//! 视图因此有了生产消费方。`Frozen::eval` 的成本上界仍按声明生效：调用方传入
//! **本轮切片**（不是整份 WAL），一轮的事件数由轮的生命周期界定。

use std::collections::HashSet;
use std::sync::Arc;

use crate::symbio_core::Event;
// 协议名的**唯一**定义在 `event/mod.rs` 的事件名字表（T-224 起），并登记在 `CELLS` 上。
// 一个事实名只有一处定义：常量在事件域，格位在网格域，声明层只引用。
use crate::symbio_core::event::{
    EVENT_FAILED, EVENT_REQUEST, EVENT_RESPONSE, EVENT_RETRY, EVENT_SENT,
};

pub(crate) mod ring;

/// 一个事实名的声明。
///
/// 字段全部是数据：`name` 是唯一的身份；`port` / `key` / `expand` 都是**字段引用**，
/// 必须出现在 `fields` 里（`seal` 检查）。空切片 = 这一维不存在，不是「以后再说」。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FactDecl {
    /// 网格名（与 `CELLS` 上的行同名；此处不判格位）。
    pub(crate) name: &'static str,
    /// 载荷里承载端口的字段名。只有协议族有端口；`None` = 这条事实与端口无关。
    pub(crate) port: Option<&'static str>,
    /// 幂等键字段：同键重复派发由适配器 / 存储按它去重。
    pub(crate) key: &'static [&'static str],
    /// 展开字段：一个事实按它展开成多个实例（工具批次用）。
    pub(crate) expand: &'static [&'static str],
    /// 载荷字段全集（含 `port` / `key` / `expand` 引用的字段）。
    pub(crate) fields: &'static [&'static str],
}

/// 一个视图的声明。
#[derive(Debug, Clone, Copy)]
pub(crate) struct ViewDecl {
    /// 视图名。与事实名共用同一名字空间（视图是合成事实）。
    pub(crate) name: &'static str,
    /// 口径版本：视图读法一变，历史事实的投影结果就变；版本让「这一轮用了哪一版」可回放。
    pub(crate) ver: u32,
    /// 读的 `(事实名, 字段名)`。这是「字段都在声明里」那条检查的输入。
    pub(crate) reads: &'static [(&'static str, &'static str)],
    /// 扫描成本上界（事件条数）。`owed` 每轮求值，成本必须有上界。
    pub(crate) cost: u32,
    /// 求值函数：**只有事实切片与时间戳**——没有写口、没有别的视图、没有时钟。
    pub(crate) eval: fn(&[Event], i64) -> serde_json::Value,
}

/// `seal()` 的检查失败项。一次返回全部，不是首错即停。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CheckError {
    /// 两个 `FactDecl` 同名。
    DuplicateFact { name: &'static str },
    /// 两个 `ViewDecl` 同名。
    DuplicateView { name: &'static str },
    /// 事实名与视图名撞名（视图是合成事实，同一名字空间）。
    NameCollision { name: &'static str },
    /// 视图引用了一个未声明的事实。
    UnknownFact {
        view: &'static str,
        fact: &'static str,
    },
    /// 视图引用了一个未声明的字段。
    UndeclaredField {
        view: &'static str,
        fact: &'static str,
        field: &'static str,
    },
    /// 事实自己的 `port` / `key` / `expand` 引用了未声明的字段。
    FactFieldUndeclared {
        fact: &'static str,
        field: &'static str,
        role: &'static str,
    },
}

/// 装载期的唯一注册口。`seal()` 之后没有可变方法（见 [`Frozen`]）。
#[derive(Debug, Default)]
pub(crate) struct Builder {
    facts: Vec<FactDecl>,
    views: Vec<ViewDecl>,
}

impl Builder {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn fact(&mut self, d: FactDecl) {
        self.facts.push(d);
    }

    pub(crate) fn view(&mut self, d: ViewDecl) {
        self.views.push(d);
    }

    /// 冻结并检查。通过 ⇒ [`Frozen`]（只读）；失败 ⇒ 全部错误。
    pub(crate) fn seal(self) -> Result<Arc<Frozen>, Vec<CheckError>> {
        let mut errors = Vec::new();

        let mut fact_names = HashSet::new();
        for f in &self.facts {
            if !fact_names.insert(f.name) {
                errors.push(CheckError::DuplicateFact { name: f.name });
            }
        }
        let mut view_names = HashSet::new();
        for v in &self.views {
            if !view_names.insert(v.name) {
                errors.push(CheckError::DuplicateView { name: v.name });
            }
        }
        for v in &self.views {
            if fact_names.contains(v.name) {
                errors.push(CheckError::NameCollision { name: v.name });
            }
        }

        for f in &self.facts {
            for (role, fields) in [
                ("port", f.port.into_iter().collect::<Vec<_>>()),
                ("key", f.key.to_vec()),
                ("expand", f.expand.to_vec()),
            ] {
                for field in fields {
                    if !f.fields.contains(&field) {
                        errors.push(CheckError::FactFieldUndeclared {
                            fact: f.name,
                            field,
                            role,
                        });
                    }
                }
            }
        }

        for v in &self.views {
            for &(fact, field) in v.reads {
                match self.facts.iter().find(|f| f.name == fact) {
                    None => errors.push(CheckError::UnknownFact { view: v.name, fact }),
                    Some(f) if !f.fields.contains(&field) => {
                        errors.push(CheckError::UndeclaredField {
                            view: v.name,
                            fact,
                            field,
                        })
                    }
                    Some(_) => {}
                }
            }
        }

        if errors.is_empty() {
            Ok(Arc::new(Frozen {
                facts: self.facts,
                views: self.views,
            }))
        } else {
            Err(errors)
        }
    }
}

/// 冻结后的声明表：只有读方法，没有 `&mut self`——运行期注册口在类型上不存在。
#[derive(Debug)]
pub(crate) struct Frozen {
    facts: Vec<FactDecl>,
    views: Vec<ViewDecl>,
}

impl Frozen {
    pub(crate) fn facts(&self) -> &[FactDecl] {
        &self.facts
    }

    pub(crate) fn views(&self) -> &[ViewDecl] {
        &self.views
    }

    pub(crate) fn fact(&self, name: &str) -> Option<&FactDecl> {
        self.facts.iter().find(|f| f.name == name)
    }

    pub(crate) fn view(&self, name: &str) -> Option<&ViewDecl> {
        self.views.iter().find(|v| v.name == name)
    }

    /// 按声明求值：未声明、或扫描条数超过声明的 `cost` 上界 ⇒ `None`。
    /// 超界不静默截断——调用方必须知道这一轮的视图没算出来。
    pub(crate) fn eval(&self, name: &str, events: &[Event], now: i64) -> Option<serde_json::Value> {
        let view = self.view(name)?;
        if events.len() > view.cost as usize {
            return None;
        }
        Some((view.eval)(events, now))
    }
}

/// 首批声明能装载并冻结（T-221 的交付物）。
pub(crate) fn builtin() -> Result<Arc<Frozen>, Vec<CheckError>> {
    let mut b = Builder::new();
    for &f in BUILTIN_FACTS {
        b.fact(f);
    }
    for &v in BUILTIN_VIEWS {
        b.view(v);
    }
    b.seal()
}

// ── 协议名（载荷里带 `port`）────────────────────────────────────────────
//
// 名字住在 `event/mod.rs` 的事件名字表（T-224 起），并登记在 `CELLS` 上。
const PROTOCOL_FIELDS: &[&str] = &["port", "id", "attempt"];

// `sent` 多两块：**派发时约定的租约**（`lease_ms` / `attempt_cap`）。
//
// ## 为什么租约参数在事实上而不只在 `PortDecl` 里
//
// `stale_sent` 是视图——它的入参只有事实切片与 `now`（ADR-115 决策 1），读不到
// 代码里的 `PortDecl`。没有这两块，视图算不出「这次派发等了多久算卡住」。
//
// ## 为什么在 `sent` 上而不是在 `request` 上
//
// 租约是**派发那一刻**的约定（“这次出去，N 毫秒没应答就算卡住”），写进 `sent`
// 就不必再按 `(port,id,attempt)` 与 `request` 做一次连接。且它同时是回放的
// 必要信息：端口参数未来改了，历史调用的租约仍是当时那个（不是「今天这个」）。
const SENT_FIELDS: &[&str] = &["port", "id", "attempt", "lease_ms", "attempt_cap"];

const BUILTIN_FACTS: &[FactDecl] = &[
    FactDecl {
        name: EVENT_REQUEST,
        port: Some("port"),
        key: &["id", "attempt"],
        expand: &[],
        fields: PROTOCOL_FIELDS,
    },
    FactDecl {
        name: EVENT_SENT,
        port: Some("port"),
        key: &["id", "attempt"],
        expand: &[],
        fields: SENT_FIELDS,
    },
    FactDecl {
        name: EVENT_RESPONSE,
        port: Some("port"),
        key: &["id", "attempt"],
        expand: &[],
        fields: PROTOCOL_FIELDS,
    },
    // `failed` 关的是**逻辑 id**（所有尝试），所以没有 `attempt`——与协议表一致：
    // 失败不是「某一次尝试失败」，是这笔调用不再重试。
    FactDecl {
        name: EVENT_FAILED,
        port: Some("port"),
        key: &["id"],
        expand: &[],
        fields: &["port", "id", "why"],
    },
    // 尝试级否定（T-223）：租约看护者落的「这一次作废、换一次」。
    // 与 `failed` 的边界：`failed` 关**逻辑 id**（不再重试），`retry` 只关一次尝试。
    FactDecl {
        name: EVENT_RETRY,
        port: Some("port"),
        key: &["id", "attempt"],
        expand: &[],
        fields: PROTOCOL_FIELDS,
    },
    // 收束格：字段来自 `turn_runner::produced_payload` + T-220 的 `status` / `reason` / `terminated`。
    FactDecl {
        name: "chat.assistant.final",
        port: None,
        key: &[],
        expand: &[],
        fields: &[
            "text",
            "model",
            "turn_id",
            "text_id",
            "reasoning_id",
            "reasoning",
            "tool_calls",
            "rounds",
            "status",
            "reason",
            "terminated",
        ],
    },
    // 助手文本事实的另一半（`report_due` 的「最近一句」锚点，T-225）：
    // 兜底与收束同为 `Closed`，其余三格落 `turn × asserted / progressed`。
    FactDecl {
        name: "chat.assistant.fallback",
        port: None,
        key: &[],
        expand: &[],
        fields: &["text", "status", "reason", "detail"],
    },
    FactDecl {
        name: "chat.assistant.reported",
        port: None,
        key: &[],
        expand: &[],
        fields: &["text", "tool_rounds", "quiet_ms", "id"],
    },
    FactDecl {
        name: "chat.assistant.opening",
        port: None,
        key: &[],
        expand: &[],
        fields: &["text", "id", "reason", "exclude_from_context"],
    },
    // 进展格载荷与收束格共用产出形状（`produced_payload`）+ `status`，
    // 只声明本视图读得到的字段，不复制整份形状（形状的 owner 是 `facts.rs`）。
    FactDecl {
        name: "chat.assistant.progress",
        port: None,
        key: &[],
        expand: &[],
        fields: &["text", "status"],
    },
    // 阶段事实：`result` 是这一步的结论（召回命中 / 续写种类，T-225）。
    FactDecl {
        name: "turn.stage",
        port: None,
        key: &[],
        expand: &[],
        fields: &["stage", "round", "source", "reason", "result"],
    },
    // 用户格：`meta` 是可选的补充元数据（`prepare.rs` 只在非空时写）。
    //
    // `tier` / `budget_ms` 是**历史形态**：T-225 起用户格先于任何判决落，档位那时
    // 还没定 ⇒ 新写入不再带它们。声明保留，读方（`declared_tier` / `declared_budget_ms`）
    // 因此仍能读旧 WAL；新事实的档位在 `turn.dispatch`。
    FactDecl {
        name: "user.message",
        port: None,
        key: &[],
        expand: &[],
        fields: &["text", "tier", "budget_ms", "id", "meta"],
    },
    // 用户补充格：静默锚点也会退到它（`report_due`），与用户格同族。
    FactDecl {
        name: "turn.supplemented",
        port: None,
        key: &[],
        expand: &[],
        fields: &["text", "count", "id", "ids"],
    },
    // 本轮配置快照（T-225）：决策只读它，不读 `SessionConfig` 活值。
    FactDecl {
        name: "session.config",
        port: None,
        key: &[],
        expand: &[],
        fields: &[
            "classify_enabled",
            "auto_compress",
            "progress_enabled",
            "progress_interval_ms",
            "progress_min_rounds",
            "progress_max_per_turn",
            "is_persistent",
        ],
    },
    // 本轮派发档（T-225）：用户格先落、档位后定 ⇒ 两者分开记（见事件常量文档）。
    FactDecl {
        name: "turn.dispatch",
        port: None,
        key: &[],
        expand: &[],
        fields: &["tier", "budget_ms", "principal"],
    },
    // 拒绝事实（T-219）：`key` 是「主体 × 能力 × 资源」的编码。
    FactDecl {
        name: "authz.denied",
        port: None,
        key: &["key"],
        expand: &[],
        fields: &["key", "port", "id", "tool", "kind", "reason", "detail"],
    },
    // 放行事实（T-226 / PL-016 §4.2）：授权级联的正向一半。
    //
    // 键取 `(port, id)` 而不是 `key`：级联的派发行要与 `sent` / `failed` 关同一
    // 逻辑调用，而协议事实的键是二元组；`key` 只是给人看的同一 id 副本。
    FactDecl {
        name: "authz.granted",
        port: None,
        key: &["port", "id"],
        expand: &[],
        fields: &["key", "port", "id", "tool", "risk", "deferred"],
    },
];

fn in_flight(events: &[Event], _now: i64) -> serde_json::Value {
    serde_json::Value::Array(
        open_calls(events)
            .into_iter()
            .map(|e| {
                serde_json::json!({
                    "id": str_field(e, "id"),
                    "port": str_field(e, "port"),
                    "attempt": u64_field(e, "attempt"),
                })
            })
            .collect(),
    )
}

/// **在途**协议调用的唯一口径：`sent` 中没有被 `response`（同 id + attempt）
/// 或 `failed`（同逻辑 id）或 `retry`（同 id + attempt）关掉的。
///
/// 两个视图（`in_flight` / `stale_sent`）共用它，而不是各写一遍关账规则：
/// 「什么算在途」只能有一处口径，否则两个视图会在某个角落说不同的话。
fn open_calls(events: &[Event]) -> Vec<&Event> {
    let mut open: Vec<&Event> = Vec::new();
    for e in events {
        match e.kind.as_str() {
            EVENT_SENT => open.push(e),
            // 一次尝试被应答 / 被作废：只关那一次。
            EVENT_RESPONSE | EVENT_RETRY => {
                let id = str_field(e, "id");
                let attempt = u64_field(e, "attempt");
                open.retain(|s| !(str_field(s, "id") == id && u64_field(s, "attempt") == attempt));
            }
            // 失败关的是**逻辑 id**（所有尝试）：这笔调用不再重试。
            EVENT_FAILED => {
                let id = str_field(e, "id");
                open.retain(|s| str_field(s, "id") != id);
            }
            _ => {}
        }
    }
    open
}

/// **卡住的在途**（T-223）：`sent` 之后超过 `lease_ms` 仍无应答的那些。
///
/// `now` 是入参而不是时钟（ADR-115 决策 1）——同一份日志给定同一个 `now`，
/// 两次求值必须得到同一结果。
///
/// 字段：`stale` 是租约到期（租约行的开账条件），`exhausted` 是尝试已到顶
/// （判死而不是再重试）。两者分开而不是折成一个布尔：它们回答的是不同问题，
/// 而「到期但还能重试」与「到期且已到顶」是两种不同的收尾。
///
/// `lease_ms == 0`（旧日志 / 未带租约的派发）⇒ 永不过期。这不是默认值而是
/// 语义：没有租约的调用不能被看护者判成卡住。
fn stale_sent(events: &[Event], now: i64) -> serde_json::Value {
    serde_json::Value::Array(
        open_calls(events)
            .into_iter()
            .map(|e| {
                let attempt = u64_field(e, "attempt").unwrap_or(0);
                let lease_ms = u64_field(e, "lease_ms").unwrap_or(0);
                let cap = u64_field(e, "attempt_cap").unwrap_or(0);
                let age_ms = (now - e.ts).max(0) as u64;
                serde_json::json!({
                    "port": str_field(e, "port"),
                    "id": str_field(e, "id"),
                    "attempt": attempt,
                    "age_ms": age_ms,
                    "stale": lease_ms > 0 && age_ms >= lease_ms,
                    "exhausted": cap > 0 && attempt >= cap,
                })
            })
            .collect(),
    )
}

/// 这一轮**最新一份配置**（按事件序取最后一条；`None` = 这一轮没落配置事实）。
fn config_of(events: &[Event], turn: u64) -> Option<&Event> {
    events
        .iter()
        .rfind(|e| e.turn == turn && e.kind == crate::symbio_core::EVENT_SESSION_CONFIG)
}

/// 当前这一幕的身份：轮号 + 幕锚（最新一条用户格 / 恢复格）。
///
/// 幕锚是「本次执行从哪开始」的唯一定义：同一轮号会被续写幕复用，按轮扫全史
/// 会把首幕的阶段误判成「已做」。
fn drive_of(events: &[Event]) -> Option<(u64, Option<&Event>)> {
    const HEAD_KINDS: &[&str] = &[
        "user.message",
        "turn.stage",
        "chat.assistant.final",
        "chat.assistant.fallback",
        "session.config",
    ];
    let turn = events
        .iter()
        .filter(|e| HEAD_KINDS.contains(&e.kind.as_str()))
        .map(|e| e.turn)
        .max()?;
    let anchor = events
        .iter()
        .filter(|e| e.turn == turn && is_drive_anchor(e))
        .max_by_key(|e| (e.ts, seq_of(e)));
    Some((turn, anchor))
}

fn is_drive_anchor(e: &Event) -> bool {
    e.kind == "user.message" || (e.kind == "turn.stage" && str_field(e, "stage") == Some("resume"))
}

fn seq_of(e: &Event) -> u64 {
    e.seq.map(|s| s.value()).unwrap_or(0)
}

/// 阶段「已做」的锚后判据：`seq` 必须存在且不早于幕锚。
///
/// `None` 的 `seq` 一律不算——未入库的事件不是事实（与 `stage_events` 的落格同判）。
fn after_anchor(e: &Event, anchor_seq: u64) -> bool {
    e.seq.is_some_and(|s| s.value() >= anchor_seq)
}

/// 轮头（T-222a 升到 v3）：五个策略行的**开账条件**全在这一格里。
///
/// `done` 三格按幕锚折叠（续写幕复用轮号，不能按轮扫全史）；`skill_hit` 取本幕
/// 召回阶段的结论，`resumed` 区分续写幕与用户新发言。
fn turn_head(events: &[Event], _now: i64) -> serde_json::Value {
    let Some((turn, anchor)) = drive_of(events) else {
        return serde_json::Value::Null;
    };
    let anchor_seq = anchor.map(seq_of).unwrap_or(0);
    // 心跳等无用户节点的轮次也落用户格（`id: null`）——那不算「这一轮有发言」
    // （与 `first_utterance.is_some()` 同判）。
    let has_utterance = anchor.is_some_and(|e| {
        e.kind == "user.message" && e.payload.get("id").is_none_or(|v| !v.is_null())
    });
    let resumed =
        anchor.is_some_and(|e| e.kind == "turn.stage" && str_field(e, "stage") == Some("resume"));
    // 请求轮次以**本幕**落下的 `round` 格为准（不是全轮最大值：续写幕会复用轮号）。
    let round = events
        .iter()
        .filter(|e| {
            e.turn == turn
                && e.kind == "turn.stage"
                && str_field(e, "stage") == Some("round")
                && after_anchor(e, anchor_seq)
        })
        .max_by_key(|e| seq_of(e))
        .and_then(|e| u64_field(e, "round"))
        .unwrap_or(0);
    let config = config_of(events, turn);
    let flag = |key: &str| {
        config
            .and_then(|e| e.payload.get(key))
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    };
    let stage_since = |stage: &str| {
        events.iter().any(|e| {
            e.turn == turn
                && e.kind == "turn.stage"
                && str_field(e, "stage") == Some(stage)
                && u64_field(e, "round") == Some(round)
                && after_anchor(e, anchor_seq)
        })
    };
    let skill_hit = events.iter().any(|e| {
        e.turn == turn
            && e.kind == "turn.stage"
            && str_field(e, "stage") == Some("recall")
            && u64_field(e, "round") == Some(round)
            && after_anchor(e, anchor_seq)
            && str_field(e, "result") == Some("hit")
    });
    serde_json::json!({
        "turn": turn,
        "round": round,
        "anchor": anchor_seq,
        "has_utterance": has_utterance,
        "resumed": resumed,
        "classify_enabled": flag("classify_enabled"),
        "classify_done": stage_since("decide"),
        "auto_compress": flag("auto_compress"),
        "compress_done": stage_since("compress"),
        "is_persistent": flag("is_persistent"),
        "recall_done": stage_since("recall"),
        "skill_hit": skill_hit,
    })
}

/// 本幕落下的阶段事实（`turn.stage`）：一次性阶段的**出口**。
///
/// 按幕锚收窄，续写幕因此不会被首幕落在同一轮号上的 `recall` / `compress` 关账。
/// 数组的每个元素是一条阶段，键绑定读 `turn` 与 `round`。
fn turn_stages(events: &[Event], _now: i64) -> serde_json::Value {
    let Some((turn, anchor)) = drive_of(events) else {
        return serde_json::Value::Array(Vec::new());
    };
    let anchor_seq = anchor.map(seq_of).unwrap_or(0);
    serde_json::Value::Array(
        events
            .iter()
            .filter(|e| e.turn == turn && e.kind == "turn.stage" && after_anchor(e, anchor_seq))
            .map(|e| {
                serde_json::json!({
                    "turn": e.turn,
                    "stage": str_field(e, "stage"),
                    "round": u64_field(e, "round").unwrap_or(0),
                })
            })
            .collect(),
    )
}

/// 本幕是否已经收束（收束 / 兜底格出现在幕锚之后）。
///
/// `[]` = 还开着；一条 `{turn}` = 已关。执行行与反射行都拿它作出口。
fn turn_closed(events: &[Event], _now: i64) -> serde_json::Value {
    let Some((turn, anchor)) = drive_of(events) else {
        return serde_json::Value::Array(Vec::new());
    };
    let anchor_seq = anchor.map(seq_of).unwrap_or(0);
    let closed = events.iter().any(|e| {
        e.turn == turn
            && matches!(
                e.kind.as_str(),
                "chat.assistant.final" | "chat.assistant.fallback"
            )
            && after_anchor(e, anchor_seq)
    });
    if closed {
        serde_json::json!([{ "turn": turn }])
    } else {
        serde_json::Value::Array(Vec::new())
    }
}

/// 中途汇报该不该说（T-225）：只吃日志与 `now`，不碰内存计数 / 墙钟。
///
/// `quiet_ms` 的起点是本轮**最近一次用户看得见的动静**：助手说过的任何一句
/// （汇报 / 开场白 / 收束 / 等待）优先；一句都没说过时退到用户格 / 补充格。
/// `min_rounds` 与 `quiet_ms` 一起交给触发点：工具轮数不在日志里（当前轮次只在
/// 收束后才计数），所以「够不够轮」由持有轮次计数的一方判；表里这一行判的是
/// 「静默够久 ∧ 配额未满」。
fn report_due(events: &[Event], now: i64) -> serde_json::Value {
    let Some((turn, drive_anchor)) = drive_of(events) else {
        return serde_json::Value::Null;
    };
    let drive_seq = drive_anchor.map(seq_of).unwrap_or(0);
    let config = config_of(events, turn);
    let enabled = config
        .and_then(|e| e.payload.get("progress_enabled"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let interval_ms = config
        .and_then(|e| e.payload.get("progress_interval_ms"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let min_rounds = config
        .and_then(|e| e.payload.get("progress_min_rounds"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let quota = config
        .and_then(|e| e.payload.get("progress_max_per_turn"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    // 配额是**请求作用域**的量（旧 `RoundProgress` 随请求新建）：续写幕重新开账，
    // 故只数本幕锚之后的汇报。
    let reports = events
        .iter()
        .filter(|e| {
            e.turn == turn && e.kind == "chat.assistant.reported" && after_anchor(e, drive_seq)
        })
        .count() as u64;
    let anchor = events
        .iter()
        .filter(|e| e.turn == turn && is_report_anchor(e))
        .max_by_key(|e| (e.ts, seq_of(e)));
    let quiet_ms = anchor.map(|e| (now - e.ts).max(0)).unwrap_or(0);
    let due = enabled && quiet_ms >= interval_ms && reports < quota;
    serde_json::json!({
        "turn": turn,
        // 静默区间的身份 = 区间起点那一格的 seq。汇报事实落格后，本视图的锚
        // 前移，下一段静默是新的一笔账（配额之外还要能逐段关账）。
        "anchor": anchor.map(seq_of).unwrap_or(0),
        "due": due,
        "quiet_ms": quiet_ms,
        "min_rounds": min_rounds,
        "reports": reports,
        "quota": quota,
    })
}

/// 中途汇报的**出口**：每一条汇报事实关掉它前一段静默区间的账。
///
/// 汇报自己会说一句话（成为新的静默起点），所以一条汇报只能关「它之前」那段：
/// 锚取**严格早于汇报 seq** 的最近一条动静。这样 `report_due.anchor` 与
/// 本视图的 `anchor` 在同一段区间上逐字相等，`(turn, anchor)` 就是区间的身份。
fn reported_anchors(events: &[Event], _now: i64) -> serde_json::Value {
    serde_json::Value::Array(
        events
            .iter()
            .filter(|e| e.kind == "chat.assistant.reported")
            .filter_map(|report| {
                let report_seq = report.seq?.value();
                let anchor = events
                    .iter()
                    .filter(|e| {
                        e.turn == report.turn
                            && e.seq.is_some_and(|s| s.value() < report_seq)
                            && is_report_anchor(e)
                    })
                    .max_by_key(|e| (e.ts, seq_of(e)));
                Some(serde_json::json!({
                    "turn": report.turn,
                    "anchor": anchor.map(seq_of).unwrap_or(0),
                }))
            })
            .collect(),
    )
}

/// 助手说过的每一句（汇报 / 开场白 / 收束 / 兜底 / 进展）。
const SPOKE_KINDS: &[&str] = &[
    "chat.assistant.reported",
    "chat.assistant.opening",
    "chat.assistant.final",
    "chat.assistant.fallback",
    "chat.assistant.progress",
];

/// 静默时钟的起点候选：用户看得见的任何一次动静。
///
/// 用户格 / 补充格 / 恢复幕都是「用户动了一下」，助手说过的每一句同理。恢复幕
/// 也算——审批 / 回填发生在请求开始时，那一幕的静默应当从此刻起算，而不是从
/// 几天前的那句用户话起算。
fn is_report_anchor(e: &Event) -> bool {
    is_drive_anchor(e) || e.kind == "turn.supplemented" || SPOKE_KINDS.contains(&e.kind.as_str())
}

const BUILTIN_VIEWS: &[ViewDecl] = &[
    // 在途 = `sent` 中没有对应 `response`（同 id + attempt）、`failed`（同 id）
    // 或 `retry`（同 id + attempt）的那些。
    //
    // v2（T-223）：加入 `retry` 一侧。没有它，被作废的那次尝试会永远留在「在途」里
    // ——而它已经不会再回来了。
    ViewDecl {
        name: "in_flight",
        ver: 2,
        reads: &[
            (EVENT_SENT, "id"),
            (EVENT_SENT, "port"),
            (EVENT_SENT, "attempt"),
            (EVENT_RESPONSE, "id"),
            (EVENT_RESPONSE, "attempt"),
            (EVENT_FAILED, "id"),
            (EVENT_RETRY, "id"),
            (EVENT_RETRY, "attempt"),
        ],
        cost: 4096,
        eval: in_flight,
    },
    // 卡住（T-223）：与 `in_flight` 同一口径的在途集合，加上租约与尝试上限两个判断。
    //
    // 时间不进欠账公式：`stale` 由本视图把 `now` 折成布尔，租约行只对 `true` 开账
    // （与 `report_due.due` 同一条纪律）。
    ViewDecl {
        name: "stale_sent",
        ver: 1,
        reads: &[
            (EVENT_SENT, "id"),
            (EVENT_SENT, "port"),
            (EVENT_SENT, "attempt"),
            (EVENT_SENT, "lease_ms"),
            (EVENT_SENT, "attempt_cap"),
            (EVENT_RESPONSE, "id"),
            (EVENT_RESPONSE, "attempt"),
            (EVENT_FAILED, "id"),
            (EVENT_RETRY, "id"),
            (EVENT_RETRY, "attempt"),
        ],
        cost: 4096,
        eval: stale_sent,
    },
    // 轮头（T-222a 升到 v3）：五个策略行的开账条件全在这一格里。
    //
    // 与 v2 的两处差别：`done` 三格按**幕锚**折叠（续写幕复用轮号），并加入
    // `resumed` / `skill_hit`——反射行从此也只读日志，不再拿执行侧的内存布尔。
    ViewDecl {
        name: "turn_head",
        ver: 3,
        reads: &[
            ("user.message", "id"),
            ("turn.stage", "stage"),
            ("turn.stage", "round"),
            ("turn.stage", "result"),
            ("session.config", "classify_enabled"),
            ("session.config", "auto_compress"),
            ("session.config", "is_persistent"),
        ],
        cost: 4096,
        eval: turn_head,
    },
    // 本幕落下的阶段事实：`decide` / `recall` / `compress` 三行的出口。
    ViewDecl {
        name: "turn_stages",
        ver: 1,
        reads: &[
            ("user.message", "id"),
            ("turn.stage", "stage"),
            ("turn.stage", "round"),
        ],
        cost: 4096,
        eval: turn_stages,
    },
    // 本幕是否收束：执行行与反射行的出口。
    ViewDecl {
        name: "turn_closed",
        ver: 1,
        reads: &[
            ("user.message", "id"),
            ("turn.stage", "stage"),
            ("chat.assistant.final", "text"),
            ("chat.assistant.fallback", "text"),
        ],
        cost: 4096,
        eval: turn_closed,
    },
    // 汇报到期：静默时长 = `now` − 最近一次用户看得见的动静的 `ts`；
    // 配额 = 本轮汇报数 vs `session.config.progress_max_per_turn`。
    //
    // v2 起输出 `anchor`（静默起点的 seq）：汇报行以 `(turn, anchor)` 标识一段
    // 静默区间，`reported_anchors` 逐条关账。v3 把 `quiet_ms` / `min_rounds`
    // 一并交给触发点——它们不再从进程配置里读，判定输入只剩事实与 `now`。
    ViewDecl {
        name: "report_due",
        ver: 3,
        reads: &[
            ("session.config", "progress_enabled"),
            ("session.config", "progress_interval_ms"),
            ("session.config", "progress_min_rounds"),
            ("session.config", "progress_max_per_turn"),
            ("chat.assistant.reported", "tool_rounds"),
            ("chat.assistant.opening", "text"),
            ("chat.assistant.final", "text"),
            ("chat.assistant.fallback", "text"),
            ("chat.assistant.progress", "status"),
            ("user.message", "text"),
            ("turn.supplemented", "text"),
            ("turn.stage", "stage"),
        ],
        cost: 4096,
        eval: report_due,
    },
    // 汇报的出口：每条汇报事实关掉它之前那段静默区间。
    ViewDecl {
        name: "reported_anchors",
        ver: 1,
        reads: &[
            ("user.message", "text"),
            ("turn.supplemented", "text"),
            ("turn.stage", "stage"),
            ("chat.assistant.reported", "text"),
            ("chat.assistant.opening", "text"),
            ("chat.assistant.final", "text"),
            ("chat.assistant.fallback", "text"),
            ("chat.assistant.progress", "status"),
        ],
        cost: 4096,
        eval: reported_anchors,
    },
];

fn str_field<'a>(e: &'a Event, key: &str) -> Option<&'a str> {
    e.payload.get(key).and_then(|v| v.as_str())
}

fn u64_field(e: &Event, key: &str) -> Option<u64> {
    e.payload.get(key).and_then(|v| v.as_u64())
}

#[cfg(test)]
#[path = "mod.test.rs"]
mod tests;
