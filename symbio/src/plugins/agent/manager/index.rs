use super::loader::ProfileLoader;
use super::model::AgentProfile;
use dashmap::DashMap;
use moka::future::Cache;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::fs;
use tokio::sync::OnceCell;

type AgentListSlot = Arc<OnceCell<Arc<Vec<AgentProfile>>>>;

pub struct AgentIndex {
    global_dir: PathBuf,
    list_cache: Cache<String, Arc<Vec<AgentProfile>>>,
    flight_gate: DashMap<String, AgentListSlot>,
}

impl AgentIndex {
    pub fn new(global_dir: PathBuf) -> Self {
        let list_cache = Cache::builder()
            .max_capacity(32)
            .time_to_idle(Duration::from_secs(10 * 60))
            .build();

        Self {
            global_dir,
            list_cache,
            flight_gate: DashMap::new(),
        }
    }

    /// 列出全部 agent
    ///
    /// **统一来源**：仅扫描全局目录 `~/.symbio/plugins/agent/`
    ///
    /// 不再扫描 `{workdir}/.symbio/agents/`，原因：
    /// - 难以遍历"所有会话的所有 workdir"
    /// - Agent 是**全局共享**的人格定义，应存放在系统级目录
    /// - UI 中 AgentView 不需要按 workdir 过滤
    ///
    /// `workdir` 参数仅用于**缓存 key 区分**（避免不同项目间的缓存混淆），
    /// 不影响扫描范围。
    pub async fn list(&self, workdir: Option<&str>) -> Vec<AgentProfile> {
        let key = workdir.unwrap_or("").to_string();

        if let Some(agents) = self.list_cache.get(&key).await {
            return (*agents).clone();
        }

        let cell = self
            .flight_gate
            .entry(key.clone())
            .or_insert_with(|| Arc::new(OnceCell::new()))
            .value()
            .clone();

        let arc_all = cell
            .get_or_init(|| async {
                let mut all = Vec::new();
                // ⭐ 只扫描全局目录
                self.scan_dir(&self.global_dir, &mut all).await;
                Arc::new(all)
            })
            .await
            .clone();

        self.list_cache.insert(key, arc_all.clone()).await;
        (*arc_all).clone()
    }

    pub async fn get(&self, workdir: Option<&str>, id: &str) -> Option<AgentProfile> {
        self.list(workdir).await.into_iter().find(|p| p.id == id)
    }

    pub async fn invalidate(&self, workdir: Option<&str>) {
        match workdir {
            Some(w) => {
                self.list_cache.invalidate(w).await;
                self.flight_gate.remove(w);
            },
            None => {
                self.list_cache.invalidate_all();
                self.flight_gate.clear();
            },
        }
    }

    async fn scan_dir(&self, dir: &Path, out: &mut Vec<AgentProfile>) {
        let Ok(mut entries) = fs::read_dir(dir).await else {
            return;
        };
        let loader = ProfileLoader::new();
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            let is_dir = entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false);
            let should_load = is_dir
                || path
                    .extension()
                    .and_then(|s| s.to_str())
                    .map(|ext| ["yaml", "json", "db"].contains(&ext.to_lowercase().as_str()))
                    .unwrap_or(false);

            if should_load {
                match loader.load_from_path(&path).await {
                    Some(profile) => out.push(profile),
                    None => tracing::debug!(
                        "[AgentIndex] Skipping agent at {:?}: no identity unit or profile file",
                        path
                    ),
                }
            }
        }
    }
}
