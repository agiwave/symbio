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
//! ## 首批声明（= 控制面声明集）
//!
//! 16 个事实：协议五名（`request` / `sent` / `response` / `failed` / `retry`）、助手文本五格
//! （`chat.assistant.final` / `fallback` / `reported` / `opening` / `progress`）、
//! `turn.stage`、`user.message`、`turn.supplemented`、`session.config`、
//! `authz.denied`、`authz.granted`；7 个视图：`in_flight` / `stale_sent` / `turn_head` /
//! `turn_stages` / `turn_closed` / `report_due` / `reported_anchors`。
//!
//! **进这张表的唯一资格**：某一行以它为 `owes` / `until` / `unless`，或某条**被行用到的**
//! 视图读它（PL-016 判据 7 的控制面声明集）。只在审计 / SLO 投影里读的断言格不进表——
//! `turn.dispatch` 是这一类的现例：它的读方是 `invariants::declared_budget_ms`，
//! 表里没有一行以它为前件或出口，所以它是业务事实，归投影管（判据 7 不约束业务事实，
//! 强迫它们各挂一行只会造出装饰行）。
//!
//! 视图点名决定了事实的**下限**：`in_flight` 读三个协议名（`sent` / `response` /
//! `failed` / `retry`），`stale_sent` 读 `sent` 的租约两字段（`lease_ms` /
//! `attempt_cap`）与三个出口，`turn_head` 读 `user.message` / `turn.stage` /
//! `session.config`，`turn_stages` 与 `turn_closed` 读本幕锚之后的阶段 / 收束格，
//! `report_due` 读助手文本五格 / `turn.supplemented` / `turn.stage`（恢复幕）/
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
use crate::symbio_core::LatencyTier;
// 协议名的**唯一**定义在 `event/mod.rs` 的事件名字表（T-224 起），并登记在 `CELLS` 上。
// 一个事实名只有一处定义：常量在事件域，格位在网格域，声明层只引用。
use crate::symbio_core::event::{
    EVENT_FAILED, EVENT_REQUEST, EVENT_RESPONSE, EVENT_RETRY, EVENT_SENT,
};

pub(crate) mod ring;

use ring::{check_ports, check_rows, overlaps, Decl, Gap, Overlap, PortDecl};

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

/// 一个**执行体**的声明（PL-016 §2.3）。
///
/// ## 为什么四个字段都不是装饰
///
/// `name` 是 `by` 的查找键；`mode` 是令牌闸门的依据（反射档拿不到模型句柄）；
/// `reads` 在冻结期逐个查视图存在（视图名打错 ⇒ 运行期静默返回空）；`writes` 是
/// 第 5 / 6 条检查的输入（写入封闭、执行体收口）。裸函数这三样全部失效。
///
/// ## `run` 不在这里
///
/// 方案 §2.3 的草图把 `run` 画进 `ActorDecl`，其签名 `BoxFuture<'static, …>` 与
/// §2.6 契约 1（`drive` 是 async、持本轮草稿）在类型上互斥：`'static` 的 future
/// 借不到草稿，也借不到 `&dyn LlmAdapter`。执行体的可调用面因此住在**注册它的
/// 插件**里（`plugins::loop::turn_exec::ACTORS`），机制层只持有静态可查的四样——
/// 「一个 `by` 一个动作」由绑定表与冻结表同时保证。裁决见 ADR-119。
///
/// 与 `actors::ActorSpec` 无关：那里的 actor 是**主体**（人格 / 推理），这里的
/// actor 是**执行体**（表里一个名字对应一次动作）。方案用同一个词，此处不改名。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ActorDecl {
    /// 执行体名——行表 `by` 指向它。
    pub(crate) name: &'static str,
    /// 时延档：这个执行体允许的**最慢**一档（与行的 `tier` 同词表）。
    pub(crate) mode: LatencyTier,
    /// 读的**视图名**（存在性检查的对象；空 = 这个执行体不读任何视图）。
    pub(crate) reads: &'static [&'static str],
    /// 写的**事实名**（写入封闭与第 6 条收口检查的对象）。
    pub(crate) writes: &'static [&'static str],
}

/// 装载期的唯一注册口。`seal()` 之后没有可变方法（见 [`Frozen`]）。
///
/// ## 六个口就是 PL-016 §3.2 的六个口
///
/// `fact` / `view` / `actor` / `port` / `row` 五个填表口 + `seal`。五张注册表在
/// [`Builder`] 里是一一对应的五个 `Vec`，`seal(self)` **消耗** `self`——冻结后连
/// `&mut Builder` 都不存在，「运行期不能注册」是类型而不是纪律（§3.3）。
///
/// ## 检查的出口只有一个
///
/// `seal()` 先查声明自己（重名 / 撞名 / 引用不存在的名字 / 字段没声明 / 执行体声明了
/// 引擎独占的格），再跑 §3.4 的七道（行侧与端口侧）。它们报的都是同一个
/// [`Gap`]，因此「装载失败即拒绝启动」只有一处判据、一处 Err——不存在「声明过了但
/// 行还没查」这种部分成功。
#[derive(Debug, Default)]
pub(crate) struct Builder {
    facts: Vec<FactDecl>,
    views: Vec<ViewDecl>,
    actors: Vec<ActorDecl>,
    ports: Vec<PortDecl>,
    rows: Vec<Decl>,
}

impl Builder {
    /// 空表：只给用例搭小世界（真表经 [`Builder::builtin`] 起头）。
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// **机制层的声明集**灌进注册口（事实 + 视图），执行体 / 端口 / 行由插件注册。
    ///
    /// 这就是 §3.3 的「装载期」：机制层只声明自己那两张表，业务那三张（actor /
    /// port / row）由控制面经注册口交进来，然后一起 `seal`。
    pub(crate) fn builtin() -> Self {
        let mut b = Self::new();
        for &f in BUILTIN_FACTS {
            b.fact(f);
        }
        for &v in BUILTIN_VIEWS {
            b.view(v);
        }
        b
    }

    pub(crate) fn fact(&mut self, d: FactDecl) {
        self.facts.push(d);
    }

    pub(crate) fn view(&mut self, d: ViewDecl) {
        self.views.push(d);
    }

    pub(crate) fn actor(&mut self, d: ActorDecl) {
        self.actors.push(d);
    }

    pub(crate) fn port(&mut self, d: PortDecl) {
        self.ports.push(d);
    }

    pub(crate) fn row(&mut self, d: Decl) {
        self.rows.push(d);
    }

    /// 冻结并跑装载期全部检查。通过 ⇒ [`Frozen`]（只读）；失败 ⇒ 全部缺口。
    pub(crate) fn seal(self) -> Result<Arc<Frozen>, Vec<Gap>> {
        let mut gaps = Vec::new();

        let mut fact_names = HashSet::new();
        for f in &self.facts {
            if !fact_names.insert(f.name) {
                gaps.push(Gap::DuplicateFact { name: f.name });
            }
        }
        let mut view_names = HashSet::new();
        for v in &self.views {
            if !view_names.insert(v.name) {
                gaps.push(Gap::DuplicateView { name: v.name });
            }
        }
        for v in &self.views {
            if fact_names.contains(v.name) {
                gaps.push(Gap::NameCollision { name: v.name });
            }
        }
        let mut actor_names = HashSet::new();
        for a in &self.actors {
            if !actor_names.insert(a.name) {
                gaps.push(Gap::DuplicateActor { name: a.name });
            }
            for view in a.reads {
                if !view_names.contains(view) {
                    gaps.push(Gap::UnknownReadView {
                        actor: a.name,
                        view,
                    });
                }
            }
            // §3.4 第 5 条（写入封闭）的声明侧：执行体的账本不许含引擎独占的格子。
            // 「实际写入 ⊆ `writes`」那一半在运行期对账，判定输入是日志尾段而不是
            // 执行体交回的陈述（ADR-123 决策 1–2；实现 `plugins/loop` 的 `drive_steps`）。
            for fact in a.writes {
                if ENGINE_ONLY_CELLS.contains(fact) {
                    gaps.push(Gap::ActorWritesEngineCell {
                        actor: a.name,
                        fact,
                    });
                }
            }
        }
        let mut port_names = HashSet::new();
        for p in &self.ports {
            if !port_names.insert(p.port) {
                gaps.push(Gap::DuplicatePort { name: p.port });
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
                        gaps.push(Gap::FactFieldUndeclared {
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
                    None => gaps.push(Gap::UnknownFact { view: v.name, fact }),
                    Some(f) if !f.fields.contains(&field) => gaps.push(Gap::UndeclaredField {
                        view: v.name,
                        fact,
                        field,
                    }),
                    Some(_) => {}
                }
            }
        }

        let frozen = Frozen {
            facts: self.facts,
            views: self.views,
            actors: self.actors,
            ports: self.ports,
            rows: self.rows,
            // 报告要到缺口清空才算得（第 4 条读的是**这张**表），检查阶段先留空。
            overlaps: Vec::new(),
        };
        // §3.4 的七道检查查的是**这张表**，所以它住在这里而不是插件里：行与端口
        // 已经进了注册口，再让调用方自己想起来跑一遍，就是「半张表继续跑」的入口。
        gaps.extend(check_rows(&frozen.rows, &frozen));
        gaps.extend(check_ports(&frozen.rows, &frozen, &frozen.ports));
        if !gaps.is_empty() {
            return Err(gaps);
        }
        // 第 4 条是**报告**不是缺口：静态只能判「可能同时命中」，裁决归 `drive`。
        // 名单随表冻结一次，读侧（装载日志与用例）只问表，不再自己算第二遍。
        let overlaps = overlaps(&frozen.rows, &frozen);
        Ok(Arc::new(Frozen {
            facts: frozen.facts,
            views: frozen.views,
            actors: frozen.actors,
            ports: frozen.ports,
            rows: frozen.rows,
            overlaps,
        }))
    }
}

/// 冻结后的声明表：只有读方法，没有 `&mut self`——运行期注册口在类型上不存在。
///
/// 五张注册表全在这里（事实 / 视图 / 执行体 / 端口 / 行）：读侧（`owed` / `drive` /
/// 派发适配器 / 租约看护者）拿的切片与这张表同源，插件的 `static` 只是**进表前**的
/// 数据形状。§3.3 的「三个时点」因此能在代码上指认：编译期收集 ⇒ `Builder` 填五张表
/// ⇒ `seal` 检查并产出这一个不可变值。
#[derive(Debug)]
pub(crate) struct Frozen {
    facts: Vec<FactDecl>,
    views: Vec<ViewDecl>,
    actors: Vec<ActorDecl>,
    ports: Vec<PortDecl>,
    rows: Vec<Decl>,
    /// §3.4 第 4 条的报告（可能同时命中的行对）。**不是缺口**——冻结成功才存在，
    /// 装载入口把它打成人读的日志，用例钉住名单。
    overlaps: Vec<Overlap>,
}

impl Frozen {
    pub(crate) fn facts(&self) -> &[FactDecl] {
        &self.facts
    }

    pub(crate) fn views(&self) -> &[ViewDecl] {
        &self.views
    }

    pub(crate) fn actors(&self) -> &[ActorDecl] {
        &self.actors
    }

    /// 行表（注册顺序不参与裁决，`(prio, name, key)` 才参与）。
    pub(crate) fn rows(&self) -> &[Decl] {
        &self.rows
    }

    pub(crate) fn overlaps(&self) -> &[Overlap] {
        &self.overlaps
    }

    pub(crate) fn fact(&self, name: &str) -> Option<&FactDecl> {
        self.facts.iter().find(|f| f.name == name)
    }

    pub(crate) fn view(&self, name: &str) -> Option<&ViewDecl> {
        self.views.iter().find(|v| v.name == name)
    }

    /// 按 `by` 里的名字取执行体声明。`None` = 这个名字没有执行体（行表与绑定表脱节）。
    pub(crate) fn actor(&self, name: &str) -> Option<&ActorDecl> {
        self.actors.iter().find(|a| a.name == name)
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

// ── 协议名（载荷里带 `port`）────────────────────────────────────────────
//
// 名字住在 `event/mod.rs` 的事件名字表（T-224 起），并登记在 `CELLS` 上。
//
// `via` 是**这一次往返是谁的**（ADR-122 决策 2）：这一轮自己的模型往返**不带**
// 它，判决 / 措辞 / 摘要那些内部往返带它。判据侧靠「有没有 `via`」把两类分开——
// 内部往返的应答 `calls` 为空（它不带工具），若混进「本轮最近一次模型应答」的口径，
// 一条判决往返就会被读成「这一轮答完了」。
const PROTOCOL_FIELDS: &[&str] = &["port", "id", "attempt", "via"];

/// **引擎独占的格子**（§3.4 第 5 条「决策者伪造 `response`」那一档的名单）。
///
/// `sent` 在派发**前**由引擎落、`response` 是 `Ok`、`failed` 是 `Err` 的兜底、
/// `retry` 归租约看护者。它们写与读都只有引擎一处，所以「谁写了这一格」永远
/// 有答案；放进执行体的 `writes` 就等于允许搬运者自己声明「我派出去了 / 我回来了」。
///
/// **`request` 不在名单里**：它留给决策者——「这一轮欠一次调用」是判断，不是执行
/// （§2.3）。
const ENGINE_ONLY_CELLS: &[&str] = &[EVENT_SENT, EVENT_RESPONSE, EVENT_FAILED, EVENT_RETRY];

// `response` 比请求多一个集合字段：模型应答带回来的工具调用（ADR-117）。
//
// 它是**创建行** `open-tools` 的展开源：一个 `response(port=model)` 的 `calls`
// 数组逐元素开债，元素键与随后的 `request(port=tool)` 同 id。`response` 的其它
// 端口（`turn` / `tool`）不带这个字段——缺席即空集合，不开债。
const RESPONSE_FIELDS: &[&str] = &["port", "id", "attempt", "calls", "via"];

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
        expand: &["calls"],
        fields: RESPONSE_FIELDS,
    },
    // `failed` 关的是**逻辑 id**（所有尝试），所以没有 `attempt`——与协议表一致：
    // 失败不是「某一次尝试失败」，是这笔调用不再重试。
    //
    // `via` 与 `request` / `response` 同形：失败也要说得出「这是谁的往返」——
    // 内部往返失败**不**让这一轮收束（它不是「模型没答成」）。
    FactDecl {
        name: EVENT_FAILED,
        port: Some("port"),
        key: &["id"],
        expand: &[],
        fields: &["port", "id", "why", "via"],
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
    //
    // `model_calls` 是**这一步打了几次模型**（ADR-122 决策 4 / 5）：`source` 记的是
    // 判决的**来源**，读不出「有没有打过模型」（模型调了而应答不可解析 ⇒
    // `source=fallback`）。这一格是那次模型往返在事实层唯一的**数**——判据侧
    // 拿它对账「观测 ⇔ 三格」。
    FactDecl {
        name: "turn.stage",
        port: None,
        key: &[],
        expand: &[],
        fields: &[
            "stage",
            "round",
            "source",
            "reason",
            "result",
            "model_calls",
        ],
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
    // 本轮派发档（`turn.dispatch`）**不在本表**：表里没有一行以它为前件或出口，
    // 也没有一条被行用到的视图读它；它的读方是审计投影
    // （`invariants::declared_budget_ms`）。按判据 7 的口径它是业务事实，
    // 名字与写方都照旧，只是不进控制面声明集。
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
///
/// 轮号跟着**最新幕锚**走，而不是取切片里最大的轮号：恢复一条被后续轮次归档
/// 的旧轮时，切片从旧轮首条事件一直延伸到今天，最大轮号指向新轮——旧轮的恢复幕
/// 在表头里根本看不见（这正是 `resume_anchor` 旁路存在的原因，T-244 步 5d）。
/// 恢复幕的 `turn.stage{stage:"resume"}` 是开演时刚落的，`ts` 比任何既有轮次都新，
/// 按 `(ts, seq)` 取最后一条幕锚即可让表头回到被续写的那一轮。
///
/// 没有幕锚的切片（测试夹具 / 只有配置事实）退回「最大轮号」——幕锚缺席时
/// 没有更诚实的轮号可给。
fn drive_of(events: &[Event]) -> Option<(u64, Option<&Event>)> {
    if let Some(anchor) = events
        .iter()
        .filter(|e| is_drive_anchor(e))
        .max_by_key(|e| (e.ts, seq_of(e)))
    {
        return Some((anchor.turn, Some(anchor)));
    }
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
    Some((turn, None))
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

/// 轮头（T-244 步 5 升到 v5，步 5d 升到 v6）：五个策略行的开账条件全在这一格里，
/// 收束行与执行行也是。
///
/// `done` 三格按幕锚折叠（续写幕复用轮号，不能按轮扫全史）；`skill_hit` 取本幕
/// 召回阶段的结论，`resumed` 区分续写幕与用户新发言。
///
/// v5 加一格 `tools_open`：本幕最近一条模型应答交回了工具调用、且其中至少一笔
/// 还没被终局关掉。执行行的开账条件要它——工具批没关时模型不许再往下走
/// （T-244 步 5b：`execute-turn` 从「跑下一步」退成「只跑手上那批」）。
/// v6 给它补了请求面（见该格的注释）：恢复幕的唤醒请求也在「有活没干完」之列。
///
/// v8（T-247 / ADR-122 决策 2）：「本幕最近一次模型应答」与「模型没答成」两格都
/// 只看**这一轮自己的**模型往返（`via` 缺席的那些）——内部往返（判决 / 措辞 / 摘要）
/// 的应答 `calls` 为空，混进来会让一条判决往返被读成「这一轮答完了」。
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
    // 本幕是否已收束（与 [`turn_closed`] 同一判法：幕锚之后出现过收束格）。
    let closed = events.iter().any(|e| {
        e.turn == turn
            && matches!(
                e.kind.as_str(),
                "chat.assistant.final" | "chat.assistant.fallback"
            )
            && after_anchor(e, anchor_seq)
    });
    // 本幕最近一条 `response(port=model)`：收束格（`calls` 空）与工具格
    // （`calls` 非空且未关完）共用同一次选取，口径只在这里。
    //
    // **只取这一轮自己的模型往返**（`via` 缺席的那些，ADR-122 决策 2）：判决 / 措辞 /
    // 摘要那些内部往返的应答 `calls` 为空（它们不带工具），混进来会让一条判决往返
    // 被读成「这一轮答完了」⇒ 收束行提前开账。
    let model_calls = events
        .iter()
        .filter(|e| {
            e.turn == turn
                && e.kind == EVENT_RESPONSE
                && str_field(e, "port") == Some("model")
                && str_field(e, "via").is_none()
                && after_anchor(e, anchor_seq)
        })
        .max_by_key(|e| seq_of(e))
        .and_then(|e| e.payload.get("calls"))
        .and_then(|v| v.as_array());
    // 收束该不该发生（ADR-121 决策 2）：口径钉在日志上——**本幕最近一条
    // `response(port=model)` 的 `calls` 为空**（`expand: ["calls"]` 是「这条响应要不要
    // 转出工具债」的唯一数据面），且本幕未收束。执行体手里那份草稿不参与判定，
    // 否则崩溃恢复之后「答完了没」就成了假话。
    //
    // `calls` 取不到数组 ⇒ 不算收束（宁可让 `close-turn` 不开账，也不拿一个来路不明的
    // 形状把没答完的一轮判成答完）；没有响应（首步就中止 / 失败的轮）同理。
    let has_final = !closed && model_calls.is_some_and(|calls| calls.is_empty());
    // 本幕模型端口的**结局**（T-244 步 5c）：本幕**派发过**的一次模型往返，最后落在
    // `failed(port=model)` 上——模型没答成，且之后没有更晚的成功应答。
    //
    // 这一格是「中止 / 适配器失败」那两种**收束**的前件：它们没有「`calls` 为空的
    // 模型应答」（答都没答成，哪来应答），`has_final` 因此不成立；但它们同样要把
    // 这一轮收掉（兜底格 / 中止格），而收束格只由 `close` 执行体写 ⇒ 行表得能
    // 判出「该收束了」，判据只能是日志上的东西。
    //
    // 「派发过」（同一 `(port, id)` 的 `request` 也在本幕锚之后）这一条不是装饰：
    // 崩溃恢复时租约看护会在**本幕锚之后**落一条 `failed(port=model)`——它收的是
    // 上一世那笔卡住的调用，而那笔的 `request` 在锚之前。少了这条，恢复幕一进门
    // 就会判「该收束了」，把「重跑一次模型往返」读成「这一轮已经答不上来了」。
    //
    // `via` 缺席的那些才是「这一轮自己的模型往返」（ADR-122 决策 2）：内部往返失败
    // （判决没打出来）退化成「判不出来 ⇒ 进工具循环」，**不是**「模型没答成 ⇒
    // 收束」——后者会让一条判决失败把整轮收掉。
    let model_outcome_failed = events
        .iter()
        .filter(|e| {
            e.turn == turn
                && matches!(e.kind.as_str(), EVENT_RESPONSE | EVENT_FAILED)
                && str_field(e, "port") == Some("model")
                && str_field(e, "via").is_none()
                && after_anchor(e, anchor_seq)
        })
        .max_by_key(|e| seq_of(e))
        .is_some_and(|e| {
            e.kind == EVENT_FAILED
                && events.iter().any(|r| {
                    r.turn == turn
                        && r.kind == EVENT_REQUEST
                        && str_field(r, "port") == Some("model")
                        && str_field(r, "via").is_none()
                        && after_anchor(r, anchor_seq)
                        && str_field(r, "id") == str_field(e, "id")
                })
        });
    // 该不该收束：`has_final`（答完了）∨ 模型没答成（不会再有应答）。两条都要求
    // 本幕未收束——收束过就不欠了。
    let closeable = !closed && (has_final || model_outcome_failed);
    // 工具批还开着（T-244 步 5b）：交回的调用里至少有一笔没被 `response` / `failed`
    // （同 `port=tool` 同 id）关掉。两格互斥（`calls` 空 vs 非空），所以执行行只在
    // 「有活没干完」的时候让位。
    //
    // v6（T-244 步 5d）再加一条**请求面**的判据：清点的是**本轮**（不是本幕）里
    // 落过、却还没有终局的 `request(port=tool)`。恢复幕交回的那一笔就靠它：模型
    // 响应是首幕的（本幕没有新的模型应答），旧批次也早已关账，唯一的「开着」是
    // 唤醒请求本身——不看请求面，执行行会在派发之前抢先跑一次模型往返。
    //
    // 判据按请求的 `seq` 取「之后有没有终局」，不按幕锚：唤醒请求落在幕锚
    // **之前**（恢复幕的锚是 `begin_drive` 才落的那条阶段事实）。
    let request_open = !closed
        && events.iter().any(|req| {
            req.turn == turn
                && req.kind == EVENT_REQUEST
                && str_field(req, "port") == Some("tool")
                && !events.iter().any(|e| {
                    e.turn == turn
                        && matches!(e.kind.as_str(), EVENT_RESPONSE | EVENT_FAILED)
                        && str_field(e, "port") == Some("tool")
                        && str_field(e, "id") == str_field(req, "id")
                        && seq_of(e) > seq_of(req)
                })
        });
    let tools_open = !closed
        && (request_open
            || model_calls.is_some_and(|calls| {
                calls.iter().any(|c| {
                    let id = c.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    !events.iter().any(|e| {
                        e.turn == turn
                            && matches!(e.kind.as_str(), EVENT_RESPONSE | EVENT_FAILED)
                            && str_field(e, "port") == Some("tool")
                            && str_field(e, "id") == Some(id)
                            && after_anchor(e, anchor_seq)
                    })
                })
            }));
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
        "has_final": has_final,
        "closeable": closeable,
        "tools_open": tools_open,
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
    // 轮头（T-244 步 5 升到 v5，步 5d 升到 v6）：五个策略行的开账条件 +
    // `close-turn` / `execute-turn` 的开账条件。
    //
    // v3 → v4 只加一格：`has_final`（口径见 [`turn_head`]）。它是收束行的**前件**，
    // 因此读面要连收束格与模型响应的 `calls` 一起声明——视图声明的字段名是装载期
    // 的孤儿事实判据（`OrphanFact`）的输入，声明与算法不一致就是给表少报一条边。
    // v4 → v5 加 `tools_open`：它读 `failed` 的 `port` / `id`（工具批的另一种终局），
    // 其余读面不变。
    // v5 → v6 给 `tools_open` 补请求面：它读 `request(port=tool)` 的 `port` / `id`。
    // v6 → v7 加 `closeable`：收束行的前件从「答完了」扩到「答完了 **或** 模型没答成」
    // ——后者的读面是 `failed` 的 `port`（`failed(port=model)`）与 `request` 的
    // `port` / `id`（判「本幕派发过」），v4 / v5 起就已在读面里，故读面不变。
    // v7 → v8：口径收窄到「这一轮自己的模型往返」——`request` / `response` / `failed`
    // 三个协议名的 `via` 字段进读面（ADR-122 决策 2）。内部往返（判决 / 措辞 /
    // 摘要）带 `via`，这一轮自己的不带；不带的那批才是「答完了 / 没答成」的判据。
    ViewDecl {
        name: "turn_head",
        ver: 8,
        reads: &[
            ("user.message", "id"),
            ("turn.stage", "stage"),
            ("turn.stage", "round"),
            ("turn.stage", "result"),
            ("session.config", "classify_enabled"),
            ("session.config", "auto_compress"),
            ("session.config", "is_persistent"),
            (EVENT_REQUEST, "port"),
            (EVENT_REQUEST, "id"),
            (EVENT_REQUEST, "via"),
            (EVENT_RESPONSE, "port"),
            (EVENT_RESPONSE, "calls"),
            (EVENT_RESPONSE, "via"),
            (EVENT_FAILED, "port"),
            (EVENT_FAILED, "id"),
            (EVENT_FAILED, "via"),
            ("chat.assistant.final", "text"),
            ("chat.assistant.fallback", "text"),
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
