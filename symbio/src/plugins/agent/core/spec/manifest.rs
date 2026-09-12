//! Bundle 清单（manifest.yaml）声明式数据结构。
//!
//! 全部类型为纯 serde 数据——零宿主依赖。字段语义见
//! `docs/design/open-agent-bundle-spec.md` §4。
//!
//! manifest 只承载**无法从约定目录推导的信息**：身份、兼容性门槛、权限上限、
//! 实例配置。能力单元（prompts / skills / mcps）由约定目录承载，manifest
//! 不重复登记。

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ═══════════════════════════════════════════════════════════════════
// manifest.yaml
// ═══════════════════════════════════════════════════════════════════

/// Bundle 清单（`manifest.yaml` / `.yml` / `.json`）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BundleManifest {
    /// 规格标识，固定 `"oab/v1"`
    pub spec: String,
    /// 全局唯一 id（反向域名风格），`[a-z0-9]` 开头
    pub id: String,
    /// 展示名
    pub name: String,
    /// semver 版本
    pub version: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub authors: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    /// 包内相对路径（图标）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,

    /// 兼容性声明
    #[serde(skip_serializing_if = "Requires::is_empty")]
    pub requires: Requires,

    /// 权限声明（沙箱「声明」半边）
    #[serde(skip_serializing_if = "Permissions::is_empty")]
    pub permissions: Permissions,

    /// 包级配置（schema 驱动宿主 UI 表单；defaults 为实例默认值）
    #[serde(skip_serializing_if = "BundleConfig::is_empty")]
    pub config: BundleConfig,
}

impl BundleManifest {
    /// spec 标识是否为 `"oab/vN"` 形态且主版本受本宿主支持
    pub fn spec_matches(&self) -> bool {
        self.spec == crate::plugins::agent::core::SPEC_ID
            || self
                .spec
                .strip_prefix("oab/v")
                .and_then(|v| v.parse::<u64>().ok())
                .map(|v| v == crate::plugins::agent::core::SPEC_MAJOR)
                .unwrap_or(false)
    }
}

/// **版本匹配判定（接入门槛，规范 §11）**。
///
/// bundle 的 `requires.spec`（如 `"^1"`）与宿主支持的 spec 主版本严格匹配：
/// major 相等 → 可接入；不相等 → **拒绝接入（硬错误，不做任何降级）**。
///
/// v1 仅支持 `^N`（或精确 `N`）形式；minor/patch 不参与匹配（协议承诺：
/// 同一主版本内新增字段全部可选，向后兼容）。
pub fn spec_requirement_matches(req: &str, host_major: u64) -> bool {
    let s = req.trim();
    let major = s.strip_prefix('^').unwrap_or(s).trim();
    major
        .parse::<u64>()
        .map(|n| n == host_major)
        .unwrap_or(false)
}

/// 兼容性声明（manifest.requires）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Requires {
    /// 宿主必须支持的 OAB 主版本区间，如 `"^1"`
    #[serde(skip_serializing_if = "String::is_empty")]
    pub spec: String,
    /// bundle 级宿主 API 需求（声明式；参考实现用于提示，不强制沙箱）
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub host_apis: Vec<String>,
}

impl Requires {
    fn is_empty(&self) -> bool {
        self.spec.is_empty() && self.host_apis.is_empty()
    }
}

/// 权限声明（manifest.permissions）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Permissions {
    /// bundle 级宿主 API 声明（白名单提示）
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub host_apis: Vec<String>,
    #[serde(skip_serializing_if = "NetworkPerms::is_empty")]
    pub network: NetworkPerms,
    #[serde(skip_serializing_if = "FsPerms::is_empty")]
    pub fs: FsPerms,
    #[serde(skip_serializing_if = "Limits::is_empty")]
    pub limits: Limits,
}

impl Permissions {
    fn is_empty(&self) -> bool {
        self.host_apis.is_empty()
            && self.network.is_empty()
            && self.fs.is_empty()
            && self.limits.is_empty()
    }
}

/// 网络权限：HTTP(S) 出网域名白名单。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct NetworkPerms {
    /// 精确域名或 `*.example.com` 通配；空 = 禁止出网
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub domains: Vec<String>,
}

impl NetworkPerms {
    fn is_empty(&self) -> bool {
        self.domains.is_empty()
    }
}

/// 文件系统权限：bundle 目录内的 glob 范围。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FsPerms {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub read: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub write: Vec<String>,
}

impl FsPerms {
    fn is_empty(&self) -> bool {
        self.read.is_empty() && self.write.is_empty()
    }
}

/// 运行限额（参考实现轻量强制）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Limits {
    /// 工具数量上限
    pub max_tools: usize,
    /// 提示词总字节数上限
    pub max_prompt_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_tools: 32,
            max_prompt_bytes: 65_536,
        }
    }
}

impl Limits {
    fn is_empty(&self) -> bool {
        false // 有非默认值时总是序列化；Default 实例由上层 skip 判断（容忍冗余序列化）
    }
}

/// 包级配置（manifest.config）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BundleConfig {
    /// JSON Schema（驱动宿主 UI 表单）
    #[serde(skip_serializing_if = "Value::is_null")]
    pub schema: Value,
    /// 实例默认配置
    #[serde(skip_serializing_if = "Value::is_null")]
    pub defaults: Value,
}

impl BundleConfig {
    fn is_empty(&self) -> bool {
        self.schema.is_null() && self.defaults.is_null()
    }
}

// ── 小工具 ──

pub(crate) fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_minimal_parses() {
        let json = r#"{
            "spec": "oab/v1",
            "id": "com.acme.cr",
            "name": "评审专家",
            "version": "1.0.0"
        }"#;
        let m: BundleManifest = serde_json::from_str(json).unwrap();
        assert!(m.spec_matches());
        // 未知字段容忍（向前兼容）
        let m2: BundleManifest =
            serde_json::from_str(&json.replace("\"version\"", "\"future_field\": 1, \"version\""))
                .unwrap();
        assert_eq!(m2.id, "com.acme.cr");
    }

    #[test]
    fn manifest_with_permissions() {
        let json = r#"{
            "spec": "oab/v1", "id": "x", "name": "X", "version": "0.1.0",
            "requires": { "spec": "^1", "host_apis": ["session.read"] },
            "permissions": {
                "host_apis": ["session.read", "kv", "log"],
                "network": { "domains": ["api.github.com"] },
                "limits": { "max_tools": 16 }
            }
        }"#;
        let m: BundleManifest = serde_json::from_str(json).unwrap();
        assert_eq!(m.permissions.limits.max_tools, 16);
        assert!(m.permissions.host_apis.contains(&"log".to_string()));
        assert_eq!(
            m.permissions.network.domains,
            vec!["api.github.com".to_string()]
        );
    }
}
