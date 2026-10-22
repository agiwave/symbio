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
//! 15 个事实：协议四名（`request` / `sent` / `response` / `failed`）、助手文本六格
//! （`chat.assistant.final` / `fallback` / `reported` / `opening` / `progress`）、
//! `turn.stage`、`user.message`、`turn.supplemented`、`session.config`、
//! `turn.dispatch`、`authz.denied`；3 个视图：`in_flight` / `turn_head` / `report_due`。
//!
//! 视图点名决定了事实的**下限**：`turn_head` 读 `user.message` / `turn.stage` /
//! `session.config`，`report_due` 读助手文本六格 / `turn.supplemented` /
//! `session.config` + `now`。`retry` 由 T-223 补声明。
//!
//! ## 为什么整模块暂时 `cfg(test)`
//!
//! 本步的范围钉死为「只做声明层，不接控制面」（`Registry` / `LoopState` 一个字不动）。
//! 没有生产消费方时，若把它编进非测试构建，`dead_code` 会逐个点名；而
//! `#[allow(dead_code)]` 是只许降的棘轮。故与 `stage_events::done` 同一形态：
//! **如实标 `cfg(test)`，不给死代码闸门开豁免**。T-222 把行表接上时删掉
//! `symbio_core/mod.rs` 里那一行 `#[cfg(test)]` 即可。

use std::collections::HashSet;
use std::sync::Arc;

use crate::symbio_core::Event;

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
// 暂时住本模块：T-224 把它们升进 `event/mod.rs` 的名字表并登记 `CELLS`。
// 现在升上去会让四个常量在非测试构建里没有消费者（死代码），而它们还不在网格上。
const REQUEST: &str = "request";
const SENT: &str = "sent";
const RESPONSE: &str = "response";
const FAILED: &str = "failed";

const PROTOCOL_FIELDS: &[&str] = &["port", "id", "attempt"];

const BUILTIN_FACTS: &[FactDecl] = &[
    FactDecl {
        name: REQUEST,
        port: Some("port"),
        key: &["id", "attempt"],
        expand: &[],
        fields: PROTOCOL_FIELDS,
    },
    FactDecl {
        name: SENT,
        port: Some("port"),
        key: &["id", "attempt"],
        expand: &[],
        fields: PROTOCOL_FIELDS,
    },
    FactDecl {
        name: RESPONSE,
        port: Some("port"),
        key: &["id", "attempt"],
        expand: &[],
        fields: PROTOCOL_FIELDS,
    },
    // `failed` 关的是**逻辑 id**（所有尝试），所以没有 `attempt`——与协议表一致：
    // 失败不是「某一次尝试失败」，是这笔调用不再重试。
    FactDecl {
        name: FAILED,
        port: Some("port"),
        key: &["id"],
        expand: &[],
        fields: &["port", "id", "why"],
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
        fields: &["key", "kind", "reason", "detail"],
    },
];

fn in_flight(events: &[Event], _now: i64) -> serde_json::Value {
    let mut open: Vec<&Event> = Vec::new();
    for e in events {
        match e.kind.as_str() {
            SENT => open.push(e),
            RESPONSE => {
                let id = str_field(e, "id");
                let attempt = u64_field(e, "attempt");
                open.retain(|s| !(str_field(s, "id") == id && u64_field(s, "attempt") == attempt));
            }
            FAILED => {
                let id = str_field(e, "id");
                open.retain(|s| str_field(s, "id") != id);
            }
            _ => {}
        }
    }
    serde_json::Value::Array(
        open.into_iter()
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

/// 这一轮**最新一份配置**（按事件序取最后一条；`None` = 这一轮没落配置事实）。
fn config_of(events: &[Event], turn: u64) -> Option<&Event> {
    events
        .iter()
        .rfind(|e| e.turn == turn && e.kind == crate::symbio_core::EVENT_SESSION_CONFIG)
}

fn turn_head(events: &[Event], _now: i64) -> serde_json::Value {
    let Some(turn) = events
        .iter()
        .filter(|e| {
            matches!(
                e.kind.as_str(),
                "user.message" | "turn.stage" | "chat.assistant.final" | "session.config"
            )
        })
        .map(|e| e.turn)
        .max()
    else {
        return serde_json::Value::Null;
    };
    let has_utterance = events
        .iter()
        .any(|e| e.turn == turn && e.kind == "user.message");
    let round = events
        .iter()
        .filter(|e| e.turn == turn && e.kind == "turn.stage")
        .filter_map(|e| u64_field(e, "round"))
        .max()
        .unwrap_or(0);
    // `classify_enabled` 只从事实读：配置在轮首落成 `session.config`，此后判决
    // 看到的就是"那一轮用的配置"，不是进程里此刻的活值（T-225）。
    let classify_enabled = config_of(events, turn)
        .and_then(|e| e.payload.get("classify_enabled"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let done = events.iter().any(|e| {
        e.turn == turn
            && e.kind == "turn.stage"
            && str_field(e, "stage") == Some("decide")
            && u64_field(e, "round") == Some(round)
    });
    serde_json::json!({
        "turn": turn,
        "round": round,
        "has_utterance": has_utterance,
        "classify_enabled": classify_enabled,
        "done": done,
    })
}

/// 中途汇报该不该说（T-225）：只吃日志与 `now`，不碰 `RoundProgress` / 墙钟。
///
/// `quiet_ms` 的起点是本轮**最近一次用户看得见的动静**：助手说过的任何一句
/// （汇报 / 开场白 / 收束 / 等待）优先；一句都没说过时退到用户格 / 补充格。
/// `min_rounds` 不在这里判——它是触发点的前提（回调只在工具批次边界上被问），
/// 而不是日志能回答的量。
fn report_due(events: &[Event], now: i64) -> serde_json::Value {
    const SPOKE: &[&str] = &[
        "chat.assistant.reported",
        "chat.assistant.opening",
        "chat.assistant.final",
        "chat.assistant.fallback",
        "chat.assistant.progress",
    ];
    let Some(turn) = events
        .iter()
        .filter(|e| {
            matches!(
                e.kind.as_str(),
                "user.message"
                    | "turn.supplemented"
                    | "chat.assistant.reported"
                    | "chat.assistant.opening"
                    | "chat.assistant.final"
                    | "chat.assistant.fallback"
                    | "chat.assistant.progress"
            )
        })
        .map(|e| e.turn)
        .max()
    else {
        return serde_json::Value::Null;
    };
    let config = config_of(events, turn);
    let enabled = config
        .and_then(|e| e.payload.get("progress_enabled"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let interval_ms = config
        .and_then(|e| e.payload.get("progress_interval_ms"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let quota = config
        .and_then(|e| e.payload.get("progress_max_per_turn"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let reports = events
        .iter()
        .filter(|e| e.turn == turn && e.kind == "chat.assistant.reported")
        .count() as u64;
    let spoke_at = events
        .iter()
        .filter(|e| e.turn == turn && SPOKE.contains(&e.kind.as_str()))
        .map(|e| e.ts)
        .max();
    let anchor = spoke_at.or_else(|| {
        events
            .iter()
            .filter(|e| {
                e.turn == turn && matches!(e.kind.as_str(), "user.message" | "turn.supplemented")
            })
            .map(|e| e.ts)
            .max()
    });
    let quiet_ms = anchor.map(|at| (now - at).max(0)).unwrap_or(0);
    let due = enabled && quiet_ms >= interval_ms && reports < quota;
    serde_json::json!({
        "turn": turn,
        "due": due,
        "reports": reports,
        "quota": quota,
    })
}

const BUILTIN_VIEWS: &[ViewDecl] = &[
    // 在途 = `sent` 中没有对应 `response`（同 id + attempt）或 `failed`（同 id）的那些。
    // `retry` 由 T-223 加进 `until` 一侧（本视图那时补读 `retry.id`）。
    ViewDecl {
        name: "in_flight",
        ver: 1,
        reads: &[
            (SENT, "id"),
            (SENT, "port"),
            (SENT, "attempt"),
            (RESPONSE, "id"),
            (RESPONSE, "attempt"),
            (FAILED, "id"),
        ],
        cost: 4096,
        eval: in_flight,
    },
    // 轮头（T-225 升到 v2）：`classify_enabled` 从 `session.config` 读，`done` 从
    // `turn.stage` 的 `decide` 格折叠——五个策略输入里这一格能四样全凭日志算出。
    ViewDecl {
        name: "turn_head",
        ver: 2,
        reads: &[
            ("user.message", "text"),
            ("turn.stage", "stage"),
            ("turn.stage", "round"),
            ("session.config", "classify_enabled"),
            ("chat.assistant.final", "text"),
        ],
        cost: 4096,
        eval: turn_head,
    },
    // 汇报到期：静默时长 = `now` − 最近一句助手文本（或用户格）的 `ts`；
    // 配额 = 本轮汇报数 vs `session.config.progress_max_per_turn`。
    ViewDecl {
        name: "report_due",
        ver: 1,
        reads: &[
            ("session.config", "progress_enabled"),
            ("session.config", "progress_interval_ms"),
            ("session.config", "progress_max_per_turn"),
            ("chat.assistant.reported", "tool_rounds"),
            ("chat.assistant.opening", "text"),
            ("chat.assistant.final", "text"),
            ("chat.assistant.fallback", "text"),
            ("chat.assistant.progress", "status"),
            ("user.message", "text"),
            ("turn.supplemented", "text"),
        ],
        cost: 4096,
        eval: report_due,
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
