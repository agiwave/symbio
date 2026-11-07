//! 会话工作态的小状态机（运行态归源）。
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
//! ## 进程局部量的边界（[D-083](../../../docs/003-design/001-核心架构.md#d-083-自上而下再推导废除服务端口进-core跨插件通货闭族在既有机制词汇切片补立前置-p5运行态归源 "DE-001-核心架构") 残余条款）
//!
//! 工作态**不参与事实投影**：它是宿主内存里的当前值，重启后按 [`WorkingState::Idle`]
//! 起算，上一轮结局再由事实投影补上。因此崩溃不会留下「永久处理中」——内存没了，
//! 状态就没了。这条边界不因本模块而改变。

/// 会话工作态。
///
/// 两态就够：收尾的差异不在状态，而在**写结局那一步**（收完写结局 / 中止收束）。
/// 多造一个「收尾中」态会让所有读方都得回答「这算不算忙」，而唯一正确的答案始终是
/// 「不算」——那只是一个布尔就能表达的事实。
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
/// 两个变体对应两类路径，没有第三条：
/// - `Begin`：开轮（`entry` 的同步置位 + `emit_session_state(Working)`）；
/// - `Finish`：收轮写结局（`emit_session_state(Finished)`）——中止收束也走这一条：
///   中止不再是「先强制让位、再写结局」，而是正常的失败/中止收束路径，收束那一步
///   同时落闲并注销中止信号（同一临界区）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkingTransition {
    Begin,
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
    /// 幂等：空闲时的 `Finish` 返回 `false`，不制造伪唤醒（伪唤醒不会错，但会让
    /// 「一次收轮 = 一次唤醒」这条判据失去意义）。
    pub(crate) fn apply(&mut self, transition: WorkingTransition) -> bool {
        let was_working = self.is_working();
        *self = match transition {
            WorkingTransition::Begin => Self::Working,
            WorkingTransition::Finish => Self::Idle,
        };
        was_working && !self.is_working()
    }
}

#[cfg(test)]
#[path = "working.test.rs"]
mod tests;
