//! OAB v1 → Agent 目录规范 v2 的迁移（规范 §12）
//!
//! ## 迁移动作
//!
//! | v1 | v2 | 说明 |
//! |---|---|---|
//! | `prompts/<n>.md` | 根 `AGENTS.md` | 按 `priority` 升序拼接，剥离 frontmatter |
//! | `skills/<n>/` | `skill/<n>/` | 整个目录搬过去（v2 由 `skill` 插件实例承载） |
//! | `mcps/<n>.yaml` | `mcp/<n>/server.json` | 转成 `mcp` 插件的持久化格式 |
//! | `manifest.yaml` | `manifest.yaml` | `spec` **与** `requires.spec` 一起升到 v2 |
//! | 其它（`assets/` `tools/` `tests/`） | 原样保留 | v2 不规定，宿主也不解释 |
//!
//! ## 幂等
//!
//! 每一步都**先检查目标是否已存在**，存在即跳过，因此对同一目录重复执行是安全的
//! （已完成的部分不会重做）。
//!
//! 入口判据是 `spec == "oab/v1"`，且**只认这个判据**：`spec` 已是 `agent-dir/v2` 的
//! 目录一律直接返回 `Ok(false)`——本迁移不去"补"一个已经声称自己是 v2 的目录。
//! （历史上 `spec` 与 `requires.spec` 分头写导致过半迁移产物，那个 bug 已在本文件
//! 内修掉：两者现在同行升级，见下。）
//!
//! ⚠️ 这是**原地改写用户数据**。调用方（[`super::plugin::AgentPlugin::sub_agent`]）
//! 只在目录确实是 v1 时才调用它，且失败时**不阻断**——宁可让该 Agent 以旧方式
//! 加载，也不要在半迁移状态下继续。
//!
//! ## 为什么 `requires.spec` 必须同行升级（§10 + §12）
//!
//! `spec` 是「本目录遵循哪个格式版本」，`requires.spec` 是「我能跑在哪个主版本的
//! 宿主上」（§10 的接入门槛）。v1 目录的 `requires.spec` 必然写着 `^1`；若迁移只改
//! `spec`，产物就是「格式声明 v2、兼容门槛仍要 v1」的自相矛盾清单——`manifest::validate`
//! 必然按 §10 拒绝它，**迁移出来的目录一行都跑不起来**。两者同行才能自洽。

use super::manifest::SPEC_MAJOR;
use super::plugin::{SPEC_V1, SPEC_V2};
use crate::symbio_core::MEMORY_AGENTS_FILE;
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

/// 把协议版本写全：`spec` **与** `requires.spec` 一起升到 v2。
///
/// 两者是同一件事的两面——`spec` 说「本目录是 v2 格式」，`requires.spec` 说「我要跑在
/// v2 宿主上」（§10）。只改前者会留下一个必然被 `manifest::validate` 拒绝的清单
/// （详见模块文档「为什么 `requires.spec` 必须同行升级」）。
///
/// 其余字段原样保留（解析成 Value 再序列化，注释会丢）。
fn write_spec(path: &Path) -> Result<bool, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut value: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(&text).map_err(|e| e.to_string())?;
    let Some(map) = value.as_mapping_mut() else {
        return Err("manifest 不是一个映射".to_string());
    };
    let key = |s: &str| serde_yaml_ng::Value::String(s.to_string());
    map.insert(key("spec"), key(SPEC_V2));

    // `requires.spec` := `^<宿主主版本>`。`requires` 缺失 / 不是映射 → 补一个空映射
    // （迁移后的目录本来就是 v2 目录，不声明门槛就等于不可加载，不是"尊重原样"）。
    let mut requires: serde_yaml_ng::Mapping = map
        .iter()
        .find(|(k, _)| k.as_str() == Some("requires"))
        .and_then(|(_, v)| v.as_mapping().cloned())
        .unwrap_or_default();
    requires.insert(key("spec"), key(&format!("^{SPEC_MAJOR}")));
    map.insert(key("requires"), serde_yaml_ng::Value::Mapping(requires));

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
    let target = dir.join(MEMORY_AGENTS_FILE);
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
#[path = "migrate.test.rs"]
mod tests;
