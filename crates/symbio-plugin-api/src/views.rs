//! 声明视图的**跨插件读口** —— 按名字读任意 owner 声明的纯折叠（DE-025 D-221）。
//!
//! ## 它补的是哪一格
//!
//! 视图（[`symbio_core::ViewDecl`]）本来就是 owner 自己交的纯函数：入参只有事实切片与
//! 时间戳，出参是 JSON。但过去**只有义务环的行引擎**在读它们（`Frozen::eval`），别的
//! 插件读不到——于是「会话统计想加一列便签数」只有两条路：在 core 里再写一份折叠
//! （同一事实两个 owner），或者依赖 notes 插件（跨插件依赖，禁止）。两条都会把
//! 「加能力」变成「改架构」。
//!
//! 本模块把读半边补上：
//!
//! ```text
//! owner 插件  ──ViewDecl + PLUGIN.yml 的 shared_views──▶ 名字公开
//! 读方插件    ──ViewRead::named(name)──▶ 冻结表里取 decl ──▶ read(events, now) ──▶ JSON
//! ```
//!
//! ## 三条边界
//!
//! - **只读公开的名字**：[`crate::view_names::SHARED_VIEWS`] 之外的视图不可见（默认私有）。
//!   公开是 owner 的显式动作，不是读方的猜测。
//! - **不抄口径**：`ver` / `cost` / 求值函数都从冻结声明表取。本模块不认识任何具体视图名。
//! - **超界不静默截断**：`events.len() > cost` ⇒ [`ViewUnavailable::OverBudget`]。读方必须
//!   知道这一格没算出来——返回一个「看起来正常」的旧值比缺一格更坏。
//!
//! ## 失败三态
//!
//! [`ViewUnavailable::NotSealed`]（装配表没冻结）/ [`ViewUnavailable::NotShared`]
//! （owner 没公开）/ [`ViewUnavailable::Unknown`]（公开了但没有声明实现）/
//! [`ViewUnavailable::OverBudget`]（切片超出声明成本）。四种都是**可诊断的缺席**，
//! 没有「静默空值」这一档。

use serde_json::Value;
use symbio_core::{Event, Frozen};

use crate::declarations::sealed_declarations;
use crate::view_names::{SharedViewDecl, SHARED_VIEWS};

/// 一条视图读不出来的原因。每一种都带足以定位的名字或数字。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewUnavailable {
    /// 声明表没有冻结（`seal` 报过缺口）：任何视图都不可读。
    NotSealed,
    /// 这个名字没有被 owner 在 `PLUGIN.yml` 的 `shared_views` 里公开。
    NotShared { name: String },
    /// owner 公开了名字，但冻结表里没有对应的 `ViewDecl`（清单与代码脱节）。
    Unknown { name: String },
    /// 事件切片超出声明的扫描成本上界——本格没算出来。
    OverBudget {
        name: &'static str,
        events: usize,
        cost: u32,
    },
}

impl ViewUnavailable {
    /// 稳定拼写的**原因词**（供读数出口落 JSON 用；人读文案另走本类型的
    /// [`std::fmt::Display`] 实现）。
    ///
    /// 四种缺席态是闭集：读方据此分支，不需要解析人话。新增一态必须同时改这里——
    /// 编译器会逼着改（`match` 无通配）。
    pub fn reason(&self) -> &'static str {
        match self {
            Self::NotSealed => "not_sealed",
            Self::NotShared { .. } => "not_shared",
            Self::Unknown { .. } => "unknown",
            Self::OverBudget { .. } => "over_budget",
        }
    }
}

impl std::fmt::Display for ViewUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotSealed => write!(f, "声明表未冻结：任何视图都不可读"),
            Self::NotShared { name } => {
                write!(f, "`{name}` 未在 owner 的 PLUGIN.yml 里登记 shared_views")
            }
            Self::Unknown { name } => write!(f, "`{name}` 已公开，但冻结声明表里没有它"),
            Self::OverBudget { name, events, cost } => write!(
                f,
                "`{name}` 超出声明扫描成本（{} 条事实 / 上界 {cost}）",
                events
            ),
        }
    }
}

/// 跨插件视图的读句柄：名字 + owner，求值仍走冻结声明表。
///
/// 句柄不持有值，也不缓存：每次 `read` 都以调用方给的切片与时间戳求值，口径与
/// owner 自己读时逐字一致。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewRead {
    name: &'static str,
    owner: &'static str,
}

impl ViewRead {
    /// 按名字取一条**已公开**的视图。未公开 / 未声明 / 装配表没冻结 ⇒ 相应的缺席态。
    pub fn named(name: &str) -> Result<Self, ViewUnavailable> {
        let sealed = sealed_declarations().ok();
        Self::named_in(sealed.as_deref(), name, true)
    }

    /// 视图名（owner 声明的原样拼写）。
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// 公开这条视图的 owner（与声明表的 owner 同形，如 `crates/symbio-plugin-notes`）。
    pub fn owner(&self) -> &'static str {
        self.owner
    }

    /// 口径版本（`ViewDecl::ver`）——读方据此判断自己理解的是哪一版。
    pub fn ver(&self) -> Result<u32, ViewUnavailable> {
        let sealed = sealed_declarations().ok();
        self.ver_in(sealed.as_deref())
    }

    /// 以 `now` 为 as-of 求值。超出声明的 `cost` ⇒ [`ViewUnavailable::OverBudget`]。
    pub fn read(&self, events: &[Event], now: i64) -> Result<Value, ViewUnavailable> {
        let sealed = sealed_declarations().ok();
        self.read_in(sealed.as_deref(), events, now)
    }

    /// 全量读：`now = i64::MAX`（落盘事实源没有「未来的事件」可滤）。
    pub fn read_all(&self, events: &[Event]) -> Result<Value, ViewUnavailable> {
        self.read(events, i64::MAX)
    }

    fn named_in(
        frozen: Option<&Frozen>,
        name: &str,
        require_shared: bool,
    ) -> Result<Self, ViewUnavailable> {
        if require_shared {
            let Some(shared) = SHARED_VIEWS.iter().find(|v| v.name == name) else {
                return Err(ViewUnavailable::NotShared {
                    name: name.to_string(),
                });
            };
            let frozen = frozen.ok_or(ViewUnavailable::NotSealed)?;
            if frozen.view(shared.name).is_none() {
                return Err(ViewUnavailable::Unknown {
                    name: name.to_string(),
                });
            }
            return Ok(Self {
                name: shared.name,
                owner: shared.owner,
            });
        }
        let frozen = frozen.ok_or(ViewUnavailable::NotSealed)?;
        let view = frozen.view(name).ok_or_else(|| ViewUnavailable::Unknown {
            name: name.to_string(),
        })?;
        Ok(Self {
            name: view.name,
            owner: frozen.view_owner(view.name).unwrap_or(""),
        })
    }

    fn ver_in(&self, frozen: Option<&Frozen>) -> Result<u32, ViewUnavailable> {
        let frozen = frozen.ok_or(ViewUnavailable::NotSealed)?;
        self.view(frozen).map(|v| v.ver)
    }

    fn view<'a>(&self, frozen: &'a Frozen) -> Result<&'a symbio_core::ViewDecl, ViewUnavailable> {
        frozen
            .view(self.name)
            .ok_or_else(|| ViewUnavailable::Unknown {
                name: self.name.to_string(),
            })
    }

    fn read_in(
        &self,
        frozen: Option<&Frozen>,
        events: &[Event],
        now: i64,
    ) -> Result<Value, ViewUnavailable> {
        let frozen = frozen.ok_or(ViewUnavailable::NotSealed)?;
        let view = self.view(frozen)?;
        if events.len() > view.cost as usize {
            return Err(ViewUnavailable::OverBudget {
                name: view.name,
                events: events.len(),
                cost: view.cost,
            });
        }
        Ok((view.eval)(events, now))
    }
}

/// 全部公开视图（清单声明序，按名字排序）。
pub fn shared() -> &'static [SharedViewDecl] {
    SHARED_VIEWS
}

/// 一次读的便捷入口：`read_from(name, events, now)` = `ViewRead::named(name)?.read(..)`。
pub fn read_from(name: &str, events: &[Event], now: i64) -> Result<Value, ViewUnavailable> {
    ViewRead::named(name)?.read(events, now)
}

/// 全量读的便捷入口（`now = i64::MAX`）。
pub fn read_all_from(name: &str, events: &[Event]) -> Result<Value, ViewUnavailable> {
    read_from(name, events, i64::MAX)
}

#[cfg(test)]
#[path = "views.test.rs"]
mod tests;
