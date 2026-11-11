//! 跨插件报表的登记面 —— owner 自己交「哪一格、怎么排版、读哪几条视图」。
//!
//! ## 为什么登记面在 owner 而不是读方
//!
//! 登记的是**报表描述符**：owner 在自己 crate 的模块作用域 `submit_reports!`
//! 一条 [`ReportDecl`]，名字只出现在 owner 自己的常量里；读方只遍历 [`reports()`]，
//! 按 `views` 逐个读、把整片值交回 `render`。空形状、`own` 行、`cap` 截断、按档并表
//! 全在 owner 的 `render` 里——读方不再替 owner 猜形状，也不必先「知道有哪几条」
//! 才能出列（DE-024 D-387）。
//!
//! ## 顺序是判据
//!
//! [`reports()`] 按 `(order, slot)` 排序：`order` 相同按 `slot` 字典序，`slot` 全局唯一
//! 由登记判据保证（见下）。排序在读取侧做而不是装配期做一次，与
//! [`crate::plugin::settle::contributors`] / [`crate::plugin::context::providers`] 同一条纪律：本仓不持有
//! 第二个可变注册表。同一份登记连跑两次，输出必须逐字节相同。
//!
//! ## 登记判据返回错误，不 panic
//!
//! [`validate`] 一次判出四类登记错误：
//!
//! - `slot` 为空或重复——两条报表撞了同一个键，读数会静默串列；
//! - `views` 为空或含空名——报表没有读数来源；
//! - 同一视图进两条报表——两处各算一遍，口径静默漂移；
//! - `fleet` 显式写成空串——落点为空时写不进去，等于静默丢实例级读数。
//!
//! ## 还有一类：出口 owner 与折账 owner 对账（`D-465`）
//!
//! 上面四类只看登记表自己；[`check_view_owners`] 还要看**冻结声明表**——它判
//! 「出这条列的人，是不是折这条账的人」。两处的输入不同（一张 inventory 表 vs 一张
//! 冻结表），所以是两个函数而不是一个：`validate` 保持纯函数（单测直接构造夹具），
//! 对账由唯一同时看得见两张表的地方调用——装配 [`crate::declarations::assemble_declarations`]。
//!
//! [`reports()`] 把非法登记作为 `Err` 交回调用方（与 [`crate::declarations::sealed_declarations`]
//! 同一形状）：装载期由部署面报出，读侧由读数方决定降级还是失败。`Gap` 是内核声明 /
//! 环的词汇，报表是出品装配面——不把错误类型塞回内核，也不拿 panic 替调用方做决定。

use std::fmt;

use serde_json::Value;
use symbio_kernel::event::Event;
use symbio_kernel::present::ReadingSection;

use crate::declarations::sealed_declarations;
use crate::plugin::views::ViewUnavailable::{self, NotSealed, Unknown};

/// 读方在装配这一刻能提供的上下文。
///
/// 只放**读方知道的、owner 不知道**的三个值：主体、容量、读侧闸的答案。owner 的
/// `render` 用它们产出自己的形状（声誉的 `own` 行、就绪集的截断、无权限时的空表），
/// 读方不解释这些形状。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReportCtx<'a> {
    /// 本会话主体：`reputation.ledger` 的 `own` 行取谁。
    pub principal: &'a str,
    /// 本轮调度容量：`task.readyset` 的截断预算。
    pub cap: usize,
    /// 读侧闸的答案：`false` = 不给你看，不是读失败（两者的空形状可以不同）。
    pub readable: bool,
}

/// 一条报表的登记：owner / 顶层键 / 次序 / 要读的视图 / 排版。
///
/// `views` 里的每个名字都来自 owner 自己的常量（与 `ViewDecl::name` 同一个 `const`），
/// 全仓只出现一次——`SV-001` 守的就是「名字不许在两个插件 crate 各写一份」。
#[derive(Debug, Clone, Copy)]
pub struct ReportDecl {
    /// 拥有这条报表的插件**短名**（与目录名同形，如 `cost` / `model`）。
    ///
    /// 与 `ViewDecl` 的 owner **不同形**：那边是 crate 路径（`crates/symbio-plugin-X`），
    /// 因为 `declare(owner, …)` 收的是「交表方」的路径标识；这里是给人读的标签。
    /// 两种拼写不统一（既成形状，不该为了判据搬家），对账靠归一：装配期
    /// [`check_view_owners`] 把视图 owner 折成同一个短名（[`owner_short`]），再与这里
    /// 逐条比对——出这条列的人必须就是折这条账的人（`D-465`，`D-463` 第三级）。
    pub owner: &'static str,
    /// `session/stats` 的顶层键；全局唯一，进 JSON 顶层就是出品契约（`SV-003`）。
    pub slot: &'static str,
    /// 面板段顺序；相同时按 `slot` 字典序。
    pub order: u8,
    /// 这条报表要读的视图，按 owner 自己的次序；同一个视图只许进一条报表。
    pub views: &'static [&'static str],
    /// 实例级落点（叶路径，如 `axes.conation` / `by_tier`）；`None` = 不参与聚合。
    pub fleet: Option<&'static str>,
    /// 会话级排版：整片视图值进、owner 的形状出（空形状也在这里）。
    pub render: fn(&[Value], &ReportCtx<'_>) -> Value,
    /// 实例级排版：各视图先走 `ViewDecl::agg`，再交给它。
    pub fleet_render: Option<fn(&[Value], &ReportCtx<'_>) -> Value>,
    /// 面板段：返回 `None` = 这条报表不进面板（仍有 JSON 列）。
    pub panel: Option<fn(&Value, &ReportCtx<'_>) -> ReadingSection>,
}

inventory::collect!(ReportDecl);

/// 提交一条或多条报表。在插件 crate 的模块作用域调用一次即可。
///
/// ```ignore
/// symbio_plugin_api::submit_reports!([
///     ReportDecl { owner: "cost", slot: "cost", order: 40, views: &[COST_LEDGER], .. },
/// ]);
/// ```
#[macro_export]
macro_rules! submit_reports {
    ([$($r:expr),* $(,)?]) => {
        $( $crate::inventory::submit! { $r } )*
    };
    ($r:expr) => {
        $crate::inventory::submit! { $r }
    };
}

/// 装配期能判出的登记错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReportError {
    /// `slot` 为空：没有顶层键可写。
    EmptySlot { owner: &'static str },
    /// `views` 为空：报表没有读数来源。
    EmptyViews {
        owner: &'static str,
        slot: &'static str,
    },
    /// `views` 里有一项是空串。
    EmptyViewName {
        owner: &'static str,
        slot: &'static str,
        index: usize,
    },
    /// `fleet` 显式写成空串：落点为空，写不进去。
    EmptyFleet {
        owner: &'static str,
        slot: &'static str,
    },
    /// 同一个 `slot` 被两条报表写：读数会静默串列。
    DuplicateSlot {
        slot: &'static str,
        first: &'static str,
        second: &'static str,
    },
    /// 同一个视图进两条报表：两处各算一遍，口径静默漂移。
    DuplicateView {
        view: &'static str,
        first_slot: &'static str,
        second_slot: &'static str,
    },
}

impl fmt::Display for ReportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptySlot { owner } => {
                write!(f, "`{owner}` 的报表 `slot` 为空")
            }
            Self::EmptyViews { owner, slot } => {
                write!(f, "`{owner}` 的报表 `{slot}` 没有 `views`")
            }
            Self::EmptyViewName { owner, slot, index } => {
                write!(f, "`{owner}` 的报表 `{slot}` 第 {index} 条视图名为空")
            }
            Self::EmptyFleet { owner, slot } => {
                write!(f, "`{owner}` 的报表 `{slot}` 把 `fleet` 写成了空串")
            }
            Self::DuplicateSlot {
                slot,
                first,
                second,
            } => {
                write!(f, "`slot` `{slot}` 被 `{first}` 与 `{second}` 同时登记")
            }
            Self::DuplicateView {
                view,
                first_slot,
                second_slot,
            } => {
                write!(
                    f,
                    "视图 `{view}` 同时进了报表 `{first_slot}` 与 `{second_slot}`"
                )
            }
        }
    }
}

/// 纯函数判据：返回全部登记错误；空数组即成立。
///
/// 不读 `inventory`、不排序，便于单测直接构造夹具——`reports()` 的排序与 `Result`
/// 都只是对它的调用。
pub fn validate(reports: &[&ReportDecl]) -> Vec<ReportError> {
    let mut errors = Vec::new();
    let mut slots: std::collections::BTreeMap<&'static str, &'static str> =
        std::collections::BTreeMap::new();
    let mut views: std::collections::BTreeMap<&'static str, &'static str> =
        std::collections::BTreeMap::new();

    for r in reports {
        if r.slot.is_empty() {
            errors.push(ReportError::EmptySlot { owner: r.owner });
        } else if let Some(first) = slots.insert(r.slot, r.owner) {
            errors.push(ReportError::DuplicateSlot {
                slot: r.slot,
                first,
                second: r.owner,
            });
        }
        if r.views.is_empty() {
            errors.push(ReportError::EmptyViews {
                owner: r.owner,
                slot: r.slot,
            });
        }
        if r.fleet == Some("") {
            errors.push(ReportError::EmptyFleet {
                owner: r.owner,
                slot: r.slot,
            });
        }
        for (index, view) in r.views.iter().enumerate() {
            let view: &'static str = view;
            if view.is_empty() {
                errors.push(ReportError::EmptyViewName {
                    owner: r.owner,
                    slot: r.slot,
                    index,
                });
                continue;
            }
            if let Some(first_slot) = views.insert(view, r.slot) {
                errors.push(ReportError::DuplicateView {
                    view,
                    first_slot,
                    second_slot: r.slot,
                });
            }
        }
    }

    errors
}

/// 交表方标识的**短名**：`crates/symbio-plugin-model` → `model`。
///
/// 两种拼写都留着（`D-465`）：`ViewDecl` 的 owner 是 `declare` 的入参，要能 grep 到目录；
/// `ReportDecl.owner` 是给人读的标签，进错误信息。判据不该要求形状统一——那样会把
/// 「对账」换成一次全仓改名，改完还是没人查。归一是一行 `strip_prefix`，比对才是判据。
///
/// `kernel`、以及任何不在 `crates/` 下的短名原样通过：它们本来就只有一种拼写。
pub fn owner_short(owner: &str) -> &str {
    let last = owner.rsplit('/').next().unwrap_or(owner);
    last.strip_prefix("symbio-plugin-").unwrap_or(last)
}

/// 出口对账：**出这条列的人，必须就是折这条账的人**（`D-465`，`D-463` 第三级）。
///
/// ## 它补的是哪一格
///
/// [`validate`] 只看登记表自己，于是 `ReportDecl.owner` 此前只是错误信息里的一个词：
/// A 插件把 B 插件的视图登记成自己的出口、视图搬走而报表没跟、视图名打错一个字母——
/// 三种都过 `validate`，运行期读到的都是 `Null`，而 `Null` 在读侧本来就有合法含义
/// （空账 / 这一轮没有），于是没有任何东西会红。
///
/// ## 为什么输入是「问名字」而不是冻结表
///
/// 判据只需要一个问题的答案：「这个名字的 owner 是谁」。把问题做成闭包，判据就留在
/// `symbio-sdk`（报表词汇的家里）而不必认识 [`symbio_kernel::decl::Frozen`]；唯一的
/// 生产调用点是装配期（`assemble_declarations`），那里同时握着两张表。
///
/// ## 空名不在这里判
///
/// `views` 里的空串归 [`ReportError::EmptyViewName`]（[`validate`] 已判）：这里跳过，
/// 否则同一处错误报两遍，读侧的数会随判据数量变化。
///
/// ## 为什么交回的是人读的句子，不是 `ReportError`
///
/// `ReportError` 是**封闭取值面**（`CS-009` 按变体数守它）：每加一个变体，内核 / 契约
/// 的公共数据面就宽一格。而这两条诊断**只有人读**——唯一的消费者是装配期的
/// `assert!` 消息，没有任何一处会 `match` 它们的种类。为一条进日志的字符串加两个变体，
/// 是拿契约面的宽度换一行文本；于是这里交 [`String`]，与 `D-462`「不设公共词表」同一条
/// 纪律：**没有第二个消费者，就不给它造形状。**
pub fn check_view_owners(
    reports: &[&ReportDecl],
    view_owner: impl Fn(&str) -> Option<&'static str>,
) -> Vec<String> {
    let mut out = Vec::new();
    for r in reports {
        for view in r.views.iter().copied().filter(|v| !v.is_empty()) {
            match view_owner(view) {
                None => out.push(format!(
                    "报表 `{}` 要读的视图 `{view}` 不在冻结声明表里（没有 owner 交这条 ViewDecl）",
                    r.slot
                )),
                Some(owner) if owner_short(owner) != r.owner => out.push(format!(
                    "报表 `{}` 署 `{}`，但它读的视图 `{view}` 由 `{owner}` 折——\
                     出这条列的人必须就是折这条账的人",
                    r.slot, r.owner
                )),
                Some(_) => {}
            }
        }
    }
    out
}

/// 按 `(order, slot)` 排序；`order` 相同按 `slot` 字典序（`slot` 全局唯一，不会退化
/// 成链接顺序）。
fn sorted(mut all: Vec<&ReportDecl>) -> Vec<&ReportDecl> {
    all.sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.slot.cmp(b.slot)));
    all
}

/// 已链接进来的全部报表，按 `(order, slot)` 排序；非法登记交回错误清单。
///
/// 排序与判据都在读取侧做：表是 `inventory` 的编译期集合，本仓不持有第二个可变注册表
/// （与 `settle::contributors` / `context::providers` 同一条纪律）。结果按进程缓存一次，
/// 与 `sealed_declarations()` 同形——非法登记是装配错误，不该每次读数重判一遍。
pub fn reports() -> Result<&'static [&'static ReportDecl], &'static [ReportError]> {
    static SEALED: std::sync::OnceLock<Result<Vec<&'static ReportDecl>, Vec<ReportError>>> =
        std::sync::OnceLock::new();
    let sealed = SEALED.get_or_init(|| {
        let all = sorted(inventory::iter::<ReportDecl>().collect());
        let errors = validate(&all);
        if errors.is_empty() {
            Ok(all)
        } else {
            Err(errors)
        }
    });
    match sealed {
        Ok(all) => Ok(all.as_slice()),
        Err(errors) => Err(errors.as_slice()),
    }
}

/// 读一条已声明视图的整片值：名字的唯一出处是 owner 的 `const`（`SV-001`）。
///
/// 读不到（声明表没冻结 / 名字不在表里）⇒ `Null` 占位并出声：owner 的 `render`
/// 据此交自己的空形状——空形状住在 owner 那里，读方不替它猜。
pub fn read_view(name: &str, snapshot: &[Event]) -> Value {
    let frozen = match crate::declarations::sealed_declarations() {
        Ok(frozen) => frozen,
        Err(_) => {
            crate::plugin_warn!(
                "symbio::reports",
                "[reports] 声明表未冻结：视图 `{name}` 按空读数交"
            );
            return Value::Null;
        }
    };
    match frozen.view(name) {
        Some(view) => (view.eval)(snapshot, i64::MAX),
        None => {
            crate::plugin_warn!(
                "symbio::reports",
                "[reports] 视图 `{name}` 不在冻结声明表里：按空读数交"
            );
            Value::Null
        }
    }
}

/// 一条已登记视图的读数**加口径版本**：供诊断列（`capability_views`）用。
///
/// 与 [`read_view`] 同一条路径（冻结声明表 + 名字）：报表登记就是公开（`D-387`），
/// 名字由 owner 的 `ReportDecl` 交出，读方不需要它再出现在第二份清单里。读不到 ⇒
/// 缺席原因（两种态之一），不是 `Null`——诊断列要说得出为什么没数。
pub fn read_view_with_ver(name: &str, snapshot: &[Event]) -> Result<(u32, Value), ViewUnavailable> {
    let frozen = sealed_declarations().map_err(|_| NotSealed)?;
    let view = frozen.view(name).ok_or_else(|| Unknown {
        name: name.to_string(),
    })?;
    Ok((view.ver, (view.eval)(snapshot, i64::MAX)))
}

/// 把**已登记的全部报表**读成一张表：键 = `slot`，值 = owner 的 `render` 排好的整条读数。
///
/// ## 它是动态列的**唯一填充者**
///
/// `session/stats` 的报表列不再由读方逐列声明：本函数遍历 [`reports()`]，加一条登记
/// 就多一格。`SessionStats::reports` 用 `#[serde(flatten)]` 把它们抬到出口顶层，于是
/// 「出口顶层键集合 == 已登记 `slot` 集合」是构造保证——全装配下的回归判据见
/// `crates/symbio` 的 `report_columns_acceptance`。
///
/// ## 为什么住在机制臂而不是读方
///
/// 报表登记表与「读全部」是同一个类型闭包：登记要 `inventory::collect!`，读取要
/// 冻结声明表（[`crate::declarations::sealed_declarations`]）——两者都在本 crate。
/// 装配宿主经门面转出的模块名调用，读方（可卸载的 `session`）因此不需要在 crate 根
/// 暴露符号（E-014 / EX-004）。
///
/// 登记表本身不合法（[`reports()`] 交回错误）时返回空表并逐条出声：降级还是失败由
/// 调用方决定（读侧不替装配面 panic）。
pub fn read_all(
    snapshot: &[Event],
    ctx: &ReportCtx<'_>,
) -> std::collections::BTreeMap<String, Value> {
    let mut out = std::collections::BTreeMap::new();
    let reports = match reports() {
        Ok(reports) => reports,
        Err(errors) => {
            for why in errors {
                crate::plugin_warn!(
                    "symbio::reports",
                    "[reports] 报表登记不合法（{why}）：整批读数读不出来"
                );
            }
            return out;
        }
    };
    for report in reports {
        let views: Vec<Value> = report
            .views
            .iter()
            .map(|name| read_view(name, snapshot))
            .collect();
        out.insert(report.slot.to_string(), (report.render)(&views, ctx));
    }
    out
}

#[cfg(test)]
#[path = "reports.test.rs"]
mod tests;
