//! 轮末贡献 —— 能力在**收束之后**以纯值交派生事实（DE-025 D-250）。
//!
//! ## 它补的是哪一格
//!
//! 一轮结束时要写的事实不都属于「这次工具调用发生了什么」（[`crate::effects`]），
//! 也不都欠成一条行（[`crate::rows`]）：记忆的编码与巩固、技能编译、任务调度都
//! 在**轮末**凭这一轮的产物算，且它们**不保证产出**——一句空发言不编码、活记忆
//! 不够四条不巩固、没有就绪任务不派活。把它们写成环上的债，就得给每条能力编一个
//! 「天然空转」的 `owes`，环会在同一个债上反复选中它直到步数上界。
//!
//! 所以这一格是**贡献**而不是债：
//!
//! ```text
//! 能力插件 ──submit_turn_contributor!──▶ inventory（编译期随二进制生效）
//! 宿主轮末 ──逐条询问（按 prio, owner）──▶ Vec<TurnDraft> ──唯一写入口──▶ 事实源
//! ```
//!
//! ## 贡献者看得见什么
//!
//! [`TurnEnd`] 只交**本轮的值**：轮号 / 时刻 / 主体 / 溯源锚 / 用户发言 / 收束正文 /
//! 任务容量 / 完整史。没有 `&mut Ctx`、没有 store、没有 `async`——与
//! [`crate::rows::RowRequest`] 同一条纪律：「下一步跑什么」仍归控制面。
//!
//! ## 顺序是判据
//!
//! 宿主按 `(prio, owner)` 逐个询问，并且**每问一个之前重取一次史**：后一个贡献者
//! 看得见前一个刚落格的草稿。这不是便利，是依赖的表达——记忆编码必须先于巩固
//! （巩固要看得见本轮刚编码的那条），效果必须先于调度（调度的输入是本轮刚开的任务）。
//! 把顺序做成「调用点的先后」而不是贡献者内部的状态，贡献者才能保持纯函数。
//!
//! ## 归属由能力声明，时刻由宿主盖章
//!
//! 草稿的 `turn` 恒为本轮；`ts` 恒为本次询问的 [`TurnEnd::now_ms`]——「轮末事实发生在
//! 收束这一刻」是宿主面。`actor` 则可能不是本轮主体：记忆的 actor 是**认知归属人**
//! （`person/local`），而跑轮主体可能是某个会话。所以 [`TurnDraft::actor`] 留了一格显式
//! 声明，缺省仍是本轮主体——「这笔账以谁的名义记」是能力自己的判断，不是宿主的默认值
//! 能替他做的。
//!
//! ## 失败口径
//!
//! 贡献者自己算不出来时记日志、返回空（与 [`crate::rows::RowHandler::run`] 的
//! 「必须保证推进」不同：这里不是债，空是合法结局）；落格失败由宿主记日志、**不打断**
//! 下一个贡献者——轮次已收束，附加事实失败不该把成功的一轮说成失败。

use symbio_core::Event;

use crate::effects::FactDraft;

/// 这一轮走的是哪一面——贡献者据此判断自己该不该落笔。
///
/// 两面的差别是**有没有驱动环**：生成轮有工具交换与派生效果；定稿答话轮一个模型
/// 请求都不发，只有轮级记忆与学习。把「哪一面」作为宿主词汇交进来，贡献者就不必
/// 从 `history` 里猜自己处在哪条路径上。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnFace {
    /// 走向环的生成轮：工具、效果与轮末派生事实都在这一面。
    Generated,
    /// 定稿答话轮：零模型往返（措辞插件的往返已结束），只该写轮级记忆。
    Answered,
}

/// 轮末贡献者的**全部输入**：本轮的值，构造一次、逐个贡献者共用。
///
/// `history` 的切片每问一个贡献者刷新一次（见模块文档「顺序是判据」）；其余字段
/// 本轮不变。
#[derive(Debug, Clone, Copy)]
pub struct TurnEnd<'a> {
    /// 这一轮走哪一面（见 [`TurnFace`]）。
    pub face: TurnFace,
    /// 轮号。
    pub turn: u64,
    /// 本次收束的时刻（Unix 毫秒）：草稿的 `ts` 由它盖章。
    pub now_ms: i64,
    /// 本会话主体（写事件的 `actor` 缺省值）。
    pub principal: &'a str,
    /// 本轮开口格的 seq——派生事实的溯源锚（I2）。
    pub user_seq: u64,
    /// 幂等键词干（`t{turn}`）——事件号的跨轮唯一性从这里来。
    pub anchor: &'a str,
    /// 本轮用户发言（`Generated` 与 `Answered` 两面都有：发言是那一轮的输入）。
    pub user_text: &'a str,
    /// 本轮成功收束的正文；兜底收束 / 中止为 `None`（固化失败套路比不记更坏）。
    pub response: Option<&'a str>,
    /// 任务段的容量上限（控制面的配置投影，不是判据）。
    pub task_cap: usize,
    /// 完整史（含归档前缀）：累计口径的贡献者（巩固 / 校准）要看得见全部。
    pub history: &'a [Event],
}

impl TurnEnd<'_> {
    /// 收束时刻的词干（复用 [`Self::anchor`]，供各处拼事件号）。
    pub fn anchor(&self) -> &str {
        self.anchor
    }
}

/// 一条**待落格**的轮末草稿：事实形状 + 归属人。
///
/// 与 [`FactDraft`] 的分界只有一格 `actor`：效果与行的事实随本轮主体记名，而记忆的
/// 事实归**认知归属人**。宿主在落格时统一盖 `turn` 与 `ts`（见模块文档）。
#[derive(Debug, Clone)]
pub struct TurnDraft {
    /// 事实本身（键、种类、格位、载荷、溯源）。
    pub fact: FactDraft,
    /// 归属人：`None` = 本轮主体；`Some(actor)` = 能力声明的归属（如记忆归人）。
    pub actor: Option<&'static str>,
}

impl TurnDraft {
    /// 以本轮主体记名。
    pub fn new(fact: FactDraft) -> Self {
        Self { fact, actor: None }
    }

    /// 改记到另一个归属人名下（认知归属人 ≠ 跑轮主体的那一类事实）。
    pub fn with_actor(mut self, actor: &'static str) -> Self {
        self.actor = Some(actor);
        self
    }

    /// 盖章入库：`turn` 与 `ts` 由宿主给，`actor` 取声明或本轮主体。
    pub fn into_event(self, end: &TurnEnd<'_>) -> Event {
        self.fact
            .into_event(end.turn, self.actor.unwrap_or(end.principal))
            .with_ts(end.now_ms)
    }
}

/// 能力对**本轮收束之后**的贡献。
///
/// 一个实现 = 一条贡献 + 一次 [`crate::submit_turn_contributor!`]；加一条贡献只改能力包。
/// 它不持状态（`Sync`、`&self`）、不碰事实源、不参与「这一轮何时停」的判断。
pub trait TurnContributor: Sync {
    /// 装配顺序的依据：同 `prio` 时按 owner 字典序（DE-025 §3 第 4 条）。
    fn owner(&self) -> &'static str;

    /// 次序：数小者先。缺省 100——只有「必须看见别人刚落格的草稿」的贡献者才调小它
    /// （编码 < 巩固），普通来源不必显式声明。
    fn prio(&self) -> u8 {
        100
    }

    /// 本轮的贡献。返回空 = 本轮没有可落的东西（**默认**：多数来源在多数轮次上是空的）。
    ///
    /// 只产草稿，**不写事实源**（I1）。需要看见本轮早先落格的事实 ⇒ 靠 `prio`
    /// 声明次序，宿主会在询问前刷新 `history`。
    fn contribute(&self, _end: &TurnEnd<'_>) -> Vec<TurnDraft> {
        Vec::new()
    }
}

/// 一条贡献者的链接期提交。
pub struct TurnContributorDecl {
    pub contributor: &'static dyn TurnContributor,
}

inventory::collect!(TurnContributorDecl);

/// 提交一条轮末贡献。在插件 crate 的模块作用域调用一次即可。
///
/// ```ignore
/// symbio_plugin_api::submit_turn_contributor!(MemoryEncode);
/// ```
#[macro_export]
macro_rules! submit_turn_contributor {
    ($c:expr) => {
        $crate::inventory::submit! {
            $crate::turn::TurnContributorDecl { contributor: &$c }
        }
    };
}

/// 已链接进来的全部贡献者，按 `(prio, owner)` 排序（同序保持提交序）。
///
/// 排序在读取侧做而不是装配期做一次：表是 `inventory` 的编译期集合，本仓不持有
/// 第二个可变注册表（与 `effects::handlers` 同一条纪律）。
pub fn contributors() -> Vec<&'static dyn TurnContributor> {
    let mut all: Vec<&'static dyn TurnContributor> = inventory::iter::<TurnContributorDecl>()
        .map(|d| d.contributor)
        .collect();
    all.sort_by_key(|c| (c.prio(), c.owner()));
    all
}

#[cfg(test)]
#[path = "turn.test.rs"]
mod tests;
