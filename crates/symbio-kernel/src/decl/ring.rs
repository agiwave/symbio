//! 义务环的**表与求解**：模式 / 行 / 债 / `owed` / `drive`。
//!
//! ## 一行是数据，不是函数
//!
//! 旧策略表（`loop_registry`，已退役）的一行是 `fn(&LoopState) -> Option<Capsule>`
//! ——判断住在函数体里。这里的一行是 [`Decl`]：`owes` / `until` / `unless` 三个
//! [`Pattern`] 加 `by` / `prio`。判断因此可 diff、可装载期检查；执行体只拿到一笔 [`Debt`]。
//!
//! ## 欠账公式
//!
//! ```text
//! open(d,key)      := ∃ f ⊨ d.owes ∧ key = bind(f) ∧ ¬∃ g ⊨ d.until ∧ key(g) = key
//! blocked(d,key)   := d.by = None ∨ ∃ h ⊨ d.unless ∧ key(h) = key
//! actionable(d,key) := open ∧ ¬blocked
//! next(now)        := min { (d,key) | actionable } 按 (prio, name, key)
//! ```
//!
//! 时间不进公式：需要时间的条件由视图把 `now` 折成布尔字段（[`Frozen::eval`]
//! 的 `now` 是入参）。因此 `owed` 只读**事实切片 + 视图值 + 触发事件时间戳**，
//! 不读配置、墙钟、内存缓存（[D-115](../../../../docs/003-design/015-义务环.md#d-115-义务环的输入闭包与执行契约owed-只读事实视图事件时间执行体异步可失败端口分两级 "DE-015-义务环") 决策 1）。
//!
//! ## 模式只做等值
//!
//! `eq` 是 `(字段, 字面量)` 列表，`key` 是参与债标识的字段名。没有否定、没有
//! 比较、没有函数——更复杂的条件要么折成视图字段，要么留在执行体里。这是
//! 「表里零代码」的类型形态（DE-015 判据 2）。
//!
//! **事实源的字段**先查载荷，再落三个信封字段：`turn` / `seq` / `ts`。它们不是
//! `FactDecl.fields` 的一部分——信封是每条事实都有的，不必逐条声明。
//!
//! **视图源**按求值结果展开：数组的每个元素是一条实例，对象是一条实例，`null`
//! 或求值失败 ⇒ 没有实例。
//!
//! ## 表之外的第二张表：`PortDecl`
//!
//! 端口级的东西（派发前先落哪个事实、租约多久、最多试几次）对**同一个端口的
//! 每一行都一样，所以不写进行里（那会是三份会漂移的副本），而是每个端口声明
//! 一次（[`PortDecl`]）。加一个端口 = 1 条 `PortDecl` + 适配器 + 2 行模板，
//! 求解器与行表不变（DE-015 §6 判据 1，执行方 `scripts/port-surface-audit.mjs`）。

use std::collections::HashSet;

use serde_json::Value;

use super::{ActorDecl, Frozen};
use crate::event::{EVENT_FAILED, EVENT_RESPONSE, EVENT_SENT};
use crate::{Event, LatencyTier};

/// 行级写入归属：`(行名, 该行执行时实际写入的事实名)`。
///
/// `ActorDecl.writes` 是**执行体级**的并集——它回答「这个执行体可能写哪些格」，
/// 装不下「共享它的每一行各自写哪些」。通用行执行体（D-241）让多行共用一条
/// `ActorDecl`，并集于是让每一行的出口都「可能被别的行写」：两行以上必然成假环。
/// 装配方把实现声明的逐行名单交进来，依赖图与第 6 条按**这一行**的名单判（D-248）。
pub type RowWrites = [(&'static str, &'static [&'static str])];

/// 这一行的实际写入名单：有逐行声明就用它，否则退回执行体的并集。
///
/// 非通用执行体的行没有逐行声明（它们的 `run` 就是那一行自己），并集即该行的名单。
fn row_writes_of<'a>(
    row: &str,
    actor: &'a ActorDecl,
    row_writes: &'a RowWrites,
) -> &'a [&'static str] {
    row_writes
        .iter()
        .find(|(name, _)| *name == row)
        .map(|(_, writes)| *writes)
        .unwrap_or(actor.writes)
}

/// 信封三格：每条事实都带，因此**不**要求写进 `FactDecl.fields`（模块文档 §「模式
/// 只做等值」）。第 3 条检查对它们放行，其余字段名一律要在声明里。
const ENVELOPE_FIELDS: &[&str] = &["turn", "seq", "ts"];

/// 模式里的一个字面量。只覆盖 JSON 的三个标量型——键绑定也只认它们。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lit {
    Bool(bool),
    Uint(u64),
    Str(&'static str),
}

/// 一条模式：`source` 上字段等值 + 键绑定。
#[derive(Debug, Clone, Copy)]
pub struct Pattern {
    /// 事实名或视图名（二者共用一个名字空间，见 [`Frozen`]）。
    pub source: &'static str,
    /// 绑定债标识的字段。空切片 ⇒ 这条源至多一笔债（例如按轮收束的格）。
    pub key: &'static [&'static str],
    /// 字段等值约束。
    pub eq: &'static [(&'static str, Lit)],
}

/// 义务环的一行。
#[derive(Debug, Clone)]
pub struct Decl {
    /// 行的身份：日志、冲突报错、对拍都按它指认。
    pub name: &'static str,
    /// **全局**裁决顺序：`(prio, name, key)` 升序，先小者胜。
    ///
    /// 与旧表的格内 `prio` 不同：义务环只有一个环、一次取最小，跨行必须可比。
    pub prio: u8,
    /// 执行体名。`None` ⇒ 恒 blocked、永不选中（「在等」的唯一声明位）。
    pub by: Option<&'static str>,
    /// 这一行动作允许的时延档。
    pub tier: LatencyTier,
    pub owes: Pattern,
    /// 集合展开：`Some((list_field, element_key_field))` ⇒ `owes` 实例的
    /// `list_field` 是数组，**逐元素**开一笔债（DE-015 §3.3；`FactDecl.expand`
    /// 是它在事实侧的另一半）。
    ///
    /// 展开后键绑定**对元素求值**：`owes.key` 里每个字段都从元素上取。
    /// `element_key_field` 必须出现在 `owes.key` 里——它是元素的身份字段，
    /// 缺了它只能开一堆同键债，`until` 无从分辨还了哪一笔。
    ///
    /// 只有 `owes` 展开：`until` / `unless` 按键命中，键已经唯一，再展开只会
    /// 把同一笔出口读成多笔。
    pub expand: Option<(&'static str, &'static str)>,
    /// 任一命中（同键）即视为还清。
    pub until: &'static [Pattern],
    /// 任一命中（同键）即挂起。
    pub unless: &'static [Pattern],
}

/// 一个**端口**的声明：适配器协议里那几项每端口不同、又必须被引擎与
/// 租约看护者读到的东西。
///
/// ## 为什么不把这些字段塞进 `Decl`
///
/// `before` / `lease_ms` / `max_attempts` 对**同一个端口的每一行**都一样，写进
/// 行里就是三份副本（dispatch / await / lease），而副本必然漂移。端口级的东西
/// 按端口声明一次：加一个端口 = 加一条 `PortDecl` + 适配器 + 2 行模板（DE-015 §6
/// 判据 1）。
///
/// ## 为什么在 `ring` 而不在插件
///
/// 它与 [`Decl`] 是同一张表的两半：行说「什么时候欠、什么算还」，端口说
/// 「派发时先落什么、等多久、试几次」。租约看护者与派发适配器都只读这张表，
/// 不看对方。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortDecl {
    /// 端口名——`request` / `sent` / … 载荷里的 `port` 取值。
    pub port: &'static str,
    /// 引擎在运行执行体**之前**落的那个事实名（协议族里的 `sent`）。
    ///
    /// 搬运行者不写它：它的 `run` 可能**不产生事实就返回**，依赖它自觉 ⇒
    /// 派发行永远不还清，而没有任何东西会红（DE-015 §2.3 判据 2）。
    pub before: &'static str,
    /// 租约：`sent` 之后多久没有应答算「卡住」（毫秒）。
    pub lease_ms: u64,
    /// 最多尝试几次（含首次）。到顶之后租约看护者判死而不是再重试。
    pub max_attempts: u64,
    /// 同一批次的子债**最多几条同时在飞**；`0` / `1` = 串行（硬闸）。
    ///
    /// 一个字段回答一个问题：这个端口允不允许同批并发、允许几笔。「允许」与
    /// 「上限」写在两处必然漂移；写在执行层的常量里则让新端口无法声明自己的上界
    /// ——加端口的成本就从声明漏回引擎（DE-015 §6 判据 1）。具体哪几条能加入
    /// 并发集合仍由执行层按风险档决定（声明与策略分开的另一半）。
    pub concurrency: u16,
}

/// 按端口名取声明。`None` = 没有这个端口（漏声明，不是默认值）。
pub fn port_of<'a>(ports: &'a [PortDecl], name: &str) -> Option<&'a PortDecl> {
    ports.iter().find(|p| p.port == name)
}

/// 一笔债的标识：模式 `key` 字段的 `(名字, 值)` 列表，按声明顺序。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Key(pub Vec<(String, String)>);

impl Key {
    /// 人对得上的形状（日志与断言用）。
    pub fn trace(&self) -> String {
        self.0
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(",")
    }
}

/// 一笔当前欠着的账。
#[derive(Debug, Clone)]
pub struct Debt<'a> {
    pub decl: &'a Decl,
    pub key: Key,
    /// `by = None` 或有 `unless` 命中：存在，但引擎不还。
    pub blocked: bool,
}

impl PartialEq for Debt<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.order() == other.order() && self.blocked == other.blocked
    }
}

impl Eq for Debt<'_> {}

impl PartialOrd for Debt<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Debt<'_> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.order().cmp(&other.order())
    }
}

impl Debt<'_> {
    fn order(&self) -> (u8, &'static str, &Key) {
        (self.decl.prio, self.decl.name, &self.key)
    }
}

/// 全量欠账：每个 `Decl` × 每个 `owes` 实例，收口（有 `until`/`unless` 同键命中）
/// 的不返回。`blocked` 的债**照样返回**——「在等」与「没事」必须可区分。
pub fn owed<'a>(rows: &'a [Decl], frozen: &Frozen, events: &[Event], now: i64) -> Vec<Debt<'a>> {
    let mut debts = Vec::new();
    for decl in rows {
        let until = keys_of_all(decl.until, frozen, events, now);
        let unless = keys_of_all(decl.unless, frozen, events, now);
        for key in keys_of_expanded(&decl.owes, decl.expand, frozen, events, now) {
            if until.contains(&key) {
                continue;
            }
            let blocked = decl.by.is_none() || unless.contains(&key);
            debts.push(Debt { decl, key, blocked });
        }
    }
    debts
}

/// 取最小的一笔可还的债。这就是 `drive` 的**选择半边**——生产入口
/// `turn_exec::drive` 直接调用它；执行半边（跑执行体、追加事实、
/// 无进展守卫）在派发器里，选择规则不再变。
///
/// 可还性只有一个声明位：[`owed`] 的 `blocked`（`by = None ∨ unless` 命中）。
/// 「执行体还没接线」不是求解器的第四种条件——那种行显式写 `by = None`，
/// 接线时与 `ActorDecl` 注册同批改回 `Some`（DE-015 §2.2 / D-118 订正 1）。
///
/// 按行点问（`crates/symbio-plugin-loop/src/loop_rows.rs::owes`）仍留给「刚写完 request，这一行
/// 还欠不欠」的单点断言；**单环驱动**（一次取下一笔、按 `prio` 裁决）是
/// 调度器唯一的选债口（D-118 决策 1–2）。
pub fn drive<'a>(
    rows: &'a [Decl],
    frozen: &Frozen,
    events: &[Event],
    now: i64,
) -> Option<Debt<'a>> {
    owed(rows, frozen, events, now)
        .into_iter()
        .filter(|d| !d.blocked)
        .min()
}

fn keys_of_all(patterns: &[Pattern], frozen: &Frozen, events: &[Event], now: i64) -> Vec<Key> {
    patterns
        .iter()
        .flat_map(|p| keys_of(p, frozen, events, now))
        .collect()
}

fn keys_of(p: &Pattern, frozen: &Frozen, events: &[Event], now: i64) -> Vec<Key> {
    keys_of_expanded(p, None, frozen, events, now)
}

/// `owes` 的键集合。`expand = None` ⇒ 一个实例一笔债；`Some` ⇒ 每个实例的
/// 列表字段逐元素展开，键绑定对**元素**求值（见 [`Decl::expand`]）。
fn keys_of_expanded(
    p: &Pattern,
    expand: Option<(&'static str, &'static str)>,
    frozen: &Frozen,
    events: &[Event],
    now: i64,
) -> Vec<Key> {
    let mut keys = Vec::new();
    if frozen.fact(p.source).is_some() {
        for e in events.iter().filter(|e| e.kind == p.source) {
            match expand {
                None => keys.extend(key_from(p, |f| fact_field(e, f))),
                Some((list, _)) => {
                    // 等值约束仍拦在**外层实例**上（`port=model`），键则取自元素。
                    if !eq_matches(p, |f| fact_field(e, f)) {
                        continue;
                    }
                    if let Some(Value::Array(items)) = fact_field(e, list) {
                        for item in items.iter() {
                            keys.extend(key_of(p, |f| item.get(f).cloned()));
                        }
                    }
                }
            }
        }
    } else if frozen.view(p.source).is_some() {
        let instances: Vec<Value> = match frozen.eval(p.source, events, now) {
            Some(Value::Array(items)) => items,
            Some(v @ Value::Object(_)) => vec![v],
            _ => Vec::new(),
        };
        for v in instances {
            match expand {
                None => keys.extend(key_from(p, |f| v.get(f).cloned())),
                Some((list, _)) => {
                    if !eq_matches(p, |f| v.get(f).cloned()) {
                        continue;
                    }
                    if let Some(Value::Array(items)) = v.get(list) {
                        for item in items.iter() {
                            keys.extend(key_of(p, |f| item.get(f).cloned()));
                        }
                    }
                }
            }
        }
    }
    keys
}

fn key_from(p: &Pattern, mut field: impl FnMut(&str) -> Option<Value>) -> Option<Key> {
    if !eq_matches(p, &mut field) {
        return None;
    }
    key_of(p, field)
}

/// 只取键字段，不再判 `eq`。
///
/// 集合展开时 `eq` 已经拦在**外层实例**上（D-116 决策 3），元素键只取身份字段；
/// 再对元素判一次 `eq` 会把「外层是 `port=model`、元素是 `port=tool`」的组合
/// 全部滤掉——那正是创建行 `open-tools` 的形态。
fn key_of(p: &Pattern, mut field: impl FnMut(&str) -> Option<Value>) -> Option<Key> {
    let mut key = Vec::with_capacity(p.key.len());
    for name in p.key {
        let value = field(name);
        key.push(((*name).to_string(), scalar(value.as_ref())?));
    }
    Some(Key(key))
}

fn eq_matches(p: &Pattern, mut field: impl FnMut(&str) -> Option<Value>) -> bool {
    p.eq.iter()
        .all(|&(name, lit)| lit_matches(lit, field(name).as_ref()))
}

fn fact_field(e: &Event, name: &str) -> Option<Value> {
    match name {
        "turn" => Some(Value::from(e.turn)),
        "seq" => e.seq.map(|s| Value::from(s.value())),
        "ts" => Some(Value::from(e.ts)),
        _ => e.payload.get(name).cloned(),
    }
}

fn lit_matches(lit: Lit, value: Option<&Value>) -> bool {
    match lit {
        Lit::Bool(b) => value.and_then(Value::as_bool) == Some(b),
        Lit::Uint(n) => value.and_then(Value::as_u64) == Some(n),
        Lit::Str(s) => value.and_then(Value::as_str) == Some(s),
    }
}

fn scalar(value: Option<&Value>) -> Option<String> {
    let value = value?;
    if let Some(b) = value.as_bool() {
        return Some(b.to_string());
    }
    if let Some(n) = value.as_u64() {
        return Some(n.to_string());
    }
    value.as_str().map(str::to_string)
}

/// 装载期的行检查（DE-015 §3.4 的第 2 / 7 条）。
///
/// ## §3.4 七道检查 → 这一层的格子（指认表）
///
/// | §3.4 | 检查 | 执行方 | 报出的格子 |
/// |---|---|---|---|
/// | 1 | 依赖图无环 | [`check_dependency_graph`]（`check_rows` 末段调用） | `DependencyCycle` |
/// | 2 | 每行收口 | [`check_rows`] | `NoExit` / `KeyMismatch` / `ExpandKeyMissing` / `UndeclaredExpand` |
/// | 3 | 字段都在声明里 | [`check_rows`]（模式侧）+ `seal()` 的声明侧（视图 `reads` 与 `FactDecl` 自引用） | `UndeclaredPatternField` + `UndeclaredField` / `FactFieldUndeclared` |
/// | 4 | 相交行对 | [`overlaps`]——**只报告，不红**（静态判不出「真的同时命中」） | `Overlap` |
/// | 5 | 写入封闭 | `seal` 的声明侧（执行体不许声明引擎格）+ [`check_ports`]（端口有应答出口）；「实际写入 ⊆ `writes`」那一半要值返回账本，归 | `ActorWritesEngineCell` / `PortBeforeUndeclared` / `PortWithoutAnswerExit` |
/// | 6 | 执行体收口 | [`check_rows`] | `UnmannedRow` / `OrphanActor` / `ActorCannotClose` / `ActorModeTooNarrow` |
/// | 7 | 声明事实有归宿 | [`check_rows`] | `OrphanFact` |
///
/// 除第 4 条（报告）以外，全部走同一个 `Err`：[`crate::decl::Builder::seal`] 是**唯一**的装载出口，
/// 声明侧与行侧共用下面这一个 [`Gap`] 类型。
///
/// - [`Gap::NoExit`]：这一行没有 `until` / `unless` ⇒ 一旦开账就永远开账；
/// - [`Gap::KeyMismatch`]：没有一条出口与 `owes` 绑定同一组键 ⇒ 出口永远匹配不上；
/// - [`Gap::UnknownSource`]：模式引用了一个没有声明的名字；
/// - [`Gap::DuplicateName`]：两行同名；
/// - [`Gap::OrphanFact`]：一个被声明的事实没有任何行**直接或经由视图**读到——
///   拼错名字的静默形态，也是旧 `Verdict::Unmanned` 那一档的可观测性替代物；
/// - [`Gap::ExpandKeyMissing`]：`expand` 的元素键字段不在 `owes.key` 里——元素
///   之间分不出谁是谁，`until` 无从按键还账；
/// - [`Gap::UndeclaredExpand`]：`owes` 是事实，但它的 `expand` 列表字段没有写进
///   那条事实的 `FactDecl.expand`——求解器会去读一个事实自己都不承认的字段；
/// - [`Gap::UnmannedRow`]：`by = Some(名字)` 而这个名字没有执行体声明——「这行
///   有人跑」是假话，选中它只会得到 `no_progress`（DE-015 §3.4 第 6 条的前半）；
/// - [`Gap::OrphanActor`]：声明了执行体却没有任何行指向它——裸函数的另一面，
///   接线接了个没人派发的名字；
/// - [`Gap::ActorCannotClose`]：执行体够不到它那一行的收口（§3.4 第 6 条的后半）——
///   出口里有可声明的领域格时，判它与出口（`until` / `unless`，视图按 `reads` 展开到
///   事实）**有无交集**；出口全是引擎独占格时（派发行的形状：引擎落 `before` / `failed`
///   替它关门），判它有没有**交出去的凭据**——名单非空，或出口里有某个端口的 `before` 格
///   （订正 1：纯派发半手里没有领域格，写的是引擎的 `sent`）。两支都指向同一个故障：
///   这一行既写不出领域格、也没有一条挂得上的派出契约；
/// - [`Gap::ActorModeTooNarrow`]：行的 `tier` 比执行体的 `mode` 更慢——反射档
///   拿不到模型句柄（§3.4 的 mode 闸门），却让一行声称要调模型；
/// - [`Gap::UndeclaredPatternField`]：模式引用了一个不在那条事实载荷声明里的字段
///   （§3.4 第 3 条）——它与「字段这次没带」在求值时同形，只能装载期拦；
/// - [`Gap::DependencyCycle`]：两行以上的「谁还清谁」成环（§3.4 第 1 条）——
///   类型级互相等待，运行期没有任何一步走得动。
///
/// 后两条不在 `check_rows` 的输入面上，由 [`check_ports`] 报（§3.4 第 5 条的
/// 端口侧）：
///
/// - [`Gap::PortBeforeUndeclared`]：端口的派发前格（`before`）没有声明；
/// - [`Gap::PortWithoutAnswerExit`]：这个端口的 `response` / `failed` 不落在任何
///   行的出口上——派出去的钱没有还的通道。
///
/// ## 声明侧的格子（同一个类型，不分两套）
///
/// 六张注册表（event / fact / view / actor / port / row）经 `Builder` 进表后，
/// 「装载期检查」只有一个出口、也就只有一个错误类型：下面这十一格查的是**声明自己**
/// （重名、撞名、引用不存在的名字、字段没声明、执行体声明了引擎独占的格），
/// 由 `seal()` 在跑上面七道之前先报。分成两个类型只会让「拒绝启动」出现两次。
///
/// - [`Gap::DuplicateEvent`] / [`Gap::DuplicateFact`] / [`Gap::DuplicateView`] /
///   [`Gap::DuplicateActor`] / [`Gap::DuplicatePort`]：同一张表里两个同名——按名字查找的
///   地方（写闸取格位、`by`、`port` 取值、视图求值）从此不确定查到的是哪一条；
/// - [`Gap::FactWithoutEvent`]：插件声明的事实没有事件行——写闸取不到格位，运行期
///   只能拒绝（没有任何回退表能替它补格位，DE-024 D-199）；
/// - [`Gap::NameCollision`]：事实名 / 事件名与视图名撞名——它们共用一个名字空间
///   （视图是合成事实）；
/// - [`Gap::UnknownFact`] / [`Gap::UndeclaredField`]：视图 `reads` 引用了未声明的事实 /
///   该事实没有这个字段；
/// - [`Gap::EvidenceNotRead`]：视图的 `evidence` 写了 `reads` 之外的名字——那条边
///   会从依赖图里**静默消失**（第 1 条按证据集配图），比多一条假边更坏；
/// - [`Gap::FactFieldUndeclared`]：事实自己的 `port` / `key` / `expand` 引用了不在
///   `fields` 里的字段；
/// - [`Gap::UnknownReadView`]：执行体 `reads` 引用了未声明的视图——运行期静默返回空；
/// - [`Gap::ActorWritesEngineCell`]：执行体将写协议四格里引擎独占的那些格（§2.3）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Gap {
    NoExit {
        row: &'static str,
    },
    KeyMismatch {
        row: &'static str,
    },
    UnknownSource {
        row: &'static str,
        source: &'static str,
    },
    DuplicateName {
        row: &'static str,
    },
    OrphanFact {
        fact: &'static str,
    },
    ExpandKeyMissing {
        row: &'static str,
        field: &'static str,
    },
    UndeclaredExpand {
        row: &'static str,
        source: &'static str,
        field: &'static str,
    },
    UnmannedRow {
        row: &'static str,
        by: &'static str,
    },
    /// 一档守卫角色没有任何声明持有（DE-024 D-203）。
    ///
    /// 判据读的是角色，角色来自 owner 的 `EventDecl.roles`——owner 删掉它不会报错，
    /// 只会让那道守卫**从拒绝变成看不见**。装配口因此钉住「每一档都有人住」。
    UnmannedRole {
        role: &'static str,
    },
    OrphanActor {
        actor: &'static str,
    },
    ActorCannotClose {
        row: &'static str,
        actor: &'static str,
    },
    ActorModeTooNarrow {
        row: &'static str,
        actor: &'static str,
    },
    UndeclaredPatternField {
        row: &'static str,
        source: &'static str,
        field: &'static str,
        role: &'static str,
    },
    DependencyCycle {
        cycle: Vec<&'static str>,
    },
    PortBeforeUndeclared {
        port: &'static str,
        fact: &'static str,
    },
    PortWithoutAnswerExit {
        port: &'static str,
        fact: &'static str,
    },
    DuplicateEvent {
        name: &'static str,
    },
    /// 插件声明了一条事实，却没有任何事件行给它格位——写闸永远认不出它
    /// （没有任何回退表能替它补格位）。
    FactWithoutEvent {
        fact: &'static str,
    },
    DuplicateFact {
        name: &'static str,
    },
    DuplicateView {
        name: &'static str,
    },
    /// 两条视图声明的 `cost` 不一样——而 [`Frozen::eval`] 的输入是**同一份**切片。
    ///
    /// 切片模型只有一个长度上界（[`Frozen::scan_bound`] 取最小值），所以「A 声明 4096、
    /// B 声明 512」这件事没有运行期解法：按 4096 切，B 每一轮都超界 ⇒ 回 `None` ⇒
    /// `owed` 折成空集 ⇒ B 那一族的行全都不开账，**零告警**；按 512 切，A 只看得到
    /// 尾部一小段，另一族的行静默漏账。两支都是「算不出来」冒充「没有欠账」。
    ///
    /// 因此它在这里判，不留给运行期兜底：声明的分歧必须在装载期红。
    ViewCostMismatch {
        view: &'static str,
        cost: u32,
        /// 全表的最小值——切片实际会用的那个上界。
        bound: u32,
    },
    DuplicateActor {
        name: &'static str,
    },
    DuplicatePort {
        name: &'static str,
    },
    NameCollision {
        name: &'static str,
    },
    UnknownFact {
        view: &'static str,
        fact: &'static str,
    },
    UndeclaredField {
        view: &'static str,
        fact: &'static str,
        field: &'static str,
    },
    EvidenceNotRead {
        view: &'static str,
        fact: &'static str,
    },
    FactFieldUndeclared {
        fact: &'static str,
        field: &'static str,
        role: &'static str,
    },
    UnknownReadView {
        actor: &'static str,
        view: &'static str,
    },
    ActorWritesEngineCell {
        actor: &'static str,
        fact: &'static str,
    },
}

/// 一个模式源展开成**事实名**：事实源就是它自己，视图源是它 `reads` 里的那些事实。
///
/// 第 6 条检查比的是「执行体写的名字」与「这一行靠什么收口」，两边都在事实层；
/// 行的出口常写成视图（`turn_closed`），不展开就会把每一个真收口判成收不了口。
fn fact_names_of(source: &'static str, frozen: &Frozen) -> Vec<&'static str> {
    match frozen.view(source) {
        Some(view) => view.reads.iter().map(|(f, _)| *f).collect(),
        None => vec![source],
    }
}

/// 一个模式源展开成**替它收口的事实名**：事实源仍是它自己，视图源只取它的
/// `evidence`（输出证据集），不取 `reads` 全集。
///
/// 只有依赖图（§3.4 第 1 条）用它。差别在锚点：`turn_closed` 读 `turn.stage` 只为
/// 折出幕锚，那一格谁都没替它关门——按 `reads` 建边会把「帮我算」读成「替我收」，
/// 于是共写 `turn.stage` 一族的行两两互等成假环（判据见 DE-015 的 D-160）。
/// 第 6 条仍按 [`fact_names_of`]：它判的是执行体**有没有能力**写到落在自己出口线
/// 上的格，保守一档只多不少。
fn exit_fact_names(source: &'static str, frozen: &Frozen) -> Vec<&'static str> {
    match frozen.view(source) {
        Some(view) => view.evidence.to_vec(),
        None => vec![source],
    }
}

pub fn check_rows(rows: &[Decl], frozen: &Frozen, row_writes: &RowWrites) -> Vec<Gap> {
    let mut gaps = Vec::new();
    let mut names = HashSet::new();
    for decl in rows {
        if !names.insert(decl.name) {
            gaps.push(Gap::DuplicateName { row: decl.name });
        }
        if decl.until.is_empty() && decl.unless.is_empty() {
            gaps.push(Gap::NoExit { row: decl.name });
        } else if !decl
            .until
            .iter()
            .chain(decl.unless.iter())
            .any(|p| p.key == decl.owes.key)
        {
            gaps.push(Gap::KeyMismatch { row: decl.name });
        }
        if let Some((list, elem_key)) = decl.expand {
            if !decl.owes.key.contains(&elem_key) {
                gaps.push(Gap::ExpandKeyMissing {
                    row: decl.name,
                    field: elem_key,
                });
            }
            if let Some(fact) = frozen.fact(decl.owes.source) {
                if !fact.expand.contains(&list) {
                    gaps.push(Gap::UndeclaredExpand {
                        row: decl.name,
                        source: decl.owes.source,
                        field: list,
                    });
                }
            }
        }
        // `owes` 与出口 / 挂起源一起查存在性；第二条布尔说「这一条的键绑在元素上」。
        let patterns = std::iter::once((&decl.owes, decl.expand.is_some()))
            .chain(decl.until.iter().map(|p| (p, false)))
            .chain(decl.unless.iter().map(|p| (p, false)));
        for (p, keys_on_elements) in patterns {
            match frozen.fact(p.source) {
                None => {
                    if frozen.view(p.source).is_none() {
                        gaps.push(Gap::UnknownSource {
                            row: decl.name,
                            source: p.source,
                        });
                    }
                }
                // 第 3 条：模式引用的**字段名**必须在那条事实的载荷声明里。
                // 拼错的字段与缺席的字段在求值时同形（取不到 ⇒ 不命中），所以
                // 「运行期静默不命中」只能在这里拦：装载期红，而不是赌一次回放。
                //
                // 两个例外都是构造性的，不是豁免：
                // - 信封三格（`turn` / `seq` / `ts`）每条事实都有，逐条声明是假话；
                // - 展开行的 `owes` 键绑在**元素**上（`calls` 数组里的一项），元素
                //   形状不属于那条事实的 `fields`（`eq` 仍绑外层实例，照查）。
                Some(fact) => {
                    let mut fields: Vec<(&str, &str)> = Vec::new();
                    if !keys_on_elements {
                        fields.extend(p.key.iter().map(|f| ("key", *f)));
                    }
                    fields.extend(p.eq.iter().map(|(f, _)| ("eq", *f)));
                    for (role, field) in fields {
                        if !fact.fields.contains(&field) && !ENVELOPE_FIELDS.contains(&field) {
                            gaps.push(Gap::UndeclaredPatternField {
                                row: decl.name,
                                source: p.source,
                                field,
                                role,
                            });
                        }
                    }
                }
            }
        }
        // 第 6 条的两半：`by` 指向一个**存在**的执行体，且它够得到这一行的收口。
        // 未接线的行不在这儿豁免——它写 `by = None`（「这笔账不由引擎执行」的唯一
        // 声明位），而不是留一个没人跑的名字。
        if let Some(by) = decl.by {
            match frozen.actor(by) {
                None => gaps.push(Gap::UnmannedRow { row: decl.name, by }),
                Some(actor) => {
                    let exits: Vec<&'static str> = decl
                        .until
                        .iter()
                        .chain(decl.unless)
                        .flat_map(|p| fact_names_of(p.source, frozen))
                        .collect();
                    // 两档（D-124，订正 1）：出口里有**可声明的领域格** ⇒ 原判据（交集非空）；
                    // 出口**全是**引擎独占格 ⇒ 那是派发行的形状——引擎按 `PortDecl::before`
                    // 在动作之前落 `sent`、失败/租约到顶落 `failed`，关门本来就不归执行体。
                    // 此时若还要求交集，就是要求执行体声明第 5 条禁止它声明的格：
                    // 两条检查互斥 ⇒ 这一行永远不许接线。
                    //
                    // 这一档判得住的是「这一行有没有**交出去的凭据**」两种形态之一：
                    // - 名单非空（旧口径）：它自己写得出领域格；
                    // - 出口里有某个端口的 `before` 格（订正 1）：它挂上了一条**派出契约**，
                    //   而 `check_ports` 已经要求那条端口有应答出口 ⇒ 关门链条完整，
                    //   动作这一侧只需要「把这一笔交出去」。纯派发半（派出去 / 收回来分处
                    //   两个环步之后的前一半）就落在这条上：它手里没有领域格可写，
                    //   写的是引擎的 `sent`。
                    //
                    // 「这次真没写」两支都由运行期的账本守卫判（D-123 决策 1）：领域格那一支
                    // 看空账本，派发那一支看 `sent` 关不关账（`waiting` ⇒ 步数上界 / `reap`）。
                    let engine_only_exits = !exits.is_empty()
                        && exits.iter().all(|f| super::ENGINE_ONLY_CELLS.contains(f));
                    // 共享执行体的行按**自己**声明的写入判（D-248）：并集会把「这
                    // 个执行体可能写」读成「这一行会写」，多行共用时每一行的出口
                    // 都被别的行的并集认领，收口检查与依赖图同时失真。
                    let writes = row_writes_of(decl.name, actor, row_writes);
                    let reaches_exit = if engine_only_exits {
                        !writes.is_empty() || frozen.ports.iter().any(|p| exits.contains(&p.before))
                    } else {
                        exits.iter().any(|f| writes.contains(f))
                    };
                    if !reaches_exit {
                        gaps.push(Gap::ActorCannotClose {
                            row: decl.name,
                            actor: actor.name,
                        });
                    }
                    if decl.tier > actor.mode {
                        gaps.push(Gap::ActorModeTooNarrow {
                            row: decl.name,
                            actor: actor.name,
                        });
                    }
                }
            }
        }
    }
    // 「有归宿」= 行直接读它，或行用了某个读它的视图。两跳都算——义务环的行
    // 面向视图写，事实是视图的下游；只认直接引用会把所有视图源的事实全判成孤儿。
    let used_sources: HashSet<&str> = rows
        .iter()
        .flat_map(|d| std::iter::once(&d.owes).chain(d.until).chain(d.unless))
        .map(|p| p.source)
        .collect();
    let read_by_used_view = |fact: &str| {
        frozen
            .views()
            .iter()
            .filter(|v| used_sources.contains(v.name))
            .any(|v| v.reads.iter().any(|(f, _)| *f == fact))
    };
    for fact in frozen.facts() {
        if !used_sources.contains(fact.name) && !read_by_used_view(fact.name) {
            gaps.push(Gap::OrphanFact { fact: fact.name });
        }
    }
    // 执行体的另一半：声明了却没有任何行指向它，等于注册了一段死代码。
    // 它与 `UnmannedRow` 合起来才是「一个 `by` 一个执行体」——两边都不许多。
    let used_by: HashSet<&str> = rows.iter().filter_map(|d| d.by).collect();
    for actor in frozen.actors() {
        if !used_by.contains(actor.name) {
            gaps.push(Gap::OrphanActor { actor: actor.name });
        }
    }
    gaps.extend(check_dependency_graph(rows, frozen, row_writes));
    gaps
}

/// §3.4 第 1 条：「谁还清谁」的依赖图不许有环。
///
/// 边 `a → b` 的意思：`b` 的执行体声明写的某个**事实名**出现在 `a` 的出口（`until ∪
/// unless`，视图按 `evidence` 展开到事实——只作锚点的读面不算关门方，见 [`exit_fact_names`]）
/// 里，**而且两边可能是同一格**——`a` 还不还得看 `b` 跑不跑。环 ⇒ 每一笔都在等环里的
/// 另一笔先被派发，而「被派发」要求它此刻就开着：类型级互相等待，运行期一步都走不动
/// （这不是「慢」，是死锁）。
///
/// 一格事实可以有好几档（`turn.stage` 的判决 / 召回 / 压缩各写自己那一档），只按
/// 事实名配边会把「同族」读成「同格」，于是共写一族的行两两互等——判据见
/// DE-015 的 [D-160](../../../../docs/003-design/015-义务环.md#d-160-依赖图的边按出口格配视图按输出证据集展开同族不同档不成边 "DE-015-义务环")；
/// 那一格的身份从两边的出口模式读，不新增声明字段。
///
/// 两处刻意保守：
///
/// - **自环不算**。一行自己写自己的出口正是正常形状（执行体干活，把这一行关掉），
///   它不构成等待；把自环算进环会让每一张真表都红。
/// - `by = None` 的行没有出边。它的出口由插件代码落，不由某个行执行体跑；这张图
///   只画「行等行」，不去猜代码里写了什么。
/// - 两边的出口**不共这一族事实**时不设防（见 [`writes_that_cell`] 的 `None` 分支）：
///   那条执行体多写的格落在哪一档，声明面说不出 ⇒ 照旧成边。
fn check_dependency_graph(rows: &[Decl], frozen: &Frozen, row_writes: &RowWrites) -> Vec<Gap> {
    let mut gaps = Vec::new();
    let mut state = vec![Visit::New; rows.len()];
    let mut stack: Vec<usize> = Vec::new();
    for start in 0..rows.len() {
        walk(
            rows, frozen, row_writes, start, &mut state, &mut stack, &mut gaps,
        );
    }
    gaps
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Visit {
    New,
    OnStack,
    Done,
}

/// 这一行的出口**格**：`(事实名, 那一格的判别条件)`。
///
/// 名字按 [`exit_fact_names`] 展开（`evidence`），条件就是出口模式自己的 `eq`——
/// 一格 `turn.stage` 说的从来不是「这一族」，而是「这一档」（`stage = compress`）。
/// 第 1 条配边要比的正是这一对（DE-015 的 D-160）。
fn exit_cells<'d>(decl: &'d Decl, frozen: &Frozen) -> Vec<(&'static str, &'d Pattern)> {
    decl.until
        .iter()
        .chain(decl.unless)
        .flat_map(|p| {
            exit_fact_names(p.source, frozen)
                .into_iter()
                .map(move |f| (f, p))
        })
        .collect()
}

/// `other` 的执行体写这一族事实时，落的**可能**就是 `at` 那一格吗。
///
/// 执行体多写哪些格不在声明面上（`ActorDecl.writes` 只有事实名），它替自己那一行
/// 关的却是具体的一格——装载期第 6 条判的正是「执行体走不走得到自己那一格」。于是
/// 这一格的身份从它自己那行的出口读：两边的判别条件相容 ⇒ 可能是同一格 ⇒ 成边；
/// 同一字段写着两个不同字面量 ⇒ 两格 ⇒ 不成边。
///
/// `other` 的出口里没有这一族事实时**照旧成边**（保守）：它写这一族是给别的东西落
/// 的，没有声明能说清落在哪一格。收窄不放松——漏一条真边比多一条假边坏得多。
fn writes_that_cell(frozen: &Frozen, other: &Decl, fact: &str, at_exit: &Pattern) -> bool {
    match exit_cells(other, frozen)
        .into_iter()
        .find(|(f, _)| *f == fact)
    {
        Some((_, theirs)) => eq_compatible(at_exit, theirs),
        None => true,
    }
}

/// `at` 的出边：哪些**别的**行的执行体会写到它的出口。
fn producers_of(rows: &[Decl], frozen: &Frozen, row_writes: &RowWrites, at: usize) -> Vec<usize> {
    let exits = exit_cells(&rows[at], frozen);
    rows.iter()
        .enumerate()
        .filter(|(i, _)| *i != at)
        .filter(|(_, other)| {
            other
                .by
                .and_then(|name| frozen.actor(name))
                .is_some_and(|actor| {
                    let writes = row_writes_of(other.name, actor, row_writes);
                    exits
                        .iter()
                        .any(|(f, p)| writes.contains(f) && writes_that_cell(frozen, other, f, p))
                })
        })
        .map(|(i, _)| i)
        .collect()
}

/// 一次深搜；回到栈上的结点 ⇒ 报那一段。每条回边只在其起点被访问时报一次，
/// 所以同一个环不会被重复报两遍。
fn walk(
    rows: &[Decl],
    frozen: &Frozen,
    row_writes: &RowWrites,
    at: usize,
    state: &mut [Visit],
    stack: &mut Vec<usize>,
    gaps: &mut Vec<Gap>,
) {
    if state[at] != Visit::New {
        if state[at] == Visit::OnStack {
            let from = stack.iter().position(|&i| i == at).unwrap_or_default();
            let mut cycle: Vec<&'static str> =
                stack[from..].iter().map(|&i| rows[i].name).collect();
            cycle.push(rows[at].name);
            gaps.push(Gap::DependencyCycle { cycle });
        }
        return;
    }
    state[at] = Visit::OnStack;
    stack.push(at);
    for next in producers_of(rows, frozen, row_writes, at) {
        walk(rows, frozen, row_writes, next, state, stack, gaps);
    }
    stack.pop();
    state[at] = Visit::Done;
}

/// 一对**可能同时开账**的行（§3.4 第 4 条的报告单元）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Overlap {
    /// 表内先后，不带裁决意味——谁先被派发由 `(prio, name, key)` 决，这里不判。
    pub rows: [&'static str; 2],
    /// 两边都当作前件的那些事实名（字典序，一对只列一条）。
    pub facts: Vec<&'static str>,
}

/// 「会不会真的同时命中」是**运行期**的问题（要读日志），静态只能回答「可能」。
/// 所以第 4 条不报错，只报告：报告让遮蔽在装载期可见，裁决仍归 [`drive`] 取最小。
///
/// 三条全成立才报，缺一条就是噪音：
///
/// 1. 两边的 `owes` 展开后有共同事实名（视图按 `reads` 展开）；
/// 2. `key` 是同一组字段——不同键的两笔债本来不会撞在同一笔上；
/// 3. `eq` 可满足：同一字段不许有两个不同字面量（`port=tool` 与 `port=model`
///    永不同时成立，报出来只会淹没真信号）。
pub fn overlaps(rows: &[Decl], frozen: &Frozen) -> Vec<Overlap> {
    let mut out = Vec::new();
    for (i, a) in rows.iter().enumerate() {
        for b in rows[i + 1..].iter() {
            if a.owes.key != b.owes.key || !eq_compatible(&a.owes, &b.owes) {
                continue;
            }
            let left = fact_names_of(a.owes.source, frozen);
            let right = fact_names_of(b.owes.source, frozen);
            // 视图的 `reads` 会把同一事实名列好几遍（每个字段一条），而报告问的是
            // 「这两行读不读同一些事实」——按名字去重，一对只出一条。
            let mut facts: Vec<&'static str> =
                left.into_iter().filter(|f| right.contains(f)).collect();
            facts.sort_unstable();
            facts.dedup();
            if !facts.is_empty() {
                out.push(Overlap {
                    rows: [a.name, b.name],
                    facts,
                });
            }
        }
    }
    out
}

/// 两个模式的 `eq` 能不能同时成立：共有字段上的字面量必须一致。
fn eq_compatible(a: &Pattern, b: &Pattern) -> bool {
    a.eq.iter().all(|(field, lit)| {
        b.eq.iter()
            .find(|(other, _)| other == field)
            .is_none_or(|(_, other)| other == lit)
    })
}

/// §3.4 第 5 条的端口侧：每个端口的协议格必须真的有人还。
///
/// 判据落在行的出口上，不落在代码路径里：[`PortDecl`] 说「这个端口派发前先落
/// `before`」，那么表里就得有一行的 `until` 能按这个端口关掉 `response` 与
/// `failed`。缺一格的形态是「应答回来了而债还在」——求解器每一幕都选中它，
/// 而它永远还不掉，正是第 5 条要拦的那类静默。
///
/// **端口字段名从声明读，不写在本文件里**：`ring.rs` 出现端口字段名就是 PTS-002
/// 判的「引擎读端口」（加端口 = 改引擎）。`FactDecl.port` 是它唯一的声明处。
pub fn check_ports(rows: &[Decl], frozen: &Frozen, ports: &[PortDecl]) -> Vec<Gap> {
    let mut gaps = Vec::new();
    let port_field = frozen.fact(EVENT_SENT).and_then(|f| f.port);
    for port in ports {
        if frozen.fact(port.before).is_none() {
            gaps.push(Gap::PortBeforeUndeclared {
                port: port.port,
                fact: port.before,
            });
        }
        for fact in [EVENT_RESPONSE, EVENT_FAILED] {
            let covered = port_field.is_some_and(|field| {
                rows.iter().flat_map(|d| d.until).any(|p| {
                    p.source == fact
                        && p.eq
                            .iter()
                            .any(|(name, lit)| *name == field && *lit == Lit::Str(port.port))
                })
            });
            if !covered {
                gaps.push(Gap::PortWithoutAnswerExit {
                    port: port.port,
                    fact,
                });
            }
        }
    }
    gaps
}

#[cfg(test)]
#[path = "ring.test.rs"]
mod tests;
