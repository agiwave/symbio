//! 文件读取工具 - 实现 Tool trait

use super::policy::SecurityPolicy;
use crate::symbio_core::{
    Capability, CapabilityMeta, InvokeRequest, InvokeRequestExt, InvokeResponse, PluginError,
    PluginPayload,
};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;

const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024; // 10MB

/// 文件读取工具
#[derive(Clone)]
pub struct FileReadTool {
    security: Arc<SecurityPolicy>,
}

impl FileReadTool {
    pub fn new(security: Arc<SecurityPolicy>) -> Self {
        Self { security }
    }

    async fn execute_inner(&self, args: Value, workdir: &str) -> Result<Value, PluginError> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PluginError::ValidationError("缺少 path 参数".to_string()))?;

        // 检查速率限制
        if self.security.is_rate_limited() {
            return Err(PluginError::InternalError("速率限制：操作过于频繁".into()));
        }

        // 获取工作区目录
        let workspace_dir = PathBuf::from(shellexpand::tilde(workdir).to_string());

        // 检查路径权限
        if !self
            .security
            .is_path_allowed_for_read(path, &workspace_dir)
            .await
        {
            return Err(PluginError::InternalError(format!(
                "路径不允许访问: {path}"
            )));
        }

        // 构建完整路径
        let full_path = if PathBuf::from(path).is_absolute() {
            PathBuf::from(path)
        } else {
            workspace_dir.join(path)
        };

        // 解析符号链接
        let resolved = tokio::fs::canonicalize(&full_path)
            .await
            .map_err(|e| PluginError::InternalError(format!("无法解析路径: {e}")))?;

        // 验证解析后的路径
        if !self
            .security
            .is_path_allowed_for_read(&resolved, &workspace_dir)
            .await
        {
            return Err(PluginError::InternalError(format!(
                "路径解析后超出工作区范围: {}",
                resolved.display()
            )));
        }

        // 检查文件大小
        let metadata = tokio::fs::metadata(&resolved)
            .await
            .map_err(|e| PluginError::InternalError(format!("无法读取文件元数据: {e}")))?;

        if metadata.len() > MAX_FILE_SIZE {
            return Err(PluginError::InternalError(format!(
                "文件过大: {} 字节 (限制: {} 字节)",
                metadata.len(),
                MAX_FILE_SIZE
            )));
        }

        // 记录动作
        self.security.record_action();

        // 图片文件：返回 base64 载荷（多模态；前端渲染为后续项，对齐 Trae Read 的图片能力）
        if let Some(mime) = image_mime(&resolved) {
            let bytes = tokio::fs::read(&resolved)
                .await
                .map_err(|e| PluginError::InternalError(format!("读取文件失败: {e}")))?;
            return Ok(json!({
                "type": "image",
                "path": path,
                "mime": mime,
                "data": base64_encode(&bytes),
            }));
        }

        // 读取文件
        let content = tokio::fs::read_to_string(&resolved)
            .await
            .map_err(|e| PluginError::InternalError(format!("读取文件失败: {e}")))?;

        // 添加行号
        let lines: Vec<&str> = content.lines().collect();
        let total = lines.len();

        let offset = args
            .get("offset")
            .and_then(|v| v.as_u64())
            .map(|v| (v.max(1) as usize).saturating_sub(1))
            .unwrap_or(0);

        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize);

        let start = offset.min(total);
        let end = limit.map(|l| (start + l).min(total)).unwrap_or(total);

        let numbered: String = lines[start..end]
            .iter()
            .enumerate()
            .map(|(i, line)| format!("{}: {}", start + i + 1, line))
            .collect::<Vec<_>>()
            .join("\n");

        let summary = if start > 0 || end < total {
            format!("\n[行 {}-{}，共 {} 行]", start + 1, end, total)
        } else {
            format!("\n[共 {total} 行]")
        };

        Ok(json!({
            "content": format!("{}{}", numbered, summary),
            "path": path,
            "total_lines": total
        }))
    }
}

#[async_trait]
impl Capability for FileReadTool {
    fn meta(&self) -> CapabilityMeta {
        CapabilityMeta {
            name: "read_file".to_string(),
            description: "读取文件内容，支持行号和分页。相对路径从工作目录开始。".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "文件路径（相对路径从工作目录开始）"
                    },
                    "offset": {
                        "type": "integer",
                        "description": "起始行号（从1开始，默认1）"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "返回的最大行数（默认全部）"
                    }
                },
                "required": ["path"]
            }),
            category: Some(crate::symbio_core::CapabilityCategory::FileOperation),
            examples: Some(vec![
                "path='README.md'".to_string(),
                "path='src/main.rs', limit=50".to_string(),
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
        let result = self.execute_inner(args, &workdir_str).await?;
        Ok(PluginPayload::new(&result))
    }
}

/// 常见图片扩展名 → MIME（用于多模态读取）
fn image_mime(path: &std::path::Path) -> Option<&'static str> {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => Some("image/png"),
        Some("jpg") | Some("jpeg") => Some("image/jpeg"),
        Some("gif") => Some("image/gif"),
        Some("webp") => Some("image/webp"),
        Some("bmp") => Some("image/bmp"),
        _ => None,
    }
}

/// 标准 base64 编码（无外部依赖）
fn base64_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((input.len() + 2) / 3 * 4);
    for chunk in input.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | (b[2] as u32);
        out.push(ALPHABET[((n >> 18) & 63) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}
