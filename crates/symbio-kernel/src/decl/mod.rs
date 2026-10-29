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
//! ## `EventDecl` 与 `FactDecl` 的分工
//!
//! [`EventDecl`] 管「名字落在哪个格子」，[`FactDecl`] 管「名字有哪些字段」。两者并存，
//! 不互相推导：格子是实体 × 动词，字段是载荷形状，合并会逼其中一方说谎。
//! 一条事实可以同时出现在两张表里（义务环要它的字段，网格要它的格位），但**两张表的
//! 登记口分开**：事实未必进环，进环的事实才需要字段声明。
//!
//! 名字 → 格位的唯一来源是 [`EventDecl`]（DE-024 D-197）：内核只声明协议五格，业务
//! 事件由 owner 插件声明。写闸只问装配表与共享表，两者都不认识 ⇒ 网格之外（fail-closed）。
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
//! `crates/symbio-plugin-loop/src/control_decl.rs` 声明，经 [`Builder::declare`] 交进来；内核不认识
//! 那些名字。新增一个视图、一个控制面事实、一处字段，改的是插件文件，不是本模块。
//!
//! **owner 就是「谁把这张表交进 `Builder`」**（DE-023 D-178）：`declare` 的第一个
//! 参数是 owner 标识，`Frozen` 逐条记下 `fact_owner` / `view_owner`。它不是
//! `FactDecl` / `ViewDecl` 上的字段——声明形状不因登记而变宽，也不存在「声明里的
//! owner 与装载方不一致」这种可漂的第二事实。
//!
//! 插件交进来的声明仍受同一条资格约束：某一行以它为 `owes` / `until` / `unless`，
//! 或某条**被行用到的**视图读它（DE-015 判据 7）。只在审计 / SLO 投影里读的断言格
//! 不进控制面表——`turn.dispatch` 是现例：它的读方是 core 的预算判据（按
//! `declares_budget` 角色读它，DE-024 D-203），表里没有一行以它为前件或出口。
//!
//! ## 生产接线
//!
//! 装载入口在 `crates/symbio-plugin-loop/src/loop_rows.rs`：它把 [`Builder::new`] 预置的协议五格与
//! 插件自己的声明集一起 `seal`，再交给 `owed` / `drive`。`Frozen::eval` 的成本上界
//! 仍按声明生效：调用方传入**本轮切片**（不是整份 WAL），一轮的事件数由轮的生命周期
//! 界定。

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::event::{Cell, Entity, Event, EventDecl, FactRole, Verb};
use crate::LatencyTier;
// 协议名的**唯一**来源是 `event/mod.rs` 的常量与 `PROTOCOL_EVENTS`（本文件的声明层
// 只引用，不另记一份名字）。
use crate::event::{EVENT_FAILED, EVENT_REQUEST, EVENT_RESPONSE, EVENT_RETRY, EVENT_SENT};

pub mod ring;

use ring::{check_ports, check_rows, overlaps, Decl, Gap, Overlap, PortDecl};

/// 一个事实名的声明。
///
/// 字段全部是数据：`name` 是唯一的身份；`port` / `key` / `expand` 都是**字段引用**，
/// 必须出现在 `fields` 里（`seal` 检查）。空切片 = 这一维不存在，不是「以后再说」。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FactDecl {
    /// 事实名。若它是事件，同名格位登记在 [`EventDecl`] 上；此处只管载荷形状。
    pub name: &'static str,
    /// 载荷里承载端口的字段名。只有协议族有端口；`None` = 这条事实与端口无关。
    pub port: Option<&'static str>,
    /// 幂等键字段：同键重复派发由适配器 / 存储按它去重。
    pub key: &'static [&'static str],
    /// 展开字段：一个事实按它展开成多个实例（工具批次用）。
    pub expand: &'static [&'static str],
    /// 载荷字段全集（含 `port` / `key` / `expand` 引用的字段）。
    pub fields: &'static [&'static str],
}

/// 一个视图的声明。
#[derive(Debug, Clone, Copy)]
pub struct ViewDecl {
    /// 视图名。与事实名共用同一名字空间（视图是合成事实）。
    pub name: &'static str,
    /// 口径版本：视图读法一变，历史事实的投影结果就变；版本让「这一轮用了哪一版」可回放。
    pub ver: u32,
    /// 读的 `(事实名, 字段名)`。这是「字段都在声明里」那条检查的输入。
    pub reads: &'static [(&'static str, &'static str)],
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
    pub evidence: &'static [&'static str],
    /// 扫描成本上界（事件条数）。`owed` 每轮求值，成本必须有上界。
    pub cost: u32,
    /// 求值函数：**只有事实切片与时间戳**——没有写口、没有别的视图、没有时钟。
    pub eval: fn(&[Event], i64) -> serde_json::Value,
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
/// 插件**里（`crates/symbio-plugin-loop/src/turn_exec::ACTORS`），机制层只持有静态可查的四样——
/// 「一个 `by` 一个动作」由绑定表与冻结表同时保证。裁决见 D-119。
///
/// 与 `actors::ActorSpec` 无关：那里的 actor 是**主体**（人格 / 推理），这里的
/// actor 是**执行体**（表里一个名字对应一次动作）。方案用同一个词，此处不改名。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActorDecl {
    /// 执行体名——行表 `by` 指向它。
    pub name: &'static str,
    /// 时延档：这个执行体允许的**最慢**一档（与行的 `tier` 同词表）。
    pub mode: LatencyTier,
    /// 读的**视图名**（存在性检查的对象；空 = 这个执行体不读任何视图）。
    pub reads: &'static [&'static str],
    /// 写的**事实名**（写入封闭与第 6 条收口检查的对象）。
    pub writes: &'static [&'static str],
}

/// 一个插件交出的全部声明：六张表一次交齐。
///
/// 这是插件与内核之间**唯一**的扩展口（[`Builder::declare`]）。声明住在插件里，
/// 内核只负责聚合、校验、冻结。`actors` 是 `Vec` 而不是静态切片：执行体的 `run`
/// 绑定住在插件自己的绑定表里，装载期只投影出静态可查的四样（D-119）。
pub struct PluginDeclarations<'a> {
    /// 事件名 → 格位。业务事件与协议事件都在这里；加名字不动内核。
    pub events: &'static [EventDecl],
    pub facts: &'static [FactDecl],
    pub views: &'static [ViewDecl],
    pub ports: &'static [PortDecl],
    pub actors: Vec<ActorDecl>,
    pub rows: &'a [Decl],
}

/// 内核自己交出的声明（协议五格）的 owner 标识。
///
/// 与插件 owner（如 `"crates/symbio-plugin-loop"`）同处一个命名空间：`Frozen` 的 owner 清单里
/// 没有第二种来源，问「这一条归谁」永远只有一个答案（DE-023 D-178）。
pub const KERNEL_OWNER: &str = "kernel";

/// 本模块用例搭小世界时的 owner（`Builder::empty` 之后的直接 `fact` / `view`）。
const DIRECT_OWNER: &str = "direct";

/// 装载期的唯一注册口。`seal()` 之后没有可变方法（见 [`Frozen`]）。
///
/// ## 两个口：`declare` 与 `seal`
///
/// 插件只经 [`Builder::declare`] 交六张表；内核在 [`Builder::new`] 里只预置自己的
/// 协议五格。填表原语是私有的：它们只服务于 `seal` 的内部实现与本模块用例，
/// 不对插件开放——否则「一个插件一次交齐」的缝会被几条零散调用绕开。
///
/// ## 检查的出口只有一个
///
/// `seal()` 先查声明自己（重名 / 撞名 / 引用不存在的名字 / 字段没声明 / 执行体声明了
/// 引擎独占的格），再跑 §3.4 的七道（行侧与端口侧）。它们报的都是同一个
/// [`Gap`]，因此「装载失败即拒绝启动」只有一处判据、一处 Err——不存在「声明过了但
/// 行还没查」这种部分成功。
#[derive(Debug, Default)]
pub struct Builder {
    events: Vec<EventDecl>,
    facts: Vec<FactDecl>,
    views: Vec<ViewDecl>,
    actors: Vec<ActorDecl>,
    ports: Vec<PortDecl>,
    rows: Vec<Decl>,
    /// 事件名 → 交表方。
    event_owners: HashMap<&'static str, &'static str>,
    /// 事实名 → 交表方（DE-023 D-178 的 owner 清单）。
    fact_owners: HashMap<&'static str, &'static str>,
    /// 视图名 → 交表方。
    view_owners: HashMap<&'static str, &'static str>,
    /// 当前交表方：`new` = 内核；`declare` 期间 = 参数；`empty` = [`DIRECT_OWNER`]。
    current_owner: &'static str,
}

impl Builder {
    /// 内核自己的声明：协议五格（引擎写、引擎读）。
    ///
    /// 业务事实与全部视图**不在**这里——它们由插件经 [`Builder::declare`] 交进来。
    pub fn new() -> Self {
        let mut b = Self::empty();
        b.current_owner = KERNEL_OWNER;
        for &e in PROTOCOL_EVENTS {
            b.event(e);
        }
        for &f in PROTOCOL_FACTS {
            b.fact(f);
        }
        b
    }

    /// 空表：只给本模块用例搭小世界。
    fn empty() -> Self {
        Self {
            current_owner: DIRECT_OWNER,
            ..Default::default()
        }
    }

    /// 一次装入一个插件的全部声明；`owner` 是**交表方**的标识（DE-023 D-178）。
    ///
    /// 同一个 owner 可以分多次交（例如测试夹具先交事实、再交视图），但一个名字只能
    /// 出现一次——重名由 `seal` 判红，owner 清单因此逐条唯一。
    pub fn declare(&mut self, owner: &'static str, p: PluginDeclarations<'_>) {
        self.current_owner = owner;
        for &e in p.events {
            self.event(e);
        }
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

    fn event(&mut self, d: EventDecl) {
        self.event_owners.insert(d.name, self.current_owner);
        self.events.push(d);
    }

    fn fact(&mut self, d: FactDecl) {
        self.fact_owners.insert(d.name, self.current_owner);
        self.facts.push(d);
    }

    fn view(&mut self, d: ViewDecl) {
        self.view_owners.insert(d.name, self.current_owner);
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
    pub fn seal(self) -> Result<Arc<Frozen>, Vec<Gap>> {
        let mut gaps = Vec::new();

        let mut event_names = HashSet::new();
        for e in &self.events {
            if !event_names.insert(e.name) {
                gaps.push(Gap::DuplicateEvent { name: e.name });
            }
        }
        let mut fact_names = HashSet::new();
        for f in &self.facts {
            if !fact_names.insert(f.name) {
                gaps.push(Gap::DuplicateFact { name: f.name });
            }
            // 插件声明的事实必须是事件（协议五格由内核预置，同样在事件表上）。
            // `Builder::empty` 的「直接小世界」是唯一例外：它只服务本模块用例，
            // 不经过装配口，也不写任何 WAL——那条缝已经由构造器私有性关住。
            if self.fact_owners.get(f.name) != Some(&DIRECT_OWNER) && !event_names.contains(f.name)
            {
                gaps.push(Gap::FactWithoutEvent { fact: f.name });
            }
        }
        let mut view_names = HashSet::new();
        for v in &self.views {
            if !view_names.insert(v.name) {
                gaps.push(Gap::DuplicateView { name: v.name });
            }
        }
        // 切点只有一个（`Frozen::scan_bound` 取最小值），所以声明的上界必须只有一种。
        // 分歧在这里判：留给运行期的两支都是「算不出来」冒充「没有欠账」（见
        // `Gap::ViewCostMismatch`）。
        if let Some(bound) = self.views.iter().map(|v| v.cost).min() {
            for v in &self.views {
                if v.cost != bound {
                    gaps.push(Gap::ViewCostMismatch {
                        view: v.name,
                        cost: v.cost,
                        bound,
                    });
                }
            }
        }
        for v in &self.views {
            if fact_names.contains(v.name) || event_names.contains(v.name) {
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
            // 执行体交回的陈述（D-123 决策 1–2；实现 `crates/symbio-plugin-loop` 的 `turn_exec::drive`）。
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
            events: self.events,
            facts: self.facts,
            views: self.views,
            actors: self.actors,
            ports: self.ports,
            rows: self.rows,
            event_owners: self.event_owners,
            fact_owners: self.fact_owners,
            view_owners: self.view_owners,
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
        // owner 清单与表同源（DE-023 D-178）：`event` / `fact` / `view` 写入时逐条登记，
        // 这里钉住「表里有几条，清单就有几条」——不成立只能是注册口被改坏了。
        assert_eq!(frozen.event_owners.len(), frozen.events.len());
        assert_eq!(frozen.fact_owners.len(), frozen.facts.len());
        assert_eq!(frozen.view_owners.len(), frozen.views.len());
        Ok(Arc::new(Frozen {
            events: frozen.events,
            facts: frozen.facts,
            views: frozen.views,
            actors: frozen.actors,
            ports: frozen.ports,
            rows: frozen.rows,
            event_owners: frozen.event_owners,
            fact_owners: frozen.fact_owners,
            view_owners: frozen.view_owners,
            overlaps,
        }))
    }
}

/// 冻结后的声明表：只有读方法，没有 `&mut self`——运行期注册口在类型上不存在。
///
/// 六张注册表全在这里（事件 / 事实 / 视图 / 执行体 / 端口 / 行）：读侧（`owed` /
/// `drive` / 派发适配器 / 租约看护者 / 写闸）拿的切片与这张表同源，插件的 `static`
/// 只是**进表前**的数据形状。§3.3 的「三个时点」因此能在代码上指认：编译期收集 ⇒
/// `Builder` 填六张表 ⇒ `seal` 检查并产出这一个不可变值。
#[derive(Debug)]
pub struct Frozen {
    events: Vec<EventDecl>,
    facts: Vec<FactDecl>,
    views: Vec<ViewDecl>,
    actors: Vec<ActorDecl>,
    ports: Vec<PortDecl>,
    rows: Vec<Decl>,
    /// 事件 / 事实 / 视图的 owner 清单（DE-023 D-178）：与六张表同源，冻结后只读。
    event_owners: HashMap<&'static str, &'static str>,
    fact_owners: HashMap<&'static str, &'static str>,
    view_owners: HashMap<&'static str, &'static str>,
    /// §3.4 第 4 条的报告（可能同时命中的行对）。**不是缺口**——冻结成功才存在，
    /// 装载入口把它打成人读的日志，用例钉住名单。
    overlaps: Vec<Overlap>,
}

impl Frozen {
    /// 判据的**词汇面**：全部事件名、格位与角色（协议五格 + 各 owner 插件交来的业务事件）。
    ///
    /// core 的守卫拿它问「这条名字扮演什么角色」，从此不抄名字（DE-024 D-203）。
    pub fn events(&self) -> &[EventDecl] {
        &self.events
    }

    /// 这个名字落在哪一格。`None` = 冻结表里没有它 ⇒ 网格之外（DE-024 D-199）；
    /// 写闸没有第三份表可问。
    pub fn event_cell(&self, name: &str) -> Option<Cell> {
        self.events.iter().find(|e| e.name == name).map(|e| e.cell)
    }

    /// 这个名字在守卫判据里扮演哪个角色。表上没有它 ⇒ `false`（不扮演任何东西）。
    pub fn holds_role(&self, name: &str, role: FactRole) -> bool {
        crate::event::holds_role(&self.events, name, role)
    }

    /// 每一档角色都有人住吗（装配口的整体检查，DE-024 D-203）。
    ///
    /// 为什么这条必须存在：判据读角色，而角色来自声明。owner 删掉一行 `roles` 的
    /// 表现不是「拒绝」而是「看不见」——守卫从拦人变成空转，全绿。这与 J3 点名的
    /// 静默失效同形，所以它在**冻结时**判，不在运行期判。
    pub fn unmanned_roles(&self) -> Vec<FactRole> {
        FactRole::ALL
            .iter()
            .copied()
            .filter(|role| !self.events.iter().any(|e| e.roles.contains(role)))
            .collect()
    }

    /// 这个事件名归谁：把含它的声明表交进 [`Builder`] 的那个 owner。
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn event_owner(&self, name: &str) -> Option<&'static str> {
        self.event_owners.get(name).copied()
    }

    pub fn facts(&self) -> &[FactDecl] {
        &self.facts
    }

    pub fn views(&self) -> &[ViewDecl] {
        &self.views
    }

    pub fn actors(&self) -> &[ActorDecl] {
        &self.actors
    }

    /// 行表（注册顺序不参与裁决，`(prio, name, key)` 才参与）。
    pub fn rows(&self) -> &[Decl] {
        &self.rows
    }

    pub fn overlaps(&self) -> &[Overlap] {
        &self.overlaps
    }

    pub fn fact(&self, name: &str) -> Option<&FactDecl> {
        self.facts.iter().find(|f| f.name == name)
    }

    /// 这一条事实归谁：把含它的声明表交进 [`Builder`] 的那个 owner。
    ///
    /// `None` 只该出现在「直接往 `Builder` 塞单条的用例小世界」里；生产装载
    /// （[`Builder::new`] / [`Builder::declare`]）逐条登记，不会缺。查询方是清单用例
    /// （生产装载不读它——owner 在 `seal` 里以「表长 = 清单长」钉住）。
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn fact_owner(&self, name: &str) -> Option<&'static str> {
        self.fact_owners.get(name).copied()
    }

    /// 这一条视图归谁。口径同 [`Frozen::fact_owner`]。
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn view_owner(&self, name: &str) -> Option<&'static str> {
        self.view_owners.get(name).copied()
    }

    pub fn view(&self, name: &str) -> Option<&ViewDecl> {
        self.views.iter().find(|v| v.name == name)
    }

    /// 按 `by` 里的名字取执行体声明。`None` = 这个名字没有执行体（行表与绑定表脱节）。
    pub fn actor(&self, name: &str) -> Option<&ActorDecl> {
        self.actors.iter().find(|a| a.name == name)
    }

    /// **一份切片最多能有多长**：所有视图声明的 `cost` 里最小的那一个。
    ///
    /// ## 为什么是 min，不是「每个视图各自切一份」
    ///
    /// [`eval`](Self::eval) 的输入是**同一份** `&[Event]`——`owed` 拿着一份切片轮询
    /// 每一行、每一行可能读不同视图。于是切片长度必须同时对**每一条**视图都不超界，
    /// 满足这一点的上界只有一个：最小值。
    ///
    /// ## 为什么这个口必须存在
    ///
    /// 切点此前是**手抄的常数**（`loop_rows::turn_slice` 不切、`chat_loop::drive` 抄了
    /// 一个 `LEASE_WINDOW = 4096` 与七处 `cost: 4096` 同值）。手抄同值不是同步：任一侧
    /// 单独改动，另一侧就静默失效，而失效的形状是 [`eval`](Self::eval) 回 `None`、
    /// `owed` 把它折成空集——**「算不出来」与「没有欠账」同形**，环照常空转，零告警。
    /// 于是切点改由这里回答，调用方不再持有那个数。
    ///
    /// `None` = 一条视图都没声明（空表）：没有可切的界，调用方按「不切」处理。
    /// 非空表的 `cost` 一致性由 [`Gap::ViewCostMismatch`] 在装载期钉住——切片模型
    /// 只支持一个上界，声明的分歧不是运行期能兜的事。
    pub fn scan_bound(&self) -> Option<usize> {
        self.views.iter().map(|v| v.cost as usize).min()
    }

    /// 按声明求值：未声明、或扫描条数超过声明的 `cost` 上界 ⇒ `None`。
    /// 超界不静默截断——调用方必须知道这一轮的视图没算出来。
    ///
    /// 「知道」的形式是这个 `None`；把它折成空集的是调用方（`ring::owed` 的
    /// `_ => Vec::new()`，字节冻结）。所以**切点在调用方**：先按
    /// [`scan_bound`](Self::scan_bound) 切片，就不会走到这条超界分支。
    pub fn eval(&self, name: &str, events: &[Event], now: i64) -> Option<serde_json::Value> {
        let view = self.view(name)?;
        if events.len() > view.cost as usize {
            return None;
        }
        Some((view.eval)(events, now))
    }
}

// ── 协议名（载荷里带 `port`）────────────────────────────────────────────
//
// 名字住在 `event/mod.rs` 的事件名字表（`PROTOCOL_EVENTS`）。
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

// 协议五格的事件名与格位。名字常量仍住 `event/mod.rs`（协议族是内核语法的闭集，
// DE-024 D-203 裁决三把它留在语法里），格位在此与 `FactDecl` 并列声明——`seal`
// 因此能判「协议事实都有事件行」。
//
// 五条都落 `turn × asserted`：它们是「关于这一轮的断言」，不是开轮 / 收束。业务事件
// 名全部由 owner 声明；本表是内核唯一的名字→格位表。
const PROTOCOL_EVENTS: &[EventDecl] = &[
    EventDecl::new(EVENT_REQUEST, (Entity::Turn, Verb::Asserted)),
    EventDecl::new(EVENT_SENT, (Entity::Turn, Verb::Asserted)),
    EventDecl::new(EVENT_RESPONSE, (Entity::Turn, Verb::Asserted)),
    EventDecl::new(EVENT_FAILED, (Entity::Turn, Verb::Asserted)),
    EventDecl::new(EVENT_RETRY, (Entity::Turn, Verb::Asserted)),
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
