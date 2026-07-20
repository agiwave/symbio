use super::model::AgentProfile;
use super::path;
use crate::plugins::agent::core::{AgentConfig, AgentStore};
use crate::plugins::agent::store::build_store;
use moka::future::Cache;
use std::sync::Arc;
use std::time::Duration;

pub struct AgentEnginePool {
    cache: Cache<String, Arc<dyn AgentStore>>,
}

impl Default for AgentEnginePool {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentEnginePool {
    pub fn new() -> Self {
        let cache = Cache::builder()
            .max_capacity(100)
            .time_to_idle(Duration::from_secs(30 * 60))
            .build();

        Self { cache }
    }

    pub async fn get(
        &self,
        profile: &AgentProfile,
        config: &AgentConfig,
    ) -> Option<Arc<dyn AgentStore>> {
        let key = path::normalize_cache_key(&profile.base_dir);

        if let Some(engine) = self.cache.get(&key).await {
            return Some(engine);
        }

        let engine = match build_store(config, &profile.base_dir).await {
            Ok(e) => e,
            Err(_) => return None,
        };
        self.cache.insert(key, engine.clone()).await;
        Some(engine)
    }

    pub async fn shutdown_all(&self) {
        let mut count = 0usize;
        for (_key, engine) in self.cache.iter() {
            engine.shutdown().await;
            count += 1;
        }
        tracing::info!(
            "[AgentEnginePool] shutdown_all: shut down {} engines",
            count
        );
    }

    /// 从缓存中驱逐一个 engine（按 base_dir 匹配）
    ///
    /// 删除 agent 时调用，避免缓存里的旧 engine 持有已删除目录的文件句柄。
    /// `Ok(true)` 表示成功驱逐，`Ok(false)` 表示缓存里没有该 key。
    pub async fn evict(&self, base_dir: &std::path::Path) -> bool {
        let key = path::normalize_cache_key(base_dir);
        self.cache.invalidate(&key).await;
        self.cache.get(&key).await.is_none()
    }
}
