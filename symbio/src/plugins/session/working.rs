//! 会话工作态的小状态机（P5 运行态归源，相应批次）。
//!
//! ## 为什么是一个闭集，而不是散落的布尔写点
//!
//! `is_working` 决定两件事：新请求是否排队（`session_busy` 守卫）与收件箱消费者何时
//! 再取下一条。此前它由多条收尾路径各写一次（正常收尾 / 中止收尾 / 失败收敛 / 崩溃
//! 兜底），写点分散的直接后果是消费者只能靠 50ms 轮询兜底——**漏掉任何一处唤醒**
//! 的表现都是「消息永远不发」，且静默。
//!
//! 现在改变工作态的路径是一个闭集 [`WorkingTransition`]，唯一迁移函数
//! [`WorkingState::apply`] 负责回答「本次迁移是否由忙变闲」；等这一下的消费者挂在
//! `ActiveSessionState::turn_done` 上。新增一条收尾路径时，只要它要改变工作态就必须
//! 经 `apply`——**绕过去状态根本不会变**，而不是「变了但没人被叫醒」。
//!
//! ## 进程局部量的边界（[D-083](../../../../docs/003-design/001-核心架构.md "D-083") 残余条款）
//!
//! 工作态**不参与事实投影**：它是宿主内存里的当前值，重启后按 [`WorkingState::Idle`]
//! 起算，上一轮结局再由事实投影补上。因此崩溃不会留下「永久处理中」——内存没了，
//! 状态就没了。这条边界不因本模块而改变。

/// 会话工作态。
///
/// 两态就够：`EarlyExit` 与 `Finish` 的区别不在状态，而在**谁**在收尾（提前让位 /
/// 收完写结局）。多造一个「收尾中」态会让所有读方都得回答「这算不算忙」，而唯一
/// 正确的答案始终是「不算」——那只是一个布尔就能表达的事实。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum WorkingState {
    /// 没有在途轮：新请求可以直接开跑。
    #[default]
    Idle,
    /// 有在途轮：新请求排队（收件箱），`resume` 直接拒绝。
    Working,
}

/// 改变工作态的**全部**事件。
///
/// 三个变体对应三类路径，没有第四条：
/// - [`Begin`](Self::Begin)：开轮（`entry` 的同步置位 + `emit_session_state(Working)`）；
/// - [`EarlyExit`](Self::EarlyExit)：收尾**之前**先让位（正常收尾 / 中止兜底 / 崩溃兜底）——
///   这几步还要清在途图、注销中止信号、写失败节点，期间必须已可接新请求，
///   否则用户点重试会被 `session_busy` 静默拒绝；
/// - [`Finish`](Self::Finish)：收轮写结局（`emit_session_state(Finished)`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkingTransition {
    Begin,
    EarlyExit,
    Finish,
}

impl WorkingState {
    /// 是否**在途忙**（`session_busy` 守卫与收件箱「忙则排队」读的就是这个）。
    pub(crate) fn is_working(self) -> bool {
        matches!(self, Self::Working)
    }

    /// 唯一迁移函数。
    ///
    /// 返回 `true` = 本次迁移让会话**由忙变闲**——等待方（收件箱消费者）据此被唤醒。
    /// 幂等：空闲时的 `EarlyExit`、以及提前让位之后才到的 `Finish`，都返回 `false`，
    /// 不制造伪唤醒（伪唤醒不会错，但会让「一次收轮 = 一次唤醒」这条判据失去意义）。
    pub(crate) fn apply(&mut self, transition: WorkingTransition) -> bool {
        let was_working = self.is_working();
        *self = match transition {
            WorkingTransition::Begin => Self::Working,
            WorkingTransition::EarlyExit | WorkingTransition::Finish => Self::Idle,
        };
        was_working && !self.is_working()
    }
}

#[cfg(test)]
#[path = "working.test.rs"]
mod tests;
