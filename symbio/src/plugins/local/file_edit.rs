//! 文件编辑工具 - 实现 Tool trait

use super::policy::SecurityPolicy;
use crate::symbio_core::{
    Capability, CapabilityMeta, InvokeRequest, InvokeRequestExt, InvokeResponse, PluginError,
    PluginPayload,
};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;

fn normalize_line_endings(s: &str) -> String {
    s.replace("\r\n", "\n").replace("\r", "\n")
}

fn find_line_ending_style(content: &str) -> &'static str {
    if content.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

/// 文件编辑工具
#[derive(Clone)]
pub struct FileEditTool {
    security: Arc<SecurityPolicy>,
}

impl FileEditTool {
    pub fn new(security: Arc<SecurityPolicy>) -> Self {
        Self { security }
    }

    async fn execute_inner(&self, args: &Value, workdir: &str) -> InvokeResponse<Value> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PluginError::ValidationError("缺少 'path' 参数".to_string()))?;

        let old_string = args
            .get("old_string")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PluginError::ValidationError("缺少 'old_string' 参数".to_string()))?;

        let new_string = args
            .get("new_string")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        // 验证 old_string 不为空
        if old_string.is_empty() {
            return Err(PluginError::ValidationError(
                "old_string 不能为空".to_string(),
            ));
        }

        // 安全检查：禁止 null 字节
        if path.contains('\0') {
            return Err(PluginError::ValidationError(
                "无效路径：不允许 null 字节".to_string(),
            ));
        }

        // 构建完整路径
        let workspace_dir = std::path::PathBuf::from(shellexpand::tilde(workdir).to_string());
        let full_path = if PathBuf::from(path).is_absolute() {
            PathBuf::from(path)
        } else {
            workspace_dir.join(path)
        };

        // 获取父目录
        let parent = full_path
            .parent()
            .ok_or_else(|| PluginError::ValidationError("无效路径：缺少父目录".to_string()))?;

        // 规范化 parent 以便验证（处理 Windows 的 \\?\ 前缀问题）
        let normalized_parent = if parent.exists() {
            // 如果存在，使用 canonicalize 获取规范路径
            tokio::fs::canonicalize(parent)
                .await
                .map_err(|e| PluginError::InternalError(format!("解析路径失败: {e}")))?
        } else {
            // 如果不存在（新文件），直接使用原始路径
            parent.to_path_buf()
        };

        // 验证解析后的路径
        if !self
            .security
            .is_path_allowed_for_write(&normalized_parent, &workspace_dir)
            .await
        {
            return Err(PluginError::ValidationError(format!(
                "路径超出工作区: {}",
                normalized_parent.display()
            )));
        }

        // 获取文件名
        let _file_name = full_path
            .file_name()
            .ok_or_else(|| PluginError::ValidationError("无效路径：缺少文件名".to_string()))?;

        // 使用原始的 full_path 进行文件操作
        let resolved_target = full_path.clone();

        // 符号链接检查
        if let Ok(meta) = tokio::fs::symlink_metadata(&resolved_target).await {
            if meta.file_type().is_symlink() {
                return Err(PluginError::ValidationError(format!(
                    "拒绝编辑符号链接: {}",
                    resolved_target.display()
                )));
            }
        }

        // 读取文件
        let raw_content = tokio::fs::read_to_string(&resolved_target)
            .await
            .map_err(|e| PluginError::InternalError(format!("读取文件失败: {e}")))?;

        // 检测原始文件的换行符风格
        let line_ending = find_line_ending_style(&raw_content);

        // 规范化内容（统一为 \n）以便匹配
        let content = normalize_line_endings(&raw_content);
        let old_normalized = normalize_line_endings(old_string);
        let new_normalized = normalize_line_endings(new_string);

        // 检查 old_string 和 new_string 是否相同
        if old_normalized == new_normalized {
            return Ok(json!({
                "success": true,
                "message": format!("已检查 {}: 内容已为最新，无需修改", path)
            }));
        }

        // 统计匹配次数
        let match_count = content.matches(&old_normalized).count();
        if match_count == 0 {
            // 安全截断：寻找 200 字节以内的最后一个合法字符边界
            let context_end = raw_content
                .char_indices()
                .map(|(idx, _)| idx)
                .rfind(|&idx| idx <= 200)
                .unwrap_or(0);
            let context = &raw_content[..context_end];

            return Err(PluginError::ValidationError(format!(
                "未找到 old_string 在文件 '{path}' 中。文件前 {} 字节上下文: {context:?}",
                context_end
            )));
        }
        if match_count > 1 {
            return Err(PluginError::ValidationError(format!(
                "old_string 在文件 '{path}' 中匹配 {match_count} 次；必须精确匹配一次"
            )));
        }

        // 执行替换
        let new_content = content.replacen(&old_normalized, &new_normalized, 1);

        // 恢复原始换行符风格
        let final_content = if line_ending == "\r\n" {
            new_content.replace("\n", "\r\n")
        } else {
            new_content
        };

        // 写入结果
        tokio::fs::write(&resolved_target, &final_content)
            .await
            .map_err(|e| PluginError::InternalError(format!("写入文件失败: {e}")))?;

        Ok(json!({
            "success": true,
            "message": format!("已编辑 {}: 替换了 1 处 ({} 字节)", path, final_content.len())
        }))
    }
}

#[async_trait]
impl Capability for FileEditTool {
    fn meta(&self) -> CapabilityMeta {
        CapabilityMeta {
            name: "file_edit".to_string(),
            description: "编辑文件（精确字符串替换）。old_string 必须在文件中精确匹配一次。"
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "文件路径"
                    },
                    "old_string": {
                        "type": "string",
                        "description": "要查找的文本（必须精确匹配一次）"
                    },
                    "new_string": {
                        "type": "string",
                        "description": "替换为的文本（默认空字符串）"
                    }
                },
                "required": ["path", "old_string"]
            }),
            category: Some(crate::symbio_core::CapabilityCategory::FileOperation),
            examples: Some(vec![
                "path='src/main.rs', old_string='fn old()', new_string='fn new()'".to_string(),
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
