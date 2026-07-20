use std::collections::HashSet;
use tokio::sync::RwLock;

/// 初始化状态追踪器
///
/// 职责：追踪哪些 workdir 已完成 archetype agent 初始化
///
/// 当前实现：
/// - 内存模式：进程重启后重新初始化
pub struct InitTracker {
    initialized: RwLock<HashSet<String>>,
}

impl Default for InitTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl InitTracker {
    pub fn new() -> Self {
        Self {
            initialized: RwLock::new(HashSet::new()),
        }
    }

    pub async fn is_initialized(&self, workdir: Option<&str>) -> bool {
        let key = workdir.unwrap_or("").to_string();
        self.initialized.read().await.contains(&key)
    }

    pub async fn mark_initialized(&self, workdir: Option<&str>) {
        let key = workdir.unwrap_or("").to_string();
        self.initialized.write().await.insert(key);
    }
}

#[cfg(test)]
#[path = "tracker_tests.rs"]
mod tests;
