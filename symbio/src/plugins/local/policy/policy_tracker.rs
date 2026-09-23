use std::sync::Mutex;
use std::time::{Duration, Instant};

/// 滑动窗口动作追踪器
#[derive(Debug)]
pub struct ActionTracker {
    actions: Mutex<Vec<Instant>>,
    window_secs: u64,
}

impl ActionTracker {
    pub fn new() -> Self {
        Self {
            actions: Mutex::new(Vec::new()),
            window_secs: 3600,
        }
    }

    pub fn record(&self) -> usize {
        let mut actions = match self.actions.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        self.cleanup_old_actions(&mut actions);
        actions.push(Instant::now());
        actions.len()
    }

    /// 窗口内的动作数
    pub fn count(&self) -> usize {
        let mut actions = match self.actions.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        self.cleanup_old_actions(&mut actions);
        actions.len()
    }

    /// 当前窗口内动作数是否已达上限；`max_actions == 0` 视为**关闭限流**。
    ///
    /// 此前这个方法是恒 `false` 的占位（真实现被注释掉），于是
    /// `max_actions_per_hour: 100` 从未生效——限流配置形同虚设。
    pub fn is_at_limit(&self, max_actions: u32) -> bool {
        if max_actions == 0 {
            return false;
        }
        self.count() >= max_actions as usize
    }

    /// 清掉窗口外的记录，只留窗口内的。
    ///
    /// ## `checked_sub` 失败是**正常分支**，不是异常
    ///
    /// `Instant` 的基准点**没有保证**（Rust 只承诺单调，不承诺起点），而窗口
    /// （默认 3600 秒）完全可能比「从基准点到此刻的时长」还长。此时窗口下界落在
    /// 时钟基准**之前**，语义上就是**没有一条记录算旧**——全部保留。
    ///
    /// ⚠️ 曾经的写法是 `.unwrap_or_else(Instant::now)`，即把「下溢」当成「窗口从此刻
    /// 开始」：一旦触发，cutoff 变成此刻，`retain` 把**全部记录**清空 ⇒ `count()` 恒为
    /// 0 ⇒ 限流**静默失效**（配置看着生效，实际从不拦）。它的触发条件是「时钟基准到
    /// 此刻的时长 < 窗口」，因此只在特定机器 / 特定启动时长下出现——本仓库的
    /// `test_rate_limit_is_enforced` 就因此在系统启动不足 1 小时时变红，看起来像 flaky。
    fn cleanup_old_actions(&self, actions: &mut Vec<Instant>) {
        let cutoff = Instant::now().checked_sub(Duration::from_secs(self.window_secs));
        actions.retain(|t| cutoff.is_none_or(|c| *t > c));
    }
}

#[cfg(test)]
#[path = "policy_tracker.test.rs"]
mod tests;

impl Default for ActionTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for ActionTracker {
    fn clone(&self) -> Self {
        let actions = match self.actions.lock() {
            Ok(g) => g.clone(),
            Err(p) => p.into_inner().clone(),
        };
        Self {
            actions: Mutex::new(actions),
            window_secs: self.window_secs,
        }
    }
}
