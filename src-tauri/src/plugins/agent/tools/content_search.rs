//! 内容搜索工具 - 使用正则表达式搜索文件内容

use crate::core::types::{PluginError, StreamChunk};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::process::Stdio;

const MAX_RESULTS: usize = 1000;
const MAX_OUTPUT_BYTES: usize = 1_048_576; // 1 MB
const TIMEOUT_SECS: u64 = 30;

/// 内容搜索工具
pub struct ContentSearchTool {
    has_rg: bool,
}

impl ContentSearchTool {
    pub fn new() -> Self {
        let has_rg = which::which("rg").is_ok();
        Self { has_rg }
    }

    /// 执行内容搜索
    pub async fn execute(
        &self,
        args: &Value,
        workspace_dir: &PathBuf,
    ) -> Result<StreamChunk, PluginError> {
        let pattern = args
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PluginError::ValidationError("缺少 'pattern' 参数".to_string()))?;

        if pattern.is_empty() {
            return Ok(StreamChunk {
                data: json!({}),
                done: true,
                error: Some("不允许使用空模式。".to_string()),
            });
        }

        let search_path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
        let output_mode = args.get("output_mode").and_then(|v| v.as_str()).unwrap_or("content");
        let include = args.get("include").and_then(|v| v.as_str());
        let case_sensitive = args.get("case_sensitive").and_then(|v| v.as_bool()).unwrap_or(true);
        let context_before = args.get("context_before").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let context_after = args.get("context_after").and_then(|v| v.as_u64()).unwrap_or(0) as usize;

        if !matches!(output_mode, "content" | "files_with_matches" | "count") {
            return Ok(StreamChunk {
                data: json!({}),
                done: true,
                error: Some(format!(
                    "无效的 output_mode '{}'。允许值: content, files_with_matches, count。",
                    output_mode
                )),
            });
        }

        // 构建搜索路径
        let full_search_path = if PathBuf::from(search_path).is_absolute() {
            PathBuf::from(search_path)
        } else {
            workspace_dir.join(search_path)
        };

        // 解析路径并验证在工作区内
        let resolved_path = match tokio::fs::canonicalize(&full_search_path).await {
            Ok(p) => p,
            Err(e) => {
                return Ok(StreamChunk {
                    data: json!({}),
                    done: true,
                    error: Some(format!("无法解析搜索路径: {}", e)),
                });
            }
        };

        if !resolved_path.starts_with(workspace_dir) {
            return Ok(StreamChunk {
                data: json!({}),
                done: true,
                error: Some("搜索路径超出工作区范围。".to_string()),
            });
        }

        // 使用 ripgrep 或 grep
        let output = if self.has_rg {
            self.search_with_rg(
                pattern,
                &resolved_path,
                output_mode,
                include,
                case_sensitive,
                context_before,
                context_after,
            ).await
        } else {
            self.search_with_grep(
                pattern,
                &resolved_path,
                output_mode,
                include,
                case_sensitive,
                context_before,
                context_after,
            ).await
        };

        match output {
            Ok(result) => {
                // 截断输出
                let truncated = result.len() > MAX_OUTPUT_BYTES;
                let result = if truncated {
                    format!("{}...\n\n[输出已截断：超过 {} 字节]", 
                        &result[..MAX_OUTPUT_BYTES], MAX_OUTPUT_BYTES)
                } else {
                    result
                };

                Ok(StreamChunk {
                    data: json!({
                        "success": true,
                        "output": result,
                        "truncated": truncated,
                        "backend": if self.has_rg { "ripgrep" } else { "grep" }
                    }),
                    done: true,
                    error: None,
                })
            }
            Err(e) => Ok(StreamChunk {
                data: json!({}),
                done: true,
                error: Some(e),
            }),
        }
    }

    async fn search_with_rg(
        &self,
        pattern: &str,
        path: &PathBuf,
        output_mode: &str,
        include: Option<&str>,
        case_sensitive: bool,
        context_before: usize,
        context_after: usize,
    ) -> Result<String, String> {
        let mut cmd = tokio::process::Command::new("rg");
        cmd.arg("--no-heading")
           .arg("--line-number")
           .arg("--color=never")
           .arg(format!("--max-count={}", MAX_RESULTS));

        if !case_sensitive {
            cmd.arg("-i");
        }

        match output_mode {
            "files_with_matches" => {
                cmd.arg("--files-with-matches");
            }
            "count" => {
                cmd.arg("--count");
            }
            _ => {
                // content mode
                if context_before > 0 {
                    cmd.arg(format!("--before-context={}", context_before));
                }
                if context_after > 0 {
                    cmd.arg(format!("--after-context={}", context_after));
                }
            }
        }

        if let Some(glob) = include {
            cmd.arg("--glob").arg(glob);
        }

        cmd.arg(pattern).arg(path);
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        let output = cmd
            .kill_on_drop(true)
            .output()
            .await
            .map_err(|e| format!("执行 ripgrep 失败: {}", e))?;

        if output.status.success() {
            String::from_utf8(output.stdout)
                .map_err(|e| format!("解析输出失败: {}", e))
        } else if output.status.code() == Some(1) {
            // ripgrep 返回 1 表示没有匹配
            Ok("未找到匹配。".to_string())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(format!("ripgrep 错误: {}", stderr))
        }
    }

    async fn search_with_grep(
        &self,
        pattern: &str,
        path: &PathBuf,
        output_mode: &str,
        include: Option<&str>,
        case_sensitive: bool,
        context_before: usize,
        context_after: usize,
    ) -> Result<String, String> {
        let mut cmd = tokio::process::Command::new("grep");
        cmd.arg("-rn")
           .arg("-E")
           .arg("--color=never");

        if !case_sensitive {
            cmd.arg("-i");
        }

        match output_mode {
            "files_with_matches" => {
                cmd.arg("-l");
            }
            "count" => {
                cmd.arg("-c");
            }
            _ => {
                // content mode
                if context_before > 0 {
                    cmd.arg(format!("-B{}", context_before));
                }
                if context_after > 0 {
                    cmd.arg(format!("-A{}", context_after));
                }
            }
        }

        if let Some(glob) = include {
            cmd.arg("--include").arg(glob);
        }

        cmd.arg(pattern).arg(path);
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        let output = cmd
            .kill_on_drop(true)
            .output()
            .await
            .map_err(|e| format!("执行 grep 失败: {}", e))?;

        if output.status.success() {
            String::from_utf8(output.stdout)
                .map_err(|e| format!("解析输出失败: {}", e))
        } else if output.status.code() == Some(1) {
            Ok("未找到匹配。".to_string())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(format!("grep 错误: {}", stderr))
        }
    }
}

impl Default for ContentSearchTool {
    fn default() -> Self {
        Self::new()
    }
}
