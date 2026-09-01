use crate::plugins::skill::types::Skill;
use crate::symbio_core::PluginError;
use regex::Regex;
use std::path::{Path, PathBuf};
use tokio::fs;
use tracing::{debug, warn};

/// 展开 `~` 到 home 目录
///
/// 与其它插件（agent/home/local/explorer/session）保持一致，统一使用
/// `shellexpand::tilde` 直接展开，避免跨模块引用 `providers::` 内部实现。
fn expand_tilde(path: &str) -> PathBuf {
    PathBuf::from(shellexpand::tilde(path).to_string())
}

/// Skill 加载配置（从 SkillConfig 透传）
#[derive(Debug, Clone, Copy)]
pub struct LoadBudget {
    /// 最多加载的 skill 数量
    pub max_skills: usize,
    /// 单个 skill body 最大字符数
    pub max_body_chars: usize,
}

impl Default for LoadBudget {
    fn default() -> Self {
        Self {
            max_skills: 20,
            max_body_chars: 8000,
        }
    }
}

/// 从多个目录加载 skills，按目录顺序累加，受 `budget` 约束
///
/// 超出 `budget.max_skills` 时跳过后续；单个 body 超过 `budget.max_body_chars`
/// 时**截断**到 `max_body_chars` 字符并 warn（避免 SKILL.md 错误导致 OOM）。
///
/// 替代了早期的 `load_skills_from_dirs`（无 budget 概念）。当生产路径
/// 确实需要无 budget 加载时，应显式传入 `LoadBudget { max_skills: usize::MAX, max_body_chars: usize::MAX }`。
///
/// ## 名称冲突策略（BUG-SR1）
///
/// 不同目录里出现**同名 skill** 时，**后加载的覆盖先加载的**（后到优先）。
/// 但会用 `tracing::warn!` 记录冲突，避免静默覆盖。
pub async fn load_skills_from_dirs_with_budget(
    dirs: &[String],
    workdir: &Path,
    budget: LoadBudget,
) -> Result<Vec<Skill>, PluginError> {
    let mut all_skills: Vec<Skill> = Vec::new();
    // 跟踪已出现的 skill name
    let mut seen_names: std::collections::HashSet<String> = std::collections::HashSet::new();

    for dir_path in dirs {
        // 预算检查：超出 max_skills 后停止
        if all_skills.len() >= budget.max_skills {
            warn!(
                loaded = all_skills.len(),
                max = budget.max_skills,
                "已达到 max_skills 上限，跳过剩余目录: {dir_path}"
            );
            break;
        }

        // 优先：~ 展开
        let expanded = expand_tilde(dir_path);
        let abs_dir = if expanded.is_absolute() {
            expanded
        } else {
            // 相对路径：相对 workdir
            workdir.join(expanded)
        };

        if abs_dir.exists() && abs_dir.is_dir() {
            let mut skills = load_skills_from_dir(&abs_dir, budget).await?;
            // 单目录加载后再做预算检查
            let remaining = budget.max_skills.saturating_sub(all_skills.len());
            if skills.len() > remaining {
                warn!(
                    dir = %abs_dir.display(),
                    loaded = skills.len(),
                    max = remaining,
                    "单目录 skill 数量超出剩余预算，将截断"
                );
                skills.truncate(remaining);
            }

            // 名称去重 + 冲突检测
            for skill in skills {
                if seen_names.contains(&skill.name) {
                    warn!(
                        skill = %skill.name,
                        file = %skill.file_path,
                        "skill 名称重复（后加载的覆盖之前的同名 skill）"
                    );
                    // 移除之前的同名 skill
                    all_skills.retain(|s| s.name != skill.name);
                }
                seen_names.insert(skill.name.clone());
                all_skills.push(skill);
            }
        }
    }
    Ok(all_skills)
}

async fn load_skills_from_dir(dir: &Path, budget: LoadBudget) -> Result<Vec<Skill>, PluginError> {
    let mut skills = Vec::new();
    let mut entries = fs::read_dir(dir)
        .await
        .map_err(|e| PluginError::InternalError(e.to_string()))?;

    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| PluginError::InternalError(e.to_string()))?
    {
        let path = entry.path();
        if path.is_dir() {
            // BUG-SR8：跳过隐藏目录（以 `.` 开头，如 `.git`、`.symbio`、`.vscode`）
            // 行业最佳实践：隐藏目录通常是工具/版本控制元数据，不应作为 skill 加载。
            if let Some(name_os) = path.file_name() {
                let name = name_os.to_string_lossy();
                if name.starts_with('.') {
                    debug!(dir = %path.display(), "跳过隐藏目录（以 . 开头）");
                    continue;
                }
            }
            let manifest_path = path.join("SKILL.md");
            if manifest_path.exists() {
                match parse_skill_file(&manifest_path, budget.max_body_chars).await {
                    Ok(skill) => skills.push(skill),
                    Err(e) => {
                        // 单个 SKILL.md 解析失败不阻断整体加载，但记录 warn
                        warn!(skill = %path.display(), error = %e, "SKILL.md 解析失败，跳过");
                    }
                }
            }
        }
    }
    Ok(skills)
}

async fn parse_skill_file(path: &Path, max_body_chars: usize) -> Result<Skill, PluginError> {
    let content = fs::read_to_string(path)
        .await
        .map_err(|e| PluginError::InternalError(e.to_string()))?;

    // Normalize line endings
    let content = content.replace("\r\n", "\n");

    // Regex to split frontmatter and body
    static RE: std::sync::LazyLock<Regex> =
        std::sync::LazyLock::new(|| Regex::new(r"(?s)^---\n(.*?)\n---(?:\n|$)(.*)$").unwrap());
    let caps = RE.captures(&content).ok_or_else(|| {
        PluginError::ValidationError("Missing YAML frontmatter in SKILL.md".to_string())
    })?;

    let yaml_str = caps.get(1).map(|m| m.as_str()).unwrap_or("");
    let body_raw = caps
        .get(2)
        .map(|m| m.as_str())
        .unwrap_or("")
        .trim()
        .to_string();

    // body 预算：超过 max_body_chars 时截断 + warn
    let body = if body_raw.len() > max_body_chars {
        warn!(
            skill = %path.display(),
            chars = body_raw.len(),
            max = max_body_chars,
            "SKILL.md body 超出 max_body_chars，已截断"
        );
        // 在 char 边界截断（按 char 而非 byte，避免切割 UTF-8 多字节字符）
        let mut idx = max_body_chars;
        while !body_raw.is_char_boundary(idx) && idx > 0 {
            idx -= 1;
        }
        let mut truncated = body_raw[..idx].to_string();
        truncated.push_str("\n\n[... body truncated due to max_body_chars budget ...]");
        truncated
    } else {
        body_raw
    };

    let frontmatter: serde_yaml_ng::Value = serde_yaml_ng::from_str(yaml_str).map_err(|e| {
        PluginError::ValidationError(format!("Failed to parse YAML frontmatter: {e}"))
    })?;

    let name = frontmatter
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            PluginError::ValidationError("Missing 'name' in skill frontmatter".to_string())
        })?
        .to_string();
    let description = frontmatter
        .get("description")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            PluginError::ValidationError("Missing 'description' in skill frontmatter".to_string())
        })?
        .to_string();

    // BUG-SR7：行业最佳实践要求 description 至少 10 字符，否则：
    // - 加载到 Skill 列表时无法给用户清晰提示
    // - LLM 难以判断何时调用该 skill
    // 给出明确错误信息引导用户修正
    const MIN_DESCRIPTION_LEN: usize = 10;
    if description.trim().chars().count() < MIN_DESCRIPTION_LEN {
        return Err(PluginError::ValidationError(format!(
            "BUG-SR7: skill '{}' 的 description 太短（{} 字符），至少需要 {} 字符",
            name,
            description.trim().chars().count(),
            MIN_DESCRIPTION_LEN
        )));
    }

    // BUG-SR6：行业硬约束——SKILL.md 所在目录名必须 == frontmatter `name`
    // 否则视为配置错误（避免 skill 路由混乱）。
    // 取 path 的 parent dir 名字（basename），与 name 严格比较。
    if let Some(parent) = path.parent() {
        if let Some(dir_name_os) = parent.file_name() {
            let dir_name = dir_name_os.to_string_lossy();
            if dir_name != name {
                return Err(PluginError::ValidationError(format!(
                    "BUG-SR6: SKILL.md 目录名 '{}' 与 frontmatter 'name' '{}' 不一致（行业硬约束：必须相等）",
                    dir_name, name
                )));
            }
        }
    }

    let allowed_tools = frontmatter
        .get("allowedTools")
        .and_then(|v| v.as_sequence())
        .map(|seq| {
            seq.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        });

    let argument_hint = frontmatter
        .get("argument_hint")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let when_to_use = frontmatter
        .get("when_to_use")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let model = frontmatter
        .get("model")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let disable_model_invocation = frontmatter
        .get("disable-model-invocation")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    Ok(Skill {
        name,
        description,
        body,
        allowed_tools,
        argument_hint,
        when_to_use,
        model,
        disable_model_invocation,
        file_path: path.to_string_lossy().to_string(),
    })
}

#[cfg(test)]
mod tests;
