use std::sync::Mutex;
use std::time::Instant;

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

    fn cleanup_old_actions(&self, actions: &mut Vec<Instant>) {
        let cutoff = Instant::now()
            .checked_sub(std::time::Duration::from_secs(self.window_secs))
            .unwrap_or_else(Instant::now);
        actions.retain(|t| *t > cutoff);
    }
}

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
