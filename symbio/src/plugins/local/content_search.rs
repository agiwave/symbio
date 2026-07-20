//! 内容搜索工具 - 使用正则表达式搜索文件内容 - 实现 Tool trait
//!
//! 直接调用 ripgrep 库（grep / grep-searcher / grep-regex / ignore）完成跨平台搜索，
//! 不再依赖 rg / PowerShell / grep 等外部可执行文件，各操作系统行为一致，并遵循 .gitignore。

use super::policy::SecurityPolicy;
use crate::symbio_core::{
    validate_params, Capability, CapabilityMeta, InvokeRequest, InvokeRequestExt, InvokeResponse,
    PluginError, PluginPayload,
};
use async_trait::async_trait;
use bstr::ByteSlice;
use grep::matcher::Matcher;
use grep::regex::RegexMatcherBuilder;
use grep::searcher::sinks::UTF8 as UTF8Sink;
use grep::searcher::SearcherBuilder;
use ignore::WalkBuilder;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

const MAX_RESULTS: usize = 1000;
const MAX_OUTPUT_BYTES: usize = 1_048_576;

/// 内容搜索工具
#[derive(Clone)]
pub struct ContentSearchTool {
    security: Arc<SecurityPolicy>,
}

impl ContentSearchTool {
    pub fn new(security: Arc<SecurityPolicy>) -> Self {
        Self { security }
    }

    async fn execute_inner(&self, args: &Value, workdir: &str) -> InvokeResponse<Value> {
        validate_params(args, &["pattern"]).map_err(PluginError::ValidationError)?;

        let pattern = match args.get("pattern").and_then(|v| v.as_str()) {
            Some(p) if !p.is_empty() => p,
            _ => {
                return Err(PluginError::ValidationError(
                    "Missing or empty 'pattern' argument".into(),
                ))
            },
        };

        if pattern.is_empty() {
            return Err(PluginError::ValidationError(
                "不允许使用空模式。".to_string(),
            ));
        }

        let search_path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
        let output_mode = args
            .get("output_mode")
            .and_then(|v| v.as_str())
            .unwrap_or("content");
        let include = args.get("include").and_then(|v| v.as_str());
        let case_sensitive = args
            .get("case_sensitive")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let context_before = args
            .get("context_before")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        let context_after = args
            .get("context_after")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        let type_filter = args.get("type").and_then(|v| v.as_str());
        let multiline = args
            .get("multiline")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let head_limit = args
            .get("head_limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        let offset = args
            .get("offset")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;

        if !matches!(output_mode, "content" | "files_with_matches" | "count") {
            return Err(PluginError::ValidationError(format!(
                "无效的 output_mode '{output_mode}'。允许值: content, files_with_matches, count。"
            )));
        }

        let workspace_dir = std::path::PathBuf::from(shellexpand::tilde(workdir).to_string());
        let full_search_path = if PathBuf::from(search_path).is_absolute() {
            PathBuf::from(search_path)
        } else {
            workspace_dir.join(search_path)
        };

        let resolved_path = tokio::fs::canonicalize(&full_search_path)
            .await
            .map_err(|e| PluginError::InternalError(format!("无法解析搜索路径: {e}")))?;

        // 使用统一的路径验证方法
        if !self
            .security
            .is_path_allowed_for_read(&resolved_path, &workspace_dir)
            .await
        {
            return Err(PluginError::ValidationError(
                "搜索路径超出工作区范围。".to_string(),
            ));
        }

        match self
            .search(
                pattern,
                &resolved_path,
                output_mode,
                include,
                type_filter,
                case_sensitive,
                context_before,
                context_after,
                multiline,
            )
            .await
        {
            Ok(result) => {
                // 分页：offset / head_limit（按行截断，对齐 Trae Grep 参数）
                let result = apply_line_pagination(result, offset, head_limit);
                let truncated = result.len() > MAX_OUTPUT_BYTES;
                let result = if truncated {
                    format!(
                        "{}...\n\n[输出已截断：超过 {} 字节]",
                        &result[..MAX_OUTPUT_BYTES],
                        MAX_OUTPUT_BYTES
                    )
                } else {
                    result
                };

                Ok(json!({
                    "output": result,
                    "truncated": truncated,
                    "backend": "ripgrep-lib"
                }))
            },
            Err(e) => Err(PluginError::InternalError(e)),
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn search(
        &self,
        pattern: &str,
        path: &std::path::Path,
        output_mode: &str,
        include: Option<&str>,
        file_type: Option<&str>,
        case_sensitive: bool,
        context_before: usize,
        context_after: usize,
        multiline: bool,
    ) -> Result<String, String> {
        // 编译正则：多行模式开启 dot-matches-newline（对齐 rg -U）
        let mut rb = RegexMatcherBuilder::new();
        rb.case_smart(false)
            .case_insensitive(!case_sensitive)
            .multi_line(true)
            .dot_matches_new_line(multiline)
            .octal(false);
        let matcher = rb
            .build(pattern)
            .map_err(|e| format!("无效的正则表达式: {e}"))?;

        // 文件名过滤：显式 include(glob) + type(ripgrep 类型名) 经 globset 合并
        let mut globs: Vec<String> = Vec::new();
        if let Some(g) = include {
            globs.push(g.to_string());
        }
        if let Some(ty) = file_type {
            for g in type_to_globs(ty) {
                globs.push((*g).to_string());
            }
        }
        let mut walk_builder = WalkBuilder::new(path);
        walk_builder
            .standard_filters(true) // 遵循 .gitignore / 隐藏文件 / 全局 ignore
            .hidden(true)
            .require_git(false)
            .parents(true)
            .ignore(true)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true);
        if !globs.is_empty() {
            let mut builder = globset::GlobSetBuilder::new();
            for g in &globs {
                builder.add(
                    globset::GlobBuilder::new(g)
                        .literal_separator(false)
                        .build()
                        .map_err(|e| format!("无效的 include/glob '{g}': {e}"))?,
                );
            }
            let set = builder
                .build()
                .map_err(|e| format!("构建 glob 失败: {e}"))?;
            walk_builder.filter_entry(move |e| {
                if e.path().is_dir() {
                    true
                } else {
                    set.is_match(e.path())
                }
            });
        }

        let mut searcher = SearcherBuilder::new().line_number(true).build();

        let mut out: Vec<String> = Vec::new();
        let mut match_count: u64 = 0;
        let mut limited = false;

        for entry in walk_builder.build() {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let fp = entry.path();
            if !fp.is_file() {
                continue;
            }

            if multiline {
                // 跨行：整文件读入后用 Matcher 手动迭代匹配（'.' 已开启匹配换行）
                let bytes = match std::fs::read(fp) {
                    Ok(b) => b,
                    Err(_) => continue,
                };
                let haystack = bstr::BString::from(bytes);
                let mut at = 0usize;
                let mut file_matches: Vec<String> = Vec::new();
                loop {
                    let m = matcher
                        .find_at(&haystack, at)
                        .map_err(|e| format!("匹配失败: {e}"))?;
                    let m = match m {
                        Some(m) => m,
                        None => break,
                    };
                    file_matches.push(haystack[m.start()..m.end()].to_str_lossy().into_owned());
                    at = m.end();
                    if m.start() == m.end() {
                        at += 1;
                    }
                    if at >= haystack.len() {
                        break;
                    }
                }
                if !file_matches.is_empty() {
                    if output_mode == "files_with_matches" {
                        out.push(fp.display().to_string());
                    } else if output_mode == "count" {
                        out.push(format!("{}:{}", fp.display(), file_matches.len()));
                    } else {
                    out.push(format!("{}", fp.display()));
                    out.extend(file_matches.clone());
                }
                match_count += file_matches.len() as u64;
                }
            } else {
                // 逐行搜索：收集匹配行号与文本
                let mut file_matches: Vec<(u64, String)> = Vec::new();
                let sink = UTF8Sink(|ln: u64, text: &str| {
                    file_matches.push((ln, text.to_string()));
                    Ok(true)
                });
                if searcher.search_path(&matcher, fp, sink).is_err() {
                    continue;
                }
                if file_matches.is_empty() {
                    continue;
                }
                match_count += file_matches.len() as u64;

                if output_mode == "files_with_matches" {
                    out.push(fp.display().to_string());
                } else if output_mode == "count" {
                    out.push(format!("{}:{}", fp.display(), file_matches.len()));
                } else if context_before == 0 && context_after == 0 {
                    for (ln, text) in &file_matches {
                        out.push(format!("{}:{}:{}", fp.display(), ln, text));
                    }
                } else {
                    // 手动上下文窗口：读文件行，合并匹配 ± 上下文行号
                    let content = match std::fs::read(fp) {
                        Ok(b) => b,
                        Err(_) => continue,
                    };
                    let owned = bstr::BString::from(content);
                    let line_vec: Vec<&[u8]> = owned.lines().collect();
                    let n = line_vec.len();
                    let mut show: BTreeSet<usize> = BTreeSet::new();
                    for (ln, _) in &file_matches {
                        let l = *ln as usize;
                        let start = l.saturating_sub(context_before);
                        let end = (l + context_after).min(n);
                        for i in start..=end {
                            show.insert(i);
                        }
                    }
                    for i in show {
                        if i >= n {
                            continue;
                        }
                        let text = line_vec[i].to_str_lossy().into_owned();
                        out.push(format!("{}:{}:{}", fp.display(), i + 1, text));
                    }
                }
            }

            if !limited && match_count as usize >= MAX_RESULTS {
                limited = true;
                break;
            }
        }

        if limited {
            out.push(format!("[结果已限制为前 {MAX_RESULTS} 条匹配]"));
        }

        if out.is_empty() {
            Ok("未找到匹配。".to_string())
        } else {
            Ok(out.join("\n"))
        }
    }
}

/// 将 ripgrep 风格的文件类型名映射为文件名 glob（用于按类型过滤）。
/// 覆盖常见类型；未知类型返回空切片（不过滤）。
fn type_to_globs(ty: &str) -> &'static [&'static str] {
    match ty.to_ascii_lowercase().as_str() {
        "rust" => &["*.rs"],
        "python" | "py" => &["*.py"],
        "js" | "javascript" => &["*.js", "*.jsx", "*.mjs", "*.cjs"],
        "ts" | "typescript" => &["*.ts", "*.tsx"],
        "toml" => &["*.toml"],
        "json" => &["*.json"],
        "markdown" | "md" => &["*.md", "*.markdown"],
        "html" => &["*.html", "*.htm"],
        "css" => &["*.css"],
        "scss" => &["*.scss"],
        "java" => &["*.java"],
        "go" => &["*.go"],
        "c" => &["*.c", "*.h"],
        "cpp" | "c++" => &["*.cpp", "*.cc", "*.cxx", "*.hpp", "*.hxx"],
        "ruby" | "rb" => &["*.rb"],
        "php" => &["*.php"],
        "sh" | "bash" | "shell" => &["*.sh"],
        "yaml" | "yml" => &["*.yml", "*.yaml"],
        "xml" => &["*.xml"],
        "sql" => &["*.sql"],
        "swift" => &["*.swift"],
        "kotlin" | "kt" => &["*.kt", "*.kts"],
        "r" => &["*.r"],
        "lua" => &["*.lua"],
        "make" => &["Makefile", "*.mk"],
        "log" => &["*.log"],
        "txt" => &["*.txt"],
        "csv" => &["*.csv"],
        _ => &[],
    }
}

/// 对搜索结果按行做分页（对齐 Trae Grep 的 offset / head_limit）
fn apply_line_pagination(result: String, offset: usize, head_limit: usize) -> String {
    if offset == 0 && head_limit == 0 {
        return result;
    }
    let lines: Vec<&str> = result.lines().collect();
    let start = offset.min(lines.len());
    let end = if head_limit > 0 {
        (start + head_limit).min(lines.len())
    } else {
        lines.len()
    };
    lines[start..end].join("\n")
}

#[async_trait]
impl Capability for ContentSearchTool {
    fn meta(&self) -> CapabilityMeta {
        CapabilityMeta {
            name: "content_search".to_string(),
            description:
                "文件内容搜索（正则表达式）。跨平台，直接调用 ripgrep 库，遵循 .gitignore。"
                    .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "正则表达式模式"
                    },
                    "path": {
                        "type": "string",
                        "description": "搜索目录（默认当前目录）"
                    },
                    "output_mode": {
                        "type": "string",
                        "enum": ["content", "files_with_matches", "count"],
                        "description": "输出模式"
                    },
                    "include": {
                        "type": "string",
                        "description": "文件过滤模式（glob，如 '*.rs'）"
                    },
                    "case_sensitive": {
                        "type": "boolean",
                        "description": "是否区分大小写"
                    },
                    "type": {
                        "type": "string",
                        "description": "文件类型过滤（ripgrep 类型名，如 rust / python / js；按文件名 glob 等效过滤）"
                    },
                    "multiline": {
                        "type": "boolean",
                        "description": "是否跨行匹配（'.' 匹配换行）"
                    },
                    "context_before": {
                        "type": "integer",
                        "description": "匹配前显示的上下文行数"
                    },
                    "context_after": {
                        "type": "integer",
                        "description": "匹配后显示的上下文行数"
                    },
                    "head_limit": {
                        "type": "integer",
                        "description": "返回的最大行数（0 表示不限，对齐 Trae Grep 的 head_limit）"
                    },
                    "offset": {
                        "type": "integer",
                        "description": "跳过前 N 行后再返回（对齐 Trae Grep 的 offset）"
                    }
                },
                "required": ["pattern"]
            }),
            category: Some(crate::symbio_core::CapabilityCategory::FileOperation),
            examples: Some(vec![
                "pattern='fn main'".to_string(),
                "pattern='TODO', include='*.rs'".to_string(),
            ]),
            ..Default::default()
        }
    }

    async fn execute(&self, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload> {
        let args: Value = ctx.payload()?;
        let workdir_str = ctx.get(crate::symbio_core::WORKDIR).ok_or_else(|| {
            PluginError::ValidationError("Missing workdir in context".to_string())
        })?;
        if workdir_str.is_empty() {
            return Err(PluginError::ValidationError(
                "Empty workdir in context".to_string(),
            ));
        }
        let data = self.execute_inner(&args, &workdir_str).await?;
        Ok(PluginPayload::new(&data))
    }
}
