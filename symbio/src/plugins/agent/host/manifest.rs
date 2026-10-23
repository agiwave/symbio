//! Agent 目录的 `manifest.yaml`（规范 §5）
//!
//! 只承载**无法从目录推导的信息**：身份、兼容性门槛、权限声明。能力由目录承载
//! （§4.1），因此这里**没有** `skills` / `mcps` / `prompts` 之类登记目录事实的字段
//! ——登记一份、目录一份，两份真相必然漂移。
//!
//! 未知字段**一律忽略**（§5.1 向前兼容），解析因此用 `#[serde(default)]` 而非
//! `deny_unknown_fields`。

use serde::{Deserialize, Serialize};
use std::path::Path;

/// 本宿主支持的协议主版本（`requires.spec: "^2"` 的判定基准，§10）
pub const SPEC_MAJOR: u64 = 2;

/// manifest 文件名（§5：`manifest.yaml` 为规范名，`.yml` / `.json` 容忍读取）
pub const MANIFEST_NAMES: [&str; 3] = ["manifest.yaml", "manifest.yml", "manifest.json"];

/// Agent 清单
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentManifest {
    /// 规格标识，固定 `"agent-dir/v2"`
    pub spec: String,
    /// 唯一 id（`[a-z0-9]` 开头，仅含小写字母/数字/`.`/`-`/`_`，长度 1–128）
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
    /// Agent 目录内的相对路径（图标）；**不得**指向目录外
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,

    /// 兼容性声明（§10）
    #[serde(skip_serializing_if = "Requires::is_empty")]
    pub requires: Requires,

    /// **声明性质**的权限清单（§5.3）：包作者与宿主之间的沟通面，**不是安全边界**。
    ///
    /// 运行期没有任何一处代码读它——四个字段（`fs` / `network` / `limits`）都没有
    /// 执行方。字段名因此回到「它到底是什么」：`declared_permissions`。
    /// 宿主侧真正的边界是授权表（[D-099](../../../../../docs/003-design/011-Agent目录规范.md#d-099-授权默认翻-fail-closed未登记主体不再派生主智能体能力集认不出的能力名按已收回判 "DE-011-Agent目录规范")）
    /// 与审批闸（[D-068](../../../../../docs/003-design/008-能力注册表与能力坐标系.md#d-068-审批闸门住在能力注册的收口点defaulttoolvisitorregister判据一份入口两处 "DE-008-能力注册表与能力坐标系")）；
    /// 为什么不在这里补执行方、以及补的时候要满足什么，见
    /// [D-102](../../../../../docs/003-design/011-Agent目录规范.md#d-102-agent-权限声明当前无执行方字段改回声明性质补齐须四字段同批 "DE-011-Agent目录规范")。
    ///
    /// **线格式不变**（`permissions`）：改名的是 Rust 侧语义，不是 Agent 包的公开键。
    #[serde(rename = "permissions", skip_serializing_if = "Permissions::is_empty")]
    pub declared_permissions: Permissions,
}

/// **版本匹配判定（接入门槛，§10）**
///
/// Agent 的 `requires.spec`（如 `"^2"`）与宿主支持的主版本**严格相等**方可接入；
/// 不相等**必须拒绝**，且不得静默降级。minor / patch 不参与匹配。
pub fn spec_requirement_matches(req: &str, host_major: u64) -> bool {
    let s = req.trim();
    let major = s.strip_prefix('^').unwrap_or(s).trim();
    major
        .parse::<u64>()
        .map(|n| n == host_major)
        .unwrap_or(false)
}

/// 兼容性声明
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Requires {
    /// 依赖的协议主版本，如 `"^2"`（§10：**必须**）
    #[serde(skip_serializing_if = "String::is_empty")]
    pub spec: String,
    /// 宿主 API 需求（声明式）
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub host_apis: Vec<String>,
}

impl Requires {
    fn is_empty(&self) -> bool {
        self.spec.is_empty() && self.host_apis.is_empty()
    }
}

/// 权限声明
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Permissions {
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

/// 网络权限：出网域名白名单
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct NetworkPerms {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub domains: Vec<String>,
}

impl NetworkPerms {
    fn is_empty(&self) -> bool {
        self.domains.is_empty()
    }
}

/// 文件系统权限：Agent 目录内的 glob 范围
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

/// 运行限额
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Limits {
    pub max_tools: usize,
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
        false
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 读取与校验
// ═══════════════════════════════════════════════════════════════════════════

/// 从 Agent 目录读取 manifest（按 [`MANIFEST_NAMES`] 依次尝试）
pub fn load(dir: &Path) -> Option<AgentManifest> {
    MANIFEST_NAMES
        .iter()
        .map(|n| dir.join(n))
        .find(|p| p.is_file())
        .and_then(|p| {
            let raw = std::fs::read_to_string(&p).ok()?;
            let is_json = p.extension().and_then(|e| e.to_str()) == Some("json");
            if is_json {
                serde_json::from_str::<AgentManifest>(&raw).ok()
            } else {
                serde_yaml_ng::from_str::<AgentManifest>(&raw).ok()
            }
        })
}

/// **加载期校验（§10）**。返回 `Err` 即拒绝接入，错误信息写明双侧版本。
///
/// 校验项：`spec` 必须是 `agent-dir/v2`；`id` 合字符集；`requires.spec` 与宿主
/// 主版本匹配。缺失 / 不合规则一律拒绝——**不静默降级为无人格的通用助手**。
pub fn validate(m: &AgentManifest) -> Result<(), String> {
    if m.spec != super::plugin::SPEC_V2 {
        return Err(format!(
            "manifest.spec 为 `{}`，本宿主只支持 `{}`",
            if m.spec.is_empty() {
                "（缺失）"
            } else {
                &m.spec
            },
            super::plugin::SPEC_V2
        ));
    }
    if !is_valid_id(&m.id) {
        return Err(format!(
            "manifest.id `{}` 不合规：须以 [a-z0-9] 开头，仅含小写字母/数字/`.`/`-`/`_`，长度 1–128",
            m.id
        ));
    }
    if m.name.trim().is_empty() {
        return Err("manifest.name 缺失".to_string());
    }
    if !spec_requirement_matches(&m.requires.spec, SPEC_MAJOR) {
        return Err(format!(
            "版本不匹配：Agent 依赖 `{}`，本宿主支持 `agent-dir/v{SPEC_MAJOR}`（§10 拒绝接入）",
            if m.requires.spec.is_empty() {
                "（未声明 requires.spec）"
            } else {
                &m.requires.spec
            }
        ));
    }
    Ok(())
}

/// id 字符集（§5.1）——**这条字符集就是「主体 id 不许自带命名空间」的执行方**。
///
/// 包侧 id 是裸 slug：主体的完整地址由 core 一处派生（`authz::principal_of` 把裸名兜底成
/// `session/<id>`，对**已带命名空间**的原样认下）。所以「不含 `/`」不是排版洁癖，是身份
/// 边界——放开斜杠等于给包开一条自述族属的入口，`id: person/local` 的包会直接从
/// 「原样认下」那一支变成人。两侧的语法与命名空间登记表之间的一致性判据钉在
/// `authz.test.rs::principal_of_derives_a_bare_asset_id_into_the_agent_family`。
fn is_valid_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    if bytes.is_empty() || bytes.len() > 128 {
        return false;
    }
    if !(bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit()) {
        return false;
    }
    bytes
        .iter()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'-' | b'_'))
}

#[cfg(test)]
#[path = "manifest.test.rs"]
mod tests;
