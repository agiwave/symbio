use super::manager::AgentManager;
use crate::plugins::agent::core::CognitiveUnit;
use crate::plugins::agent::core::{AgentConfig, StorageFormat};
use std::path::Path;
use tokio::fs;

/// 预设智能体的内置定义版本号。
///
/// 设计改进：用 semver `"X.Y.Z"` 取代之前的 `"1"` 字符串：
/// - `X`：破坏性变更（必须重写所有系统认知单元）
/// - `Y`：新增系统认知单元
/// - `Z`：仅修正错误
const BUILTIN_AGENTS_VERSION: &str = "1.0.0";

/// 版本标记文件名，存储在 agents 根目录下
const VERSION_MARKER_FILE: &str = ".symbio_version";

pub struct AgentRegistry;

impl AgentRegistry {
    pub async fn ensure_initialized(
        store: &AgentManager,
        workdir: Option<&str>,
        config: &AgentConfig,
    ) -> std::io::Result<()> {
        if store.is_initialized(workdir).await {
            return Ok(());
        }

        // ⭐ 统一来源：只初始化全局目录
        let global_dir = store.global_dir();
        if !global_dir.exists() {
            fs::create_dir_all(global_dir).await?;
        }

        Self::init_default_agents(store, config).await?;
        store.mark_initialized(workdir).await;

        Ok(())
    }

    /// 检查版本标记，判断是否需要初始化或升级
    async fn read_version_marker(dir: &Path) -> Option<String> {
        let marker_path = dir.join(VERSION_MARKER_FILE);
        fs::read_to_string(marker_path)
            .await
            .ok()
            .map(|s| s.trim().to_string())
    }

    async fn write_version_marker(dir: &Path, version: &str) -> std::io::Result<()> {
        let marker_path = dir.join(VERSION_MARKER_FILE);
        fs::write(marker_path, version).await
    }

    /// 初始化/升级预置 archetype Agents
    ///
    /// 流程：
    /// 1. 读取磁盘上的版本标记
    /// 2. 首次安装：写入全部系统认知单元，写入版本标记
    /// 3. 版本一致：跳过
    /// 4. 版本升级：重新写入所有系统认知单元（id 稳定，所以是 upsert 语义）
    ///
    /// 改进点：
    /// - 错误处理用 `?` + 显式 `match`，不再静默吞错
    /// - 加日志输出每个 archetype 的处理数量
    async fn init_default_agents(
        store: &AgentManager,
        config: &AgentConfig,
    ) -> std::io::Result<()> {
        let agents: &[(&str, &str)] = &[
            ("normal", include_str!("normal_agent_units.jsonl")),
            ("deep_thinker", include_str!("thinker_agent_units.jsonl")),
            ("code_expert", include_str!("expert_agent_units.jsonl")),
        ];

        let global_dir = store.global_dir();
        let installed_version = Self::read_version_marker(global_dir).await;

        // 首次安装：直接写入全部
        if installed_version.is_none() {
            for (id, agent_units_str) in agents {
                let p_dir = global_dir.join(id);
                if !p_dir.exists() {
                    fs::create_dir_all(&p_dir).await?;
                }
                let count = Self::write_units_to_dir(&p_dir, agent_units_str, config).await?;
                crate::plugin_info!("agent", "Installed archetype '{}' with {} units", id, count);
            }
            Self::write_version_marker(global_dir, BUILTIN_AGENTS_VERSION).await?;
            return Ok(());
        }

        // 版本一致：跳过
        if installed_version.as_deref() == Some(BUILTIN_AGENTS_VERSION) {
            return Ok(());
        }

        // 版本升级
        crate::plugin_info!(
            "agent",
            "Upgrading builtin agents from {:?} to {}",
            installed_version,
            BUILTIN_AGENTS_VERSION
        );

        for (id, agent_units_str) in agents {
            let p_dir = global_dir.join(id);
            if !p_dir.exists() {
                fs::create_dir_all(&p_dir).await?;
            }
            // 写入/更新（注意：不清理已移除的旧系统认知单元，保留用户可能的自定义内容）
            let count = Self::write_units_to_dir(&p_dir, agent_units_str, config).await?;
            crate::plugin_info!("agent", "Upgraded archetype '{}' with {} units", id, count);
        }

        Self::write_version_marker(global_dir, BUILTIN_AGENTS_VERSION).await?;
        Ok(())
    }

    /// 把 JSONL 内容写入到指定目录
    ///
    /// 返回成功写入的认知单元数量
    async fn write_units_to_dir(
        p_dir: &Path,
        agent_units_str: &str,
        config: &AgentConfig,
    ) -> std::io::Result<usize> {
        let mut count = 0;
        for line in agent_units_str.lines() {
            let au: CognitiveUnit = match serde_json::from_str::<CognitiveUnit>(line) {
                Ok(v) => v,
                Err(e) => {
                    crate::plugin_warn!(
                        "agent",
                        "Failed to parse agent unit line in {}: {}",
                        p_dir.display(),
                        e
                    );
                    continue;
                }
            };
            Self::write_unit_to_storage(p_dir, au, config).await?;
            count += 1;
        }
        Ok(count)
    }

    async fn write_unit_to_storage(
        p_dir: &Path,
        au: CognitiveUnit,
        config: &AgentConfig,
    ) -> std::io::Result<()> {
        let raw_id = au.id();
        let id = if raw_id.is_empty() { "unknown" } else { raw_id };
        let safe_id = id.replace("::", "__").replace('/', "_");
        let ext = if config.storage_format == StorageFormat::Yaml {
            "yaml"
        } else {
            "json"
        };
        let path = p_dir.join(format!("{safe_id}.{ext}"));

        let content = if config.storage_format == StorageFormat::Yaml {
            serde_yaml_ng::to_string(&au).map_err(std::io::Error::other)?
        } else {
            serde_json::to_string_pretty(&au).map_err(std::io::Error::other)?
        };

        fs::write(path, content).await?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
