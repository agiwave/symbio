//! 行执行体 —— 能力在**轮级**的写口（DE-025 D-241）。
//!
//! ## 它补的是哪一格
//!
//! 能力四元组里事实、视图、行都已是插件可交的，只有执行体不是：`ActorEntry` /
//! `Action` 住在 `loop` 的 `pub(crate)` 面里，且 `Action` 收 `&mut Ctx`（整轮可变
//! 状态）。给每个能力加一条 `ActorEntry` 只是把 `loop` 的分支换成 `loop` 的表——
//! 绑定表随能力数增长，可扩展性没有增加。
//!
//! 本模块补一个**共用**的执行体 `plugin_row`：
//!
//! ```text
//! 能力插件 ──submit_row!──▶ inventory（编译期随二进制生效）
//! 义务环选债 ──债实例 + 本行事实切片（纯值）──▶ RowHandler::run
//! 宿主唯一写口 ──FactDraft::into_event──▶ 事实源
//! ```
//!
//! 执行体不认识任何能力名：它按 `Decl.by = plugin_row` 找到这一行的实现，把纯值
//! 交进去，把 `Vec<FactDraft>` 拿回来。顺序由既有的 `(prio, name, key)` 决定，
//! 失败由既有的账本守卫与步数上界兜底（D-123 / D-115）。
//!
//! ## 为什么只收纯值
//!
//! `&mut Ctx` 里有本轮草稿、端口与轮首状态：把它开给能力，等于让能力参与「下一步
//! 跑什么」的判断，顺序与失败都不可判（D-241 的「被否」第一条）。这里交出去的是
//! 一笔债的身份（`row` + `key`）、轮锚（`turn` / `now_ms` / `user_seq`）、写事件的
//! 主体，以及**本轮**的事实切片——能力要判断什么，从事实里读。
//!
//! ## 写入的声明在实现手里，不在 `ActorDecl` 里
//!
//! 一行写哪些领域格随能力而变，`ActorDecl` 是宿主侧的静态投影，装不下每行不同的
//! 名单。于是 [`RowHandler::writes`] 是那一行的写入声明：装配期把它并进
//! `plugin_row` 的 `ActorDecl`（装载检查看得见），派发时按**这一行**的名单对账
//! （运行期账本守卫仍要求「声明 ⊆ 强制」，D-123）。

use symbio_core::Event;

use crate::effects::FactDraft;

/// 通用行执行体的名字：`Decl.by` 与 `ActorDecl.name` 都用它。
///
/// 它是**宿主词汇**（与 `behaviors` 同一层）：能力包写这个名字，宿主认这个名字；
/// 两边都不需要认识对方的具体身份。
pub const PLUGIN_ROW: &str = "plugin_row";

/// 一笔债的实例外加本行看得见的事实切片——执行体的唯一入参。
///
/// 字段全部是纯值：借用生命周期只覆盖一次 `run` 调用，执行体拿不走、也存不下。
#[derive(Debug, Clone, Copy)]
pub struct RowRequest<'a> {
    /// 行名（`Decl.name`）：实现据此确认「这笔债是我的」。
    pub row: &'static str,
    /// 债键：`Decl.owes.key` 各字段的 `(名字, 值)`，按声明顺序。
    pub key: &'a [(String, String)],
    /// 本轮序号（跨轮唯一的事件号词干从这里来）。
    pub turn: u64,
    /// 本次 drive 的 `now`（幕锚那条事实的 `ts`，D-115）：按时间判的前件读它。
    pub now_ms: i64,
    /// 写事件的 `actor`（本会话主体）。
    pub principal: &'a str,
    /// 本轮开口格的 seq——派生事实的溯源锚（I2）。
    pub user_seq: u64,
    /// **本轮**的事实切片（不早于本轮第一条事件）：判断只从事实里读。
    pub facts: &'a [Event],
}

impl RowRequest<'_> {
    /// 债键里某个字段的取值。键由 `Decl.owes.key` 决定，这里不猜顺序。
    pub fn key_of(&self, field: &str) -> Option<&str> {
        self.key
            .iter()
            .find(|(k, _)| k == field)
            .map(|(_, v)| v.as_str())
    }

    /// 轮锚词干（`t{turn}`）——事件号的跨轮唯一性从这里来。
    pub fn anchor(&self) -> String {
        format!("t{}", self.turn)
    }
}

/// 能力对自己的**一行**的实现。
///
/// 一个实现 = 一行 + 一次 `submit_row!`；加一行只改能力包。行仍然由能力的
/// `Decl` 声明「什么时候欠、什么算还」，本实现只回答「欠着的时候写什么」。
pub trait RowHandler: Sync {
    /// 归属（日志与装配顺序用）：同一条债只该有一个实现。
    fn owner(&self) -> &'static str;

    /// 本实现管辖的行名，必须是本能力自己 `Decl.name` 里的一条。
    fn row(&self) -> &'static str;

    /// 本行**直接落格**的领域事实名。
    ///
    /// 装配期并进 `plugin_row` 的 `ActorDecl.writes`（装载期第 6 条与依赖图看得见）；
    /// 派发时按这一行的名单对账（运行期账本守卫，D-123 决策 2）。写名单之外的事实
    /// 会让这一步以 `undeclared-write` 收束，而不是静默放行。
    fn writes(&self) -> &'static [&'static str];

    /// 这一笔债的产物：待落格的事实草稿。空 = 本步没有效果。
    ///
    /// **必须保证推进**：返回空而债仍开着，环会在同一个债上反复选中它，直到步数
    /// 上界把这一轮判成 `step_limit`。因此「这一笔不该记账」要在行的 `owes` 里
    /// 排除（用 `eq` 收窄），而不是在实现里返回空。
    fn run(&self, req: &RowRequest<'_>) -> Vec<FactDraft>;
}

/// 一条行实现的链接期提交。
pub struct RowDecl {
    pub handler: &'static dyn RowHandler,
}

inventory::collect!(RowDecl);

/// 提交一条行实现。在插件 crate 的模块作用域调用一次即可。
///
/// ```ignore
/// symbio_plugin_api::submit_row!(NoteIndex);
/// ```
#[macro_export]
macro_rules! submit_row {
    ($h:expr) => {
        $crate::inventory::submit! {
            $crate::rows::RowDecl { handler: &$h }
        }
    };
}

/// 已链接进来的全部行实现，按 `(row, owner)` 排序。
fn handlers() -> Vec<&'static dyn RowHandler> {
    let mut all: Vec<&'static dyn RowHandler> =
        inventory::iter::<RowDecl>().map(|d| d.handler).collect();
    all.sort_by_key(|h| (h.row(), h.owner()));
    all
}

/// 这一行的实现。`None` = 没有能力认领它（装载期据此判红，见 `EX-005`）。
///
/// 同名多条时取 `(row, owner)` 序的第一条：确定性由排序给，而不是由链接顺序给。
pub fn handler_for(row: &str) -> Option<&'static dyn RowHandler> {
    handlers().into_iter().find(|h| h.row() == row)
}

/// 这一行的写入声明（运行期账本守卫用）。
pub fn writes_of(row: &str) -> Option<&'static [&'static str]> {
    handler_for(row).map(|h| h.writes())
}

/// 全部行实现的写入并集（装配期并进 `plugin_row` 的 `ActorDecl`）。
///
/// 并集而不是逐行：`ActorDecl` 是执行体级的静态声明，装不下每行不同的名单；
/// 装载期第 6 条与依赖图读的是「这个执行体**可能**写哪些格」，保守方向正确。
pub fn declared_writes() -> Vec<&'static str> {
    let mut all: Vec<&'static str> = handlers()
        .iter()
        .flat_map(|h| h.writes().iter().copied())
        .collect();
    all.sort_unstable();
    all.dedup();
    all
}

#[cfg(test)]
#[path = "rows.test.rs"]
mod tests;
