use super::engine_pool::AgentEnginePool;
use super::index::AgentIndex;
use super::model::AgentProfile;
use super::path::resolve_global_agents_dir;
use super::tracker::InitTracker;
use crate::plugins::agent::core::AgentConfig;
use crate::plugins::agent::core::AgentStore;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Agent 管理器（轻量门面）
///
/// 职责：聚合子模块，对外提供统一 API
///
/// 子模块：
/// - `AgentIndex`：Agent 发现、磁盘扫描、列表缓存
/// - `AgentEnginePool`：AgentStore 引擎缓存、生命周期管理
/// - `InitTracker`：初始化状态追踪
pub struct AgentManager {
    global_dir: PathBuf,
    index: AgentIndex,
    engine_pool: AgentEnginePool,
    tracker: InitTracker,
}

impl Default for AgentManager {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentManager {
    pub fn new() -> Self {
        // ⭐ 自动迁移：若旧目录 `~/.symbio/agents` 存在但新目录
        // `~/.symbio/plugins/agent` 不存在，把旧数据 move 到新位置
        Self::migrate_global_dir();

        let global_dir = resolve_global_agents_dir();
        Self {
            global_dir: global_dir.clone(),
            index: AgentIndex::new(global_dir),
            engine_pool: AgentEnginePool::new(),
            tracker: InitTracker::new(),
        }
    }

    /// 迁移旧全局 Agent 目录到新位置
    ///
    /// - 旧：`<homedir>/agents/`（仅当 homedir=~/.symbio 时存在）
    /// - 新：`<homedir>/plugins/agent/`
    ///
    /// 仅在新位置**完全不存在**时执行迁移，避免误覆盖。
    fn migrate_global_dir() {
        let new_dir = resolve_global_agents_dir();
        if new_dir.exists() {
            return;
        }
        // 旧路径只在原 ~/.symbio 布局下存在；homedir 切换后无需迁移
        let Some(home) = dirs::home_dir() else {
            return;
        };
        let legacy_dir = home.join(".symbio").join("agents");
        if !legacy_dir.exists() || !legacy_dir.is_dir() {
            return;
        }

        // 确保 plugins 父目录存在
        if let Some(parent) = new_dir.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        match std::fs::rename(&legacy_dir, &new_dir) {
            Ok(_) => {
                crate::plugin_info!(
                    "agent",
                    "全局 Agent 目录已从 {} 迁移到 {}",
                    legacy_dir.display(),
                    new_dir.display()
                );
            },
            Err(e) => {
                crate::plugin_warn!(
                    "agent",
                    "无法直接 move {} -> {}: {}，尝试逐项复制",
                    legacy_dir.display(),
                    new_dir.display(),
                    e
                );
                if let Err(copy_err) = Self::copy_dir_recursive(&legacy_dir, &new_dir) {
                    crate::plugin_error!("agent", "全局 Agent 数据迁移失败: {}", copy_err);
                } else {
                    let _ = std::fs::remove_dir_all(&legacy_dir);
                }
            },
        }
    }

    /// 递归复制目录
    fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dst)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            let ty = entry.file_type()?;
            let src_child = entry.path();
            let dst_child = dst.join(entry.file_name());
            if ty.is_dir() {
                Self::copy_dir_recursive(&src_child, &dst_child)?;
            } else {
                std::fs::copy(&src_child, &dst_child)?;
            }
        }
        Ok(())
    }

    pub async fn list_agents(&self, workdir: Option<&str>) -> Vec<AgentProfile> {
        self.index.list(workdir).await
    }

    pub async fn get_agent(&self, workdir: Option<&str>, id: &str) -> Option<AgentProfile> {
        self.index.get(workdir, id).await
    }

    pub fn global_dir(&self) -> &Path {
        &self.global_dir
    }

    pub async fn get_agent_engine(
        &self,
        workdir: Option<&str>,
        agent_id: &str,
        config: &AgentConfig,
    ) -> Option<Arc<dyn AgentStore>> {
        let profile = self.get_agent(workdir, agent_id).await?;
        self.engine_pool.get(&profile, config).await
    }

    pub async fn invalidate_cache_for_workdir(&self, workdir: Option<&str>) {
        self.index.invalidate(workdir).await;
    }

    /// 删除指定 agent 的物理目录并清理索引 / 引擎缓存
    ///
    /// 返回：
    /// - `Ok(true)`：找到并删除
    /// - `Ok(false)`：agent 不存在（幂等）
    /// - `Err(msg)`：删除失败
    ///
    /// **统一来源**：仅删除 `~/.symbio/plugins/agent/{id}` 下的目录，
    /// 不再 fallback 到 `{workdir}/.symbio/agents/{id}`（已弃用 workspace 级别存储）。
    pub async fn delete_agent(&self, _workdir: Option<&str>, id: &str) -> Result<bool, String> {
        use tokio::fs;

        // 1. 在全局索引中找到 agent 的物理路径
        let Some(profile) = self.index.get(None, id).await else {
            return Ok(false);
        };

        // profile.base_dir 是 index 扫描时记录的物理目录
        // 对于全局 agent，base_dir 应该就是 global_dir/{id}
        let agent_dir = if profile.base_dir.starts_with(&self.global_dir) {
            profile.base_dir.clone()
        } else {
            // 兜底：旧数据可能不在 global_dir 下，构造预期路径
            self.global_dir.join(id)
        };

        // 2. 清理 engine pool 中的缓存
        self.engine_pool.evict(&agent_dir).await;

        // 3. 物理删除目录
        if agent_dir.exists() {
            fs::remove_dir_all(&agent_dir)
                .await
                .map_err(|e| format!("删除目录 {agent_dir:?} 失败: {e}"))?;
        }

        // 4. 失效索引缓存
        self.invalidate_cache_for_workdir(None).await;

        Ok(true)
    }

    pub async fn shutdown_all(&self) {
        self.engine_pool.shutdown_all().await;
    }

    pub async fn is_initialized(&self, workdir: Option<&str>) -> bool {
        self.tracker.is_initialized(workdir).await
    }

    pub async fn mark_initialized(&self, workdir: Option<&str>) {
        self.tracker.mark_initialized(workdir).await;
    }

    /// 解析 agent 物理路径（仅全局目录）
    ///
    /// `workdir` 参数已弃用，保留仅为测试兼容。所有 agent 都存放在 `~/.symbio/plugins/agent/`。
    #[cfg(test)]
    #[allow(unused_variables)]
    pub(crate) fn get_agent_path(
        &self,
        workdir: Option<&str>,
        id: &str,
    ) -> Result<PathBuf, String> {
        if id.trim().is_empty() {
            return Err("agent_id cannot be empty".to_string());
        }

        let p = self.global_dir.join(id);
        if p.exists() {
            return Ok(p);
        }
        for ext in ["yaml", "json", "db"] {
            let p = self.global_dir.join(format!("{id}.{ext}"));
            if p.exists() {
                return Ok(p);
            }
        }

        Err(format!("Agent '{id}' not found"))
    }
}

#[cfg(test)]
#[path = "manager_tests.rs"]
mod tests;
