//! 跨插件报表的登记面 —— owner 自己交「哪一格、怎么排版、读哪几条视图」。
//!
//! ## 它替换的是什么
//!
//! 此前一条视图要给出品读数，owner 得在 `PLUGIN.yml` 写 `shared_views`，生成器再把
//! 名字镜像进 `plugin-api/src/view_names.rs`；读方（`session/stats`）要么 import
//! 生成的常量、要么照名字猜空形状。名字因此作为业务身份住进了机制层，且读方仍要
//! 「知道有哪几条」才能出列（DE-024 D-387）。
//!
//! 现在登记的是**报表描述符**：owner 在自己 crate 的模块作用域 `submit_reports!`
//! 一条 [`ReportDecl`]，名字只出现在 owner 自己的常量里；读方只遍历 [`reports()`]，
//! 按 `views` 逐个读、把整片值交回 `render`。空形状、`own` 行、`cap` 截断、按档并表
//! 全在 owner 的 `render` 里——读方不再替 owner 猜形状。
//!
//! ## 顺序是判据
//!
//! [`reports()`] 按 `(order, slot)` 排序：`order` 相同按 `slot` 字典序，`slot` 全局唯一
//! 由装配期判据保证（见下）。排序在读取侧做而不是装配期做一次，与
//! [`crate::settle::contributors`] / [`crate::context::providers`] 同一条纪律：本仓不持有
//! 第二个可变注册表。同一份登记连跑两次，输出必须逐字节相同。
//!
//! ## 装配期 fail-closed
//!
//! [`validate`] 一次判出四类登记错误：
//!
//! - `slot` 为空或重复——两条报表撞了同一个键，读数会静默串列；
//! - `views` 为空或含空名——报表没有读数来源；
//! - 同一视图进两条报表——两处各算一遍，口径静默漂移；
//! - `fleet` 显式写成空串——落点为空时写不进去，等于静默丢实例级读数。
//!
//! [`reports()`] 在读到非法登记时当场失败（fail-closed），不返回一份「能读多少算多少」
//! 的残缺表。与 [`crate::port::check_port_name`] 同一条取舍：带病启动比崩掉更难诊断。

use std::fmt;

use serde_json::Value;
use symbio_kernel::present::ReadingSection;

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
    /// 拥有这条报表的插件名（与 `ViewDecl` 的 owner 同源）。
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
/// 不读 `inventory`、不排序，便于单测直接构造夹具——`reports()` 的排序与 panic
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

/// 按 `(order, slot)` 排序；`order` 相同按 `slot` 字典序（`slot` 全局唯一，不会退化
/// 成链接顺序）。
fn sorted(mut all: Vec<&ReportDecl>) -> Vec<&ReportDecl> {
    all.sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.slot.cmp(b.slot)));
    all
}

/// 已链接进来的全部报表，按 `(order, slot)` 排序；非法登记当场失败。
///
/// 排序与 fail-closed 都在读取侧做：表是 `inventory` 的编译期集合，本仓不持有第二个
/// 可变注册表（与 `settle::contributors` / `context::providers` 同一条纪律）。
pub fn reports() -> Vec<&'static ReportDecl> {
    let all = sorted(inventory::iter::<ReportDecl>().collect());
    let errors = validate(&all);
    if !errors.is_empty() {
        // panic-allow crates/symbio-plugin-api/src/reports.rs::reports: 装配期 fail-closed——slot 重复 / 视图重复 / 空 slot / 空 views 会让读数静默丢列或串列，带病启动比当场响更难诊断（D-387）。
        panic!(
            "报表登记不合法：\n{}",
            errors
                .iter()
                .map(|e| format!("- {e}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
    all
}

#[cfg(test)]
#[path = "reports.test.rs"]
mod tests;
