//! 义务环的**表与求解**（T-222a）：模式 / 行 / 债 / `owed` / `drive`。
//!
//! ## 一行是数据，不是函数
//!
//! 旧策略表（`loop_registry`，T-222b 已退役）的一行是 `fn(&LoopState) -> Option<Capsule>`
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
//! 不读配置、墙钟、内存缓存（[ADR-115](../../../../docs/010-adr/115.md) 决策 1）。
//!
//! ## 模式只做等值
//!
//! `eq` 是 `(字段, 字面量)` 列表，`key` 是参与债标识的字段名。没有否定、没有
//! 比较、没有函数——更复杂的条件要么折成视图字段，要么留在执行体里。这是
//! 「表里零代码」的类型形态（PL-016 判据 2）。
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
//! 求解器与行表不变（PL-016 §6 判据 1，执行方 `scripts/port-surface-audit.mjs`）。

use std::collections::HashSet;

use serde_json::Value;

use super::Frozen;
use crate::symbio_core::{Event, LatencyTier};

/// 模式里的一个字面量。只覆盖 JSON 的三个标量型——键绑定也只认它们。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Lit {
    Bool(bool),
    Uint(u64),
    Str(&'static str),
}

/// 一条模式：`source` 上字段等值 + 键绑定。
#[derive(Debug, Clone, Copy)]
pub(crate) struct Pattern {
    /// 事实名或视图名（二者共用一个名字空间，见 [`Frozen`]）。
    pub(crate) source: &'static str,
    /// 绑定债标识的字段。空切片 ⇒ 这条源至多一笔债（例如按轮收束的格）。
    pub(crate) key: &'static [&'static str],
    /// 字段等值约束。
    pub(crate) eq: &'static [(&'static str, Lit)],
}

/// 义务环的一行。
#[derive(Debug, Clone)]
pub(crate) struct Decl {
    /// 行的身份：日志、冲突报错、对拍都按它指认。
    pub(crate) name: &'static str,
    /// **全局**裁决顺序：`(prio, name, key)` 升序，先小者胜。
    ///
    /// 与旧表的格内 `prio` 不同：义务环只有一个环、一次取最小，跨行必须可比。
    pub(crate) prio: u8,
    /// 执行体名。`None` ⇒ 恒 blocked、永不选中（「在等」的唯一声明位）。
    pub(crate) by: Option<&'static str>,
    /// 这一行动作允许的时延档。
    pub(crate) tier: LatencyTier,
    pub(crate) owes: Pattern,
    /// 集合展开：`Some((list_field, element_key_field))` ⇒ `owes` 实例的
    /// `list_field` 是数组，**逐元素**开一笔债（PL-017 §3.3；`FactDecl.expand`
    /// 是它在事实侧的另一半）。
    ///
    /// 展开后键绑定**对元素求值**：`owes.key` 里每个字段都从元素上取。
    /// `element_key_field` 必须出现在 `owes.key` 里——它是元素的身份字段，
    /// 缺了它只能开一堆同键债，`until` 无从分辨还了哪一笔。
    ///
    /// 只有 `owes` 展开：`until` / `unless` 按键命中，键已经唯一，再展开只会
    /// 把同一笔出口读成多笔。
    pub(crate) expand: Option<(&'static str, &'static str)>,
    /// 任一命中（同键）即视为还清。
    pub(crate) until: &'static [Pattern],
    /// 任一命中（同键）即挂起。
    pub(crate) unless: &'static [Pattern],
}

/// 一个**端口**的声明（T-223）：适配器协议里那几项每端口不同、又必须被引擎与
/// 租约看护者读到的东西。
///
/// ## 为什么不把这些字段塞进 `Decl`
///
/// `before` / `lease_ms` / `max_attempts` 对**同一个端口的每一行**都一样，写进
/// 行里就是三份副本（dispatch / await / lease），而副本必然漂移。端口级的东西
/// 按端口声明一次：加一个端口 = 加一条 `PortDecl` + 适配器 + 2 行模板（PL-016 §6
/// 判据 1）。
///
/// ## 为什么在 `ring` 而不在插件
///
/// 它与 [`Decl`] 是同一张表的两半：行说「什么时候欠、什么算还」，端口说
/// 「派发时先落什么、等多久、试几次」。租约看护者与派发适配器都只读这张表，
/// 不看对方。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PortDecl {
    /// 端口名——`request` / `sent` / … 载荷里的 `port` 取值。
    pub(crate) port: &'static str,
    /// 引擎在运行执行体**之前**落的那个事实名（协议族里的 `sent`）。
    ///
    /// 搬运行者不写它：它的 `run` 可能**不产生事实就返回**，依赖它自觉 ⇒
    /// 派发行永远不还清，而没有任何东西会红（PL-016 §2.3；T-223 判据 2）。
    pub(crate) before: &'static str,
    /// 租约：`sent` 之后多久没有应答算「卡住」（毫秒）。
    pub(crate) lease_ms: u64,
    /// 最多尝试几次（含首次）。到顶之后租约看护者判死而不是再重试。
    pub(crate) max_attempts: u64,
    /// 同一批次的子债是否允许并发派发（PL-017 §3.3）。
    ///
    /// **声明与策略分开**：这里只说「这个端口允许同批并发」；具体哪几条能同时在途
    /// 仍由执行层的风险档决定（`tools/dispatch/parallel.rs`）。`false` 是硬闸——
    /// 执行层不得以风险档为由绕过它。
    pub(crate) concurrent: bool,
}

/// 按端口名取声明。`None` = 没有这个端口（漏声明，不是默认值）。
pub(crate) fn port_of<'a>(ports: &'a [PortDecl], name: &str) -> Option<&'a PortDecl> {
    ports.iter().find(|p| p.port == name)
}

/// 一笔债的标识：模式 `key` 字段的 `(名字, 值)` 列表，按声明顺序。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Key(pub(crate) Vec<(String, String)>);

impl Key {
    /// 人对得上的形状（日志与断言用）。
    pub(crate) fn trace(&self) -> String {
        self.0
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(",")
    }
}

/// 一笔当前欠着的账。
#[derive(Debug, Clone)]
pub(crate) struct Debt<'a> {
    pub(crate) decl: &'a Decl,
    pub(crate) key: Key,
    /// `by = None` 或有 `unless` 命中：存在，但引擎不还。
    pub(crate) blocked: bool,
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
pub(crate) fn owed<'a>(
    rows: &'a [Decl],
    frozen: &Frozen,
    events: &[Event],
    now: i64,
) -> Vec<Debt<'a>> {
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
/// `turn_exec::drive_steps` 直接调用它；执行半边（跑执行体、追加事实、
/// 无进展守卫）在派发器里，选择规则不再变。
///
/// 按行点问（`plugins::loop::loop_rows::owes`）仍留给「刚写完 request，这一行
/// 还欠不欠」的单点断言；**单环驱动**（一次取下一笔、按 `prio` 裁决）是
/// 调度器唯一的选债口（ADR-118 决策 1–2）。
pub(crate) fn drive<'a>(
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
/// 集合展开时 `eq` 已经拦在**外层实例**上（ADR-116 决策 3），元素键只取身份字段；
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

/// 装载期的行检查（PL-016 §3.4 的第 2 / 7 条）。
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
///   那条事实的 `FactDecl.expand`——求解器会去读一个事实自己都不承认的字段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Gap {
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
}

pub(crate) fn check_rows(rows: &[Decl], frozen: &Frozen) -> Vec<Gap> {
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
        for p in std::iter::once(&decl.owes)
            .chain(decl.until)
            .chain(decl.unless)
        {
            if frozen.fact(p.source).is_none() && frozen.view(p.source).is_none() {
                gaps.push(Gap::UnknownSource {
                    row: decl.name,
                    source: p.source,
                });
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
    gaps
}

#[cfg(test)]
#[path = "ring.test.rs"]
mod tests;
