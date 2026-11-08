//! 本插件拥有的事实名、格位、归宿行与对外可读的视图（DE-024 D-197 / DE-025 D-209 / D-221 / D-281）。
//!
//! 一条事实 `conation.expressed`：**「我想要 X」这件事被表达过**。名字、格位、载荷形状与
//! 归宿行同在这里；写方（会话的自主触发）从清单生成物取名字，读方（[`CONATION_INTENTS`]
//! 视图）从本模块取算法——不存在第二份拼写。
//!
//! ## 格位为什么是 `Conation × Opened`
//!
//! `Entity::Conation` 是内核**认识但不解释**的一格：内核不为它保留任何名字常量，写闸只按
//! 本声明校验名字与格位。用 `Opened` 而不是 `Asserted`，因为表达一条欲**开的是**一条待定
//! 的意图（它可能升格成任务，也可能一直停在「想要」），不是对世界下断言。
//!
//! ## 视图为什么也在这里
//!
//! [`CONATION_INTENTS`] 是纯折叠：从 `conation.expressed` 折出「表达过哪些欲、各自升没升格」。
//! 它在 `PLUGIN.yml` 的 `shared_views:` 登记为**对外可读**，读方经
//! `symbio_plugin_api::views` 按名字读——不依赖本 crate，也不在任何控制面里再写一份（D-221）。
//!
//! ## 为什么折叠要交两半（D-281）
//!
//! 读方要的是两个基数上的同一个口径：**这一片**的采纳率（单会话列）与 **N 片**的采纳率
//! （实例级聚合）。`eval` 交前者，`agg` 交后者；两半都由本插件交、同经冻结声明表被调用，
//! 所以「率 = 已升格 / 已表达」这条算式只有一个 owner。少了 `agg`，跨会话合并就只能在读方
//! 再折一遍——那是同一件事的第二份口径。
//!
//! ## 第三条视图为什么是「判决」（`D-384`）
//!
//! `conation.policy` 回答的不是「发生过什么」，是「**这一条欲该不该升格**」：开关
//! （`session.config` 的 `conation_enabled`）与目标宽度（[`MAX_GOAL_LEN`]）都是内在动机
//! 这条能力自己的策略。会话只保留强制点（授权 + 不可伪造的批准类型），策略经名字读——
//! 与 `conation.intents` 同一通道，不新增端口、不改机制面。
//!
//! 它**不交** `agg`：判决是逐会话的一次评估，没有「N 片合起来该不该升格」这种东西。
//! `NotAggregable` 是可辨的缺席（`D-281`），不是「汇总下来为零」。
//!
//! ## 三条随包事实为什么只交声明（`#36` / `#50` / `#51`）
//!
//! 主动求知、好奇与探索、回避倾向在名册里是「拆（未建）」：它们没有控制面实现可搬，
//! 而是**随本包一起建出来**。最小能力面取「事实」这一格——名字、格位、载荷字段、归宿行
//! 都先立住，`DE-026` 的逐行处置就成立了；**写方（谁在什么时候表达一条求知 / 好奇 / 回避）
//! 不在这一批**，所以三条行都是 `by = None`，也不进 `shared_events`（没有第二个消费方）。
//!
//! 为什么不顺带补一个视图：视图至少要有一条能读的事实，而这三条事实今天没有写方，
//! 折出来的只会是「有据的零」——那会把「能力还没接」伪装成「读数是零」。声明面是
//! 诚实的下限：它说得出「这个能力叫什么、落在哪一格」，说不出的时候不编读数。

use serde_json::{json, Value};

use symbio_plugin_api::ring::{Decl, Pattern};
use symbio_plugin_api::{
    Entity, Event, EventDecl, FactDecl, LatencyTier, PluginDeclarations, Verb, ViewDecl,
};

/// 「欲」的表达。跨插件共享：**写方是会话**（自主触发的落格是会话存储所有者的写窗口），
/// 所以名字经 `PLUGIN.yml` 的 `shared_events:` 投影进 `symbio_plugin_api::events`，
/// 写方按生成物命名，不引本 crate（E-009）。
pub(crate) use symbio_plugin_api::events::EVENT_CONATION_EXPRESSED;

/// 升格溯源的另一半：`task.opened` 的**信封**字段 `produced_by` 指回那条欲事件。
///
/// 它只被当作**溯源边**读，不作为领域事实读——所以它不进 `VIEWS` 的 `reads`：`reads` 列的是
/// 「读了哪个事实的哪个载荷字段」，而 `produced_by` 是信封字段，没有对应的 `FactDecl`
/// （`task.opened` 的 owner 是任务插件，它没声明这条事实的形状）。把一条没声明的名字写进
/// `reads` 会被 `seal` 判 `UnknownFact`——那才是真正的撒谎。
pub(crate) use symbio_plugin_api::events::EVENT_TASK_OPENED;

/// 自主程度折叠读的两格（任务族，`D-282` 第二种读法）：挂起与闭环。
pub(crate) use symbio_plugin_api::events::{EVENT_TASK_ASSERTED, EVENT_TASK_HELD};

use symbio_plugin_api::events::EVENT_SESSION_CONFIG;

/// 对外可读的视图：表达过的欲及其升格状态。
pub(crate) const CONATION_INTENTS: &str = "conation.intents";

/// 对外可读的第二条视图：无人干预完成率（DE-008 §3.1 自主程度）。
pub(crate) const CONATION_AUTONOMY: &str = "conation.autonomy";

/// 对外可读的第三条视图：这一条「欲」该不该升格（E3 治理策略，`D-384`）。
pub(crate) const CONATION_POLICY: &str = "conation.policy";

/// 目标长度上界（字节）：过宽的目标 = 不可评估的意图。
///
/// 它是**策略参数**，住 owner（`D-384`）：迁移前它以 `ConationPolicy::default()` 的
/// 字面量躺在会话里，读方替能力包记着一条它自己的规则。
pub(crate) const MAX_GOAL_LEN: usize = 200;

/// 开关的出厂值：快照里没有这一格 / 不是布尔 ⇒ 关（`D-252` / `D-379`）。
pub(crate) const DEFAULT_ENABLED: bool = false;

/// 名册 #36「主动求知」的最小能力面：**一次求知被发起**。
///
/// 它与 `conation.expressed` 同格（`Conation × Opened`），但不是同一条事实：「我想知道 X」
/// 与「我想要 X」在治理上是两件事——前者升格成一次检索 / 提问，后者升格成任务
/// （`conation.policy` 的判决只对后者生效）。
///
/// 名字**只在本包内使用**：没有写方之前不进 `shared_events`（CS-007：单消费方的名字随
/// owner 走，进了共享表就是第二张没人用的内核表）。
pub(crate) const EVENT_CONATION_INQUIRY: &str = "conation.inquiry";

/// 名册 #50「好奇与探索」的最小能力面：**对某个方向的好奇被表达**。
///
/// 与求知的分界：求知带着一个可被回答的问题，好奇只给一个值得探索的方向——两者都不
/// 承诺会有产物，所以是 `Opened` 而不是 `Asserted`。
pub(crate) const EVENT_CONATION_CURIOSITY: &str = "conation.curiosity";

/// 名册 #51「回避倾向」的最小能力面：**某类目标被自述为倾向回避**。
///
/// 格位是 `Conation × Asserted`：回避是对**自己**的一条断言，不打开任何待定意图；
/// 它归自我模型（名册的轴是「自我模型精度」），不是新一轮的欲。
pub(crate) const EVENT_CONATION_AVOIDANCE: &str = "conation.avoidance";

/// owner —— 与目录名同形（`crates/symbio-plugin-conation`），装配表按它排序与报缺口。
pub(crate) const OWNER: &str = "crates/symbio-plugin-conation";

/// 扫描成本上界（事件条数）。它是环内窗口求值的预算；跨插件读口不查它——读侧按
/// 调用方给的切片全量折（`D-284`）。
///
/// 声明的上界必须与全仓**同一个值**（`seal` 的 `ViewCostMismatch`：切点只有一个），
/// 所以这里不按「欲通常很少」调小——调小会改整张表的切点。
const COST: u32 = 4096;

pub(crate) const EVENTS: &[EventDecl] = &[
    EventDecl::new(EVENT_CONATION_EXPRESSED, (Entity::Conation, Verb::Opened)),
    // 名册 #36 / #50 / #51 的随包能力面：名字与格位先立住，写方落地时只接生产侧。
    EventDecl::new(EVENT_CONATION_INQUIRY, (Entity::Conation, Verb::Opened)),
    EventDecl::new(EVENT_CONATION_CURIOSITY, (Entity::Conation, Verb::Opened)),
    EventDecl::new(EVENT_CONATION_AVOIDANCE, (Entity::Conation, Verb::Asserted)),
];

pub(crate) const FACTS: &[FactDecl] = &[
    FactDecl {
        name: EVENT_CONATION_EXPRESSED,
        port: None,
        key: &[],
        expand: &[],
        // 只声明视图真正读的那一格。`ts` / `seq` / `produced_by` 是信封字段，不是载荷。
        fields: &["goal"],
    },
    // 三条随包事实各只交一格：`topic` 是「关于什么」的稳定主语（一个问题 / 一个方向 /
    // 一类被回避的目标）。判据（该不该追问、该不该探索）还没有 owner，所以不放进来。
    FactDecl {
        name: EVENT_CONATION_INQUIRY,
        port: None,
        key: &[],
        expand: &[],
        fields: &["topic"],
    },
    FactDecl {
        name: EVENT_CONATION_CURIOSITY,
        port: None,
        key: &[],
        expand: &[],
        fields: &["topic"],
    },
    FactDecl {
        name: EVENT_CONATION_AVOIDANCE,
        port: None,
        key: &[],
        expand: &[],
        fields: &["topic"],
    },
];

/// 本插件事实的**归宿行**：`owes` 与 `until` 同源——有表达即闭合，没有则不欠。
///
/// 行表是义务，不是触发器；这条自读行只回答「这条事实归谁看护」。没有它，`seal` 会把
/// `conation.expressed` 判成 `OrphanFact`（每条事实都要有归宿行）。
///
/// 三条随包事实各有一条同形行（名册 #36 / #50 / #51）：`by = None` 是「这笔账不由引擎
/// 执行」的声明位——写方还没接，行不假装有人跑它。接上写方时只改 `by` 与写侧，不动名字。
pub(crate) const ROWS: &[Decl] = &[
    Decl {
        name: EVENT_CONATION_EXPRESSED,
        by: None,
        tier: LatencyTier::Reflex,
        expand: None,
        owes: Pattern {
            source: EVENT_CONATION_EXPRESSED,
            key: &[],
            eq: &[],
        },
        until: &[Pattern {
            source: EVENT_CONATION_EXPRESSED,
            key: &[],
            eq: &[],
        }],
        unless: &[],
    },
    Decl {
        name: EVENT_CONATION_INQUIRY,
        by: None,
        tier: LatencyTier::Reflex,
        expand: None,
        owes: Pattern {
            source: EVENT_CONATION_INQUIRY,
            key: &[],
            eq: &[],
        },
        until: &[Pattern {
            source: EVENT_CONATION_INQUIRY,
            key: &[],
            eq: &[],
        }],
        unless: &[],
    },
    Decl {
        name: EVENT_CONATION_CURIOSITY,
        by: None,
        tier: LatencyTier::Reflex,
        expand: None,
        owes: Pattern {
            source: EVENT_CONATION_CURIOSITY,
            key: &[],
            eq: &[],
        },
        until: &[Pattern {
            source: EVENT_CONATION_CURIOSITY,
            key: &[],
            eq: &[],
        }],
        unless: &[],
    },
    Decl {
        name: EVENT_CONATION_AVOIDANCE,
        by: None,
        tier: LatencyTier::Reflex,
        expand: None,
        owes: Pattern {
            source: EVENT_CONATION_AVOIDANCE,
            key: &[],
            eq: &[],
        },
        until: &[Pattern {
            source: EVENT_CONATION_AVOIDANCE,
            key: &[],
            eq: &[],
        }],
        unless: &[],
    },
];

pub(crate) const VIEWS: &[ViewDecl] = &[
    ViewDecl {
        name: CONATION_INTENTS,
        ver: 1,
        // 只声明载荷字段。升格那一半读的是 `task.opened` 的**信封**（`produced_by`），
        // 见 [`EVENT_TASK_OPENED`] 的注释：信封不是事实面，`reads` 表达不了也不该表达。
        reads: &[(EVENT_CONATION_EXPRESSED, "goal")],
        // 读出口径不是收束判据：本视图不替任何事实关门。
        evidence: &[],
        cost: COST,
        eval: intents,
        agg: Some(merge),
    },
    ViewDecl {
        name: CONATION_AUTONOMY,
        ver: 1,
        // 任务族只有 `EventDecl`、没有 `FactDecl`：这两对按 `D-282` 的第二种读法登记
        // （名字由 `EventDecl` 保证拼对，字段无从校验），与 `task.readyset` 同形。
        reads: &[
            (EVENT_TASK_HELD, "task_id"),
            (EVENT_TASK_ASSERTED, "task_id"),
        ],
        // 读数不替任何事实关门。
        evidence: &[],
        cost: COST,
        eval: crate::autonomy::autonomy,
        agg: Some(crate::autonomy::merge),
    },
    ViewDecl {
        name: CONATION_POLICY,
        ver: 1,
        // `session.config` 没有 `FactDecl`（owner 是 `loop` 的词汇表）：这两对同样走
        // `D-282` 的第二种读法。
        reads: &[
            (EVENT_CONATION_EXPRESSED, "goal"),
            (EVENT_SESSION_CONFIG, "conation_enabled"),
        ],
        // 判决不替任何事实关门。
        evidence: &[],
        cost: COST,
        eval: policy,
        // 逐会话的判决，不参与跨会话汇总（`D-384`）。
        agg: None,
    },
];

/// 采纳率：**一条算式**，单片与 N 片都调它。
///
/// 无表达 ⇒ `0.0`（**有据的零**：没主动发起过，不是「采纳率未知」）。分母含被否决 / 被忽略
/// 的欲——表达过但没升格的欲照常入视图，它是**事实**，藏起来才是撒谎。
fn rate(expressed: u64, escalated: u64) -> f64 {
    if expressed == 0 {
        return 0.0;
    }
    escalated as f64 / expressed as f64
}

/// 纯折叠：`conation.expressed` + `task.opened` 的溯源边 → 意图流 + 两个计数 + 这一片的率。
///
/// 两条口径（与迁移前的投影逐字同源）：
///
/// - **升格靠溯源认**：一条欲「已升格」当且仅当存在一条 `task.opened` 的 `produced_by` 指向
///   它。不靠 goal 字符串相等——字符串相等会把**别的**来源开的同目标任务也算成这条欲的产物。
/// - **as-of**：形参 `now` 就是锚，`ts > now` 的欲与升格一概不见。
///
/// 欲是**流**：按 seq 升序交回（顺序即发生顺序），不截断、不打分——截断它等于悄悄丢掉
/// 「想要过什么」的事实。
fn intents(events: &[Event], now: i64) -> Value {
    // 欲事件按到达序收；升格事件可能先于 / 晚于欲事件到达，故两趟合并。
    let mut items: Vec<(u64, i64, String)> = Vec::new();
    let mut promoted: std::collections::BTreeMap<u64, String> = std::collections::BTreeMap::new();
    for e in events {
        if e.ts > now {
            continue;
        }
        match e.entity {
            Entity::Conation => {
                if e.kind.as_str() == EVENT_CONATION_EXPRESSED {
                    let seq = e.seq.map(|s| s.value()).unwrap_or(u64::MAX);
                    let goal = e
                        .payload
                        .get("goal")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    items.push((seq, e.ts, goal));
                }
            }
            Entity::Task => {
                if e.kind.as_str() == EVENT_TASK_OPENED {
                    if let (Some(src), Some(tid)) = (
                        e.produced_by,
                        e.payload.get("task_id").and_then(Value::as_str),
                    ) {
                        promoted.insert(src, tid.to_string());
                    }
                }
            }
            _ => {}
        }
    }
    // 欲是流：按发生顺序（seq 升序）排，不打分、不按时间倒序。
    items.sort_by_key(|(seq, _, _)| *seq);
    let expressed = items.len() as u64;
    let mut escalated = 0u64;
    let mut declared: Vec<String> = Vec::new();
    let stream: Vec<Value> = items
        .into_iter()
        .map(|(seq, ts, goal)| {
            let task_id = promoted.get(&seq).cloned();
            if task_id.is_some() {
                escalated += 1;
                declared.push(goal.clone());
            }
            json!({ "seq": seq, "ts": ts, "goal": goal, "task_id": task_id })
        })
        .collect();
    json!({
        "intents": stream,
        // 「哪些目标已经声明过」由 owner 判（= 升格过的那几条），读方只做**查表**。
        // 让读方自己扫 `intents` 找 `task_id.is_some()`，等于把「已声明」的定义搬到读方——
        // 那是第二份口径，而它今天恰好只有一行，最容易在两处各改一半。
        "declared": declared,
        "expressed": expressed,
        "escalated": escalated,
        "adoption_rate": rate(expressed, escalated),
    })
}

/// **判决折叠**：最新一条「欲」+ 最新一条 `session.config` → 这一条该不该升格（`D-384`）。
///
/// 三条口径：
///
/// - **只判带溯源的那条**：无溯源的欲构造不出候选（I2），会话根本不会拿它来问；把它当
///   判决对象只会让会话按 `seq` 对不上而白跑一趟。所以取「最近一条 `produced_by` 非空」
///   的表达。
/// - **开关读最近一条快照**：缺格 / 非布尔 ⇒ 出厂值关（`D-252` / `D-379`）。读不到不等于
///   开着——装配问题不得被读成放行。
/// - **as-of**：`ts > now` 的快照与欲一概不见，与其余视图同口径。
///
/// 输出的 `seq` 是判决的**绑定锚**：会话拿候选的 seq 来对，对不上就不升格。少这一格，
/// 「读到的是哪一条欲的判决」只能靠猜「反正最后一条就是我刚写的那条」。
pub(crate) fn policy(events: &[Event], now: i64) -> Value {
    let seq_key = |e: &&Event| e.seq.map(|s| s.value()).unwrap_or(u64::MAX);
    let enabled = events
        .iter()
        .filter(|e| e.ts <= now && e.kind.as_str() == EVENT_SESSION_CONFIG)
        .max_by_key(seq_key)
        .and_then(|e| e.payload.get("conation_enabled").and_then(Value::as_bool))
        .unwrap_or(DEFAULT_ENABLED);
    let want = events
        .iter()
        .filter(|e| {
            e.ts <= now && e.kind.as_str() == EVENT_CONATION_EXPRESSED && e.produced_by.is_some()
        })
        .max_by_key(seq_key);
    let Some(want) = want else {
        return json!({
            "seq": Value::Null,
            "goal": Value::Null,
            "enabled": enabled,
            "max_goal_len": MAX_GOAL_LEN,
            "promotable": false,
            "reason": "no expressed intent",
        });
    };
    let goal = want
        .payload
        .get("goal")
        .and_then(Value::as_str)
        .unwrap_or("");
    let (promotable, reason) = if !enabled {
        (false, Some("conation disabled"))
    } else if goal.len() > MAX_GOAL_LEN {
        (false, Some("goal too broad"))
    } else {
        (true, None)
    };
    json!({
        "seq": want.seq.map(|s| s.value()),
        "goal": goal,
        "enabled": enabled,
        "max_goal_len": MAX_GOAL_LEN,
        "promotable": promotable,
        "reason": reason,
    })
}

/// **跨会话合并**：N 份 [`intents`] 的产物 → 一份汇总（`D-281`）。
///
/// 只收分子 / 分母，率由 [`rate`] 从汇总值重算——合并的是**可加的量**，率是同一套算式在
/// 汇总值上的再应用，不是另一份口径。不合并 `intents`：跨会话的意图流没有统一顺序，
/// 拼起来既不是「这一片」也不是「全部」，读方要流就逐会话读。
///
/// 认不出的载荷按 0 计：形状是本插件自己上一版交的，读不出说明版本对不上——那会让率变小，
/// 而不会让率凭空出现（缺项不许伪装成读数，`D-281` 理由 3）。
fn merge(parts: &[Value]) -> Value {
    let mut expressed = 0u64;
    let mut escalated = 0u64;
    for p in parts {
        expressed += p.get("expressed").and_then(Value::as_u64).unwrap_or(0);
        escalated += p.get("escalated").and_then(Value::as_u64).unwrap_or(0);
    }
    json!({
        "expressed": expressed,
        "escalated": escalated,
        "adoption_rate": rate(expressed, escalated),
    })
}

// 装配口的提交：宿主链接本 crate 即生效。本插件不认识也不依赖任何兄弟插件。
symbio_plugin_api::submit_declarations!(OWNER, submitted_declarations);

fn submitted_declarations() -> PluginDeclarations<'static> {
    PluginDeclarations {
        events: EVENTS,
        facts: FACTS,
        views: VIEWS,
        ports: &[],
        actors: Vec::new(),
        rows: ROWS,
    }
}

#[cfg(test)]
#[path = "decl.test.rs"]
mod tests;
