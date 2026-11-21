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

    /// 指定窗口长度构造。生产路径恒用 [`Self::new`]（一小时）；测试用短窗口把
    /// 「等待后放行」从小时级变成毫秒级可验证行为。
    #[cfg(test)]
    pub fn with_window_secs(window_secs: u64) -> Self {
        Self {
            actions: Mutex::new(Vec::new()),
            window_secs,
        }
    }

    #[cfg(test)]
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
    #[cfg(test)]
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
    #[cfg(test)]
    pub fn is_at_limit(&self, max_actions: u32) -> bool {
        if max_actions == 0 {
            return false;
        }
        self.count() >= max_actions as usize
    }

    /// 尝试**原子地**占用一个动作名额。
    ///
    /// - `max_actions == 0`（关闭限流）或窗口内未满：记录本次动作并返回 `Ok(())`；
    /// - 已满：返回 `Err(距离最早一条记录滑出窗口的时长)`，**不记录**。调用方等待该
    ///   时长后必须**重新调用**——并发下名额可能被另一个调用抢走。
    ///
    /// ## 检查与记录为什么在同一把锁内
    ///
    /// 拆成「先问 `is_at_limit`、再 `record`」会让并发调用同时看到未满并一起放行，
    /// 名额瞬时超卖。把两者合成一次临界区，等待留在锁外由调用方做，既无超卖也不阻塞
    /// 别的线程。
    pub fn try_record(&self, max_actions: u32) -> Result<(), Duration> {
        let mut actions = match self.actions.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        self.cleanup_old_actions(&mut actions);
        if max_actions == 0 || actions.len() < max_actions as usize {
            actions.push(Instant::now());
            return Ok(());
        }
        // 已满：等最早一条过期。`cleanup_old_actions` 刚跑过，它必然还在窗口内，
        // 故到期时刻严格在未来；`unwrap_or` 只是防 `checked_add` 溢出的理论兜底。
        let now = Instant::now();
        let wait = actions
            .iter()
            .copied()
            .min()
            .and_then(|earliest| earliest.checked_add(Duration::from_secs(self.window_secs)))
            .map(|expiry| expiry.saturating_duration_since(now))
            .unwrap_or(Duration::from_millis(1));
        Err(wait)
    }

    /// 清掉窗口外的记录，只留窗口内的。
    ///
    /// ## `checked_sub` 失败是**正常分支**，不是异常
    ///
    /// `Instant` 的基准点**没有保证**（Rust 只承诺单调，不承诺起点），而窗口
    /// （默认 3600 秒）完全可能比「从基准点到此刻的时长」还长。此时窗口下界落在
    /// 时钟基准**之前**，语义上就是**没有一条记录算旧**——全部保留。
    ///
    /// ⚠️ 不得写成 `.unwrap_or_else(Instant::now)`：那等于把「下溢」当成「窗口从此刻
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
