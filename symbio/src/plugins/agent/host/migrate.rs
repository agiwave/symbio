//! OAB v1 → Agent 目录规范 v2 的迁移（规范 §12）
//!
//! ## 迁移动作
//!
//! | v1 | v2 | 说明 |
//! |---|---|---|
//! | `prompts/<n>.md` | 根 `AGENTS.md` | 按 `priority` 升序拼接，剥离 frontmatter |
//! | `skills/<n>/` | `skill/<n>/` | 整个目录搬过去（v2 由 `skill` 插件实例承载） |
//! | `mcps/<n>.yaml` | `mcp/<n>/server.json` | 转成 `mcp` 插件的持久化格式 |
//! | `manifest.yaml` | `manifest.yaml` | 只把 `spec` 改成 `agent-dir/v2` |
//! | 其它（`assets/` `tools/` `tests/`） | 原样保留 | v2 不规定，宿主也不解释 |
//!
//! ## 幂等
//!
//! 每一步都**先检查目标是否已存在**，存在即跳过。因此可以重复执行，也可以对
//! 已经半迁移的目录补完。已不是 v1 的目录直接返回 `Ok(false)`（什么都不做）。
//!
//! ⚠️ 这是**原地改写用户数据**。调用方（[`super::plugin::AgentPlugin::sub_agent`]）
//! 只在目录确实是 v1 时才调用它，且失败时**不阻断**——宁可让该 Agent 以旧方式
//! 加载，也不要在半迁移状态下继续。

use super::plugin::{SPEC_V1, SPEC_V2};
use crate::symbio_core::AGENTS_FILE;
use std::path::Path;

/// 执行迁移。返回是否真的做了迁移动作（幂等：已迁移 / 非 v1 目录 → `false`）
pub fn migrate_v1_to_v2(dir: &Path) -> Result<bool, String> {
    let manifest_path = dir.join("manifest.yaml");
    if !manifest_path.exists() || read_spec(&manifest_path).as_deref() != Some(SPEC_V1) {
        return Ok(false);
    }

    let mut acted = false;
    if migrate_prompts_to_agents_md(dir)? {
        acted = true;
    }
    if move_dir_if_absent(&dir.join("skills"), &dir.join("skill"))? {
        acted = true;
    }
    if migrate_mcps(dir)? {
        acted = true;
    }
    // 最后改写 spec：前面任一步失败都不会把目录标成"已迁移"
    if write_spec(&manifest_path)? {
        acted = true;
    }
    Ok(acted)
}

fn read_spec(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let value: serde_yaml_ng::Value = serde_yaml_ng::from_str(&text).ok()?;
    value.get("spec")?.as_str().map(|s| s.to_string())
}

/// 只改 `spec` 字段，其余原样保留（解析成 Value 再序列化，注释会丢）
fn write_spec(path: &Path) -> Result<bool, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut value: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(&text).map_err(|e| e.to_string())?;
    let Some(map) = value.as_mapping_mut() else {
        return Err("manifest 不是一个映射".to_string());
    };
    map.insert(
        serde_yaml_ng::Value::String("spec".to_string()),
        serde_yaml_ng::Value::String(SPEC_V2.to_string()),
    );
    let out = serde_yaml_ng::to_string(&value).map_err(|e| e.to_string())?;
    std::fs::write(path, out).map_err(|e| e.to_string())?;
    Ok(true)
}

/// `prompts/*.md` → 根 `AGENTS.md`
///
/// 排序与 v1 的装配规则一致：`(priority, source)` 升序，`priority` 缺省 10。
fn migrate_prompts_to_agents_md(dir: &Path) -> Result<bool, String> {
    let prompts = dir.join("prompts");
    if !prompts.is_dir() {
        return Ok(false);
    }
    let target = dir.join(AGENTS_FILE);
    if target.exists() {
        return Ok(false);
    }

    let mut entries: Vec<(i64, String, String)> = Vec::new();
    for entry in std::fs::read_dir(&prompts)
        .map_err(|e| e.to_string())?
        .flatten()
    {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !matches!(ext, "md" | "markdown") {
            continue;
        }
        let raw = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        let (front, body) = split_frontmatter(&raw);
        let priority = front
            .as_ref()
            .and_then(|f| f.get("priority"))
            .and_then(|v| v.as_i64())
            .unwrap_or(10);
        let source = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        entries.push((priority, source, body));
    }
    if entries.is_empty() {
        return Ok(false);
    }
    // `(priority, source)` 升序，与 v1 §7.4 一致
    entries.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));

    let mut parts: Vec<String> = Vec::with_capacity(entries.len());
    for (_, _, body) in entries {
        let text = body.trim();
        if !text.is_empty() {
            parts.push(text.to_string());
        }
    }
    if parts.is_empty() {
        return Ok(false);
    }
    std::fs::write(&target, parts.join("\n\n") + "\n").map_err(|e| e.to_string())?;
    Ok(true)
}

/// 剥离 YAML frontmatter，返回 `(Some(frontmatter), 正文)`
fn split_frontmatter(raw: &str) -> (Option<serde_yaml_ng::Value>, String) {
    let text = raw.strip_prefix('\u{feff}').unwrap_or(raw);
    if !text.starts_with("---") {
        return (None, text.to_string());
    }
    let rest = &text[3..];
    let Some(end) = rest.find("\n---") else {
        return (None, text.to_string());
    };
    let fm = &rest[..end];
    let body = rest[end + 4..].trim_start_matches(['\r', '\n']);
    let value = serde_yaml_ng::from_str::<serde_yaml_ng::Value>(fm).ok();
    (value, body.to_string())
}

/// `skills/<n>/` → `skill/<n>/`（整个目录搬过去）
fn move_dir_if_absent(from: &Path, to: &Path) -> Result<bool, String> {
    if !from.is_dir() || to.exists() {
        return Ok(false);
    }
    std::fs::rename(from, to).map_err(|e| e.to_string())?;
    Ok(true)
}

/// `mcps/<n>…` → `mcp/<n>/server.json`
///
/// v1 存的是**行业原生的 MCP server 配置**（`command` / `args` / `transport`），
/// v2 由 `mcp` 插件承载，持久化格式是 `server.json`：字段基本一致，但
/// 传输类型在 v1 叫 `transport`、在 `mcp` 插件里叫 `type`。
fn migrate_mcps(dir: &Path) -> Result<bool, String> {
    let mcps = dir.join("mcps");
    if !mcps.is_dir() {
        return Ok(false);
    }
    let target_root = dir.join("mcp");
    let mut acted = false;

    for entry in std::fs::read_dir(&mcps)
        .map_err(|e| e.to_string())?
        .flatten()
    {
        let path = entry.path();
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        if name.is_empty() {
            continue;
        }
        let (raw, is_json) = if path.is_dir() {
            // 目录形态：取其中第一个配置文件
            let Some(inner) = std::fs::read_dir(&path)
                .map_err(|e| e.to_string())?
                .flatten()
                .find(|e| e.path().is_file())
            else {
                continue;
            };
            (
                std::fs::read_to_string(inner.path()).map_err(|e| e.to_string())?,
                inner.path().extension().and_then(|e| e.to_str()) == Some("json"),
            )
        } else {
            (
                std::fs::read_to_string(&path).map_err(|e| e.to_string())?,
                path.extension().and_then(|e| e.to_str()) == Some("json"),
            )
        };

        let mut value: serde_json::Value = if is_json {
            serde_json::from_str(&raw).map_err(|e| e.to_string())?
        } else {
            let yaml: serde_yaml_ng::Value =
                serde_yaml_ng::from_str(&raw).map_err(|e| e.to_string())?;
            serde_json::to_value(yaml).map_err(|e| e.to_string())?
        };
        let Some(map) = value.as_object_mut() else {
            continue;
        };
        // transport → type（缺省 stdio）
        let transport = map
            .remove("transport")
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| "stdio".to_string());
        map.insert("type".to_string(), serde_json::Value::String(transport));

        let out_dir = target_root.join(&name);
        if out_dir.exists() {
            continue;
        }
        std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
        let out = serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?;
        std::fs::write(out_dir.join("server.json"), out).map_err(|e| e.to_string())?;
        acted = true;
    }
    Ok(acted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn v1_dir(tmp: &Path, id: &str) -> PathBuf {
        let d = tmp.join(id);
        std::fs::create_dir_all(d.join("prompts")).unwrap();
        std::fs::create_dir_all(d.join("skills").join("playbook")).unwrap();
        std::fs::create_dir_all(d.join("mcps")).unwrap();
        std::fs::write(
            d.join("manifest.yaml"),
            format!("spec: \"oab/v1\"\nid: \"{id}\"\nname: \"测试\"\nversion: \"1.0.0\"\nrequires:\n  spec: \"^1\"\n"),
        )
        .unwrap();
        std::fs::write(
            d.join("prompts").join("persona.md"),
            "---\npriority: 0\n---\n\n你是测试。\n",
        )
        .unwrap();
        std::fs::write(
            d.join("skills").join("playbook").join("SKILL.md"),
            "---\nname: playbook\ndescription: 交付流程手册\n---\n\n# 正文\n",
        )
        .unwrap();
        std::fs::write(
            d.join("mcps").join("demo.yaml"),
            "command: npx\nargs:\n  - \"-y\"\n  - \"demo\"\n",
        )
        .unwrap();
        d
    }

    #[test]
    fn migrates_v1_layout_to_v2() {
        let tmp = tempfile::tempdir().unwrap();
        let d = v1_dir(tmp.path(), "demo");

        assert!(migrate_v1_to_v2(&d).unwrap());

        // manifest 的 spec 已改写
        assert_eq!(
            read_spec(&d.join("manifest.yaml")).as_deref(),
            Some(SPEC_V2)
        );
        // prompts → 根 AGENTS.md（frontmatter 已剥离）
        assert_eq!(
            std::fs::read_to_string(d.join("AGENTS.md")).unwrap().trim(),
            "你是测试。"
        );
        // skills → skill
        assert!(d.join("skill").join("playbook").join("SKILL.md").exists());
        assert!(!d.join("skills").exists());
        // mcps → mcp/<n>/server.json，transport 改名为 type
        let server: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(d.join("mcp").join("demo").join("server.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(server["type"], "stdio");
        assert_eq!(server["command"], "npx");
    }

    #[test]
    fn is_idempotent_and_ignores_non_v1() {
        let tmp = tempfile::tempdir().unwrap();
        let d = v1_dir(tmp.path(), "demo");
        assert!(migrate_v1_to_v2(&d).unwrap());
        // 第二次执行：无事可做
        assert!(!migrate_v1_to_v2(&d).unwrap());

        // 非 v1 目录不动
        let other = tmp.path().join("plain");
        std::fs::create_dir_all(&other).unwrap();
        assert!(!migrate_v1_to_v2(&other).unwrap());
    }
}
