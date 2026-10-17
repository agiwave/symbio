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
//! `docs/016-turn-runner-steps.md`（[AR-016-turn-runner-steps](docs/016-turn-runner-steps.md)）。
//!
//! ## 两行为什么能共用同一个 `LoopState`
//!
//! 因为它们在**结构上互斥**：`policy.classify` 要求 `tool_rounds == 0`，
//! `policy.report` 要求 `tool_rounds > 0`。这不是巧合，是两个决策点各自的成立条件
//! （轮首判"这句话该派活吗"，轮边界判"干了一会儿了该说一句吗"）。
//!
//! 也因此 **`prio` 目前不参与裁决**——没有两行会在同一格同时命中。它仍然必须存在
//! 并被检查：第三行（步骤 3 的反射 / 压缩 / 记忆 / 技能）一定会带来同点竞争，
//! 到那时"谁先"必须是数据而不是巧合。现在就把 fail-fast 立起来，比那时再补便宜。

use std::sync::LazyLock;

use super::chat_loop::ProgressPolicy;

/// 一次派发要执行的那条**环上的臂**（PL-014 `Step::Act` 的载荷）。
///
/// 名字是"臂"不是"插件"：环不认识插件，只认识"该跑哪段执行体"。步骤 4 之后
/// 胶囊长出带类型的入参（`Capsule { cap, args }`），届时本类型是它唯一的构造口。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Capsule(pub(crate) &'static str);

impl Capsule {
    /// 轮首判决：问 `classify` 这一轮该派活还是直接答，再交 `compose` 出话。
    ///
    /// 判决与措辞是同一条臂的两段（`decide` → `apply_verdict`），不拆成两个胶囊：
    /// 拆开会让"判决说了什么"变成环要在两格之间记住的量，而它此刻只是局部变量。
    pub(crate) const DECIDE: Self = Self("decide");

    /// 轮边界汇报：`compose` 的模板产线说一句"干到哪了"。
    pub(crate) const REPORT: Self = Self("report");
}

/// 策略行看到的**全部世界**（PL-014 §3 硬规则 a：只携带引用/值，不带句柄）。
///
/// 每个字段都能回答"它是从哪来的"——没有一个是环的私有簿记：
///
/// | 字段 | 来源 |
/// |---|---|
/// | `tool_rounds` | 本轮已完成轮次（`TurnState` / 轮边界回调的计数器） |
/// | `has_utterance` | 本轮是否有用户发言（`first_utterance.is_some()`） |
/// | `classify_enabled` | 配置快照（`SessionConfig::classify_enabled`） |
/// | `quiet_ms` | 时钟差（`clock_now_ms() - last_user_facing_at`） |
/// | `reports` | 本轮已汇报次数（配额） |
/// | `progress` | 配置快照（`SessionConfig` 的四个旋钮） |
///
/// 两个构造器对应两个决策点，字段组合因此不会出现"半真半假"的中间态。
#[derive(Debug, Clone)]
pub(crate) struct LoopState {
    pub(crate) tool_rounds: usize,
    pub(crate) has_utterance: bool,
    pub(crate) classify_enabled: bool,
    pub(crate) quiet_ms: i64,
    pub(crate) reports: u32,
    pub(crate) progress: ProgressPolicy,
}

impl LoopState {
    /// 轮首（步骤 2b）的状态：只填判决用得到的三个量。
    ///
    /// `progress` 填 [`ProgressPolicy::disabled`]——轮首没有"进展"可报，这不是
    /// "恰好没触发"而是**语义上不存在**：此刻一轮工具都还没跑。
    pub(crate) fn turn_head(
        tool_rounds: usize,
        has_utterance: bool,
        classify_enabled: bool,
    ) -> Self {
        Self {
            tool_rounds,
            has_utterance,
            classify_enabled,
            quiet_ms: 0,
            reports: 0,
            progress: ProgressPolicy::disabled(),
        }
    }

    /// 轮边界（步骤 2d / 轮边界回调）的状态：只填汇报用得到的量。
    ///
    /// `has_utterance` 恒 `false`、`classify_enabled` 恒 `false`——轮边界不重判
    /// 用户那句话（判过了，见 `decide.rs`）。填假值而不是"留空"，是因为这两个
    /// 字段在 `tool_rounds > 0` 时本来就挡得住 `policy.classify`，假值只是让
    /// "此处不适用"在数据上也可读。
    pub(crate) fn round_boundary(
        tool_rounds: usize,
        quiet_ms: i64,
        reports: u32,
        progress: ProgressPolicy,
    ) -> Self {
        Self {
            tool_rounds,
            has_utterance: false,
            classify_enabled: false,
            quiet_ms,
            reports,
            progress,
        }
    }
}

/// 策略行：一个判据 + 它派发的臂。
///
/// `decide` 返回 `None` = 本行不管这一格（交给后面的行）；`Some` = 本行认领。
pub(crate) struct Row {
    /// 行的身份，进日志、进冲突报错。命名 `policy.<名字>`，与能力行区分。
    pub(crate) name: &'static str,
    /// 裁决顺序：`(prio, name)` 升序，先命中者胜。
    pub(crate) prio: u32,
    pub(crate) decide: fn(&LoopState) -> Option<Capsule>,
}

/// 注册期的行冲突（**启动期 fail-fast**，不静默兜底）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RowConflict {
    /// 同名行已存在——重名会让日志与 `get` 指认不出是哪一行。
    DuplicateName { name: &'static str },
    /// 同一个 `prio` 已被占用。
    ///
    /// 不按"字母序兜底"：那会让"谁先"取决于名字拼写，改个名字就改了行为。
    /// 要么给一个不同的 `prio`，要么合并两行。
    PrioConflict {
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
                prio,
                kept,
                incoming,
            } => write!(
                f,
                "策略行 prio 冲突：{prio} 已由 {kept} 占用，{incoming} 需要换一个 prio"
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

/// 表本体。
///
/// `rows` **始终有序**（`put` 按 `(prio, name)` 插入到正确位置），因此 `decide`
/// 只是顺序扫描、零分配——它是热路径上的函数（每个轮边界调一次）。
#[derive(Default)]
pub(crate) struct Registry {
    rows: Vec<Row>,
}

impl Registry {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// 注册一行。重名或 `prio` 冲突 ⇒ `Err`（不插入）。
    pub(crate) fn put(&mut self, row: Row) -> Result<(), RowConflict> {
        if let Some(prev) = self.get(row.name) {
            return Err(RowConflict::DuplicateName { name: prev.name });
        }
        if let Some(prev) = self.rows.iter().find(|r| r.prio == row.prio) {
            return Err(RowConflict::PrioConflict {
                prio: row.prio,
                kept: prev.name,
                incoming: row.name,
            });
        }
        let at = self
            .rows
            .partition_point(|r| (r.prio, r.name) < (row.prio, row.name));
        self.rows.insert(at, row);
        Ok(())
    }

    pub(crate) fn get(&self, name: &str) -> Option<&Row> {
        self.rows.iter().find(|r| r.name == name)
    }

    /// 全部行，按裁决顺序（`(prio, name)` 升序）。
    ///
    /// 生产读者只有一个：bootstrap 时打一行日志（见 [`Registry::with_builtin_rows`]）——
    /// 让「顺序是数据」在运行期可观测，而不是只能读源码。步骤 4 的清单导出
    /// （工具清单逐条比对）会成为第二个读者。
    pub(crate) fn list(&self) -> &[Row] {
        &self.rows
    }

    /// 裁决：按序问每行「这一格归你吗」，先命中者胜；全不命中 ⇒ `None`（= 不动点）。
    pub(crate) fn decide(&self, state: &LoopState) -> Option<Hit> {
        self.rows.iter().find_map(|row| {
            (row.decide)(state).map(|capsule| Hit {
                row: row.name,
                capsule,
            })
        })
    }

    /// 内建行（bootstrap）。装配期一次性构造，此后只读。
    ///
    /// 用户行（步骤 4 的"注册即事实"）覆盖内建行，不在此处。
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
    fn with_builtin_rows() -> Self {
        let mut reg = Self::new();
        for row in [
            Row {
                name: "policy.classify",
                prio: 10,
                decide: policy_classify,
            },
            Row {
                name: "policy.report",
                prio: 20,
                decide: policy_report,
            },
        ] {
            if let Err(e) = reg.put(row) {
                crate::plugin_warn!("session", "[Loop] 内建策略行注册失败（CI 应已拦下）：{e}");
            }
        }
        // 一行日志，只打一次（`LazyLock` 初始化）：让「顺序是数据」在运行期可观测。
        // 排查"某一步怎么没发生"时，第一件事就是看表里有没有这一行、排第几。
        crate::plugin_debug!(
            "session",
            "[Loop] 策略表就绪：{}",
            reg.list()
                .iter()
                .map(|r| format!("{}@{}", r.name, r.prio))
                .collect::<Vec<_>>()
                .join(" → ")
        );
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
/// - `tool_rounds == 0`：判决的对象是**用户这一轮说了什么**，只在轮首成立。
///   工具轮次里循环回顶部时用户没再说话，再判一次就是拿同一句话重判。
/// - `has_utterance`：`resume` / 心跳 / 无发言轮次没有可判的话。
/// - `classify_enabled`：调用方关掉判决 ⇒ 与未挂载 `classify` 逐字一致（全进工具循环）。
fn policy_classify(state: &LoopState) -> Option<Capsule> {
    (state.tool_rounds == 0 && state.has_utterance && state.classify_enabled)
        .then_some(Capsule::DECIDE)
}

/// 轮边界汇报的判据（步骤 2d / 轮边界回调）。
///
/// 前半段 `tool_rounds > 0` 是**独立**条件，不并进 `progress.due`：
/// `ProgressPolicy::min_rounds` 可配 0，而"轮首汇报"是语义错误——那时还一轮工具
/// 都没跑，没有"进展"这回事。实测过：把它交给 `due` 判，`min_rounds = 0` 的配置
/// 会让每轮开头先说一句废话。
fn policy_report(state: &LoopState) -> Option<Capsule> {
    (state.tool_rounds > 0
        && state
            .progress
            .due(state.quiet_ms, state.tool_rounds, state.reports))
    .then_some(Capsule::REPORT)
}

#[cfg(test)]
#[path = "loop_registry.test.rs"]
mod tests;
