//! 义务环的行表：五条策略行 + 一条执行决策行 + 端口 / 授权级联行（/ ）。
//!
//! ## 一行只写「什么时候欠、什么算还」
//!
//! 判据全部是数据：`owes` / `until` / `unless` 三个模式，`prio` 是**全局**裁决序
//! （`(prio, name, key)` 升序）。执行体只拿到一笔 `Debt`，
//! 不再读任何请求作用域状态——旧 `LoopState` 的五个变体在这里对应五个视图读到的事实。
//!
//! ## 五条策略行的前后关系
//!
//! ```text
//! 0 authorize-tool─ 授权级联：一笔工具请求先判「放行 / 拒绝」
//! 10 classify      ── 轮首判决（本幕 round=0 且有发言）
//! 20 report        ── 中途汇报（一段静默到期，未用完配额）
//! 30 compress      ── 自动压缩（开关开着且本幕还没压过）
//! 40 recall        ── 记忆 / 技能 / 任务段（有持久事实源且本幕还没召回）
//! 50 reflex        ── 技能快路（非续写幕且本幕召回命中）
//! 55 close-turn ── 收束：本幕最近一次模型应答没有工具调用
//! 60 execute       ── 执行决策：本幕未收束且手上没有开着的工具批 ⇒ 跑一次模型往返
//! 65 dispatch-turn ── 端口派发：有 `request` ⇒ 引擎落 `sent` 并跑执行体
//! 66 dispatch-model─ 端口派发：模型往返（在 `turn` 执行体内部）
//! 67 open-tools ── 创建：模型响应的 `calls` 集合展开成 N 笔工具调用（D-117）
//! 68 dispatch-tools─ 端口派发：工具批次里已放行的单次调用
//! 69 deny-tools ── 拒绝收尾：被拒的那几笔按原因落 `failed`（b）
//! 70 await-turn    ── 等应答：`by = None`，唯一的「在等」声明位
//! 71 await-model   ── 等应答（模型端口）
//! 72 await-tools   ── 等应答（工具端口）
//! 80 lease         ── 租约看护：`sent` 超时无应答（一个行管所有端口）
//! ```
//!
//! 顺序进表而不是进调用点：同一点上多行竞争时，`drive` 取最小的一笔。旧表的
//! `prio` 只是格内序，这里第一次跨行可比（判据 10）。
//!
//! ## 一行**不在表里**的条件（写在这里，不写在事后）
//!
//! **汇报的轮次前提**：`轮数 > 0 ∧ 轮数 ≥ progress_min_rounds`。日志里没有
//! 「当前工具轮数」这一格（`chat.assistant.reported.tool_rounds` 只在说出口之后
//! 才有），所以它归触发点——`round_report_hook` 自己数批次边界，表判「静默够久
//! ∧ 配额未满」。`report_due` 视图把 `min_rounds` 一并交出，触发点因此也不用读配置。
//!
//! ## 查询口：事实是输入，`now` 也是输入
//!
//! [`owes`] / [`view`] 只收事实切片、轮号与 `now`——不读配置、不读时钟、不碰内存
//! 缓存（D-115 决策 1）。切片由 [`turn_slice`] 收窄到本轮：视图声明的扫描成本
//! 上界针对**一轮**，不是整份 WAL。
//!
//! ## 装载期检查
//!
//! 首次查询时经 [`load`] 把五张表灌进 `Builder` 的六个注册口并 `seal`，**任何缺口都是
//! 拒绝**（DE-015 §3.4 末句「装载失败即拒绝启动，不许部分成功」）：缺口逐条记 error 后
//! 装载返回 `None`，[`table`] 随之恒 `Err`。开幕（`chat_loop::drive::begin_drive`）先要
//! 表再干活，拿不到表就带着错误退出——不存在「半张表继续跑」的形态。

use std::sync::{Arc, LazyLock};

use serde_json::Value;

use crate::symbio_core::ring::{owed, Decl, Gap, Lit, Pattern, PortDecl};
use crate::symbio_core::{
    ActorDecl, Builder, Event, Frozen, LatencyTier, EVENT_ARTIFACT_ADDED, EVENT_ASSISTANT_FALLBACK,
    EVENT_ASSISTANT_FINAL, EVENT_ASSISTANT_OPENING, EVENT_ASSISTANT_PROGRESS,
    EVENT_ASSISTANT_REPORTED, EVENT_AUTHZ_DENIED, EVENT_AUTHZ_GRANTED, EVENT_FAILED, EVENT_REQUEST,
    EVENT_RESPONSE, EVENT_RETRY, EVENT_SENT, EVENT_SESSION_CONSOLIDATED, EVENT_TURN_DISPATCH,
    EVENT_TURN_STAGE, EVENT_TURN_SUPPLEMENTED,
};

/// 这张表里**接了线的执行体**：`execute`（整轮执行体）、`close`（收束）、
/// `opener` / `authorizer` / `courier`（工具批三笔债， 步 5b）、`recall`（轮首准备，
/// D-159 第一批——它住在另一台 host 的绑定表里，名字面上与其它没有区别）。
///
/// `by` 只有两种取值（DE-015 §2.2）：`Some(名字)` ⇒ 引擎跑它，`None` ⇒ 引擎什么都不做。
/// 「还没接线」不是第三种取值，也不是求解器的第四个过滤条件（D-118 订正 1）——
/// 未接线的行显式写 `None`，进表、可 diff，接线时把 `Some(名字)` 与
/// `turn_exec` 里那条 `ActorEntry` 同批改回。
/// 检查方在装载期：`by = Some(a)` 而 `a` 没有声明 ⇒
/// `Gap::UnmannedRow` ⇒ 装载红。
///
/// 声明本身（DE-015 §2.3）与 `turn_exec::ACTORS` 的
/// 绑定项一起构造，这里只放静态可查的四样。
///
/// `writes` 是这个执行体**直接落格**的领域事实（账本语义：单写者是 store，不是引擎）。
/// 引擎独占的四格（`sent` / `response` / `failed` / `retry`）不在任何执行体的
/// `writes` 里；`request` 例外——它是**决策者**写的（§2.3），工具批的决策者就是
/// `opener`，所以它出现在 [`ACTOR_OPENER`] 的名单里（运行期账本仍把它当协议格豁免，
/// 两份名单在这一点上不冲突：豁免的是「不必声明」，声明的是「确由我写」）。
const EXECUTE: &str = "execute";
/// 收束执行体：把寄存的那一次应答落成收束格。
const CLOSE: &str = "close";
/// 创建执行体（b）：模型交回的调用逐条落 `request(port=tool)`。
const OPENER: &str = "opener";
/// 授权执行体（b）：对已开账的工具调用落放行 / 拒绝事实。
const AUTHORIZER: &str = "authorizer";
/// 派发执行体（b）：放行的落 `sent` 交给执行层并关账，被拒的落 `failed`。
const COURIER: &str = "courier";
/// 轮首准备执行体（D-159 第一批）：`policy.recall` 行选中的动作。
///
/// 它是**第二台 host** 上的名字：动作借走的是本轮状态 + 会话上下文 + 请求句柄
/// （[`super::head_exec::HeadStep`]），不是轮内那张草稿。`by` 面上两台 host 没有区别——
/// 「这行归谁跑」照旧只写在行上，host 由绑定表回答（D-159 裁决 1）。
const RECALL: &str = "recall";

/// `execute` 执行体的声明（名单 = 运行期对账的**唯一依据**）。
///
/// 对外的用处不止 [`ACTORS`]：判据 2 / 判据 4 的**夹具绑定表**在同一个 `by` 上换动作时，
/// 照抄的就是这一份声明——名单一改，夹具与生产一起改，不会各拿一份名单。
pub(crate) const ACTOR_EXECUTE: ActorDecl = ActorDecl {
    name: EXECUTE,
    // 整轮执行体调模型：深度档。
    mode: LatencyTier::Deep,
    // 它不读视图：视图面由 `execute-turn` 行的 `owes` / `until` 承担（行已查存在）。
    reads: &[],
    // **收束格不在这里**（c）：`chat.assistant.final` / `chat.assistant.fallback`
    // 归 `close` 执行体。两个执行体写同一批格 ⇒ 依赖图成环（`close → execute` 与
    // `execute → close`），装载红（DE-015 §3.4 第 1 条）。
    writes: &[
        EVENT_ASSISTANT_OPENING,
        EVENT_TURN_STAGE,
        EVENT_TURN_DISPATCH,
        EVENT_ARTIFACT_ADDED,
        EVENT_SESSION_CONSOLIDATED,
        EVENT_AUTHZ_DENIED,
    ],
};

/// 创建执行体（b）：一个模型响应的 `calls` 逐条落 `request(port=tool)`。
///
/// `request` 是**决策者**的事实（§2.3），批次里做这个决定的就是它；`sent` 之后
/// 的协议格仍只由引擎写。装载期「出口全是引擎格时名单不许空」（D-124）在
/// `open-tools` 上不走那一档——它的出口里有可声明的 `request`，判交集。
const ACTOR_OPENER: ActorDecl = ActorDecl {
    name: OPENER,
    // 只写请求、不调模型、不跑工具：快速档。
    mode: LatencyTier::Fast,
    reads: &[],
    writes: &[EVENT_REQUEST],
};

/// 授权执行体（b）：风险档 vs 会话阈值的一次纯比较。
const ACTOR_AUTHORIZER: ActorDecl = ActorDecl {
    name: AUTHORIZER,
    mode: LatencyTier::Fast,
    reads: &[],
    writes: &[EVENT_AUTHZ_GRANTED, EVENT_AUTHZ_DENIED],
};

/// 派发执行体（b）：已放行的落 `sent` 并交给执行层；批尾把每一笔关掉
/// （应答 / 没跑到 / 被拒），落产物、等待格与轮内注入。
const ACTOR_COURIER: ActorDecl = ActorDecl {
    name: COURIER,
    // 派发本身不调模型（模型在执行层之外）。
    mode: LatencyTier::Fast,
    reads: &[],
    writes: &[
        EVENT_ARTIFACT_ADDED,
        EVENT_ASSISTANT_PROGRESS,
        EVENT_ASSISTANT_REPORTED,
        EVENT_TURN_SUPPLEMENTED,
    ],
};

/// 收束执行体（步 5c 起在**执行体之间**独占收束格）：它不调模型，只把寄存的那一次
/// 模型往返的**结局**落成收束格。
///
/// 「独占」说的是**执行体之间**：两个执行体写同一批格 ⇒ 依赖图成环（`close → execute`
/// 与 `execute → close`），表装不上（DE-015 §3.4 第 1 条）——所以收束格只能归一个执行体。
/// 它**不**覆盖轮首准备的 1b 分支（定稿答话轮）：那一轮一次模型往返都不发，收束格由
/// `turn_runner/prepare.rs` 就地落（缺口与归属见 ）。
///
/// 两条收束格都在它名下：`chat.assistant.final`（答完了 / 中止）与
/// `chat.assistant.fallback`（模型没答成 / 空答）。独占不是洁癖，是装载期的硬约束：
/// 两个执行体写同一批格 ⇒ 依赖图成环（`close → execute` 与 `execute → close`），
/// 表装不上（DE-015 §3.4 第 1 条）。`authz.denied` 是写侧闸拒那一条路留下的否定事实
/// （C4），它和收束格同一次动作里落，所以也在名单上。
const ACTOR_CLOSE: ActorDecl = ActorDecl {
    name: CLOSE,
    // 收束就是「把已经拿到手的应答入格」：反射档，不调模型。
    mode: LatencyTier::Reflex,
    reads: &[],
    writes: &[
        EVENT_ASSISTANT_FINAL,
        EVENT_ASSISTANT_FALLBACK,
        EVENT_AUTHZ_DENIED,
    ],
};

/// 轮首准备执行体的声明（D-159 第一批）。
///
/// 名单只有 `turn.stage` 一格：召回这一步的**出口**就是那一格（`stage:"recall"`
/// 带 `result`），装载期第 6 条判的正是「执行体走不走得到自己那一行的收口」。
/// 它的三份产物（记忆视图 / 调度段 / 技能命中）寄存在本轮状态里交编排层取——
/// 执行体不交回产出（D-123 决策 5 在轮首照用，D-159 裁决 4）。
const ACTOR_RECALL: ActorDecl = ActorDecl {
    name: RECALL,
    // 事实投影的纯读 + 一次技能路由判定，不调模型：与行同档（更慢就装载红）。
    mode: LatencyTier::Reflex,
    reads: &[],
    writes: &[EVENT_TURN_STAGE],
};

/// **执行体绑定表**（DE-015 §3.2 的 `actor` 口 + D-118 决策 2）：一条 = 一个 `by`。
///
/// 声明与动作在**同一条**里：装载期查得到的名字（[`actor_decls`] 是这张表的 `decl`
/// 投影，经 `Builder::actor` 入口冻结）与派发器跑得动的动作（[`next`] 查出的
/// [`Next::action`]）因此不可能一个有一个没有。接线一个执行体 = 在这里加一条 +
/// 把那一行的 `by` 改回 `Some(名字)`，同一个提交。
///
/// 动作住在 [`turn_exec`]（它要本轮草稿与七个端口），这张表只把它指到名字上。
///
/// `transitional` 是这条执行体**今天确实写得出、但 `decl.writes` 声明不了**的领域格。
/// 声明不了不是遗漏，是装载期会红：把收束格写进 `execute` 的名单，`close-turn` 的出口格
/// 就同时是两个执行体的出口格 ⇒
/// `Gap::DependencyCycle`（注①）。
/// 账本对账因此需要一个**有名有姓、会自己变红**的过渡豁免：它只在这条表项上生效，且由
/// `the_transitional_exemptions_are_pinned` 钉死内容——多一条就红，
/// 而不是将错就错地放宽名单。
///
/// 删除条件：收束格**全部**归 `close` 执行体之后。步 5c 已把中止轮与适配器失败两条
/// 挪进收束动作，剩下的是两条控制面失败（见 [`ACTOR_EXECUTE`] 那条表项的注释）——
/// 它们在生产里不可达，仍留豁免；真要让它们也走收束动作，得先给「模型往返压根没派发」
/// 与「工具批开不出来」各立一个日志事实（视图折不出这两条：折出来的判据分不开
/// 「开不出来」与「还没开」）。
///
/// 这条豁免的留与重开条件写在
/// [D-148](../../../../docs/003-design/015-义务环.md#d-148-transitional-豁免是结构事实不是兼容层留并把重开条件写成可观测的一条 "DE-015-义务环")：
/// 它不是兼容层，而是「声明面表达不了的一次写出」；能触发它退役的信号只有一个——端口记账层
/// 开始为「请求构造失败」落 `failed(port=model)`。
///
/// 这张表是**派发器的入参**（[`super::turn_exec::drive_steps`] 的 `actors`，D-123
/// 决策 4）：生产传 [`ACTORS`]，测试在同一张表的形状上注册别的东西。判据 14 那条
/// 「动作写了零格 ⇒ `exhausted{no_progress}`」的用例就靠这条接缝——没有它只能永远说
/// 那条分支「按构造不可达」。
pub(crate) struct ActorEntry {
    pub(crate) decl: ActorDecl,
    pub(crate) run: super::turn_exec::Action,
    pub(crate) transitional: &'static [&'static str],
}

pub(crate) static ACTORS: &[ActorEntry] = &[
    ActorEntry {
        decl: ACTOR_EXECUTE,
        run: super::turn_exec::run_execute,
        // 两条**控制面**失败就地落的兜底格：模型往返派不出去（行不欠 / 端口没声明）
        // 与工具批开不出来（模型要了工具而本轮没有分发通道）。这两条路在日志上
        // **没有**模型的终局事实（`failed(port=model)` 落不下来）⇒ 收束行开不出账
        // ⇒ 那两格只能由 `execute` 写。声明不了不是遗漏：写进 `decl.writes` 会让
        // `close-turn` 的出口格被两个执行体写 ⇒ 依赖图成环，装载红。
        //
        // 两条在生产里都不可达（`turn_exec` 的模型调用点恒带分发通道；行表装载失败
        // 时整幕开不起来），只有单测走得到——故留一条**有名有姓**的豁免，而不是
        // 把收束格的名额还给 `execute`。重开条件只有一条可观测的信号：端口记账层开始
        // 为「请求构造失败」落 `failed(port=model)` 时，兜底写与这条豁免一起删
        // （[D-148](../../../../docs/003-design/015-义务环.md#d-148-transitional-豁免是结构事实不是兼容层留并把重开条件写成可观测的一条 "DE-015-义务环")）。
        transitional: &[EVENT_ASSISTANT_FALLBACK],
    },
    ActorEntry {
        decl: ACTOR_CLOSE,
        run: super::turn_exec::run_close,
        transitional: &[],
    },
    ActorEntry {
        decl: ACTOR_OPENER,
        run: super::turn_exec::run_open,
        transitional: &[],
    },
    ActorEntry {
        decl: ACTOR_AUTHORIZER,
        run: super::turn_exec::run_authorize,
        transitional: &[],
    },
    ActorEntry {
        decl: ACTOR_COURIER,
        run: super::turn_exec::run_dispatch,
        transitional: &[],
    },
];

/// **轮首 host** 的执行体绑定表（D-159）：形状与 [`ACTORS`] 逐字相同，差别只在动作
/// 借走的那片状态（[`super::head_exec::HeadStep`] = 本轮状态 + 会话上下文 + 请求句柄，
/// 不是轮内那张草稿）。
///
/// 一台 host 一张表，**行不复制**：这张表的名字集合就是「轮首派发点该看哪些行」，
/// 行子集由它派生（[`host_rows`]）——`Decl` 上不加 `host` 字段，否则「谁跑它」与
/// 「它归谁」有两处 owner，可以互相矛盾而装载照绿（D-159 裁决 1）。
///
/// 这张表同样按入参交给派发点（D-123 决策 4 的形状）：夹具要在同名 `by` 上换手法时
/// 传另一张表，不必改生产。
pub(crate) struct HeadEntry {
    pub(crate) decl: ActorDecl,
    pub(crate) run: super::head_exec::HeadAction,
    pub(crate) transitional: &'static [&'static str],
}

pub(crate) static HEAD_ACTORS: &[HeadEntry] = &[HeadEntry {
    decl: ACTOR_RECALL,
    run: super::head_exec::run_recall,
    transitional: &[],
}];

/// `by` → **轮首**绑定表里的那一条（[`entry_of`] 的同一条纪律：声明与动作一次查全）。
fn head_entry_of(by: &'static str, actors: &'static [HeadEntry]) -> Option<&'static HeadEntry> {
    actors.iter().find(|e| e.decl.name == by)
}

/// 一台 host 的**行子集**：`by` 落在它绑定表里的那些行（D-159 裁决 2）。
///
/// 为什么不向 `ring::drive` 传整张行表：`prio` 是**全局**序，整张表会让别的 host 的
/// 低 `prio` 债被本派发点选中——而本派发点的绑定表里没有那一条，`next` 只能报
/// 「表脱节」并停住，别的 host 排在后面的债永远轮不到。那是两台 host 互相卡死，
/// 不是双轨。子集派生自绑定表 ⇒ 「这行归我跑」只有一个读法，`ROWS` 本身一个字不动。
///
/// 未接线的行（`by: None`）不在任何一台 host 的子集里：它照旧由外层代码直问
/// `owes`（调度旁路，归零对象），而 [`owes`] 读的是**整张**冻结表，两者互不影响。
fn host_rows(frozen: &Frozen, names: &[&'static str]) -> Vec<Decl> {
    frozen
        .rows()
        .iter()
        .filter(|d| d.by.is_some_and(|by| names.contains(&by)))
        .cloned()
        .collect()
}

/// 绑定表的名字集合（[`host_rows`] 的入参）。
fn actor_names(actors: &[ActorEntry]) -> Vec<&'static str> {
    actors.iter().map(|e| e.decl.name).collect()
}

/// 轮首绑定表的名字集合。
fn head_actor_names(actors: &[HeadEntry]) -> Vec<&'static str> {
    actors.iter().map(|e| e.decl.name).collect()
}

/// 交给装载期冻结的执行体声明（`loop_rows::assemble` 的唯一用处）。
///
/// **两台 host 并成一集**（D-159 裁决 3）：装载面仍然是一张表——七道检查判的是
/// 「名字有没有人认领、认领的人走不走得到出口」，与执行体借走哪片可变状态无关。
/// 分开注册会冻出两份 `Frozen`，行表就有了两个真源。
pub(crate) fn actor_decls() -> Vec<ActorDecl> {
    ACTORS
        .iter()
        .map(|e| e.decl)
        .chain(HEAD_ACTORS.iter().map(|e| e.decl))
        .collect()
}

/// `by` → 绑定表里的那一条。`None` = 表里没有这个名字（装载期的 `UnmannedRow` 已拦，
/// 这里是派发面把「表被改坏」显形，不猜动作、也不猜它写了什么）。
///
/// 动作与声明**必须一次查全**：派发器读账本时要豁免的协议五格、以及
/// 「这个执行体声明过哪些领域格」，都来自同一条 `ActorEntry`。分两次查就是
/// 给「声明与动作漂移」留缝。
///
/// 查的是**传进来的那张表**（D-123 决策 4）：`ACTORS` 是生产表，接缝让夹具能在
/// 同名 `by` 上注册别的动作。
fn entry_of(by: &'static str, actors: &'static [ActorEntry]) -> Option<&'static ActorEntry> {
    actors.iter().find(|e| e.decl.name == by)
}

/// 协议五格：运行期写入封闭的**豁免名单**。
///
/// 它们由引擎写（`port.rs` 那一个文件，PTS-005 钉着），因此**不该**出现在任何
/// `ActorDecl::writes` 里——装载期 [`crate::symbio_core::decl`] 的
/// `ENGINE_ONLY_CELLS` 检查拦的就是「执行体声明了引擎独占的格」。运行期读账本时
/// 它们当然在：粗端口期 `port::dispatch` 就在执行体内部跑（DE-015 §7.2 承认的过渡
/// 形状），端口行接回派发器之后写点位置变、豁免名单不变。
const PROTOCOL_CELLS: &[&str] = &[
    EVENT_REQUEST,
    EVENT_SENT,
    EVENT_RESPONSE,
    EVENT_FAILED,
    EVENT_RETRY,
];

/// [`Next`] 上的运行期对账口（DE-015 §3.4 第 5 条的**运行期那一半**，D-123 决策 2）。
///
/// 装载期只能查声明面（名字存在、不与引擎独占的格相撞），「执行体实际只写它声明过的
/// 领域格」只能拿**实际写入**对账——而实际写入的唯一可信取法是日志尾段（决策 1）。
impl<A> Next<A> {
    /// 账本上的这一格收不收得拢：每一格都要**在这张表里对得上一个名字**——协议五格
    /// （引擎写）、`decl.writes`（执行体声明）、`transitional`（过渡豁免）。
    /// 对不上就是越界，派发器不静默。
    pub(crate) fn writes_closed(&self, kind: &str) -> bool {
        PROTOCOL_CELLS.contains(&kind)
            || self.writes.contains(&kind)
            || self.tolerated.contains(&kind)
    }
}

/// 端口声明——**加一个端口只加这一条**，不加事实名、不改求解器。
///
/// 当前三个端口：`turn`（一轮的一次请求往返）、`model`（一次模型往返， 步 2）、
/// `tool`（批次里的单次工具调用， 步 3）。
///
/// ## 租约与尝试上限怎么定的
///
/// `lease_ms` 是「派出去多久没应答算卡住」：`turn` 是一次模型往返，且含工具轮，
/// 慢的那一头可能是模型排队——太短会把正在进行的一轮误判成卡住（重试会再调一次
/// 模型，代价真实）。取 10 分钟：既比最慢的可接受往返长，又比「用户已经以为坏了」短。
///
/// `max_attempts = 1`——**`turn` 调用不重试**。理由不是成本，是**没人 await**：
/// 租约到期只发生在进程重启之后（时间只在事件上推进），而等这笔应答的 future
/// 已经随进程一起没了。重试只会留下一条**永不派发**的 `request`，让「欠一次
/// 派发」变成一句谎。到顶即判死（`failed{why:"lease_expired"}`），恢复走
/// resume 幕（新的逻辑 id，旧那笔作为「没回来」留在日志里）。
///
/// `retry` 因此只给**有活着的等待方**的尝试用（D-138）。上限是端口级的天花板，
/// 逐次尝试是否真能重试由 `sent.awaited` 决定（见 [`super::port::reap`]）。
///
/// ## `model` 端口
///
/// 一次**模型往返**。它在 `turn` 执行体**内部**发起（D-115 决策 5：两者之间
/// 没有双调度器——`turn` 是普通 `by=Some` 执行体，DE-015 只是把它的内部拆成
/// 更多行）。
///
/// 租约与 `turn` 同量级（慢的那一头同样是模型排队）；`max_attempts = 1` 同理：
/// 进程重启后等这笔应答的 future 已经不在了，重试只会留下一条永不派发的
/// `request`。等步 4 的唤醒事实接上（有活着的等待方）时，这个上限才可以 > 1。
pub(crate) static PORTS: &[PortDecl] = &[
    PortDecl {
        port: "turn",
        // 引擎在跑执行体**之前**落它（判据 2）：搬运者可能不产事实就返回。
        before: EVENT_SENT,
        lease_ms: 600_000,
        max_attempts: 1,
        // 一轮只有一个 `turn` 调用，没有同批子债。
        concurrent: false,
    },
    PortDecl {
        port: "model",
        before: EVENT_SENT,
        lease_ms: 600_000,
        max_attempts: 1,
        // 一次模型往返是一条调用；同批并发在它下面的工具端口。
        concurrent: false,
    },
    // 工具批次里的**单次调用**。一次模型响应可带 N 个调用 ⇒
    // 同一批写 N 条 `request`，各自一个 id。
    //
    // ## 为什么上限是 2，而普通批次仍然不重试
    //
    // 上限是端口级的天花板，**不是**「每个调用都会重试」的承诺（D-138）。工具端口
    // 里有两类尝试：
    //
    // - 普通批次调用：结果等在当前草稿里，进程重启后等待方随草稿消失 ⇒
    //   `sent.awaited = false` ⇒ 到顶/到期即判死，恢复走 `resume` 幕（新 id）；
    // - 唤醒调用：`wake_batch` 能从事实重建那一笔（`resume` / `tool` / `args` 都在
    //   `request` 载荷里）⇒ `awaited = true` ⇒ 租约看护可以重试一次。
    //
    // 上限 2 = 首次 + 一次重试；再抬就需要新的等待方形态，而不是改数字。
    PortDecl {
        port: "tool",
        before: EVENT_SENT,
        lease_ms: 600_000,
        max_attempts: 2,
        // 一次模型响应可带 N 个工具调用：同批子债允许并发，
        // 具体哪几条同时在途仍由风险档决定（`dispatch/parallel.rs`）。
        concurrent: true,
    },
];

/// 内建行表。注册顺序**不参与**裁决——同 `prio` 由 `(name, key)` 决出。
pub(crate) static ROWS: &[Decl] = &[
    // ── 授权级联（⇒ 步 3）────────────────────────────────
    //
    // 两行，不是一行：
    //
    //   authorize-tool   owes: request(port=tool)      until: granted | denied
    //   dispatch-tools   owes: granted(port=tool)      until: sent | failed
    //
    // ## 为什么派发行 `owes` 写 `granted` 而不是 `request ∧ granted`
    //
    // `Decl.owes` 是**一个** `Pattern`（单源等值），跨源合取写不出来；要写就得
    // 改求解器，而求解器的字节是 PTS-003 的判据面（改它必须挂 ADR）。这里用
    // **蕴含**代替合取：`granted` 只可能由 `authorize-tool` 从一笔 `request`
    // 派生，所以 `granted ⇒ request` 在数据上成立；`dispatch-tools` 只欠
    // `granted`，与 `request ∧ granted` 的可达债集合相同。
    //
    // 这不是近似：`denied` ⇒ 没有 `granted` ⇒ 派发行**永不**成立（fail-closed），
    // 而「还没判」⇒ `authorize-tool` 仍开着、`dispatch-tools` 不欠——两种状态在
    // 匹配上可分，正是判据 8b 要的那条区分。全程没有否定原子。
    //
    // ## 为什么两行的 `key` 都是 `(port, id, attempt)`
    //
    // `authorize-tool` 的出口是 `authz.granted` / `authz.denied`，`dispatch-tools`
    // 的出口是 `sent` / `failed`：它们必须与协议事实关同一**次尝试**（D-138）。
    // 逻辑键会让上一 attempt 的出口把行提前关掉，重试出来的 `request{attempt+1}`
    // 就再也欠不动授权与派发。`authz.denied` 的**收束格拒绝**那条载荷只有 `key`
    // （目标格名），没有 `port` / `id` / `attempt`——按三元组绑定自然取不到实例，
    // 不会误关一笔工具授权。
    Decl {
        name: "authorize-tool",
        // 的口径：判在派发之前。0 是这一环里最早的一档。
        prio: 0,
        // 判定是纯比较（风险档 vs 会话阈值），不调模型（b 接线）。
        by: Some(AUTHORIZER),
        tier: LatencyTier::Fast,
        expand: None,
        owes: Pattern {
            source: EVENT_REQUEST,
            key: &["port", "id", "attempt"],
            eq: &[("port", Lit::Str("tool"))],
        },
        until: &[
            Pattern {
                source: EVENT_AUTHZ_GRANTED,
                key: &["port", "id", "attempt"],
                eq: &[("port", Lit::Str("tool"))],
            },
            Pattern {
                source: EVENT_AUTHZ_DENIED,
                key: &["port", "id", "attempt"],
                eq: &[("port", Lit::Str("tool"))],
            },
        ],
        unless: &[],
    },
    // ── 轮首判决 ─────────────────────────────────────────────────────────
    Decl {
        name: "policy.classify",
        prio: 10,
        // 未接线：判决由 `chat_loop` 按 `owes` 点问自己执行；与 `decide` 执行体同批接回。
        by: None,
        // 判决可能落到模型，但不生成：快速档（与旧行 `effects.tier` 同值）。
        tier: LatencyTier::Fast,
        expand: None,
        owes: Pattern {
            source: "turn_head",
            key: &["turn", "round"],
            eq: &[
                ("round", Lit::Uint(0)),
                ("has_utterance", Lit::Bool(true)),
                ("classify_enabled", Lit::Bool(true)),
                ("classify_done", Lit::Bool(false)),
            ],
        },
        until: &[Pattern {
            source: "turn_stages",
            key: &["turn", "round"],
            eq: &[("stage", Lit::Str("decide"))],
        }],
        unless: &[],
    },
    // ── 中途汇报 ─────────────────────────────────────────────────────────
    //
    // 一段静默 = 一个 `(turn, anchor)`。汇报事实落格后静默锚前移，下一段静默
    // 是另一笔账——配额之内可以汇报多次，逐段关账而不是「一说就全关」。
    Decl {
        name: "policy.report",
        prio: 20,
        // 未接线：触发点在 `round_report_hook`（它自己数批次边界）；与 `report` 执行体同批接回。
        by: None,
        // 模板产线，零模型往返。
        tier: LatencyTier::Reflex,
        expand: None,
        owes: Pattern {
            source: "report_due",
            key: &["turn", "anchor"],
            eq: &[("due", Lit::Bool(true))],
        },
        until: &[Pattern {
            source: "reported_anchors",
            key: &["turn", "anchor"],
            eq: &[],
        }],
        unless: &[],
    },
    // ── 自动语义压缩 ─────────────────────────────────────────────────────
    Decl {
        name: "policy.compress",
        prio: 30,
        // 未接线：压缩在 `assemble` 段按 `owes` 点问执行；与 `compress` 执行体同批接回。
        by: None,
        // 语义压缩调模型：深度档。
        tier: LatencyTier::Deep,
        expand: None,
        owes: Pattern {
            source: "turn_head",
            key: &["turn", "round"],
            eq: &[
                ("auto_compress", Lit::Bool(true)),
                ("compress_done", Lit::Bool(false)),
            ],
        },
        until: &[Pattern {
            source: "turn_stages",
            key: &["turn", "round"],
            eq: &[("stage", Lit::Str("compress"))],
        }],
        unless: &[],
    },
    // ── 轮首准备（记忆 / 技能 / 任务段）─────────────────────────────────
    Decl {
        name: "policy.recall",
        prio: 40,
        // 轮首 host 的执行体（D-159 第一批）：派发点在 `chat_loop::inputs` 的轮首锚，
        // 出口那一格由执行体自己落——它不交回产出，但「已做」这件事照旧只写在日志上。
        by: Some(RECALL),
        // 事实投影的纯读 + 技能路由，不调模型。
        tier: LatencyTier::Reflex,
        expand: None,
        owes: Pattern {
            source: "turn_head",
            key: &["turn", "round"],
            eq: &[
                ("round", Lit::Uint(0)),
                ("is_persistent", Lit::Bool(true)),
                ("recall_done", Lit::Bool(false)),
            ],
        },
        until: &[Pattern {
            source: "turn_stages",
            key: &["turn", "round"],
            eq: &[("stage", Lit::Str("recall"))],
        }],
        unless: &[],
    },
    // ── 反射档（技能快路）────────────────────────────────────────────────
    //
    // 续写幕不走快路（落格形状会撞幂等键）；`skill_hit` 来自本幕召回阶段的
    // `result`，所以「命中」这件事第一次有了事实形态。
    Decl {
        name: "policy.reflex",
        prio: 50,
        // 未接线：快路在装配段命中即收束（它不调模型）；与 `reflex` 执行体同批接回。
        by: None,
        // 快路就是「不调模型」：反射档是它的定义。
        tier: LatencyTier::Reflex,
        expand: None,
        owes: Pattern {
            source: "turn_head",
            key: &["turn"],
            eq: &[
                ("resumed", Lit::Bool(false)),
                ("skill_hit", Lit::Bool(true)),
            ],
        },
        until: &[Pattern {
            source: "turn_closed",
            key: &["turn"],
            eq: &[],
        }],
        unless: &[],
    },
    // ── 收束（/ 步 5c）────────────────────────────────────────
    //
    // 「这一轮该收束了、还没入格」是一笔独立的账：前件是日志上的 `turn.closeable`
    // （`has_final`〔本幕最近一条 `response(port=model)` 的 `calls` 为空 ∧ 本幕未收束，
    // D-121 决策 2〕∨「本幕派发过的模型往返以失败收场」），执行体是 [`CLOSE`]——
    // 方案（DE-015 §3.1）要的就是一个独立的 `closer`，不是整轮执行体里的第四个内部动作。
    // 它排在 `execute-turn` **之前**（55 < 60）：应答一落地，下一个该还的债就是入格，
    // 而不是再跑一次模型往返——两行在同一切片上竞争，`drive` 取最小的一笔。
    //
    // ## 出口是 `final | fallback`（DE-015 §3.1 的字面形状，步 5c 复原）
    //
    // 步 5 第一批照抄 §3.1 装不上表：`execute-turn` 与 `close-turn` 共用同一个出口
    // 事实集时，两个执行体互为对方的出口写方（`execute` 写错误路径的 `fallback`，
    // `close` 写 `final`），依赖图于是有 `close → execute` 与 `execute → close` 两条边
    // ——两节点环，装载红。环本身是**假阳性**（收束一旦入格两行同时关门），但第 1 条是
    // 字节判据面，改它要挂 ADR。
    //
    // 步 5c 断环的方式是**让两个执行体写的格不相交**，不是改判据：收束格
    // （`final` 与 `fallback`）全部归 `close` 执行体——`execute` 那一步只跑模型往返，
    // 收束格由 `close-turn` 行的执行体落（中止 / 适配器失败两条路径的收束格因此
    // 从执行体内部挪到了收束动作里）。断环之后 `close-turn` 没有出边（只有 `close`
    // 写它的出口，而自环不算），`execute-turn → close-turn` 单独一条边，表装得上。
    //
    // 两条**控制面**失败（模型往返派不出去 / 工具批开不出来）仍在 `execute` 那一步
    // 就地落兜底格：它们的模型端口**没有终局事实**（`failed(port=model)` 落不下来）
    // ⇒ 收束行开不出账 ⇒ 那两格由 `execute` 写，并走它的**过渡豁免**（见
    // [`ACTOR_EXECUTE`] 的 `transitional`）。
    Decl {
        name: "close-turn",
        prio: 55,
        by: Some(CLOSE),
        // 入格不调模型：反射档（比执行体的 `mode` 更慢就装载红，第 6 条的 mode 闸门）。
        tier: LatencyTier::Reflex,
        expand: None,
        owes: Pattern {
            source: "turn_head",
            key: &["turn"],
            eq: &[("closeable", Lit::Bool(true))],
        },
        until: &[
            Pattern {
                source: EVENT_ASSISTANT_FINAL,
                key: &["turn"],
                eq: &[],
            },
            Pattern {
                source: EVENT_ASSISTANT_FALLBACK,
                key: &["turn"],
                eq: &[],
            },
        ],
        unless: &[],
    },
    // ── 执行（决策行）────────────────────────────────────────────────────
    //
    // 只要本幕还开着、且手上没有一批没关掉的工具调用，就欠一次执行。它的**后果**是
    // 写一条 `request`（决策者的动作），派发本身归 `dispatch-turn`。没把它并进协议行：
    // 两者的粒度不同（本幕 vs 一次请求往返）——一幕可以有多次往返（auto 续写），
    // 而「本幕还欠不欠」只有一个答案。
    //
    // `tools_open = false`（b）：模型交回了工具调用、其中还有没关账的，
    // 这一步就让位——否则它每次都比创建 / 授权 / 派发（67 / 68 / 69）先生效，
    // 工具批永远等不到人跑（下一轮模型往返会带着没有结果的交换再次出发）。
    Decl {
        name: "execute-turn",
        prio: 60,
        by: Some(EXECUTE),
        tier: LatencyTier::Deep,
        expand: None,
        owes: Pattern {
            source: "turn_head",
            key: &["turn"],
            eq: &[("tools_open", Lit::Bool(false))],
        },
        until: &[Pattern {
            source: "turn_closed",
            key: &["turn"],
            eq: &[],
        }],
        unless: &[],
    },
    // ── 端口 `turn` 的两行主模板───────────────────────────
    //
    // 每端口固定两行：`dispatch-<port>` 与 `await-<port>`，差异只有 `by`。
    // 第三行（租约）**每端口不写**——一个 `lease` 行管所有端口，它按
    // `stale_sent` 的实例而不是按端口开账。
    //
    // 键取 **`(port, id, attempt)`**（D-138）：重试写出的 `request{attempt+1}`
    // 必须重新打开派发行；逻辑键会让上一 attempt 的 `sent` 把它提前关掉。
    Decl {
        name: "dispatch-turn",
        prio: 65,
        // 未接线：`turn` 端口的往返就在整轮执行体里跑，`courier` 执行体随细端口接回。
        by: None,
        // 派发本身不调模型（模型在执行体里）。
        tier: LatencyTier::Fast,
        expand: None,
        owes: Pattern {
            source: EVENT_REQUEST,
            key: &["port", "id", "attempt"],
            eq: &[("port", Lit::Str("turn"))],
        },
        until: &[
            Pattern {
                source: EVENT_SENT,
                key: &["port", "id", "attempt"],
                eq: &[("port", Lit::Str("turn"))],
            },
            Pattern {
                source: EVENT_FAILED,
                key: &["port", "id", "attempt"],
                eq: &[("port", Lit::Str("turn"))],
            },
        ],
        unless: &[],
    },
    // ── 端口 `model` 的两行──────────────────────────
    //
    // 与 `turn` 的两行**同模板**，只差端口取值：模型调用是轮内的一次往返，它在
    // `turn` 执行体内部发起（D-115 决策 5：没有第二个调度器）。
    //
    // 键同样取 `(port, id, attempt)`（D-138）：逻辑 id 在一次调用的各次尝试间不变，
    // 尝试号随重试 +1；按逻辑键开账会让上一 attempt 的 `sent` 把重试后的请求提前关掉。
    //
    // ## 端口粒度为什么是「一次模型往返」而不是「一次工具批次」
    //
    // 模型调用是**最内层**的单次对外调用（它下面没有别的调用了），因此它是最小
    // 的可恢复单位：崩溃后重放到「上一次模型请求」即可，不必重放整轮。工具批次
    // 在它**外面**（一个模型响应可带多个工具调用），所以工具端口的 `expand` 是
    // 下一层的事（DE-015 步 3）。
    Decl {
        name: "dispatch-model",
        prio: 66,
        // 未接线：引擎面在 `port::round_trip`（落 `sent` + 跑一次往返），调用点还在执行体内。
        by: None,
        // 派发本身不调模型（模型在执行体里）。
        tier: LatencyTier::Fast,
        expand: None,
        owes: Pattern {
            source: EVENT_REQUEST,
            key: &["port", "id", "attempt"],
            eq: &[("port", Lit::Str("model"))],
        },
        until: &[
            Pattern {
                source: EVENT_SENT,
                key: &["port", "id", "attempt"],
                eq: &[("port", Lit::Str("model"))],
            },
            Pattern {
                source: EVENT_FAILED,
                key: &["port", "id", "attempt"],
                eq: &[("port", Lit::Str("model"))],
            },
        ],
        unless: &[],
    },
    // 工具调用的**创建**（D-117）：一个模型响应的 `calls` 集合逐元素开债。
    //
    // 与派发行分开：创建行只回答「这次响应里的调用有没有落成 request」，派发行
    // 才回答「已放行的那一笔有没有 sent」。把两者合成一行会让授权拒绝失去
    // 作用点——被拒绝的调用同样有模型响应，派发会绕过 `authz.denied`。
    //
    // `expand` 只展开 `owes`；`until` 按键命中。元素键 `id` 由模型响应携带
    // （`tool-{turn}-d{drive}-{模型调用 id}`），与随后的 `request(port=tool)` 同 id。
    Decl {
        name: "open-tools",
        prio: 67,
        // 创建动作（b）：逐条落 `request(port=tool)`，不调模型、不跑工具。
        by: Some(OPENER),
        tier: LatencyTier::Fast,
        expand: Some(("calls", "id")),
        owes: Pattern {
            source: EVENT_RESPONSE,
            key: &["port", "id"],
            eq: &[("port", Lit::Str("model"))],
        },
        until: &[
            Pattern {
                source: EVENT_REQUEST,
                key: &["port", "id"],
                eq: &[("port", Lit::Str("tool"))],
            },
            Pattern {
                source: EVENT_FAILED,
                key: &["port", "id"],
                eq: &[("port", Lit::Str("tool"))],
            },
        ],
        unless: &[],
    },
    // 授权级联的第二行（第一行 `authorize-tool` 在表首）：只对已放行的那笔
    // 落 `sent`。`granted` 已蕴含 `request`，所以这里不再复写请求前件。
    Decl {
        name: "dispatch-tools",
        prio: 68,
        // 派发动作（b）：放行的落 `sent` 并交执行层，批尾把每一笔关掉。
        by: Some(COURIER),
        // 派发本身不调工具（执行在批次里）。
        tier: LatencyTier::Fast,
        expand: None,
        owes: Pattern {
            source: EVENT_AUTHZ_GRANTED,
            key: &["port", "id", "attempt"],
            eq: &[("port", Lit::Str("tool"))],
        },
        until: &[
            Pattern {
                source: EVENT_SENT,
                key: &["port", "id", "attempt"],
                eq: &[("port", Lit::Str("tool"))],
            },
            Pattern {
                source: EVENT_FAILED,
                key: &["port", "id", "attempt"],
                eq: &[("port", Lit::Str("tool"))],
            },
        ],
        unless: &[],
    },
    // 被拒的调用也要有人收尾（b）：`authz.denied` 关掉了授权行，但
    // `dispatch-tools` 只欠 `granted`——一批**全被拒**（或全是非法调用）时它不欠，
    // 执行层就永远跑不到，`failed{why}` 也永远不落。这一行把「拒绝之后还必须做一次
    // 收尾」说成债务：出口与派发行同一格，动作也是同一个派发执行体（混合批次里
    // 那一笔由 68 顺带关掉，本行随后自然不欠）。
    //
    // 键取 `(port, id, attempt)` 与 `authz.denied` 同形（D-138）；不带 `port`
    // 的收束格拒绝（`deny()` 写的那一条）按 eq 取不到实例，不会误开一笔工具债。
    Decl {
        name: "deny-tools",
        prio: 69,
        by: Some(COURIER),
        tier: LatencyTier::Fast,
        expand: None,
        owes: Pattern {
            source: EVENT_AUTHZ_DENIED,
            key: &["port", "id", "attempt"],
            eq: &[("port", Lit::Str("tool"))],
        },
        until: &[Pattern {
            source: EVENT_FAILED,
            key: &["port", "id", "attempt"],
            eq: &[("port", Lit::Str("tool"))],
        }],
        unless: &[],
    },
    // 「已派出、等应答」——`by: None` 就是 `Wait` 的折叠：对引擎透明（恒 blocked、
    // 永不选中），但它让「在等」与「没事」可区分（DE-015 §2.3）。
    //
    // 键取 `(port, id, attempt)`（D-138）：`sent` / `response` / `failed` / `retry`
    // 都是同一次尝试的事实；按逻辑键开账时，上一 attempt 的应答会把这笔重试后的
    // 等待提前关掉。
    Decl {
        name: "await-turn",
        prio: 70,
        by: None,
        tier: LatencyTier::Deep,
        expand: None,
        owes: Pattern {
            source: EVENT_SENT,
            key: &["port", "id", "attempt"],
            eq: &[("port", Lit::Str("turn"))],
        },
        until: &[
            Pattern {
                source: EVENT_RESPONSE,
                key: &["port", "id", "attempt"],
                eq: &[("port", Lit::Str("turn"))],
            },
            Pattern {
                source: EVENT_FAILED,
                key: &["port", "id", "attempt"],
                eq: &[("port", Lit::Str("turn"))],
            },
            Pattern {
                source: EVENT_RETRY,
                key: &["port", "id", "attempt"],
                eq: &[("port", Lit::Str("turn"))],
            },
        ],
        unless: &[],
    },
    // 「已派出、等应答」——与 `await-turn` 同形：`by: None` 就是 `Wait` 的折叠。
    Decl {
        name: "await-model",
        prio: 71,
        by: None,
        tier: LatencyTier::Deep,
        expand: None,
        owes: Pattern {
            source: EVENT_SENT,
            key: &["port", "id", "attempt"],
            eq: &[("port", Lit::Str("model"))],
        },
        until: &[
            Pattern {
                source: EVENT_RESPONSE,
                key: &["port", "id", "attempt"],
                eq: &[("port", Lit::Str("model"))],
            },
            Pattern {
                source: EVENT_FAILED,
                key: &["port", "id", "attempt"],
                eq: &[("port", Lit::Str("model"))],
            },
            Pattern {
                source: EVENT_RETRY,
                key: &["port", "id", "attempt"],
                eq: &[("port", Lit::Str("model"))],
            },
        ],
        unless: &[],
    },
    // 一批 N 笔，一笔一条 `sent`；`await-tools` 与其它端口的 await 同形。
    //
    // 键取 `(port, id, attempt)`：重试后新 attempt 的 `sent` 要重新开一笔等待，
    // 而不是被旧 attempt 的应答关掉（D-138）。
    Decl {
        name: "await-tools",
        prio: 72,
        by: None,
        tier: LatencyTier::Deep,
        expand: None,
        owes: Pattern {
            source: EVENT_SENT,
            key: &["port", "id", "attempt"],
            eq: &[("port", Lit::Str("tool"))],
        },
        until: &[
            Pattern {
                source: EVENT_RESPONSE,
                key: &["port", "id", "attempt"],
                eq: &[("port", Lit::Str("tool"))],
            },
            Pattern {
                source: EVENT_FAILED,
                key: &["port", "id", "attempt"],
                eq: &[("port", Lit::Str("tool"))],
            },
            Pattern {
                source: EVENT_RETRY,
                key: &["port", "id", "attempt"],
                eq: &[("port", Lit::Str("tool"))],
            },
        ],
        unless: &[],
    },
    // ── 租约（**每端口不写**，一个行管所有端口）───────────────────
    //
    // 开账条件是 `stale_sent.stale`（视图把 `now` 与 `lease_ms` 折成布尔），
    // 所以时间不进欠账公式。
    //
    // 键取 `(port, id, attempt)`：一笔调用可能卡在重试链的某一环上，
    // 看护者要能分别说出「第几次尝试卡住了」（D-138）；逻辑键会让旧尝试的
    // `retry` 把新尝试的卡住一并关掉。
    //
    // ## 为什么没有 `unless: attempt ≥ max_attempts`
    //
    // DE-015 §2.3 的草稿把「尝试到顶」写成 `unless`——那会让行在到顶时挂起，
    // 而「判死」（落 `failed`）恰恰要在到顶时发生。到顶是**看护者的判断**，不是
    // 行的出口：行只负责「这笔卡住了，有人要管」，重试还是判死归执行体。
    Decl {
        name: "lease",
        prio: 80,
        // 未接线：看护者在唤醒路径上自己扫 `stale_sent`，不经派发器选债；与
        // `lease_keeper` 执行体同批接回。
        by: None,
        // 看护本身是记账，不调模型。
        tier: LatencyTier::Fast,
        expand: None,
        owes: Pattern {
            source: "stale_sent",
            key: &["port", "id", "attempt"],
            eq: &[("stale", Lit::Bool(true))],
        },
        until: &[
            Pattern {
                source: EVENT_RESPONSE,
                key: &["port", "id", "attempt"],
                eq: &[],
            },
            Pattern {
                source: EVENT_FAILED,
                key: &["port", "id", "attempt"],
                eq: &[],
            },
            Pattern {
                source: EVENT_RETRY,
                key: &["port", "id", "attempt"],
                eq: &[],
            },
        ],
        unless: &[],
    },
];

/// 装载：五张注册表经 DE-015 §3.2 的**六个口**进 `Builder`，`seal` 跑完 §3.4 的七道。
///
/// 行切片与端口切片从这里进 `row` / `port` 两个入口——检查跟着表走，不由调用方
/// 自己想起来再跑一遍：`seal` 是装载的唯一出口。
///
/// **任何缺口都是装载失败**——`seal` 一次报全的每一格（声明侧重名 / 撞名 / 视图引用
/// 未声明的事实或字段 / 执行体声明了引擎独占的格，行侧无出口 / 键不一致 / 未声明源 /
/// 未声明字段 / 展开键不匹配 / 孤儿事实名 / 无人认领的 `by` / 收不了口的执行体 /
/// 依赖成环 / 端口没有应答出口）都在同一个 `Vec<Gap>` 里，不降级成告警：一个没人
/// 消费的事实名就是判据 7 拦的那条「拼错的名字静默永不被消费」，它在装载期就该挡住启动。
///
/// 执行体声明与绑定项一起交（[`actor_decls`] 就是绑定表的 `decl` 投影），所以装载期
/// 能看到的名字与派发器跑得动的名字**同源**。
pub(crate) fn load(rows: &[Decl]) -> Result<Arc<Frozen>, Vec<Gap>> {
    let mut b = Builder::builtin();
    for decl in actor_decls() {
        b.actor(decl);
    }
    for &p in PORTS {
        b.port(p);
    }
    for r in rows {
        b.row(r.clone());
    }
    b.seal()
}

/// 真表装载，并把 §3.4 第 4 条的报告打出来。
///
/// 第 4 条是**报告**不是缺口：静态只能判「可能同时命中」，谁先派发归 `drive` 的
/// `(prio, name, key)`。名单随表冻结（[`Frozen::overlaps`]），这里只负责让人看见。
fn assemble() -> Result<Arc<Frozen>, Vec<Gap>> {
    let frozen = load(ROWS)?;
    let overlapping = frozen.overlaps();
    if !overlapping.is_empty() {
        crate::plugin_info!(
            "session",
            "[Loop] 相交行对 {} 组（可能同时开账，裁决归 prio）：{}",
            overlapping.len(),
            overlapping
                .iter()
                .map(|o| format!("{}×{}@{}", o.rows[0], o.rows[1], o.facts.join("+")))
                .collect::<Vec<_>>()
                .join(" / ")
        );
    }
    Ok(frozen)
}

/// 冻结的声明表（进程内唯一）。装载有任何缺口 ⇒ `None`，[`table`] 恒 `Err`。
///
/// ## 为什么进程级单例 ≡ 按会话注入
///
/// 五张表灌进 `Builder` 的全是 `&'static` 数据（`Decl` / `PortDecl` / `ActorDecl` 的
/// 每个字段都是 `&'static str` / `&'static [_]` / `Copy`），`seal(self)` 又是**消耗**
/// `self` 的：同一份程序文本只能冻出同一张表，按会话构造得到的就是这个值。于是
/// `LazyLock` 不是「一条全局可变注册通道」，而是一个纯函数的记忆化——它的
/// `Option` 缓存的是**失败**（缺口已逐条进 error 日志），不是可写的状态。
///
/// 与按会话注入真正不同的只有**开幕时机**：这里首次查询即装载，失败缓存后每一次开幕
/// 都带同一个 `Err` 退出（[`table`]），不存在「这一届会话拿到了半张表」。类型面上
/// `Session` 只持 `Arc<Frozen>`，`Frozen` 没有 `&mut self` 方法、没有内部可变性。
static FROZEN: LazyLock<Option<Arc<Frozen>>> = LazyLock::new(|| match assemble() {
    Ok(frozen) => Some(frozen),
    Err(gaps) => {
        crate::plugin_error!(
            "session",
            "[Loop] 行表装载失败，义务环拒绝启动（开幕带错退出）：{:?}",
            gaps
        );
        None
    }
});

/// 冻结的声明表（租约看护者自己也要求它——它不重复装载一份）。
///
/// 装载有任何缺口 ⇒ `Err`：调用方（开幕）带着它退出，不存在「表没装好但轮照常跑」
/// 这种部分成功。
pub(crate) fn table() -> Result<Arc<Frozen>, crate::symbio_core::PluginError> {
    frozen().ok_or_else(|| {
        crate::symbio_core::PluginError::InternalError(
            "义务环行表装载失败（装载期检查有缺口），本轮拒绝开幕；详见启动日志 [Loop] 行表装载失败".into(),
        )
    })
}

fn frozen() -> Option<Arc<Frozen>> {
    FROZEN.clone()
}

/// 本轮的事实切片：视图只扫**一轮**（声明的成本上界针对轮，不针对整份 WAL）。
///
/// 轮号单调，因此「本轮第一条事件」就是本轮窗口的起点；续写幕复用轮号时，
/// 首幕的事件也在切片里，而视图的 `drive_of` 会按**最新幕锚**收窄。
fn turn_slice(events: &[Event], turn: u64) -> &[Event] {
    match events.iter().position(|e| e.turn == turn) {
        Some(start) => &events[start..],
        None => &[],
    }
}

/// 本次 `drive` 的 `now`：**幕锚那条事实的 `ts`**（判据 12，DE-015 §2.1）。
///
/// 幕锚 = `drive_seq`：新轮是用户格，续写幕是恢复阶段事实——两者都是**触发本次
/// drive 的那条事实**。墙钟进入日志的唯一合法通道是落这条事实时 `Store::append`
/// 补的落格时刻，此后本次 drive 的所有时间条件（静默、租约）都只读它的 `ts`。
/// 于是同一个日志重放两次，`now` 相同，`owed` 逐字相同；而现取墙钟会让「回放可
/// 重建」在这些路径上成为假话。
///
/// 切片里没有这一条 ⇒ `0`（切片比幕旧，或幕锚填的是占位值）：没有任何时间条件
/// 可以满足，宁可判「还不该做」也不拿一个来路不明的时刻去凑。
pub(crate) fn wake_now_ms(events: &[Event], drive_seq: u64) -> i64 {
    events
        .iter()
        .find(|e| e.seq.is_some_and(|s| s.value() == drive_seq))
        .map(|e| e.ts)
        .unwrap_or(0)
}

/// 这一行此刻**欠着**吗（`owes` 命中且未被 `until` 关掉）。
///
/// 这是 §2.2 的 `open`，不是 `actionable`：`by = None` 的行照样欠着。「在等」与
/// 「没事」可区分正是 `by = None` 存在的理由，所以点问口不能把它折掉——调用点
/// （`port.rs` 的往返、`tool_dispatch` 的三段、`chat_loop` 的策略段）问的都是
/// 「这笔账在不在」，而「引擎该不该现在去还」只由 [`next`] 回答。
///
/// 返回布尔而不是 `Debt`：调用点只决定「这一步
/// 做不做」。要裁决顺序（同一点多行竞争）时用 `ring::drive`，本函数不代劳。
pub(crate) fn owes(name: &str, events: &[Event], turn: u64, now: i64) -> bool {
    let Some(frozen) = frozen() else {
        return false;
    };
    let debt = owed(frozen.rows(), &frozen, turn_slice(events, turn), now)
        .into_iter()
        .find(|d| d.decl.name == name);
    if let Some(debt) = &debt {
        trace(&frozen, debt);
    }
    debt.is_some()
}

/// 这一刻环上有没有**已派出、未收清**的账——「在等」的可观测形态。
///
/// 与 [`owes`] 的分工：`owes` 按**行名**问，本函数按**形状**问——开账条件是
/// `sent` 的那些行（`await-<port>`）。**刻意不按行名前缀匹配**：派发器不认行名
/// （D-118 决策 1），而「开账条件 = `sent`」正是「一次请求已经派出去、应答还没
/// 回来」这件事在表上的唯一写法，它不依赖命名约定——改了行的名字，这个判据照样成立。
///
/// ## 它存在的理由：`await-*` 行不是装饰行
///
/// `drive_steps` 选不到可还债时，有两种**完全不同**的处境：真的没事可做，与有一笔
/// 往返还在路上。两者在 `owed` 里可分（`blocked`），但停机理由此前一律写
/// `no_progress`——于是「这一轮为什么停」在事实面上不可区分，而「在等」恰恰是
/// 恢复路径最需要看见的那一种（进程在等应答时被打断，重启后这笔账要靠 `lease`
/// 判死，前提是它当时确实被记成「在等」）。
///
/// `await-*` 行的**唯一生产读者**就是这里（D-126）：它们由表形状保证「与派发行
/// 对偶」，由本函数给出可观测后果。两条缺一，它们就退化成行表里的装饰。
pub(crate) fn waiting(events: &[Event], turn: u64, now: i64) -> bool {
    let Some(frozen) = frozen() else {
        return false;
    };
    owed(frozen.rows(), &frozen, turn_slice(events, turn), now)
        .iter()
        .any(|d| d.blocked && d.decl.owes.source == EVENT_SENT)
}

/// 引擎的**选债口**：本幕当前最小的一笔可还债，连带它的动作。
///
/// 与 [`owes`] 的分工：`owes` 回答「某一行欠不欠」，本类型回答「下一步谁欠、
/// 跑什么」。[`next`] 是 `turn_exec::drive_steps` 的唯一选债依据（D-118 决策 1–2）——
/// 派发器不按行名 / 端口名 / 业务语义分支，只调 `Next.action` 这**一个**动作。
///
/// 参数 `A` 是**这台 host 的动作类型**（D-159）：轮内是 `turn_exec::Action`，轮首是
/// `head_exec::HeadAction`。选债、同源绑定、对账名单这三件事两台 host 完全一样，
/// 所以只有一个 `Next`，不是两个形状。
pub(crate) struct Next<A> {
    /// 行名（日志与停机理由用）。
    pub(crate) row: &'static str,
    /// 执行体名。`drive` 已滤掉 `by: None`，这里必有值。
    pub(crate) by: &'static str,
    /// 债键的人读形状（日志用）。
    pub(crate) key: String,
    /// 按 `by` 在绑定表里查出的动作。查不到就不会有这一项（见 [`next`] 的 `None`）。
    pub(crate) action: A,
    /// 同一个动作的**声明面**：它登记过会写哪些领域格。
    ///
    /// 派发器拿账本（日志尾段）对它对账（[`Next::writes_closed`]）。放在 `Next` 上而不是
    /// 让派发器再查一次表，是因为「跑哪个动作」与「它写了什么」必须同源——
    /// 分两次查就能查出两份答案（D-123 决策 2）。
    pub(crate) writes: &'static [&'static str],
    /// 这条执行体的**过渡豁免名单**（[`ActorEntry::transitional`] 的同一份，同源）。
    pub(crate) tolerated: &'static [&'static str],
}

/// 取最小可还债；没有可还的债 ⇒ `None`（收束 / 装载失败 / 只剩 blocked 行）。
///
/// 这是 `ring::drive` 的生产包装：切片只含**本轮**事件（视图成本上界按轮），
/// 并复用与 [`owes`] 同一份冻结声明表。装载失败返回 `None` 而不是 panic——
/// 派发器据此停住，不拿半张表继续走。
///
/// 可还性只有一个声明位：`owed` 的 `blocked`（`by = None ∨ unless`）。
/// 「注册表没覆盖的 `by`」不是第四个过滤条件——那种行直接写 `by = None`，
/// 接回时与它的 `ActorEntry` 同批改（D-118 订正 1）。
///
/// `actors` 是派发器手里的那张绑定表（生产 = [`ACTORS`]，D-123 决策 4）：行表决定
/// 「谁欠什么」，这张表决定「这个名字跑哪个动作、它登记写哪些格」，读账本对账用的
/// 名单也来自同一处。
///
/// 送给求解器的是**由这张表派生的行子集**（[`host_rows`]，D-159 裁决 2）：求解器
/// 字节一字未动，变的只是每个派发点看得到哪些行。
pub(crate) fn next(
    events: &[Event],
    turn: u64,
    now: i64,
    actors: &'static [ActorEntry],
) -> Option<Next<super::turn_exec::Action>> {
    let frozen = frozen()?;
    let rows = host_rows(&frozen, &actor_names(actors));
    pick(&frozen, &rows, events, turn, now, |by| {
        entry_of(by, actors).map(|e| (e.run, e.decl.writes, e.transitional))
    })
}

/// [`next`] 的**轮首**那一台（D-159）：同一个求解器、同一份对账名单，只是绑定表与
/// 动作类型换了一张。
pub(crate) fn next_head(
    events: &[Event],
    turn: u64,
    now: i64,
    actors: &'static [HeadEntry],
) -> Option<Next<super::head_exec::HeadAction>> {
    let frozen = frozen()?;
    let rows = host_rows(&frozen, &head_actor_names(actors));
    pick(&frozen, &rows, events, turn, now, |by| {
        head_entry_of(by, actors).map(|e| (e.run, e.decl.writes, e.transitional))
    })
}

/// 两台的选债半边共用这一段：求最小可还债 → 按 `by` 一次查全（动作 + 声明名单 +
/// 过渡豁免）→ 折成 [`Next`]。
///
/// 查不到动作就按「没有可还的债」显形并出声（不猜动作、也不猜它写了什么）：那是
/// 行表与绑定表被改坏的形状，不是第四种可还性。
fn pick<A>(
    frozen: &Frozen,
    rows: &[Decl],
    events: &[Event],
    turn: u64,
    now: i64,
    bind: impl Fn(&'static str) -> Option<(A, &'static [&'static str], &'static [&'static str])>,
) -> Option<Next<A>> {
    let debt = crate::symbio_core::ring::drive(rows, frozen, turn_slice(events, turn), now)?;
    trace(frozen, &debt);
    let Some((action, writes, tolerated)) = debt.decl.by.and_then(bind) else {
        crate::plugin_warn!(
            "session",
            "[Loop] 行表与执行体绑定表脱节（行 {} 的 `by={:?}` 没有动作），本步不派发",
            debt.decl.name,
            debt.decl.by
        );
        return None;
    };
    Some(Next {
        row: debt.decl.name,
        by: debt.decl.by?,
        key: debt.key.trace(),
        action,
        writes,
        tolerated,
    })
}

/// 按声明求一个视图值（本轮切片）。未声明 / 扫描超界 ⇒ `None`。
pub(crate) fn view(name: &str, events: &[Event], turn: u64, now: i64) -> Option<Value> {
    frozen()?.eval(name, turn_slice(events, turn), now)
}

/// 这一行携带的档位。未登记 ⇒ 深度档（fail-safe：不静默提速）。
///
/// 读的是**冻结表**里的那一行（`frozen.rows()`），不是 `static ROWS`：装载入口用
/// `static` 灌表，读侧只认表——否则「改了 static 但表没重装」这种漂移就没有出口。
pub(crate) fn tier(name: &str) -> LatencyTier {
    frozen()
        .and_then(|frozen| {
            frozen
                .rows()
                .iter()
                .find(|d| d.name == name)
                .map(|d| d.tier)
        })
        .unwrap_or(LatencyTier::Deep)
}

/// 一笔可还的**可观测面**：行 / 债标识 / 档位 / 视图口径。与旧 `Hit::trace` 同一
/// 目的——「某一步怎么发生了」在日志里要能指认到行与那一笔债，而不只是行名本身。
///
/// 视图版本一并记下（`ViewDecl.ver`）：投影读法一变，同一份事实折出的结论就变，
/// 回放时「这一轮用了哪一版」必须在日志里有答案。
fn trace(frozen: &Frozen, debt: &crate::symbio_core::ring::Debt<'_>) {
    let decl = debt.decl;
    let key = debt.key.trace();
    let view = frozen.view(decl.owes.source);
    crate::plugin_debug!(
        "session",
        "[Loop] 行 {} 可还（{} / {}ms；债 {}；视图 {} v{}）",
        decl.name,
        decl.tier.name(),
        decl.tier.budget_ms(),
        if key.is_empty() { "-" } else { key.as_str() },
        decl.owes.source,
        view.map(|v| v.ver).unwrap_or(0)
    );
}

#[cfg(test)]
#[path = "loop_rows.test.rs"]
mod tests;
