//! OAB 约定目录装配（纯数据 + 文件系统扫描，零宿主依赖）。
//!
//! ## 三个能力来源（规范 §3 / §5）
//!
//! | 目录 | 协议语义 | 说明 |
//! |---|---|---|
//! | `prompts/<name>.md` | OAB 原生（唯一新增） | Markdown 片段，**直接追加进系统提示词**；frontmatter 可带 `priority` |
//! | `skills/<name>/SKILL.md` | 行业 Skill | 正文作为提示词片段（优先级默认 50），与行业 SKILL 格式完全兼容 |
//! | `mcps/<name>…` | 行业 MCP | MCP server 配置，**工具唯一来源**；原样透传给宿主 MCP 客户端 |
//!
//! ## 为什么没有「声明式工具 / 内置执行器」
//!
//! 早期草案曾用 `module: oab.echo` 之类的声明式工具挂载，要求每个接入方都实现
//! `oab.echo`——**把某个具体宿主的内置能力写进协议，会让协议失去通用性**：
//! 一个 agent 包不该依赖「宿主恰好实现了某个专有执行器」。
//! 因此 v1 收敛为：
//!
//! - **工具**：只来自 MCP（行业标准，宿主复用已有 MCP 客户端）；
//! - **提示词**：只来自 `prompts/`（OAB 原生）与 `skills/`（行业标准）。
//!
//! 这样任何宿主只要能「读 Markdown + 启动 MCP server」即可接入，无需实现任何
//! OAB 专有运行时。
//!
//! 单条目解析失败 = **软故障**（记 [`Diagnostic`]，跳过该条目）；bundle 装配不中断。
//! bundle 加载 / manifest 校验失败 = **硬错误**（在 [`crate::plugins::agent::core::spec::validate`]）。

use crate::plugins::agent::core::spec::manifest::BundleManifest;
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

/// 提示词片段（已剥离 frontmatter、已渲染 bundle 变量）。
#[derive(Debug, Clone)]
pub struct PromptFragment {
    /// 升序装配，小的在前；`0` 保留给「身份锚定」
    pub priority: i64,
    pub text: String,
    /// 来源标识，如 `prompt:persona` / `skill:playbook`
    pub source: String,
}

/// MCP server 声明（行业配置对象，原样透传给宿主 MCP 客户端）。
#[derive(Debug, Clone)]
pub struct McpServerSpec {
    pub name: String,
    pub config: Value,
    pub source: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticLevel {
    Warn,
}

/// 装配期软故障诊断（不阻断装配）。
#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub level: DiagnosticLevel,
    pub code: String,
    pub message: String,
}

/// 装配结果（约定目录扫描的产出）。
#[derive(Debug, Clone, Default)]
pub struct Assembly {
    /// 系统提示词片段（`prompts/` + `skills/`）
    pub fragments: Vec<PromptFragment>,
    /// MCP server 声明（工具唯一来源）
    pub mcp_servers: Vec<McpServerSpec>,
    pub diagnostics: Vec<Diagnostic>,
}

impl Assembly {
    /// 按 `(priority, source)` 升序拼接全部提示词片段（段间空行）。
    ///
    /// 这段拼接文本即 bundle 对**系统提示词**的贡献，由宿主注册为
    /// `agent_identity` 身份工具（老 agent 插件同款语义，不侵入会话编排）。
    pub fn identity_text(&self) -> String {
        let mut frags = self.fragments.clone();
        frags.sort_by(|a, b| (a.priority, &a.source).cmp(&(b.priority, &b.source)));
        frags
            .iter()
            .map(|f| f.text.trim())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

/// 扫描 bundle 目录，产出装配结果（软故障记入 `diagnostics`）。
pub fn assemble_bundle(dir: &Path, manifest: &BundleManifest) -> Assembly {
    let mut a = Assembly::default();
    scan_prompts(dir, manifest, &mut a);
    scan_skills(dir, manifest, &mut a);
    scan_mcps(dir, &mut a);
    a
}

/// `prompts/<name>.md`：OAB 原生的系统提示词片段。
///
/// 与 Skill 标准形似（Markdown + 可选 YAML frontmatter），但语义不同：
/// **它会无条件追加进系统提示词**，用于承载人格、全局规则、工作流这类常驻指令。
fn scan_prompts(dir: &Path, manifest: &BundleManifest, a: &mut Assembly) {
    let root = dir.join("prompts");
    let Ok(entries) = std::fs::read_dir(&root) else {
        return;
    };
    let mut names: Vec<(String, PathBuf)> = entries
        .flatten()
        .map(|e| (e.file_name().to_string_lossy().to_string(), e.path()))
        .collect();
    // 同名时按目录序稳定装配
    names.sort_by(|x, y| x.0.cmp(&y.0));

    for (file_name, path) in names {
        if !path.is_file() {
            continue;
        }
        let Some(stem) = file_name
            .strip_suffix(".md")
            .or_else(|| file_name.strip_suffix(".markdown"))
        else {
            continue; // 非 Markdown 文件忽略（不报错：可能是作者放的备注/实体）
        };
        match std::fs::read_to_string(&path) {
            Ok(content) => {
                let (fm, body) = split_frontmatter(&content);
                let priority = fm.get("priority").and_then(|v| v.as_i64()).unwrap_or(10);
                let text = render_bundle_vars(body.trim(), manifest);
                a.fragments.push(PromptFragment {
                    priority,
                    text,
                    source: format!("prompt:{stem}"),
                });
            }
            Err(e) => a.diagnostics.push(Diagnostic {
                level: DiagnosticLevel::Warn,
                code: "read_error".into(),
                message: format!("prompt `{stem}` 读取失败: {e}"),
            }),
        }
    }
}

/// `skills/<name>/SKILL.md`：行业 Skill，正文作为提示词片段。
fn scan_skills(dir: &Path, manifest: &BundleManifest, a: &mut Assembly) {
    let root = dir.join("skills");
    let Ok(entries) = std::fs::read_dir(&root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let skill_path = path.join("SKILL.md");
        if !skill_path.is_file() {
            continue;
        }
        match std::fs::read_to_string(&skill_path) {
            Ok(content) => {
                let (fm, body) = split_frontmatter(&content);
                let text = render_bundle_vars(body.trim(), manifest);
                // 技能正文作为提示词片段；优先级默认 50（晚于 prompt 的常驻指令）
                let priority = fm.get("priority").and_then(|v| v.as_i64()).unwrap_or(50);
                a.fragments.push(PromptFragment {
                    priority,
                    text,
                    source: format!("skill:{name}"),
                });
            }
            Err(e) => a.diagnostics.push(Diagnostic {
                level: DiagnosticLevel::Warn,
                code: "read_error".into(),
                message: format!("skill `{name}` SKILL.md 读取失败: {e}"),
            }),
        }
    }
}

/// `mcps/<name>…`：行业 MCP server 配置（文件或目录两种形态）。
fn scan_mcps(dir: &Path, a: &mut Assembly) {
    let root = dir.join("mcps");
    let Ok(entries) = std::fs::read_dir(&root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        // 文件形态：mcps/<name>.yaml | .yml | .json
        if path.is_file() {
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if matches!(ext, "yaml" | "yml" | "json") {
                    match read_config_value(&path, ext) {
                        Some(cfg) => a.mcp_servers.push(McpServerSpec {
                            name: name.clone(),
                            config: cfg,
                            source: format!("mcp:{name}"),
                        }),
                        None => a.diagnostics.push(Diagnostic {
                            level: DiagnosticLevel::Warn,
                            code: "mcp_parse".into(),
                            message: format!("mcp `{name}` 配置解析失败"),
                        }),
                    }
                }
            }
            continue;
        }

        // 目录形态：mcps/<name>/config.yaml（或 server.yaml / mcp.yaml / .json）
        if path.is_dir() {
            let candidates = [
                "config.yaml",
                "config.yml",
                "server.yaml",
                "server.yml",
                "mcp.yaml",
                "mcp.yml",
                "config.json",
                "server.json",
                "mcp.json",
            ];
            let found = candidates
                .iter()
                .map(|c| {
                    (
                        path.join(c),
                        c.rsplit('.').next().unwrap_or("yaml").to_string(),
                    )
                })
                .find(|(p, _)| p.is_file());
            match found {
                Some((p, ext)) => match read_config_value(&p, &ext) {
                    Some(cfg) => a.mcp_servers.push(McpServerSpec {
                        name: name.clone(),
                        config: cfg,
                        source: format!("mcp:{name}"),
                    }),
                    None => a.diagnostics.push(Diagnostic {
                        level: DiagnosticLevel::Warn,
                        code: "mcp_parse".into(),
                        message: format!("mcp `{name}` 配置解析失败"),
                    }),
                },
                None => a.diagnostics.push(Diagnostic {
                    level: DiagnosticLevel::Warn,
                    code: "mcp_no_config".into(),
                    message: format!("mcp `{name}` 目录缺少配置（期望 config.yaml 等）"),
                }),
            }
        }
    }
}

/// 读取并解析一份 YAML / JSON 配置为 `Value`。
fn read_config_value(path: &PathBuf, ext: &str) -> Option<Value> {
    let raw = std::fs::read_to_string(path).ok()?;
    if ext == "json" {
        serde_json::from_str(&raw).ok()
    } else {
        serde_yaml_ng::from_str(&raw).ok()
    }
}

/// 剥离 YAML frontmatter，返回 (元数据 map, 正文)。
///
/// 支持 `---\n...\n---\n` 与 `---\r\n...\r\n---\r\n` 两种换行；无 frontmatter
/// 时返回空 map + 原文。正文不含 frontmatter 分隔行。
/// 公开给 host 层复用（如 bundle 内部实体管理需要解析 priority）。
pub fn split_frontmatter(content: &str) -> (Map<String, Value>, String) {
    let bytes = content.as_bytes();
    // 必须以 "---" 开头且其后紧跟换行
    if !(bytes.starts_with(b"---") && bytes.len() >= 3 && (bytes[3] == b'\n' || bytes[3] == b'\r'))
    {
        return (Map::new(), content.to_string());
    }
    let after_open = if bytes[3] == b'\r' && bytes.get(4) == Some(&b'\n') {
        5
    } else {
        4
    };
    let rest = &content[after_open..];
    // 找下一个独立行 "---" 作为结束分隔符（\n--- 或 \r\n---）
    let end = rest.find("\n---").or_else(|| rest.find("\r\n---"));
    let (fm_text, body) = match end {
        Some(pos) => {
            let fm = &rest[..pos];
            let after = &rest[pos..];
            let after = after
                .strip_prefix("\r\n---")
                .or_else(|| after.strip_prefix("\n---"))
                .unwrap_or(after);
            let body = after
                .strip_prefix('\r')
                .or_else(|| after.strip_prefix('\n'))
                .unwrap_or(after);
            (fm.to_string(), body.to_string())
        }
        None => (String::new(), content.to_string()),
    };
    let fm: Value = serde_yaml_ng::from_str(&fm_text).unwrap_or(Value::Null);
    (fm.as_object().cloned().unwrap_or_default(), body)
}

/// 渲染 bundle 级别的模板变量（装配期即可得，无需宿主上下文）。
///
/// 支持 `$bundle.id` / `${bundle.id}` 等形态；v1 仅暴露 id / name / version。
fn render_bundle_vars(text: &str, m: &BundleManifest) -> String {
    let replacements = [
        ("$bundle.id", m.id.as_str()),
        ("${bundle.id}", m.id.as_str()),
        ("$bundle.name", m.name.as_str()),
        ("${bundle.name}", m.name.as_str()),
        ("$bundle.version", m.version.as_str()),
        ("${bundle.version}", m.version.as_str()),
    ];
    let mut out = text.to_string();
    for (pat, val) in replacements {
        if !val.is_empty() {
            out = out.replace(pat, val);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_split() {
        let (fm, body) = split_frontmatter("---\npriority: 5\n---\nhello world");
        assert_eq!(fm.get("priority").and_then(|v| v.as_i64()), Some(5));
        assert_eq!(body.trim(), "hello world");
    }

    #[test]
    fn frontmatter_absent() {
        let (fm, body) = split_frontmatter("just text");
        assert!(fm.is_empty());
        assert_eq!(body, "just text");
    }

    #[test]
    fn bundle_vars_rendered() {
        let m = BundleManifest {
            id: "com.x.y".into(),
            name: "X".into(),
            version: "1.2.3".into(),
            ..Default::default()
        };
        let out = render_bundle_vars("id=$bundle.id v=$bundle.version", &m);
        assert_eq!(out, "id=com.x.y v=1.2.3");
    }
}
