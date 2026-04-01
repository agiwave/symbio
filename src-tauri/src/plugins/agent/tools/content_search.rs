//! 内容搜索工具 - 使用正则表达式搜索文件内容 - 实现 Plugin trait

use crate::core::traits::Plugin;
use crate::core::types::{PluginMeta, PluginError, PluginResult, InvokeStream, StreamChunk};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::Stdio;

const MAX_RESULTS: usize = 1000;
const MAX_OUTPUT_BYTES: usize = 1_048_576;
const TIMEOUT_SECS: u64 = 30;

/// 内容搜索工具
pub struct ContentSearchTool {
    workspace_dir: PathBuf,
    has_rg: bool,
}

impl ContentSearchTool {
    pub fn new(workspace_dir: PathBuf) -> Self {
        let has_rg = which::which("rg").is_ok();
        Self { workspace_dir, has_rg }
    }

    fn create_meta() -> PluginMeta {
        PluginMeta {
            name: "content_search".to_string(),
            description: "文件内容搜索（正则表达式）。使用 ripgrep 或 grep 进行搜索。".to_string(),
            version: "0.1.0".to_string(),
            input: Some(json!({
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
                        "description": "文件过滤模式"
                    },
                    "case_sensitive": {
                        "type": "boolean",
                        "description": "是否区分大小写"
                    }
                },
                "required": ["pattern"]
            })),
            output: Some(json!({
                "type": "object",
                "properties": {
                    "success": { "type": "boolean" },
                    "output": { "type": "string" },
                    "truncated": { "type": "boolean" }
                }
            })),
            author: Some("Symbio Team".to_string()),
        }
    }

    async fn execute_inner(&self, args: &Value) -> Result<StreamChunk, PluginError> {
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

        let full_search_path = if PathBuf::from(search_path).is_absolute() {
            PathBuf::from(search_path)
        } else {
            self.workspace_dir.join(search_path)
        };

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

        if !resolved_path.starts_with(&self.workspace_dir) {
            return Ok(StreamChunk {
                data: json!({}),
                done: true,
                error: Some("搜索路径超出工作区范围。".to_string()),
            });
        }

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

#[async_trait]
impl Plugin for ContentSearchTool {
    fn meta(&self, path: &str) -> PluginResult<PluginMeta> {
        if path.is_empty() {
            Ok(Self::create_meta())
        } else {
            Err(PluginError::NotFound(format!("路径不存在: {}", path)))
        }
    }

    fn invoke(&self, path: &str, input: Value) -> PluginResult<InvokeStream> {
        if !path.is_empty() {
            return Err(PluginError::NotFound(format!("路径不存在: {}", path)));
        }

        let result = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                self.execute_inner(&input).await
            })
        })?;

        Ok(InvokeStream::Single(result))
    }
}