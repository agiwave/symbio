//! 事实与视图的**声明层**——让「字段都在声明里」第一次可查（DE-015 §3.2 / §3.4）。
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
//! 视图自己读时钟，同一份日志重放两次必须得到同一结果（D-115 决策 1）。
//! DE-015 判据 3（视图互不可见）因此落在类型上，不落在纪律上。
//!
//! ## 声明归谁
//!
//! 内核只声明**协议五格**（`request` / `sent` / `response` / `failed` / `retry`）——
//! 它们是引擎自己的写读面，属于语法而非业务。控制面事实与全部视图由
//! `plugins::loop::control_decl` 声明，经 [`Builder::declare`] 交进来；内核不认识
//! 那些名字。新增一个视图、一个控制面事实、一处字段，改的是插件文件，不是本模块。
//!
//! 插件交进来的声明仍受同一条资格约束：某一行以它为 `owes` / `until` / `unless`，
//! 或某条**被行用到的**视图读它（DE-015 判据 7）。只在审计 / SLO 投影里读的断言格
//! 不进控制面表——`turn.dispatch` 是现例：它的读方是
//! `invariants::declared_budget_ms`，表里没有一行以它为前件或出口。
//!
//! ## 生产接线
//!
//! 装载入口在 `plugins::loop::loop_rows`：它把 [`Builder::new`] 预置的协议五格与
//! 插件自己的声明集一起 `seal`，再交给 `owed` / `drive`。`Frozen::eval` 的成本上界
//! 仍按声明生效：调用方传入**本轮切片**（不是整份 WAL），一轮的事件数由轮的生命周期
//! 界定。

use std::collections::HashSet;
use std::sync::Arc;

use crate::symbio_core::Event;
use crate::symbio_core::LatencyTier;
// 协议名的**唯一**定义在 `event/mod.rs` 的事件名字表，并登记在 `CELLS` 上。
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
    /// **输出证据集**：本视图的输出是哪些事实名的证据。装载期查它 ⊆ `reads` 的事实名。
    ///
    /// 与 `reads` 分别是两件事：`reads` 回答「它读什么」（第 3 / 7 条的输入，为定位
    /// 窗口而读也算消费），`evidence` 回答「它替谁关门」。视图名常出现在行的 `until`
    /// 里，而依赖图（§3.4 第 1 条）把出口展开成事实名时**只能按 `evidence`**——按
    /// `reads` 全集展开会把「帮我算幕锚」当成「替我收口」，共写一族事实的每一行互相
    /// 连成假环，装载直接红（判据与被否的形状见 DE-015 的 D-160）。
    ///
    /// 例：`turn_closed` 读 `user.message` / `turn.stage` 是为了折出幕锚，它的输出
    /// 只证据两格收束格 ⇒ `evidence = [chat.assistant.final, chat.assistant.fallback]`。
    pub(crate) evidence: &'static [&'static str],
    /// 扫描成本上界（事件条数）。`owed` 每轮求值，成本必须有上界。
    pub(crate) cost: u32,
    /// 求值函数：**只有事实切片与时间戳**——没有写口、没有别的视图、没有时钟。
    pub(crate) eval: fn(&[Event], i64) -> serde_json::Value,
}

/// 一个**执行体**的声明（DE-015 §2.3）。
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
/// 「一个 `by` 一个动作」由绑定表与冻结表同时保证。裁决见 D-119。
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

/// 一个插件交出的全部声明：五张表一次交齐。
///
/// 这是插件与内核之间**唯一**的扩展口（[`Builder::declare`]）。声明住在插件里，
/// 内核只负责聚合、校验、冻结。`actors` 是 `Vec` 而不是静态切片：执行体的 `run`
/// 绑定住在插件自己的绑定表里，装载期只投影出静态可查的四样（D-119）。
pub(crate) struct PluginDeclarations<'a> {
    pub(crate) facts: &'static [FactDecl],
    pub(crate) views: &'static [ViewDecl],
    pub(crate) ports: &'static [PortDecl],
    pub(crate) actors: Vec<ActorDecl>,
    pub(crate) rows: &'a [Decl],
}

/// 装载期的唯一注册口。`seal()` 之后没有可变方法（见 [`Frozen`]）。
///
/// ## 两个口：`declare` 与 `seal`
///
/// 插件只经 [`Builder::declare`] 交五张表；内核在 [`Builder::new`] 里只预置自己的
/// 协议五格。填表原语是私有的：它们只服务于 `seal` 的内部实现与本模块用例，
/// 不对插件开放——否则「一个插件一次交齐」的缝会被五条零散调用绕开。
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
    /// 内核自己的声明：协议五格（引擎写、引擎读）。
    ///
    /// 业务事实与全部视图**不在**这里——它们由插件经 [`Builder::declare`] 交进来。
    pub(crate) fn new() -> Self {
        let mut b = Self::empty();
        for &f in PROTOCOL_FACTS {
            b.fact(f);
        }
        b
    }

    /// 空表：只给本模块用例搭小世界。
    fn empty() -> Self {
        Self::default()
    }

    /// 一次装入一个插件的全部声明。
    pub(crate) fn declare(&mut self, p: PluginDeclarations<'_>) {
        for &f in p.facts {
            self.fact(f);
        }
        for &v in p.views {
            self.view(v);
        }
        for &port in p.ports {
            self.port(port);
        }
        for actor in p.actors {
            self.actor(actor);
        }
        for row in p.rows {
            self.row(row.clone());
        }
    }

    fn fact(&mut self, d: FactDecl) {
        self.facts.push(d);
    }

    fn view(&mut self, d: ViewDecl) {
        self.views.push(d);
    }

    fn actor(&mut self, d: ActorDecl) {
        self.actors.push(d);
    }

    fn port(&mut self, d: PortDecl) {
        self.ports.push(d);
    }

    fn row(&mut self, d: Decl) {
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
            // 执行体交回的陈述（D-123 决策 1–2；实现 `plugins/loop` 的 `drive_steps`）。
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
            // 输出证据集必须是读面子集（DE-015 的 D-160）。反过来的漏网更坏：
            // `evidence` 里写一个没读的名字，依赖图就少一条边，而**装载是绿的**——
            // 假环至少红得看得见，静默少边只在真死锁那天显形。
            for fact in v.evidence {
                if !v.reads.iter().any(|(f, _)| f == fact) {
                    gaps.push(Gap::EvidenceNotRead { view: v.name, fact });
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
// 名字住在 `event/mod.rs` 的事件名字表，并登记在 `CELLS` 上。
//
// `via` 是**这一次往返是谁的**（D-122 决策 2）：这一轮自己的模型往返**不带**
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

// `response` 比请求多两类字段：
//
// 1. **交回的工具调用**（D-117）：创建行 `open-tools` 的展开源——一个
//    `response(port=model)` 的 `calls` 数组逐元素开债，元素键与随后的
//    `request(port=tool)` 同 id。其它端口的应答不带这个字段（缺席即空集合）。
// 2. **这一轮说了什么**（见 `OUTPUT_FIELDS`）：正文 / 思考与它们的节点身份。
//    收束 / 进展 / 兜底格只引用 `answer_id`，正文归这条事实自己——重建按它在
//    **应答落格的位置**折出节点，顺序才是实时的「思考→卡片→思考→卡片」。
const RESPONSE_FIELDS: &[&str] = &[
    "port",
    "id",
    "attempt",
    "calls",
    "via",
    "turn_id",
    "text",
    "text_id",
    "reasoning",
    "reasoning_id",
    "answer",
    "answer_id",
    "truncated",
    "principal",
];

// `sent` 多三块：**派发时约定的租约**（`lease_ms` / `attempt_cap`）与
// **这次尝试有没有活着的等待方**（`awaited`）。
//
// ## 为什么租约参数在事实上而不只在 `PortDecl` 里
//
// `stale_sent` 是视图——它的入参只有事实切片与 `now`（D-115 决策 1），读不到
// 代码里的 `PortDecl`。没有这两块，视图算不出「这次派发等了多久算卡住」。
//
// ## 为什么在 `sent` 上而不是在 `request` 上
//
// 租约是**派发那一刻**的约定（“这次出去，N 毫秒没应答就算卡住”），写进 `sent`
// 就不必再按 `(port,id,attempt)` 与 `request` 做一次连接。且它同时是回放的
// 必要信息：端口参数未来改了，历史调用的租约仍是当时那个（不是「今天这个」）。
//
// `awaited` 是**逐次尝试**的属性，不是端口属性：同一个端口里，普通批次调用
// 崩溃后没有等待方（草稿随进程消失），唤醒调用却能由 `wake_batch` 从事实重建。
// 租约看护只对 `awaited = true` 的那几次尝试重试——否则会造出一条永远没人派发的
// `request`（D-138）。
const SENT_FIELDS: &[&str] = &[
    "port",
    "id",
    "attempt",
    "lease_ms",
    "attempt_cap",
    "awaited",
];

const PROTOCOL_FACTS: &[FactDecl] = &[
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
    // `failed` 与其它协议格同形，**带 `attempt`**（D-138）：它关的是「这一次尝试
    // 失败」；「这笔调用不再重试」由「没有后继 `request`」表达，不由键的粒度表达。
    //
    // 这样 `dispatch-<port>` / `await-<port>` / `lease` 才能按 `(port, id, attempt)`
    // 与它同键收口；`open-tools` / `deny-tools` 仍按 `(port, id)` 读它（模式只取
    // 自己声明的键字段，多出来的 `attempt` 不影响逻辑级读者）。
    //
    // `via` 与 `request` / `response` 同形：失败也要说得出「这是谁的往返」——
    // 内部往返失败**不**让这一轮收束（它不是「模型没答成」）。
    FactDecl {
        name: EVENT_FAILED,
        port: Some("port"),
        key: &["id", "attempt"],
        expand: &[],
        fields: &[
            "port",
            "id",
            "attempt",
            "why",
            "via",
            "status",
            "turn_id",
            "text",
            "text_id",
            "reasoning",
            "reasoning_id",
            "answer",
            "answer_id",
            "truncated",
            "principal",
        ],
    },
    // 尝试级否定：租约看护者落的「这一次作废、换一次」。
    // 与 `failed` 的边界：`failed` 是一次尝试的终局，`retry` 是「这次不算数」；
    // 两者都只关同一次尝试（D-138）。
    FactDecl {
        name: EVENT_RETRY,
        port: Some("port"),
        key: &["id", "attempt"],
        expand: &[],
        fields: PROTOCOL_FIELDS,
    },
];


#[cfg(test)]
#[path = "mod.test.rs"]
mod tests;
