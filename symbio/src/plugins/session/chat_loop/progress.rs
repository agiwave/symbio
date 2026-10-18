//! 中途汇报 —— 「干到一半也要能说一句话」的**触发点**与**判定**。
//!
//! ## 两件事分开：触发权归编排层，措辞权归 `compose`
//!
//! 本文件只回答「**该不该**在现在说一句」，一个字都不写；说什么由 `compose` 从
//! [`RunSnapshot`] 组织（见 `compose/templates.rs` 的 `progress_text`），落点由
//! `chat_loop/compose.rs` 执行。合成一处就会出现"该说话时没人说话"——判定方
//! 不知道有什么可说，措辞方不知道什么时候该说。
//!
//! ## 触发点在**轮边界**，不在后台定时器
//!
//! 判定挂在 `chat_loop::round_report_hook` 上、由 `turn_exec` 交进运行器：**一个工具
//! 批次的结果接进 `exchange` 之后**回调一次，于是「又走完了一轮」恰好在那些轮边界
//! 上成立。三条理由，缺一条都会让这个判定长到别处去：
//!
//! 1. **写入者唯一**：汇报节点要落在 `context.messages` 上才会被 `persist_messages`
//!    带上并上线（ADR-020：转写只有一个写入者）。那个列表是主循环的**局部变量**，
//!    后台任务够不着；从别处写进转写只会得到一个"回合结束就消失"的节点。
//! 2. **判定所需的量就在这里**：轮次、静默时长、已汇报次数都是**请求作用域**的量
//!    （随轮次复位），由下面的 `RoundProgress` 自己持有。放在会话级状态里反而要
//!    额外回答"什么时候清零"。
//! 3. **不引入第二份会话状态**：后台定时器要读这些量就得把它们搬到跨任务共享的状态
//!    上，那是给同一份事实造第二个来源——而它必然漂移。
//!
//! ## 因此**单次工具执行期间不汇报**
//!
//! 判定点只在批次边界上：一个跑 10 分钟的工具在它结束前不会被汇报。那段时间的
//! 「进展」由前端从 ToolCall 节点自己的运行态（`Streaming` + 起始时刻）直接呈现
//! ——**它不需要经过本插件**，也就不该在这里再抄一份。本文件报的是"**又走完了一轮**"，
//! 那是批次边界上才成立的事实。
//!
//! ## 判定是一个**纯函数**
//!
//! [`ProgressPolicy::due`] 只吃四个数（静默时长 / 已完成轮次 / 已汇报次数 / 策略），
//! 因此它可以被穷举测试——这类"错了不会报错、只表现为用户被打扰或收不到消息"的
//! 判定，必须靠单测钉住，不能靠 e2e 里那几条路径碰巧覆盖。

use crate::symbio_core::clock_now_ms;
use crate::symbio_core::RunSnapshot;

/// 中途汇报的策略快照（`SessionConfig` 的四个旋钮）。
///
/// 构造点唯一：`orchestrator/consume.rs`——那里拿着插件、能读配置，与
/// `ChatOrchestrator::classify_enabled` / `compose_enabled` 是同一条取值纪律
/// （取**值快照**，不把插件交给主循环）。
#[derive(Debug, Clone)]
pub struct ProgressPolicy {
    /// 总开关（`SessionConfig::progress_enabled`）
    pub enabled: bool,
    /// 静默阈值（毫秒）
    pub interval_ms: i64,
    /// 最少工具轮次
    pub min_rounds: usize,
    /// 每轮汇报次数上限
    pub max_per_turn: u32,
}

impl ProgressPolicy {
    /// 该不该汇报（**纯函数**，无 IO、无时钟）。
    ///
    /// 四个条件是与关系，任一不成立就不说：
    ///
    /// | 条件 | 挡掉的是 |
    /// |---|---|
    /// | `enabled` | 调用方关掉了这个特性（J2 平凡值） |
    /// | `quiet_ms >= interval_ms` | 刚说过话就再说一遍（用户还没等） |
    /// | `tool_rounds >= min_rounds` | 一轮就完事的任务（"进展"还谈不上） |
    /// | `reports < max_per_turn` | 一个长任务反复刷屏 |
    ///
    /// 取 `>=` 而不是 `>`：阈值是"到了就该说"，不是"超过才说"。
    pub(crate) fn due(&self, quiet_ms: i64, tool_rounds: usize, reports: u32) -> bool {
        self.enabled
            && quiet_ms >= self.interval_ms
            && tool_rounds >= self.min_rounds
            && reports < self.max_per_turn
    }

    /// 「此处不存在汇报」的策略快照（轮首用）。
    ///
    /// 与"配置里关掉了汇报"是**同一件事在数据上的两种来源**：`enabled = false`
    /// 让 [`Self::due`] 恒假，因此 `policy.report` 在轮首自然不命中，不必让
    /// 环在构造状态时特判"这一格不适用"。
    ///
    /// 其余三个旋钮填**不可能通过**的值而不是 0：`min_rounds = 0` 与
    /// `max_per_turn = 0` 在合法配置里都是"更宽松"，留着它们会让本快照在
    /// 将来某次误读 `enabled` 时静默变成"每次都该汇报"。
    pub(crate) fn disabled() -> Self {
        Self {
            enabled: false,
            interval_ms: i64::MAX,
            min_rounds: usize::MAX,
            max_per_turn: 0,
        }
    }
}

/// **轮内**的汇报状态（缺口 4）——轮次与静默时钟的**唯一持有者**。
///
/// 它不是 `TurnState` 的字段，因为工具循环在 v2 运行器里：`run_chat_loop` 一次请求
/// 只经过一次，那里没有任何时刻能观察到「第 N 个工具轮结束」。计数因此挂在
/// 轮边界回调**自己持有**的结构上——那个回调是 `Fn + 'static`，它捕获不了
/// `run_chat_loop` 的栈局部。
///
/// 静默时钟的种子（请求开始 / 轮首说过话的那一刻）由 `TurnState::last_user_facing_at`
/// 递进来（见 `round_report_hook`），此后归零归本结构自己管。
pub(crate) struct RoundProgress {
    policy: ProgressPolicy,
    /// 本轮已完成几轮工具调用（轮边界回调每跑一次 +1）。
    rounds: usize,
    /// 上一次「用户看得见的一句话」的时刻。
    last_spoke_at: i64,
    /// 本轮已汇报几次（配额）。
    reports: u32,
}

impl RoundProgress {
    /// `last_spoke_at` 由调用方给（轮首 compose 说过话就要传它，否则本轮第一次汇报
    /// 会用「会话开头」当起点 ⇒ 静默时长虚高 ⇒ 抢在用户前面报进度）。
    pub(crate) fn new(policy: ProgressPolicy, last_spoke_at: i64) -> Self {
        Self {
            policy,
            rounds: 0,
            last_spoke_at,
            reports: 0,
        }
    }

    /// 轮边界推进一格，返回此刻的运行现状。
    pub(crate) fn advance(&mut self) -> RunSnapshot {
        self.rounds += 1;
        RunSnapshot {
            tool_rounds: self.rounds,
            quiet_ms: clock_now_ms() - self.last_spoke_at,
        }
    }

    /// 该不该说（纯判定）。
    ///
    /// 判据在 [`super::super::loop_registry`] 的 `policy.report` 行——本方法只把
    /// 自己持有的三格数（轮次 / 静默 / 已汇报次数）连同策略快照折成 `LoopState`
    /// 交给表。**同一份判据只有一个 owner**：改阈值语义改那一行，不改这里。
    pub(crate) fn due(&self, snapshot: &RunSnapshot) -> bool {
        let state = super::super::loop_registry::LoopState::round_boundary(
            snapshot.tool_rounds,
            snapshot.quiet_ms,
            self.reports,
            self.policy.clone(),
        );
        super::super::loop_registry::builtin()
            .decide(&state)
            .map(|h| h.capsule)
            == Some(super::super::loop_registry::Capsule::REPORT)
    }

    /// 说了：配额 +1、静默时钟归零。
    ///
    /// 归零的理由：不归零的话下一个轮边界会因为「距上次说话仍然超过阈值」而立刻
    /// 再说一句——配额会被同一段静默连续吃掉。
    pub(crate) fn spoke(&mut self) {
        self.reports += 1;
        self.last_spoke_at = clock_now_ms();
    }

    /// 对话线上有了动静（用户在轮内补充）：静默时钟归零，**不占配额**。
    ///
    /// 与 [`Self::spoke`] 的差别就是这条：那是助手说了话，这只是用户说了话。
    /// 取两者较大值——时钟只往前走。
    pub(crate) fn stirred(&mut self, at: i64) {
        self.last_spoke_at = self.last_spoke_at.max(at);
    }
}

#[cfg(test)]
#[path = "progress.test.rs"]
mod tests;
