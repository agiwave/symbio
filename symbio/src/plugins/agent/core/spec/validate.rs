//! manifest 加载期校验（规范 §4 / §8 / §11）。
//!
//! 校验是**接入判定的第一道门**：任何一项失败 = bundle 拒绝加载（硬错误），
//! 不存在静默降级。bundle 作者应在分发前用同等规则自检。

use super::manifest::{spec_requirement_matches, BundleManifest};

/// 校验结果：Err 携带全部问题（bundle 作者一次看全）。
pub fn validate_manifest(m: &BundleManifest, host_major: u64) -> Result<(), Vec<String>> {
    let mut errs: Vec<String> = Vec::new();

    // ── spec 标识（oab/v1 形态且主版本受支持）──
    if m.spec.is_empty() {
        errs.push("缺少 `spec` 字段（如 \"oab/v1\"）".into());
    } else if !m.spec_matches() {
        errs.push(format!(
            "不支持的 spec 标识 `{}`（宿主支持 oab/v{host_major}）",
            m.spec
        ));
    }

    // ── 版本匹配（硬性接入门槛）──
    if !m.requires.spec.is_empty() && !spec_requirement_matches(&m.requires.spec, host_major) {
        errs.push(format!(
            "版本不兼容：bundle 要求 oab `{}`，宿主支持 oab/v{host_major}（拒绝接入）",
            m.requires.spec
        ));
    }

    // ── 身份字段 ──
    if m.id.is_empty() {
        errs.push("缺少 `id` 字段".into());
    } else if !valid_bundle_id(&m.id) {
        errs.push(format!(
            "id `{}` 非法：仅允许小写字母/数字/./-/ _，且以字母或数字开头",
            m.id
        ));
    }
    if m.name.trim().is_empty() {
        errs.push("缺少 `name` 字段".into());
    }
    if m.version.trim().is_empty() {
        errs.push("缺少 `version` 字段（semver）".into());
    }

    // 说明：约定优于配置——provider 由约定目录承载（prompts/ skills/ mcps/，
    // 存在即安装），manifest 不列出 provider 清单，因此这里没有 provider 清单校验；
    // 目录内容的问题（缺 SKILL.md、未知 native 模块等）在 activate 期以
    // diagnostic 呈现（软故障），不影响接入判定。

    // ── 权限声明格式 ──
    for api in &m.permissions.host_apis {
        if !valid_api_name(api) {
            errs.push(format!(
                "permissions.host_apis 中的 `{api}` 非法（应为 `scope` 形式）"
            ));
        }
    }
    for d in &m.permissions.network.domains {
        if d.starts_with("http") || d.contains('/') {
            errs.push(format!(
                "permissions.network.domains 中的 `{d}` 非法：只写域名（如 api.example.com 或 *.example.com）"
            ));
        }
    }
    for pat in m
        .permissions
        .fs
        .read
        .iter()
        .chain(m.permissions.fs.write.iter())
    {
        if pat.split(['/', '\\']).any(|seg| seg == "..") {
            errs.push(format!(
                "permissions.fs 中 `{pat}` 非法：不得包含 `..`（bundle 目录即边界）"
            ));
        }
    }

    if errs.is_empty() {
        Ok(())
    } else {
        Err(errs)
    }
}

/// bundle id：`[a-z0-9]` 开头，仅小写字母/数字/`.`/`-`/`_`，1–128。
pub fn valid_bundle_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    !id.is_empty()
        && id.len() <= 128
        && (bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit())
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '-' | '_'))
}

/// 宿主 API scope 名：形如 `host/log` / `session.read`。
/// 允许小写字母/数字/`.`/`_`/`/`（斜杠为宿主 method 命名惯例，如 `session/read`）。
pub fn valid_api_name(api: &str) -> bool {
    !api.is_empty()
        && api.len() <= 64
        && api
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_manifest() -> serde_json::Value {
        serde_json::json!({
            "spec": "oab/v1",
            "id": "com.acme.cr",
            "name": "CR",
            "version": "1.0.0"
        })
    }

    #[test]
    fn valid_manifest_passes() {
        let m: BundleManifest = serde_json::from_value(base_manifest()).unwrap();
        assert_eq!(validate_manifest(&m, 1), Ok(()));
    }

    #[test]
    fn version_mismatch_rejects() {
        let mut v = base_manifest();
        v["requires"] = serde_json::json!({ "spec": "^2" });
        let m: BundleManifest = serde_json::from_value(v).unwrap();
        let errs = validate_manifest(&m, 1).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("版本不兼容")), "{errs:?}");
    }

    #[test]
    fn spec_requirement_semantics() {
        assert!(spec_requirement_matches("^1", 1));
        assert!(spec_requirement_matches("1", 1));
        assert!(!spec_requirement_matches("^1", 2), "major 不同必须拒绝");
        assert!(!spec_requirement_matches("^2", 1));
        assert!(!spec_requirement_matches("garbage", 1));
    }

    #[test]
    fn structural_errors_are_collected() {
        let mut v = base_manifest();
        v["id"] = "Bad ID!".into();
        let m: BundleManifest = serde_json::from_value(v).unwrap();
        let errs = validate_manifest(&m, 1).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("id `Bad ID!` 非法")));
    }
}
