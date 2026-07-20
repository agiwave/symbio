//! 文件写入工具 - 实现 Tool trait

use super::policy::SecurityPolicy;
use crate::symbio_core::{
    Capability, CapabilityMeta, InvokeRequest, InvokeRequestExt, InvokeResponse, PluginError,
    PluginPayload,
};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;

/// 文件写入工具
#[derive(Clone)]
pub struct FileWriteTool {
    security: Arc<SecurityPolicy>,
}

impl FileWriteTool {
    pub fn new(security: Arc<SecurityPolicy>) -> Self {
        Self { security }
    }

    async fn execute_inner(&self, args: Value, workdir: &str) -> Result<Value, PluginError> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PluginError::ValidationError("缺少 path 参数".to_string()))?;

        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PluginError::ValidationError("缺少 content 参数".to_string()))?;

        // 检查速率限制
        if self.security.is_rate_limited() {
            return Err(PluginError::InternalError("速率限制：操作过于频繁".into()));
        }

        // 获取工作区目录
        let workspace_dir = std::path::PathBuf::from(shellexpand::tilde(workdir).to_string());

        // 检查路径权限
        if !self
            .security
            .is_path_allowed_for_write(path, &workspace_dir)
            .await
        {
            return Err(PluginError::InternalError(format!(
                "路径不允许写入: {path}"
            )));
        }

        // 构建完整路径
        let full_path = if PathBuf::from(path).is_absolute() {
            PathBuf::from(path)
        } else {
            workspace_dir.join(path)
        };

        // 确保父目录存在
        if let Some(parent) = full_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| PluginError::InternalError(format!("创建目录失败: {e}")))?;
        }

        // 记录动作
        self.security.record_action();

        // 写入文件
        tokio::fs::write(&full_path, content)
            .await
            .map_err(|e| PluginError::InternalError(format!("写入文件失败: {e}")))?;

        Ok(json!({
            "success": true,
            "path": path,
            "bytes_written": content.len()
        }))
    }
}

#[async_trait]
impl Capability for FileWriteTool {
    fn meta(&self) -> CapabilityMeta {
        CapabilityMeta {
            name: "write_file".to_string(),
            description: "写入文件内容。相对路径从工作目录开始。".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "文件路径"
                    },
                    "content": {
                        "type": "string",
                        "description": "文件内容"
                    }
                },
                "required": ["path", "content"]
            }),
            category: Some(crate::symbio_core::CapabilityCategory::FileOperation),
            examples: Some(vec!["path='test.txt', content='Hello World'".to_string()]),
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
