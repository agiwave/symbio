use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, RwLock};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use symbio_plugin_api::collectors::ApprovalGrants;
use symbio_plugin_api::CapabilityRiskLevel;

const FILE: &str = "approval-grants.json";
const VERSION: u8 = 1;
const LEGACY_FIELD: &str = "approval_grants";

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default)]
struct GrantFile {
    version: u8,
    profiles: BTreeMap<String, ApprovalGrants>,
}

pub(crate) struct GrantStore {
    path: PathBuf,
    data: Mutex<GrantFile>,
    load_error: RwLock<Option<String>>,
}

impl GrantStore {
    pub(crate) fn load(dir: &Path, legacy_session_dir: &Path) -> Self {
        let path = dir.join(FILE);
        let mut store = Self {
            path,
            data: Mutex::new(GrantFile::default()),
            load_error: RwLock::new(None),
        };
        if let Err(error) = store.load_locked(legacy_session_dir) {
            *store.load_error.write().unwrap_or_else(|p| p.into_inner()) = Some(error);
        }
        store
    }

    pub(crate) fn ensure_ready(&self) -> Result<(), String> {
        self.load_error
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
            .map_or(Ok(()), Err)
    }

    pub(crate) fn list(&self, profile_id: &str) -> ApprovalGrants {
        self.data
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .profiles
            .get(profile_id)
            .cloned()
            .unwrap_or_default()
    }

    pub(crate) fn grant(
        &self,
        profile_id: &str,
        tool: &str,
        risk: CapabilityRiskLevel,
    ) -> Result<ApprovalGrants, String> {
        self.ensure_ready()?;
        let mut next = self.data.lock().unwrap_or_else(|p| p.into_inner()).clone();
        let grants = next.profiles.entry(profile_id.to_string()).or_default();
        grants.raise(tool, risk);
        let result = grants.clone();
        self.write(next)?;
        Ok(result)
    }

    pub(crate) fn revoke(&self, profile_id: &str, tool: &str) -> Result<ApprovalGrants, String> {
        self.ensure_ready()?;
        let mut next = self.data.lock().unwrap_or_else(|p| p.into_inner()).clone();
        let grants = next.profiles.entry(profile_id.to_string()).or_default();
        grants.revoke(tool);
        let result = grants.clone();
        self.write(next)?;
        Ok(result)
    }

    fn load_locked(&mut self, legacy_session_dir: &Path) -> Result<(), String> {
        if self.path.exists() {
            let text = fs::read_to_string(&self.path)
                .map_err(|e| format!("读取 {} 失败：{e}", self.path.display()))?;
            let file: GrantFile = serde_json::from_str(&text)
                .map_err(|e| format!("解析 {} 失败：{e}", self.path.display()))?;
            if file.version != VERSION {
                return Err(format!("不支持的授权存储版本：{}", file.version));
            }
            *self.data.lock().unwrap_or_else(|p| p.into_inner()) = file;
            return Ok(());
        }

        let legacy_path = legacy_session_dir.join("PLUGIN.yml");
        if !legacy_path.exists() {
            return Ok(());
        }
        let text = fs::read_to_string(&legacy_path)
            .map_err(|e| format!("读取旧会话配置 {} 失败：{e}", legacy_path.display()))?;
        let mut manifest: Value = serde_yaml_ng::from_str(&text)
            .map_err(|e| format!("解析旧会话配置 {} 失败：{e}", legacy_path.display()))?;
        let Some(object) = manifest.as_object_mut() else {
            return Err(format!("{} 顶层必须是映射", legacy_path.display()));
        };
        let Some(raw) = object.get(LEGACY_FIELD) else {
            return Ok(());
        };
        let legacy: ApprovalGrants = serde_json::from_value(raw.clone())
            .map_err(|e| format!("旧会话永久授权无法迁移：{e}"))?;
        if legacy.is_empty() {
            object.remove(LEGACY_FIELD);
            atomic_write(&legacy_path, &manifest)?;
            return Ok(());
        }

        let backup = legacy_path.with_file_name("PLUGIN.yml.approval-grants.bak");
        if backup.exists() {
            return Err(format!("旧授权备份已存在，拒绝覆盖：{}", backup.display()));
        }
        fs::write(&backup, &text)
            .map_err(|e| format!("备份旧会话授权 {} 失败：{e}", backup.display()))?;

        let mut next = GrantFile {
            version: VERSION,
            ..GrantFile::default()
        };
        next.profiles.insert("default".into(), legacy);
        atomic_write(&self.path, &next)?;
        object.remove(LEGACY_FIELD);
        atomic_write(&legacy_path, &manifest)?;
        *self.data.lock().unwrap_or_else(|p| p.into_inner()) = next;
        Ok(())
    }

    fn write(&self, mut next: GrantFile) -> Result<(), String> {
        next.version = VERSION;
        atomic_write(&self.path, &next)?;
        *self.data.lock().unwrap_or_else(|p| p.into_inner()) = next;
        Ok(())
    }
}

fn atomic_write(path: &Path, value: &impl Serialize) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("创建授权目录 {} 失败：{e}", parent.display()))?;
    }
    let text = if path.extension().is_some_and(|ext| ext == "json") {
        serde_json::to_string_pretty(value)
            .map_err(|e| format!("序列化 {} 失败：{e}", path.display()))?
    } else {
        serde_yaml_ng::to_string(value)
            .map_err(|e| format!("序列化 {} 失败：{e}", path.display()))?
    };
    let tmp = path.with_extension(format!(
        "{}.{}.tmp",
        path.extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("data"),
        std::process::id()
    ));
    fs::write(&tmp, text).map_err(|e| format!("写入 {} 失败：{e}", tmp.display()))?;
    fs::rename(&tmp, path).map_err(|e| format!("落盘 {} 失败：{e}", path.display()))
}

#[cfg(test)]
#[path = "grants.test.rs"]
mod tests;
