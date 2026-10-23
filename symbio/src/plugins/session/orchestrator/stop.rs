//! Stop 钩子的幂等触发器（[D-086](../../../../../docs/003-design/014-单环一表.md "D-086") §6）。
//!
//! 契约：**一个请求生命周期内，Stop 恰好触发一次**——无论该生命周期以何种方式
//! 结束（正常完成 / 各类错误 / abort / 软上限 / 消费循环超时 / 任务 panic）。
//!
//! ## 为什么住 session 而不随驱动器迁走
//!
//! Stop 是**会话生命周期**的一部分：`WorkingGuard` 的兜底、`run_chat_loop_task`
//! 的收尾分支都在 session。驱动器只交回 `TurnReport.last_message`，由 session
//! 在收尾统一触发——这样「恰好一次」仍由生命周期保证，而不是由两侧各记一次。
//!
//! ## 双轨
//!
//! 显式触发点携带准确的"本轮最后一条消息"；[`StopSignal::drop`] 兜底仅在显式
//! 触发全部未发生时生效（例如驱动器任务 panic 被 JoinError 吞掉、消费循环 1800s
//! 超时提前 return、provider 解析失败根本没能进入 loop）。

use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

pub(crate) struct StopSignal {
    parent: Option<Arc<dyn Plugin>>,
    /// Stop 钩子要投递的请求上下文（`fire_hook` 内部会再 fork 一份并设置
    /// PATH=payload，故此处持有的是原始 chat 上下文）。
    ctx: Arc<dyn PluginInvokeRequest>,
    fired: AtomicBool,
}

impl StopSignal {
    pub(crate) fn new(parent: Option<Arc<dyn Plugin>>, ctx: Arc<dyn PluginInvokeRequest>) -> Self {
        Self {
            parent,
            ctx,
            fired: AtomicBool::new(false),
        }
    }

    /// 本请求生命周期内 Stop 是否已触发。
    pub(crate) fn fired(&self) -> bool {
        self.fired.load(Ordering::SeqCst)
    }

    /// 触发 Stop；返回 `true` 表示本次调用是真正生效的那一次。
    ///
    /// `last_message` 由驱动器经 [`TurnReport::last_message`] 交回
    /// （无消息的失败出口传空串）。
    ///
    /// [TurnReport::last_message]: crate::symbio_core::TurnReport::last_message
    pub(crate) async fn fire_text(&self, last_message: String) -> bool {
        if self.fired.swap(true, Ordering::SeqCst) {
            return false;
        }
        let _ = crate::symbio_core::fire_hook(
            &self.parent,
            crate::symbio_core::HookEvent::Stop { last_message },
            self.ctx.clone(),
        )
        .await;
        true
    }

    /// 兜底触发（同步、幂等）：显式触发点一次都没执行过时，补发一次 Stop。
    ///
    /// 由 `WorkingGuard::drop`（panic / 消费循环超时 / provider 解析失败）与
    /// [`StopSignal::drop`]（最后防线）共用。Drop 语境不能 await，故投递到
    /// detached 任务；无 tokio 运行时（进程退出路径）时跳过外发并告警，
    /// `fired` 保持置位、不再重试。
    pub(crate) fn fire_fallback(&self) {
        if self.fired.swap(true, Ordering::SeqCst) {
            return;
        }
        if self.parent.is_none() {
            // 无父插件时 fire_hook 本身就是 no-op，不打噪声日志
            return;
        }
        crate::plugin_warn!(
            "session",
            "[Stop] 显式 Stop 触发点未执行，由 StopSignal 生命周期兜底补发一次（last_message 为空）"
        );
        let parent = self.parent.clone();
        let ctx = self.ctx.clone();
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                handle.spawn(async move {
                    let _ = crate::symbio_core::fire_hook(
                        &parent,
                        crate::symbio_core::HookEvent::Stop {
                            last_message: String::new(),
                        },
                        ctx,
                    )
                    .await;
                });
            }
            Err(_) => {
                // 没有运行时可承载：保持 fired 置位、不再重试。回滚标记没有意义——
                // 本方法的全部调用点（WorkingGuard::drop / StopSignal::drop）都处在
                // 同一条同步析构链上，下一层 Drop 同样不会有运行时，重试只会失败并
                // 重复告警（进程退出路径本就放弃了外发）。
                crate::plugin_warn!(
                    "session",
                    "[Stop] 当前无 tokio 运行时，Stop 兜底触发被跳过（进程退出路径）"
                );
            }
        }
    }
}

impl Drop for StopSignal {
    fn drop(&mut self) {
        // 最后防线：显式触发点一个都没走到（panic / 消费循环超时 / 提前 return）。
        // 已触发过则直接返回——与 `fire_fallback` 的幂等判定等价，但避免在
        // 正常路径（绝大多数请求都显式 fire 过）上多做一次原子写。
        if self.fired() {
            return;
        }
        self.fire_fallback();
    }
}

#[cfg(test)]
#[path = "stop.test.rs"]
mod tests;
