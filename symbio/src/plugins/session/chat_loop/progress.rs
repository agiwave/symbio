//! 中途汇报 —— 「干到一半也要能说一句话」的**触发点**与**判定**。
//!
//! ## 两件事分开：触发权归编排层，措辞权归 `reply`
//!
//! 本文件只回答「**该不该**在现在说一句」，一个字都不写；说什么由 `reply` 从
//! [`RunSnapshot`] 组织（见 `reply/templates.rs` 的 `progress_text`），落点由
//! `chat_loop/compose.rs` 执行。合成一处就会出现"该说话时没人说话"——判定方
//! 不知道有什么可说，措辞方不知道什么时候该说。
//!
//! ## 触发点在**轮边界**，不在后台定时器
//!
//! 判定发生在 `run_chat_loop` 的轮次顶部（一轮工具刚跑完、准备发下一次请求），
//! 与轮边界抽干点相邻。三条理由，缺一条都会让这个判定长到别处去：
//!
//! 1. **写入者唯一**：汇报节点要落在 `context.messages` 上才会被 `persist_messages`
//!    带上并上线（ADR-020：转写只有一个写入者）。那个列表是主循环的**局部变量**，
//!    后台任务够不着；从别处写进转写只会得到一个"回合结束就消失"的节点。
//! 2. **判定所需的量就在这里**：`tool_rounds`（已完成几轮）是 [`TurnState`] 的字段，
//!    而静默时长与已汇报次数也是**请求作用域**的量（随轮次复位）。放在会话级状态里
//!    反而要额外回答"什么时候清零"。
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

use std::sync::Arc;

use crate::symbio_core::schemas::dialog::{RunSnapshot, Verdict};
use crate::symbio_core::{clock_now_ms, PluginInvokeRequest};

use super::state::{ChatOrchestrator, SessionContext, TurnState};
use super::{apply_verdict, VerdictEffect};

/// 中途汇报的策略快照（`SessionConfig` 的四个旋钮）。
///
/// 构造点唯一：`orchestrator/consume.rs`——那里拿着插件、能读配置，与
/// `ChatOrchestrator::triage_enabled` / `reply_enabled` 是同一条取值纪律
/// （取**值快照**，不把插件交给主循环）。
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
}

/// 轮边界的汇报判定与执行。返回 `true` = 真的汇报了一句。
///
/// 调用点见 `chat_loop.rs` 的步骤 2d（轮边界抽干点之后）——**顺序有意**：抽干会把
/// 用户刚补充的那句话折进上下文，而那句话本身就是"对话线上的动静"，因此静默时长
/// 会随之归零，本轮自然不汇报。反过来先汇报再抽干，就会出现"用户刚说完话，
/// 助手抢着报了一句进度"。
pub(crate) async fn report_if_due(
    orchestrator: &ChatOrchestrator,
    ctx: &Arc<dyn PluginInvokeRequest>,
    context: &mut SessionContext,
    turn: &mut TurnState,
) -> bool {
    // 轮首没有"进展"可报：那里的判定对象是用户那句话（归 `decide.rs`）。
    if turn.tool_rounds == 0 {
        return false;
    }

    let quiet_ms = clock_now_ms() - turn.last_user_facing_at;
    if !orchestrator
        .progress
        .due(quiet_ms, turn.tool_rounds, turn.progress_reports)
    {
        return false;
    }

    let snapshot = RunSnapshot {
        tool_rounds: turn.tool_rounds,
        quiet_ms,
    };

    match apply_verdict(orchestrator, ctx, context, Verdict::Report, &snapshot).await {
        VerdictEffect::Spoke => {
            turn.progress_reports += 1;
            // 刚说了一句话 ⇒ 静默时钟归零。不归零的话，下一个轮边界会因为"距上次
            // 说话仍然超过阈值"而立刻再说一句——配额会被同一段静默连续吃掉。
            turn.last_user_facing_at = clock_now_ms();
            crate::plugin_info!(
                "session",
                "[Progress] 已汇报第 {} 次（tool_rounds={}, quiet_ms={}）",
                turn.progress_reports,
                turn.tool_rounds,
                quiet_ms
            );
            true
        }
        // 拿不到措辞（未挂载 `reply` / 措辞为空）⇒ 没汇报，也就**不消耗配额**：
        // 下一个轮边界还会再试一次。方向与 `Answered` 的降级一致——**降级而不失效**。
        _ => {
            crate::plugin_debug!(
                "session",
                "[Progress] 该汇报但取不到措辞，本轮跳过（不消耗配额）"
            );
            false
        }
    }
}

#[cfg(test)]
#[path = "progress.test.rs"]
mod tests;
