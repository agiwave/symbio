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
//! 交进去，把 `Vec<FactDraft>` 拿回来。顺序由既有的 `(phase, anchor, name, key)` 决定，
//! 失败由既有的账本守卫与步数上界兜底（D-123 / D-115）。
//!
//! ## 为什么只收纯值
//!
//! `&mut Ctx` 里有本轮草稿、端口与尚未落格的宿主状态：把它开给能力，等于让能力参与「下一步
//! 跑什么」的判断，顺序与失败都不可判（D-241 的「被否」第一条）。这里交出去的是
//! 一笔债的身份（`row` + `key`）、宿主面（幂等词干 / `now_ms` / `user_seq`）、写事件的
//! 主体，以及**本轮**的事实切片——能力要判断什么，从事实里读。
//!
//! ## 需要模型的能力：提示 + 解析函数（D-242）
//!
//! 反思 / 压缩 / 技能编译这类能力要一次 LLM 往返。它们不拿 provider：`run` 返回
//! [`RowStep::Ask`] 交一条提示，宿主完成调用、预算、租约与四格协议事实，再把回执
//! 以 [`ModelReply`] 交回 [`RowHandler::parse`]。能力看到的始终只有纯值——提示
//! 是自己造的那条，回执是模型答的那条；用哪个供应商、重试几次、花多少预算都是
//! 宿主的事（D-242 的「被否」第一条）。
//!
//! ## 写入的声明在实现手里，不在 `ActorDecl` 里
//!
//! 一行写哪些领域格随能力而变，`ActorDecl` 是宿主侧的静态投影，装不下每行不同的
//! 名单。于是 [`RowHandler::writes`] 是那一行的写入声明：装配期把它并进
//! `plugin_row` 的 `ActorDecl`（装载检查看得见），派发时按**这一行**的名单对账
//! （运行期账本守卫仍要求「声明 ⊆ 强制」，D-123）。

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use symbio_kernel::event::Event;

use crate::effects::FactDraft;

/// 通用行执行体的名字：`Decl.by` 与 `ActorDecl.name` 都用它。
///
/// 它是**宿主词汇**（与 `behaviors` 同一层）：能力包写这个名字，宿主认这个名字；
/// 两边都不需要认识对方的具体身份。
pub const PLUGIN_ROW: &str = "plugin_row";

// 共享事件 → 写出它的端口，投影在 `events::port_of_fact`（由 owner 的 `PLUGIN.yml`
// 的 `shared_event_ports:` 行生成）。机制层不再手写这张映射：一旦这里出现一个世界名，
// 它就有了第二个 owner（`D-393` 同批收窄）。

/// 「这一行在等一笔模型往返」的事实种类（D-338）。
///
/// 静态债行表达不了「此刻在 Ask」，所以 `Ask` 必须落成事实。种类是**一个**名字
/// （行名进载荷，不进种类）：世界格子只认声明表上的名字，逐行造名会落进 OffGrid。
pub const ASK_KIND: &str = "plugin.row.asked";

/// 「在等」事实的身份：由 `(row, key, attempt)` 派生（D-338）。
///
/// 机制侧与申报方用同一个函数派生，配对不靠插件载荷里的自报 id。
pub fn ask_id(row: &str, key: &[(String, String)], attempt: u32) -> String {
    let trace = key
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(",");
    format!("{row}:{trace}:{attempt}")
}

/// 通用端口执行体（`DE-048` D-301）：端口行的 `by` 写它，环按名字找动作。
///
/// 与 [`PLUGIN_ROW`] 同一层（宿主词汇）：端口所属插件写这个名字，环认这个名字；
/// 两边都不需要认识对方的具体身份。它是**机制名**，不是任何世界名——新增端口只由
/// 所属插件声明端口与行，绑定表不随端口数增长。
pub const PORT_ACTOR: &str = "port";

/// 一笔债的实例外加本行看得见的事实切片——执行体的唯一入参。
///
/// 字段全部是纯值：借用生命周期只覆盖一次 `run` 调用，执行体拿不走、也存不下。
#[derive(Debug, Clone, Copy)]
pub struct RowRequest<'a> {
    /// 行名（`Decl.name`）：实现据此确认「这笔债是我的」。
    pub row: &'static str,
    /// 债键：`Decl.owes.key` 各字段的 `(名字, 值)`，按声明顺序。
    pub key: &'a [(String, String)],
    /// 本轮的幂等词干（事件号的跨轮唯一性从这里来；由宿主构造，见 DE-026 D-252）。
    pub anchor: &'a str,
    /// 本次 drive 的 `now`（幕锚那条事实的 `ts`，D-115）：按时间判的前件读它。
    pub now_ms: i64,
    /// 写事件的 `actor`（本会话主体）。
    pub principal: &'a str,
    /// 溯源锚（I2）：本轮开口那条事实的存储 `seq`，由宿主给。
    pub user_seq: u64,
    /// **本轮**的事实切片（不早于本轮第一条事件）：判断只从事实里读。
    pub facts: &'a [Event],
    /// 完整史（含本轮之前）：累计口径的判断（技能表、巩固、校准）按它读。
    ///
    /// 与 [`crate::settle::Settling::history`] 同形：视图求解器按轮收窄
    /// 是为了给每个视图一个成本上界，行是一次单调用、按需读，不该被同一个上界误伤。
    pub history: &'a [Event],
    /// 本行声明的**上游端口**（`DebtSpec.after_port`）：读别端口回执的行按它取，
    /// 不引全局端口常量。`None` = 本行没有声明上游端口。
    pub after_port: Option<&'a str>,
}

impl RowRequest<'_> {
    /// 债键里某个字段的取值。键由 `Decl.owes.key` 决定，这里不猜顺序。
    pub fn key_of(&self, field: &str) -> Option<&str> {
        self.key
            .iter()
            .find(|(k, _)| k == field)
            .map(|(_, v)| v.as_str())
    }
}

/// 一行**这一步**的产物：直接落格，或要一次模型往返（D-242）。
pub enum RowStep {
    /// 本步直接落格（不需要往返）：中间产物，落完继续欠下一步。
    Facts(Vec<FactDraft>),
    /// 本步落事实**并交效果草稿**（D-293）：事实先由写口落，效果再交三个宿主写口。
    ///
    /// 只有声明了对应 [`RowHandler::effects`] 的行才能走这一档：写口按声明复核形状，
    /// 未声明的效果在写口一处被拒。
    FactsWithEffects(Vec<FactDraft>, Vec<crate::effects::EffectDraft>),
    /// 本步欠一笔 `request(port=model)`（D-304）：宿主据它发起一次往返并记账，
    /// 回执交 [`RowHandler::parse`] 关账。
    Ask(ModelAsk),
}

/// 一次模型往返的**请求**（纯值）：只有提示正文。
///
/// 不带模型名、温度、凭据与预算：那些是宿主与 provider 之间的事。带上它们等于让
/// 能力知道「用哪个供应商、花多少钱」，跨插件依赖 provider crate 只是时间问题
/// （D-242 的「被否」第一条）。
pub struct ModelAsk {
    /// 交给模型的提示正文。
    pub prompt: String,
}

/// 一次模型往返的**回执**（纯值）：答成的正文，或失败的理由。
///
/// 失败也是回执：宿主已经把 `failed(port=model)` 落格，解析函数仍要产出一笔关账
/// 的事实——「推进优先于猜测」，而不是把债留在原地再打一次模型。
#[derive(Debug, Clone, Copy)]
pub enum ModelReply<'a> {
    /// 模型答成了；这里是正文。
    Text(&'a str),
    /// 这一次往返失败（`failed` 已落格）；理由由端口给出。
    Failed(&'a str),
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

    /// 这一笔债的产物：直接落的事实草稿，或一次模型往返。
    ///
    /// **必须保证推进**：返回空而债仍开着，环会在同一个债上反复选中它，直到步数
    /// 上界把这一轮判成 `step_limit`。因此「这一笔不该记账」要在行的 `owes` 里
    /// 排除（用 `eq` 收窄），而不是在实现里返回空。
    fn run(&self, req: &RowRequest<'_>) -> RowStep;

    /// 同一行的**带上下文**入口：默认原样转调 [`RowHandler::run`]。
    ///
    /// 绝大多数行只读事实、直接落格——那类行保持 `run` 即可；`run_ctx` 的默认实现
    /// 把 `run` 包成一次已完成的异步调用，因此**其它行实现零改动**。
    ///
    /// `ctx` 与派发这次调用的 `PortExecContext.ctx` 是**同一份**上下文：不另造句柄，
    /// 也不追加 `parent` 或别的字段（D-344）。
    /// 行拿它只做一件事：`fork` 出子上下文、设 `PATH` 与载荷，经 `ctx.parent()` 的
    /// **插件路由**取别的插件的能力答复。跨插件调用只走 route、不直连插件；谁想用
    /// `ctx` 绕过路由去摸别处的数据，按 D-344 驳回。
    fn run_ctx<'a>(
        &'a self,
        req: &'a RowRequest<'_>,
        ctx: Option<Arc<dyn crate::PluginInvokeRequest>>,
    ) -> Pin<Box<dyn Future<Output = RowStep> + Send + 'a>> {
        let _ = ctx;
        Box::pin(async move { self.run(req) })
    }

    /// 本行声明产出的**效果面**（D-293）：默认空 = 这一行不交任何效果。
    ///
    /// 声明只用于装配期把面折成事实生产者（阶段格 / 对话面各对应一条宿主事实），
    /// 以及运行期对草稿形状的校验；它不给出写句柄——落格仍在宿主唯一写口。
    fn effects(&self) -> &'static [crate::effects::EffectFace] {
        &[]
    }

    /// 模型往返的回执 → 这一笔债的产物（D-242）。
    ///
    /// 只有 `run` 交过 [`RowStep::Ask`] 的实现会被调到；收的仍然是纯值。
    ///
    /// **必须保证推进**：即使回执是 [`ModelReply::Failed`] 也要落一笔关账的事实。
    /// 宿主把「交过 `Ask` 却一笔都不落」判成实现违约并报错——那不是可重试的暂态，
    /// 而是这一行没有回答「拿到回执之后写什么」。
    fn parse(&self, req: &RowRequest<'_>, reply: ModelReply<'_>) -> Vec<FactDraft> {
        let _ = (req, reply);
        Vec::new()
    }
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

/// 这一行声明的效果面（未登记的行 = 空，`None` 与空在判据上同义）。
pub fn effects_of(row: &str) -> Option<&'static [crate::effects::EffectFace]> {
    handler_for(row).map(|h| h.effects())
}

/// 全部行实现声明的效果面并集（装配期折成宿主事实的生产者面，D-293）。
pub fn declared_effects() -> Vec<crate::effects::EffectFace> {
    let mut all: Vec<crate::effects::EffectFace> = handlers()
        .iter()
        .flat_map(|h| h.effects().iter().copied())
        .collect();
    all.sort_unstable();
    all.dedup();
    all
}

/// 逐行的写入名单：`(行名, 该行实现声明写的事实名)`。
///
/// 依赖图与第 6 条按**这一行**判，而不是按执行体并集（D-248）：多行共用一个
/// `plugin_row` 时，并集会让每一行的出口都被别的行认领，两行以上必然成假环。
pub fn row_writes() -> Vec<(&'static str, &'static [&'static str])> {
    handlers()
        .into_iter()
        .map(|h| (h.row(), h.writes()))
        .collect()
}

#[cfg(test)]
#[path = "rows.test.rs"]
mod tests;
