//! 单环的**统一注册表** —— PL-014 §4 机制 2 的第一步。
//!
//! ## 它解决什么
//!
//! 收口前，「这一步该不该做」的判断散落在主循环的 `if` 里：轮首判决的条件写在
//! `chat_loop.rs` 步骤 2b，汇报的条件写在 `progress.rs`。加一条新策略 = 在主循环
//! 里插一段 `if`，于是循环只增不减。
//!
//! 本模块把「判据」从「控制流」里剥出来：**每行 = 一个纯函数 `fn(&LoopState) -> Option<Capsule>`**，
//! 环只做三件事——取状态、按表问「下一步是什么」、按胶囊名分派。加一条策略 =
//! 一个函数 + 一行注册，**主循环零改动**（PL-014 §6 判据 1）。
//!
//! ## 为什么是 `fn` 指针，不是 `Box<dyn Fn>` 或 trait object
//!
//! 三条：
//!
//! 1. **纯**：`fn` 指针捕获不了环境，因此策略**结构上**读不到环的任何私有状态——
//!    它只能看 `&LoopState`。这不是自觉，是签名本身（与 `adapters` 的令牌闸门同一条路子）。
//! 2. **可比较**：`fn` 指针可 `==`、可放进 `static`、无分配；`Box<dyn Fn>` 每次
//!    构造都要堆分配，而这张表是进程级常量。
//! 3. **顺序是数据**：`prio` 是行的一个字段，不是插入顺序。顺序进日志、可 diff、
//!    可回放（PL-014 §4）。
//!
//! 代价是策略拿不到配置——配置必须**先折进 `LoopState`**（`progress` 那四个旋钮
//! 就是这么进来的）。这不是缺陷：PL-014 §6 判据 5 要求「`State` 的输入必须是现有
//! Store 的投影」，配置快照本来就是投影的一部分。
//!
//! ## 本批只挂**策略行**，且只有两行
//!
//! PL-014 §4 的目标形状是 `RowKind { Policy, Tool, Model, Prompt, Mount, Option }`
//! 一表异 kind。本批只落 `Policy`：能力行（`Run::Cap`）随步骤 4 迁入。
//!
//! 因此这里**没有** `RowKind` 字段——一个只有 `Policy` 一个变体的枚举会被
//! `dead_code` 判红，而"先造出空壳枚举再等它被用上"正是本项目反复反对的
//! 「结构先于生产」。kind 在步骤 4 加进来时，`Row` 多一个字段、`put` 多一条
//! 冲突判据，都是局部改动。
//!
//! 同理，`Step` 的五个值本批只落 `Act` 的载荷 [`Capsule`]：`Ask` / `Emit` / `Wait` /
//! `Halt` 的**生产者**分别在步骤 3（压缩 / 汇报 / 闸门）与步骤 5（收尾），
//! 现在写出来就是四个永不构造的变体。五值的完整映射见
//! `symbio/src/plugins/session/docs/016-turn-runner-steps.md`（[AR-016-turn-runner-steps](../session/docs/016-turn-runner-steps.md)）。
//!
//! ## 五行为什么能共用同一个 `LoopState`
//!
//! 因为 `LoopState` **按格分形**：它是个枚举，五个变体各带自己那几格的字段，
//! 而 `decide` 只问本格的行。所以「两格互相误命中」不是靠字段清单核对出来的，
//! 是**写不出来**的——`policy_classify` 拿到的只能是 `TurnHead` 变体，它里面
//! 没有 `quiet_ms`；`policy_report` 想读 `has_utterance` 也写不出来（编译错误）。
//!
//! 这也是 `blank()` 消失的原因：此前「这一格不适用」用**假值**表示（`false` /
//! `0` / `ProgressPolicy::disabled()`），于是「某行读了别格没填的字段」会静默按
//! 假值判定。枚举把「不适用」变成「不可表示」。
//!
//! **`prio` 真的参与裁决**：`put` 按 `(prio, name)` 插入到格内有序位置，`decide`
//! 顺序扫描取先命中者——所以「先」由 `prio` 决定，不由注册顺序或名字拼写决定。
//! 但今天的内建表里**每格恰好一行**，于是这条规则事实上从未被行使过：用例
//! `lower_prio_wins_regardless_of_insertion_order` 用两行同格竞争的合成表把它
//! 钉住，否则「prio 生效」只是一句读代码得来的信念。
//!
//! 真正同点竞争的候选是 `policy.recall` 内部那三件事（记忆 / 技能 / 任务段）：
//! 拆开需要「这一步做完了吗」这个簿记量，而它在直线代码里只能靠代码位置隐含。
//!
//! ## 本批的过渡形态（诚实划界）
//!
//! 现在每个决策点各有一个构造器、各调一次 `decide`，因为主循环还是**直线代码**：
//! 它按顺序走七步，不是「迭代 `decide` 至不动点」。表在直线代码上的正确用法就是
//! 「每一步问一次」，代价是构造器随决策点增长。
//!
//! **这一步由 [`Site`] 收窄了代价**：格由 `LoopState` 的变体自证（`site()`），
//! 进了表成为数据，于是加一行策略真的只是「1 个函数 + 1 行注册」，同格加第二行
//! 也不再需要从全局编号里挤位。真正消掉构造器的是 PL-014 §4 的
//! `drive`：`loop { project → decide → execute }`，届时 N 个构造器合并成一个
//! `project()`，`policy.recall` 也会拆成 `policy.memory` / `policy.skill` 两行
//! （区分它们的量「本轮已召回否」在 drive 里自然存在，在直线代码里只能靠代码
//! 位置隐含）。那一步属步骤 5。
//!
//! ## 反射档为什么只判「走不走」，不判「走哪条」
//!
//! `policy.reflex` 的判据是 `resume_allowed ∧ has_match`，命中后返回
//! [`Capsule::Reflex`]——**胶囊不携带命中的那条技能**。这不是漏了载荷：
//! 技能对象留在调用方手里（`turn_exec` 的 `hit`），表只回答"这一格要不要走快路"。
//!
//! 两者不会漂移，因为 `has_match` 就是**同一个** `hit` 派生出来的布尔：判的是它、
//! 用的也是它，中间没有第二次匹配。把技能对象塞进胶囊只会在表里多一份可变借用，
//! 换不来任何一致性——`Capsule` 长出带类型载荷是步骤 4 为**能力行**准备的，
//! 不是为了这一格。

use std::sync::LazyLock;

use super::chat_loop::ProgressPolicy;

/// 一次派发要执行的那条**环上的臂**（PL-014 `Step::Act` 的载荷）。
///
/// 名字是"臂"不是"插件"：环不认识插件，只认识"该跑哪段执行体"。步骤 4 之后
/// 胶囊长出带类型的入参（`Capsule { cap, args }`），届时本类型是它唯一的构造口。
///
/// ## 为什么是**闭集枚举**，不是 `&'static str`
///
/// 2026-10-19 之前它是 `struct Capsule(&'static str)`：臂名是自由字符串，
/// 加一条新臂可以拼错、可以重名、可以在日志里出现一个环里根本没有的字符串——
/// 而**没有任何东西会红**。闭集之后，拼错是编译错误；`name()` 是臂名的唯一来源
/// （日志、冲突报错、测试断言都从这里取），表与环不可能各持一份字面量。
///
/// 代价是「加一条臂要改这个枚举」——**这正是想要的**：臂是环认识的**指令集**，
/// 不是插件可自由扩展的注册项。表能自由扩的是「哪一行在什么时候认领」，不是
/// 「环会做什么」。两者的分界就是本枚举。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Capsule {
    /// 轮首判决：问 `classify` 这一轮该派活还是直接答，再交 `compose` 出话。
    ///
    /// 判决与措辞是同一条臂的两段（`decide` → `apply_verdict`），不拆成两个胶囊：
    /// 拆开会让"判决说了什么"变成环要在两格之间记住的量，而它此刻只是局部变量。
    Decide,
    /// 轮边界汇报：`compose` 的模板产线说一句"干到哪了"。
    Report,
    /// 自动语义压缩（L1，70% 触发）：就地改写上下文。
    Compress,
    /// 轮首准备：长期记忆召回 + 技能路由 + 就绪任务段。
    ///
    /// 三件事一个胶囊，因为它们共用**同一个前提**（有落盘目录）与**同一个时机**
    /// （本轮第一次请求之前），且在直线代码里就是同一段 `if`。拆成三个胶囊需要
    /// 「这一步做完了吗」这个簿记量，而它只在 drive 循环里才自然存在——见
    /// 模块文档「本批的过渡形态」。
    Recall,
    /// 反射档（快路）：命中已编译技能 ⇒ 以技能正文收束，一次模型调用都不发生。
    Reflex,
}

impl Capsule {
    /// 全部臂——**测试**遍历的唯一取值点。
    ///
    /// 与 [`Site::ALL`] 不同，生产侧不需要这份全集（派发按变体匹配，不遍历），
    /// 所以它只在 `cfg(test)` 下存在；它的价值全在
    /// `every_capsule_has_a_producing_row` 那条对偶检查上。
    ///
    /// 加一个变体必须同时加进这里：该测试按它遍历，漏加时「这条臂没有任何行产出」
    /// 不会被发现——臂是环的指令集，一条永不派发的指令等于一个假的能力。
    #[cfg(test)]
    pub(crate) const ALL: &'static [Capsule] = &[
        Capsule::Decide,
        Capsule::Report,
        Capsule::Compress,
        Capsule::Recall,
        Capsule::Reflex,
    ];

    /// 臂名——日志、冲突报错、测试断言**唯一**的取值点。
    ///
    /// 名字与枚举变体一一对应，改名时编译器会指出每一处 `match`；字符串只在
    /// 这一屏出现一次，所以「日志里写着 `decide`、代码里却叫别的」不可能发生。
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Decide => "decide",
            Self::Report => "report",
            Self::Compress => "compress",
            Self::Recall => "recall",
            Self::Reflex => "reflex",
        }
    }
}

/// 决策点闭集 —— **这一问发生在环的哪一格**（PL-014 §3 硬规则 a 的「格」）。
///
/// ## 为什么要把它变成类型（而不是让 `prio` 暗示）
///
/// 此前「哪一行属于哪一格」只是调用点的隐含约定：`chat_loop` 用 `turn_head`
/// 构造的状态、期待 `Decide`；`progress` 用 `round_boundary`、期待 `Report`。
/// 表里读不到这层信息，于是三件事都没有答案：
///
/// 1. **`prio` 被迫全局唯一** —— 想在 `policy.recall` 那一格加第二行（PL-014 §5
///    步骤 3 明确要拆的「记忆 / 技能」两行），得从全局编号里挤一个数，而
///    「50 已被占」这个冲突与「同一格内谁先」根本不是一回事。
/// 2. **每格扫全表** —— 5 格 × 5 行 = 25 次判定，其中 20 次是陪跑（本格的行
///    之外，其余行的判据全是 `blank()` 的假值，恒不成立）。
/// 3. **`None` 三义不分** —— 「本格没有行」「本格有行但都不认领」「这一格不属于
///    任何一行」在返回值上长得一样。
///
/// 所以「格」进入 `Row` 成为数据：`prio` 降为**格内**唯一，`decide` 只扫本格，
/// 返回 [`Verdict`] 区分三种情形。加一行策略 = 1 个函数 + 1 行注册（带 `site`），
/// 驱动器零改动——PL-014 §6 判据 1 由此从「碰巧成立」变成「结构上成立」。
///
/// ## 加一格要改哪里
///
/// 与 [`Capsule`] 同理，闭集的价值就是「加值必须改枚举」：这里加一个变体，
/// `name()` 的 `match` 会编译错误，`every_site_is_manned` 会红（提示去挂行）。
/// 格是环的**决策点清单**，不是插件可自由扩展的注册项——与「臂是环的指令集」
/// 同一条界线。
///
/// `Ord` 的唯一用途是让同格的行在表里相邻（`(site, prio, name)` 全序），
/// **不表达执行顺序**：执行顺序由直线代码的调用点决定，见模块文档「过渡形态」。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Site {
    /// 轮首判决（步骤 2b）：问 classify 这一轮该派活还是直接答。
    TurnHead,
    /// 轮边界汇报（`chat_loop::round_report_hook`）。
    RoundBoundary,
    /// 自动压缩（步骤 3 ①）。
    Compress,
    /// 轮首准备（步骤 3 ③）：记忆 / 技能 / 任务段。
    TurnPrepare,
    /// 反射档（步骤 3 ④）。
    Reflex,
}

impl Site {
    /// 全部格——bootstrap 校验与测试遍历**唯一**的取值点。
    pub(crate) const ALL: &'static [Site] = &[
        Site::TurnHead,
        Site::RoundBoundary,
        Site::Compress,
        Site::TurnPrepare,
        Site::Reflex,
    ];

    /// 格名——日志、冲突报错、测试断言唯一的取值点（与 [`Capsule::name`] 同规矩）。
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::TurnHead => "turn_head",
            Self::RoundBoundary => "round_boundary",
            Self::Compress => "compress",
            Self::TurnPrepare => "turn_prepare",
            Self::Reflex => "reflex",
        }
    }
}

/// 策略行看到的**世界** —— 按决策点分形，不是十个字段的并集。
///
/// ## 为什么是枚举
///
/// 此前是 `struct LoopState` + 五个构造器 + `blank()` 填假值：构造器只填自己那
/// 几格，其余由 `..blank()` 兜底。于是「行只读本格字段」这条不变量**只能靠人
/// 读五个构造器确认**——某行读了一个别格没填的字段，会静默按假值（`0` /
/// `false` / `disabled`）判定，不崩、不报。
///
/// 改为枚举后它成了**编译期**性质：`TurnHead` 变体里根本没有 `quiet_ms`，
/// `policy_report` 想读它也写不出来。运行时的哨兵用例随之删除——一条能被
/// 编译器拦下的东西不该再花一个测试去守。
///
/// 附带收益是状态空间真的变小了：十个字段的并集里，绝大多数组合（「轮首格带
/// 汇报配额」「压缩格带静默时钟」）在语义上不存在。枚举把它们从「不可能但可
/// 表示」变成「不可表示」。
///
/// ## 每个字段的来源
///
/// | 字段 | 变体 | 来源 |
/// |---|---|---|
/// | `round` | `TurnHead` / `RoundBoundary` / `TurnPrepare` | 轮首与准备格填 `TurnState::request_round`（本轮请求数），轮边界格填 `RoundProgress` 持有的运行器真实工具轮数——各行的判据都是「0 = 轮首」 |
/// | `has_utterance` | `TurnHead` | `first_utterance.is_some()` |
/// | `classify_enabled` | `TurnHead` | 配置快照（`SessionConfig::classify_enabled`） |
/// | `quiet_ms` | `RoundBoundary` | 时钟差（`clock_now_ms() - last_user_facing_at`） |
/// | `reports` | `RoundBoundary` | 本轮已汇报次数（配额） |
/// | `progress` | `RoundBoundary` | 配置快照（`SessionConfig` 的四个旋钮） |
/// | `auto_compress` | `Compress` | 请求快照（`TurnRequest::auto_compress`） |
/// | `has_facts` | `TurnPrepare` | 存储层的能力布尔（`SessionStore::is_persistent`，临时会话为假） |
/// | `done` | `TurnHead` / `Compress` / `TurnPrepare` | 一次性阶段本次执行是否已做（`stage_events` 的 `turn.stage` 折叠） |
/// | `resume_allowed` | `Reflex` | 续写锚点为空（续写轮不走快路，见 `turn_exec`） |
/// | `has_match` | `Reflex` | `skill_events::take_match` 的结论（同一次查询，判它用的就是它） |
///
/// `done` 只出现在**一次性**的三格里：它们的判据在一轮内恒真（`round == 0` 直到
/// 收轮才变、`auto_compress` 是开关不是水位），「这一步做完了没有」是唯一能让
/// 它们转假的量（ADR-107 判据 2）。`RoundBoundary` / `Reflex` 每轮重新判定，
/// 没有「已做」这回事——它们不持有该字段，所以也读不到。
///
/// `round` 出现在三个变体里是**同一个语义**（「这是第几次请求 / 第几个工具轮」），
/// 不是三份状态：它在三个决策点各有自己的取值点，而这三处本来就要各自回答
/// 「此刻算不算轮首」。
///
/// 构造器随决策点增长——这是直线代码的过渡形态，见模块文档。
#[derive(Debug, Clone)]
pub(crate) enum LoopState {
    /// 轮首判决（步骤 2b）。
    TurnHead {
        round: usize,
        has_utterance: bool,
        classify_enabled: bool,
        /// 本轮判决是否已执行过（`turn.stage` 的 `decide` 格）。
        done: bool,
    },
    /// 轮边界汇报。
    RoundBoundary {
        round: usize,
        quiet_ms: i64,
        reports: u32,
        progress: ProgressPolicy,
    },
    /// 自动压缩（步骤 3 ①）。
    Compress {
        auto_compress: bool,
        /// 本请求轮是否已进过压缩产线（`turn.stage` 的 `compress` 格）。
        done: bool,
    },
    /// 轮首准备（步骤 3 ③）。
    TurnPrepare {
        round: usize,
        has_facts: bool,
        /// 本轮是否已召回（`turn.stage` 的 `recall` 格）。
        done: bool,
    },
    /// 反射档（步骤 3 ④）。
    Reflex {
        resume_allowed: bool,
        has_match: bool,
    },
}

impl LoopState {
    /// 轮首（步骤 2b）：判决只读这四个量。
    pub(crate) fn turn_head(
        round: usize,
        has_utterance: bool,
        classify_enabled: bool,
        done: bool,
    ) -> Self {
        Self::TurnHead {
            round,
            has_utterance,
            classify_enabled,
            done,
        }
    }

    /// 轮边界（`chat_loop::round_report_hook`）：汇报只读这四个量。
    ///
    /// 轮首那两个字段（`has_utterance` / `classify_enabled`）在这里**不存在**：
    /// 轮边界不重判用户那句话（判过了，见 `decide.rs`）。此前它们以 `blank()`
    /// 的 `false` 存在，靠「`round > 0` 本来就挡得住」自证无害——那是一条每次
    /// 改动都要复核的推理，现在不需要了。
    pub(crate) fn round_boundary(
        round: usize,
        quiet_ms: i64,
        reports: u32,
        progress: ProgressPolicy,
    ) -> Self {
        Self::RoundBoundary {
            round,
            quiet_ms,
            reports,
            progress,
        }
    }

    /// 自动压缩格（步骤 3 ①）：开关 + 本请求轮是否已进过产线。
    pub(crate) fn compress(auto_compress: bool, done: bool) -> Self {
        Self::Compress {
            auto_compress,
            done,
        }
    }

    /// 轮首准备格（步骤 3 ③）：记忆召回 / 技能路由 / 任务段共用这一格。
    pub(crate) fn turn_prepare(round: usize, has_facts: bool, done: bool) -> Self {
        Self::TurnPrepare {
            round,
            has_facts,
            done,
        }
    }

    /// 反射档格（步骤 3 ④）：两个条件都是布尔，都是同一次查询的结果。
    ///
    /// `has_match` 由调用方在**同一次** `take_match` 结果上取——判的是它、用的也是
    /// 它，中间没有第二次匹配（S11 §2「一次判定、两个出口」）。
    pub(crate) fn reflex(resume_allowed: bool, has_match: bool) -> Self {
        Self::Reflex {
            resume_allowed,
            has_match,
        }
    }

    /// 这一问发生在哪一格——**由变体自证**，不需要调用方另传一个 `site`。
    ///
    /// 加变体时本 `match` 与 [`Site::name`] 会同时编译错误，两份提示合起来就是
    /// 「格」这个概念的完整加值清单。
    pub(crate) fn site(&self) -> Site {
        match self {
            Self::TurnHead { .. } => Site::TurnHead,
            Self::RoundBoundary { .. } => Site::RoundBoundary,
            Self::Compress { .. } => Site::Compress,
            Self::TurnPrepare { .. } => Site::TurnPrepare,
            Self::Reflex { .. } => Site::Reflex,
        }
    }
}

/// 策略行：一个判据 + 它派发的臂。
///
/// `decide` 返回 `None` = 本行不管这一问（交给同格后面的行）；`Some` = 本行认领。
pub(crate) struct Row {
    /// 行的身份，进日志、进冲突报错。命名 `policy.<名字>`，与能力行区分。
    pub(crate) name: &'static str,
    /// **这一行在哪一格生效**。`decide` 只问同格的行——见 [`Site`] 的三条理由。
    pub(crate) site: Site,
    /// 格内裁决顺序：`(prio, name)` 升序，先命中者胜。
    ///
    /// **不跨格比较**：不同格的行从不在同一次判定里相遇，因此两格用同一个
    /// `prio` 不构成冲突，也不需要协调编号。这是「同格加第二行」从「挤全局
    /// 编号」变成「在格内挑一个序」的全部改动。
    pub(crate) prio: u32,
    pub(crate) decide: fn(&LoopState) -> Option<Capsule>,
}

/// 注册期的行冲突（**启动期 fail-fast**，不静默兜底）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RowConflict {
    /// 同名行已存在——重名会让日志与 `get` 指认不出是哪一行。
    DuplicateName { name: &'static str },
    /// 同一个格内 `prio` 已被占用。
    ///
    /// 不按"字母序兜底"：那会让"谁先"取决于名字拼写，改个名字就改了行为。
    /// 要么给一个不同的 `prio`，要么合并两行。
    ///
    /// 判据**只在格内**成立：两格共用一个 `prio` 是正常的，它们从不在同一次
    /// 判定里相遇（见 [`Row::prio`]）。
    PrioConflict {
        site: Site,
        prio: u32,
        kept: &'static str,
        incoming: &'static str,
    },
}

impl std::fmt::Display for RowConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuplicateName { name } => write!(f, "策略行重名：{name}"),
            Self::PrioConflict {
                site,
                prio,
                kept,
                incoming,
            } => write!(
                f,
                "策略行 prio 冲突：{} 格内 {prio} 已由 {kept} 占用，{incoming} 需要换一个 prio",
                site.name()
            ),
        }
    }
}

/// 一次裁决的结果：命中哪一行、该跑哪条臂。
///
/// 带行名是为了让日志能说"这一步是 `policy.report` 决定的"——只记胶囊名的话，
/// 两行派发同一条臂时（将来会有的）就分不清是谁判的。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Hit {
    pub(crate) row: &'static str,
    pub(crate) capsule: Capsule,
}

/// 一次裁决的结果。
///
/// 三值而不是 `Option`，因为「没有答案」有两种完全不同的成因，混在一起会让
/// 排查「某一步怎么没发生」时无从下手（PL-014 §3 规则 g：`None` 是缺口，
/// 不是出口；迁移期它必须**可分辨**）：
///
/// - [`Verdict::Declined`]：本格有行，判据都不成立 —— **正常**，本步不做。
/// - [`Verdict::Unmanned`]：本格一行都没有 —— **配置缺口**。内建表由
///   `every_site_is_manned` 在 CI 拦下；动态注册（步骤 4 的用户行）不该出现，
///   因为 `site` 是闭集，没人能"新开一格"。
///
/// 两者在调用点的行为一样（都不做事），但日志与测试能把它们分开——这正是
/// 「规则缺口必须可见」在直线代码上的落法：不需要 drive 的 `Halt(NoRule)`，
/// 只需要返回值不再把两件事压成一个 `None`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// 命中一行。
    Hit(Hit),
    /// 本格有 `rows` 行，都不认领。
    Declined { site: Site, rows: usize },
    /// 本格没有行。
    Unmanned { site: Site },
}

impl Verdict {
    /// 命中时派发的臂；未命中一律 `None`（调用点只关心"要不要做这件事"）。
    ///
    /// 把「未命中」压回一个 `Option` 是**调用点的**选择，不是 Verdict 的语义：
    /// 需要区分两种未命中的地方（bootstrap 校验、日志、测试）读变体本身。
    pub(crate) fn capsule(self) -> Option<Capsule> {
        match self {
            Self::Hit(hit) => Some(hit.capsule),
            Self::Declined { .. } | Self::Unmanned { .. } => None,
        }
    }

    /// 命中行本身（要行名 + 臂名一起记日志的地方用它）。未命中 ⇒ `None`。
    pub(crate) fn hit(self) -> Option<Hit> {
        match self {
            Self::Hit(hit) => Some(hit),
            _ => None,
        }
    }
}

/// 表本体。
///
/// `rows` **始终有序**（`put` 按 `(site, prio, name)` 插入到正确位置），因此
/// `decide` 只需二分找出本格那一段、再顺序扫描——零分配，且不扫别格的行。
#[derive(Default)]
pub(crate) struct Registry {
    rows: Vec<Row>,
}

impl Registry {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// 注册一行。重名、或**同格内** `prio` 冲突 ⇒ `Err`（不插入）。
    ///
    /// 两处判据的范围不同，是有意的：
    /// - 名字**全局**唯一（`get(name)` 要能唯一定位一行，日志要能唯一定位一行）；
    /// - `prio` 只要求**格内**唯一（跨格的行从不在同一次判定里相遇）。
    pub(crate) fn put(&mut self, row: Row) -> Result<(), RowConflict> {
        if let Some(prev) = self.get(row.name) {
            return Err(RowConflict::DuplicateName { name: prev.name });
        }
        if let Some(prev) = self
            .rows
            .iter()
            .find(|r| r.site == row.site && r.prio == row.prio)
        {
            return Err(RowConflict::PrioConflict {
                site: row.site,
                prio: row.prio,
                kept: prev.name,
                incoming: row.name,
            });
        }
        let at = self
            .rows
            .partition_point(|r| (r.site, r.prio, r.name) < (row.site, row.prio, row.name));
        self.rows.insert(at, row);
        Ok(())
    }

    pub(crate) fn get(&self, name: &str) -> Option<&Row> {
        self.rows.iter().find(|r| r.name == name)
    }

    /// 某一格的全部行，按裁决顺序（`(prio, name)` 升序）。
    pub(crate) fn rows_of(&self, site: Site) -> &[Row] {
        let start = self.rows.partition_point(|r| r.site < site);
        let len = self.rows[start..].partition_point(|r| r.site == site);
        &self.rows[start..start + len]
    }

    /// 一行都没有的格。
    ///
    /// 内建表为空 ⇒ 那一格**永远不会做任何事**，而调用点看到的是「判据不成立」
    /// （`Verdict::Declined` 与 `Verdict::Unmanned` 在调用点同为"不做事"）。
    /// 这条检查把「配错」从「恰好不触发」里摘出来——CI 与启动期各一次。
    pub(crate) fn unmanned_sites(&self) -> Vec<Site> {
        Site::ALL
            .iter()
            .copied()
            .filter(|s| self.rows_of(*s).is_empty())
            .collect()
    }

    /// 裁决：在 `call.site` 那一格里按序问每行「这一问归你吗」，先命中者胜。
    ///
    /// **只扫本格**——既省掉陪跑（此前 5 格各扫 5 行），也让「行只读本格字段」
    /// 成为结构性质：别格的字段在本格永远是 `blank()` 的假值，而本格的行不可能
    /// 读到它们（判据见 `no_row_reads_across_sites`）。
    pub(crate) fn decide(&self, state: &LoopState) -> Verdict {
        let site = state.site();
        let rows = self.rows_of(site);
        if rows.is_empty() {
            return Verdict::Unmanned { site };
        }
        match rows.iter().find_map(|row| {
            (row.decide)(state).map(|capsule| Hit {
                row: row.name,
                capsule,
            })
        }) {
            Some(hit) => Verdict::Hit(hit),
            None => Verdict::Declined {
                site,
                rows: rows.len(),
            },
        }
    }

    /// 内建行（bootstrap）。装配期一次性构造，此后只读。
    ///
    /// 用户行（步骤 4 的"注册即事实"）覆盖内建行，不在此处。
    ///
    /// ## `prio` 从 10 起、步长 10
    ///
    /// 它只是**格内**序，跨格无意义；留步长是为了插行（把 `policy.recall` 拆成
    /// `policy.memory` / `policy.skill` 时，在新行上填 10 / 20 即可，不必动别格）。
    ///
    /// ## 注册失败为什么不 `expect`
    ///
    /// 内建行冲突只可能来自**改错这张常量表**，那是一个应当被 CI 拦下的错误，
    /// 不是一个用户该在启动时看到的崩溃。因此这里告警并继续，真正的守门人是
    /// 单测 `builtin_rows_are_conflict_free_and_ordered`——它在 CI 里红，
    /// 而不是在用户机器上炸。
    ///
    /// 早先的写法是 `.expect(...)`，被 `panic-audit`（PN-001）当场拦下：
    /// 两处 panic 面让全仓计数从 71 涨到 73。那条棘轮是对的——一个只可能因
    /// 改错常量而触发的崩溃，不值得占用两个 panic 面。
    ///
    /// 「某一格没有行」同样只可能是改错常量表，因此走同一条路：启动期告警
    /// 一次 + 单测 `every_site_is_manned` 在 CI 红。
    fn with_builtin_rows() -> Self {
        let mut reg = Self::new();
        for row in [
            Row {
                name: "policy.classify",
                site: Site::TurnHead,
                prio: 10,
                decide: policy_classify,
            },
            Row {
                name: "policy.report",
                site: Site::RoundBoundary,
                prio: 10,
                decide: policy_report,
            },
            Row {
                name: "policy.compress",
                site: Site::Compress,
                prio: 10,
                decide: policy_compress,
            },
            Row {
                name: "policy.recall",
                site: Site::TurnPrepare,
                prio: 10,
                decide: policy_recall,
            },
            Row {
                name: "policy.reflex",
                site: Site::Reflex,
                prio: 10,
                decide: policy_reflex,
            },
        ] {
            if let Err(e) = reg.put(row) {
                crate::plugin_warn!("session", "[Loop] 内建策略行注册失败（CI 应已拦下）：{e}");
            }
        }
        // 一行日志，只打一次（`LazyLock` 初始化）：让「每格有谁、谁先」在运行期
        // 可观测。排查"某一步怎么没发生"时，第一件事就是看那一格有没有行。
        crate::plugin_debug!(
            "session",
            "[Loop] 策略表就绪：{}",
            Site::ALL
                .iter()
                .map(|s| format!(
                    "{}[{}]",
                    s.name(),
                    reg.rows_of(*s)
                        .iter()
                        .map(|r| format!("{}@{}", r.name, r.prio))
                        .collect::<Vec<_>>()
                        .join(",")
                ))
                .collect::<Vec<_>>()
                .join(" → ")
        );
        let unmanned = reg.unmanned_sites();
        if !unmanned.is_empty() {
            crate::plugin_warn!(
                "session",
                "[Loop] 以下决策点没有任何策略行（那一格永远不会做事）：{:?}",
                unmanned.iter().map(|s| s.name()).collect::<Vec<_>>()
            );
        }
        reg
    }
}

/// 内建表（进程级单例）。
///
/// 行是 `fn` 指针、无捕获，因此这张表**没有内部可变状态**，`&'static` 共享即可；
/// 步骤 4 的动态行（MCP 桥接工具运行期 put/remove）会需要 `RwLock`，届时另立
/// 一个可写表，不把这张只读表改成锁。
pub(crate) fn builtin() -> &'static Registry {
    static BUILTIN: LazyLock<Registry> = LazyLock::new(Registry::with_builtin_rows);
    &BUILTIN
}

/// 轮首判决的判据（步骤 2b）。
///
/// 三个条件缺一不可，各自挡掉一类误判：
///
/// - `round == 0`：判决的对象是**用户这一轮说了什么**，只在轮首成立。
///   工具轮次里循环回顶部时用户没再说话，再判一次就是拿同一句话重判。
/// - `has_utterance`：`resume` / 心跳 / 无发言轮次没有可判的话。
/// - `classify_enabled`：调用方关掉判决 ⇒ 与未挂载 `classify` 逐字一致（全进工具循环）。
/// - `!done`：本轮的判决**已经执行过**（`turn.stage` 的 `decide` 格）⇒ 不再认领。
///   前三条在一轮内恒真，没有这一条，外环 `drive` 的第二次迭代会把同一句话再判
///   一遍（ADR-107 §3 的第一条实例）。
///
/// 四个字段只有 `TurnHead` 变体有——判据写在别的变体上不了编译。`let ... else`
/// 的 `else` 是**兜底不是路径**：`decide` 已按 `state.site()` 过滤，走到 else
/// 意味着这一行的 `site` 与判据接受的变体对不上（改错常量表）。此处不 panic
/// ——只可能因改错常量而触发的问题不值得占一个 panic 面（见 `with_builtin_rows`
/// 的同类裁决），改由 `every_row_is_reachable_from_its_declared_site` 在 CI 拦。
fn policy_classify(state: &LoopState) -> Option<Capsule> {
    let LoopState::TurnHead {
        round,
        has_utterance,
        classify_enabled,
        done,
    } = state
    else {
        return None;
    };
    (*round == 0 && *has_utterance && *classify_enabled && !*done).then_some(Capsule::Decide)
}

/// 轮边界汇报的判据（`chat_loop::round_report_hook` 每跑完一个工具批次问一次）。
///
/// 前半段 `round > 0` 是**独立**条件，不并进 `progress.due`：
/// `ProgressPolicy::min_rounds` 可配 0，而"轮首汇报"是语义错误——那时还一轮工具
/// 都没跑，没有"进展"这回事。实测过：把它交给 `due` 判，`min_rounds = 0` 的配置
/// 会让每轮开头先说一句废话。
fn policy_report(state: &LoopState) -> Option<Capsule> {
    let LoopState::RoundBoundary {
        round,
        quiet_ms,
        reports,
        progress,
    } = state
    else {
        return None;
    };
    (*round > 0 && progress.due(*quiet_ms, *round, *reports)).then_some(Capsule::Report)
}

/// 自动语义压缩的判据（步骤 3 ①）。
///
/// 开关 + 本请求轮是否已进过产线：**压不压得动**（水位、熔断、失败回滚）是压缩
/// 内核自己的事，本行答的是"这一格要不要进压缩这条产线"。把水位也搬进来会变成
/// 两份阈值——一份在 `auto_compress_process` 里真正生效，一份在这里只是重复判断。
///
/// `!done` 的口径是**本请求轮**（`TurnState::request_round`）：内核每轮重新取水位，
/// 未过水位时返回 `Ok(None)`——那不是失败，下一轮照常再试。若把「已做」扩到整轮，
/// 后段的压缩就永远不会发生（ADR-107 §3 的第三条实例是 drive 反复空转，直线代码
/// 的失效形态则是「压不上了」）。
fn policy_compress(state: &LoopState) -> Option<Capsule> {
    let LoopState::Compress {
        auto_compress,
        done,
    } = state
    else {
        return None;
    };
    (*auto_compress && !*done).then_some(Capsule::Compress)
}

/// 轮首准备的判据（步骤 3 ③）。
///
/// - `round == 0`：同一轮内记忆不漂移、事实源只扫一遍，后续工具轮复用
///   同一份视图（与 classify / history 同一条「每轮一次」口径）；
/// - `has_facts`：临时会话没有事实源——没有事实源就没有记忆，不读不写；
/// - `!done`：本轮已召回（`turn.stage` 的 `recall` 格）⇒ 不再认领。续写幕是
///   **新的执行**、`done` 从空开始 ⇒ 照常重跑（恢复后事实源变了，本轮视图应当
///   重取），撞幂等键由 `stage_events::land` 按「已做过」吸收。
fn policy_recall(state: &LoopState) -> Option<Capsule> {
    let LoopState::TurnPrepare {
        round,
        has_facts,
        done,
    } = state
    else {
        return None;
    };
    (*round == 0 && *has_facts && !*done).then_some(Capsule::Recall)
}

/// 反射档的判据（步骤 3 ④）。两关缺一不可：
///
/// - `resume_allowed`：**续写轮不走快路**。续写轮不新开用户格（运行器复用
///   `resume.user_seq`），而反射档的落格形状就是新开一轮 ⇒ 走它会撞幂等键
///   （`AppendError::Duplicate` → Failed 出口）。语义上同样说不通：续写要跑的是
///   刚被用户批准的那次工具，不是拿技能顶掉它。
/// - `has_match`：逐字命中一条已编译技能（判据是数据，见 `skill_events::take_match`）。
fn policy_reflex(state: &LoopState) -> Option<Capsule> {
    let LoopState::Reflex {
        resume_allowed,
        has_match,
    } = state
    else {
        return None;
    };
    (*resume_allowed && *has_match).then_some(Capsule::Reflex)
}

#[cfg(test)]
#[path = "loop_registry.test.rs"]
mod tests;
