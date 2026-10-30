use crate::types::Skill;
use regex::Regex;
use std::path::{Path, PathBuf};
use symbio_plugin_api::PluginError;
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

/// 从多个目录加载 skills：按目录顺序**合并成一个集合**，`budget.max_skills` 只作用在这个集合上。
///
/// 单个 body 超过 `budget.max_body_chars` 时**截断**到 `max_body_chars` 字符并 warn（避免 SKILL.md 错误导致 OOM）。
///
/// 需要无上限加载时，显式传入
/// `LoadBudget { max_skills: usize::MAX, max_body_chars: usize::MAX }`。
///
/// ## 名称冲突策略（BUG-SR1）
///
/// 不同目录里出现**同名 skill** 时，**后加载的覆盖先加载的**（后到优先）。
/// 但会用 `tracing::warn!` 记录冲突，避免静默覆盖。
///
/// ## 为什么名额不能在合并前逐目录扣（项二）
///
/// `max_skills` 是对**清单**的约束，不是对**某一个目录**的约束。旧写法每读一个目录就
/// 把名额扣干、扣干即 `break`，于是：目录 A `[s1, s2]` + 目录 B `[s1′, s3]`、
/// `max_skills = 2` 时，`s3` 根本没被看过——清单只剩 `s1 / s2`。两个后果一个是数量，
/// 一个是身份：
///
/// - **数量**：清单可以比预算小，而多出来的那些技能在资源树上看得见、在工具面召不出来
///   （一份清单两个来源，正是本项要收敛的东西）；
/// - **身份**：名额扣在合并之前，同名覆盖就还没发生，谁占格取决于目录传入顺序而不是
///   覆盖关系——`s1′` 本该顶掉 `s1`，却因为它排在扣干之后而整个消失。
///
/// 改成合并后一次裁剪，覆盖先于名额，清单恰为 `max_skills` 个**去重后**的技能。
/// 代价是后来的目录也会被扫一遍（不再中途 `break`）：后到优先要求看到全部来源才成立。
pub async fn load_skills_from_dirs_with_budget(
    dirs: &[String],
    workdir: &Path,
    budget: LoadBudget,
) -> Result<Vec<Skill>, PluginError> {
    // 合并后的唯一集合：名字集合就是它的 `name` 投影，不再另存一份 seen_names
    let mut merged: Vec<Skill> = Vec::new();

    for dir_path in dirs {
        // 优先：~ 展开
        let expanded = expand_tilde(dir_path);
        let abs_dir = if expanded.is_absolute() {
            expanded
        } else {
            // 相对路径：相对 workdir
            workdir.join(expanded)
        };

        if abs_dir.exists() && abs_dir.is_dir() {
            // 名称去重 + 冲突检测（后到优先）
            for skill in load_skills_from_dir(&abs_dir, budget).await? {
                if merged.iter().any(|s| s.name == skill.name) {
                    warn!(
                        skill = %skill.name,
                        file = %skill.file_path,
                        "skill 名称重复（后加载的覆盖之前的同名 skill）"
                    );
                    // 移除之前的同名 skill
                    merged.retain(|s| s.name != skill.name);
                }
                merged.push(skill);
            }
        }
    }

    // 预算裁剪：作用在合并后的集合上，一次判完
    if merged.len() > budget.max_skills {
        warn!(
            merged = merged.len(),
            kept = budget.max_skills,
            "合并后的技能数超出 max_skills，按目录顺序裁剪尾部"
        );
        merged.truncate(budget.max_skills);
    }

    Ok(merged)
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
    //
    // 注意：`max_body_chars` 的语义是**字符**数（字段名与文档注释均如此），
    // 而 `str::len()` 返回**字节**数——中文 SKILL.md 会被按 1/3 预算过度截断。
    // 这里显式按 char 计数取字节偏移，两者对齐。
    let body_char_count = body_raw.chars().count();
    let body = if body_char_count > max_body_chars {
        warn!(
            skill = %path.display(),
            chars = body_char_count,
            max = max_body_chars,
            "SKILL.md body 超出 max_body_chars，已截断"
        );
        // 第 max_body_chars 个字符的字节起点即为合法截断点（天然 char 边界）
        let idx = body_raw
            .char_indices()
            .nth(max_body_chars)
            .map(|(i, _)| i)
            .unwrap_or(body_raw.len());
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
#[path = "loader.test.rs"]
mod tests;

// ── panic 面登记（PN-001…003）─────────────────────────────────────────
// 本文件每一处 `unwrap` / `expect` / `panic!` / `unreachable!` 的理由。登记放在
// 文件内而不是集中一张表：理由与它解释的那段代码会一起被 review、一起被删。
// 判据见 `scripts/panic-audit.mjs`。**加一处 panic 必须同时加一行登记，理由非空。**
// panic-allow crates/symbio-plugin-skill/src/loader.rs::parse_skill_file: 编译期常量正则，模式写错在首次调用即崩——属开发期事故，不是运行期输入。
