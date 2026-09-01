use super::model::AgentProfile;
use crate::plugins::agent::core::AgentConfig;
use crate::plugins::agent::store::build_store;
use std::path::Path;

pub struct ProfileLoader {
    config: AgentConfig,
}

impl Default for ProfileLoader {
    fn default() -> Self {
        Self::new()
    }
}

impl ProfileLoader {
    pub fn new() -> Self {
        Self {
            config: AgentConfig::default(),
        }
    }

    pub async fn load_from_path(&self, path: &Path) -> Option<AgentProfile> {
        let agent_id = extract_agent_id(path)?;
        // 合并用户配置与路径自动检测的配置
        let mut config = self.detect_config(path);
        if self.config.storage_format != config.storage_format {
            config.storage_format = self.config.storage_format;
        }
        let store = match build_store(&config, path).await {
            Ok(s) => s,
            Err(_e) => return None,
        };

        if let Ok(Some(identity_unit)) = store.get("identity").await {
            let name = identity_unit.name().unwrap_or(&agent_id).to_string();
            let description = identity_unit.description().unwrap_or_default().to_string();
            return Some(AgentProfile {
                id: agent_id,
                name,
                description,
                base_dir: path.to_path_buf(),
            });
        }

        if path.is_dir() {
            self.try_read_profile_file(path, agent_id).await
        } else {
            None
        }
    }

    fn detect_config(&self, path: &Path) -> AgentConfig {
        if path.is_file() {
            let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("yaml");
            match ext.to_lowercase().as_str() {
                "db" | "sqlite" => AgentConfig {
                    storage_backend: crate::plugins::agent::core::StorageBackendType::Sqlite,
                    ..AgentConfig::default()
                },
                _ => AgentConfig {
                    storage_backend: crate::plugins::agent::core::StorageBackendType::Dir,
                    ..AgentConfig::default()
                },
            }
        } else {
            AgentConfig::default()
        }
    }

    async fn try_read_profile_file(&self, dir: &Path, agent_id: String) -> Option<AgentProfile> {
        let json_path = dir.join("profile.json");
        if json_path.exists() {
            if let Ok(content) = tokio::fs::read_to_string(&json_path).await {
                if let Ok(mut profile) = serde_json::from_str::<AgentProfile>(&content) {
                    profile.id = agent_id;
                    profile.base_dir = dir.to_path_buf();
                    return Some(profile);
                }
            }
        }

        let yaml_path = dir.join("profile.yaml");
        if yaml_path.exists() {
            if let Ok(content) = tokio::fs::read_to_string(&yaml_path).await {
                if let Ok(mut profile) = serde_yaml_ng::from_str::<AgentProfile>(&content) {
                    profile.id = agent_id;
                    profile.base_dir = dir.to_path_buf();
                    return Some(profile);
                }
            }
        }

        None
    }
}

fn extract_agent_id(path: &Path) -> Option<String> {
    if path.is_dir() {
        path.file_name()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
    } else {
        path.file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
    }
}

#[cfg(test)]
#[path = "loader_tests.rs"]
mod tests;
