//! 常驻任务的句柄册（[D-075](../../../../../docs/003-design/002-架构总览.md "DE-002-架构总览") 前置 P1 的落地件）。
//!
//! 「插件可独立卸载」这条判据今天缺的一半是：**没人持有两条常驻 loop 的句柄**，
//! 于是 `shift_remove` 之后任务照跑、还反过来拖着 `Arc<SessionPlugin>` 不放。
//! 本册只干一件事：把句柄从构造点接到停机点。
//!
//! 三个方法**全是同步的**，锁因此结构上不可能跨 `await`——这不是风格偏好：
//! 守卫 S-002 按文件文本判「std 锁 + 五行内有 `.await`」，把册子单独成文件，
//! 判据就落在它自己能自证的地方（宿主 `plugin.rs` 里 `await` 密集，混在一起
//! 只有豁免注释能过，而豁免注释会在下一次真出错时静默）。

/// 两条常驻 loop（心跳调度器 + 收件箱消费者）的句柄册。
///
/// 写入只在构造点 `SessionPlugin::start_resident_tasks`，取出只在 `Plugin::stop`。
#[derive(Default)]
pub(crate) struct ResidentTasks {
    handles: std::sync::Mutex<Vec<tokio::task::JoinHandle<()>>>,
}

impl ResidentTasks {
    /// 取锁。std 锁中毒只意味着**别处**已经 panic 过，这份 Vec 没有需要保守的
    /// 不变量，按无中毒取——代价是这里不产生 panic 面（PN-001 要求每处 panic
    /// 都是显式登记的事实，最省的做法是让它不是）。
    fn handles(&self) -> std::sync::MutexGuard<'_, Vec<tokio::task::JoinHandle<()>>> {
        self.handles
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// 记下一条常驻任务。
    pub(crate) fn push(&self, join: tokio::task::JoinHandle<()>) {
        self.handles().push(join);
    }

    /// 交出全部句柄（停机点）。取走不等于销毁：等完由 [`Self::restore`] 还回来，
    /// 判据用例要凭在册的句柄断言「真的退出了」。
    pub(crate) fn take_all(&self) -> Vec<tokio::task::JoinHandle<()>> {
        std::mem::take(&mut *self.handles())
    }

    /// 归还句柄册。
    pub(crate) fn restore(&self, tasks: Vec<tokio::task::JoinHandle<()>>) {
        *self.handles() = tasks;
    }

    /// 在册任务数（判据用例据此确认「两条都起了」）。
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.handles().len()
    }

    /// 在册任务是否都已退出（判据用例据此确认 `stop` 真的等到了它们）。
    #[cfg(test)]
    pub(crate) fn all_finished(&self) -> bool {
        self.handles().iter().all(|task| task.is_finished())
    }
}
