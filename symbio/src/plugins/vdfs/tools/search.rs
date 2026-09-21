//! `vdfs_search` —— 文件名 Glob 搜索（**面向 LLM：结果摘要 message**）
//!
//! 移植自已下线的原生 `glob_search`：原生 `glob_search` 直接 `glob` 遍历工作区；
//! 本工具把「访问文件系统」改为「调用封装 provider 的 `search`」，拿到
//! `VdfsSearchResult` 后封装成原生 `glob_search` 的 `{results, truncated, message}`
//! 形状（message 为结果列表 + 总计 + 截断提示）。

use super::{request_of, tool, ToolVdfs};
use crate::symbio_core::{Capability, CapabilityMeta, ExecEnv, InvokeRequest, PluginError};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

const MAX_RESULTS: usize = 1000;

/// 文件名搜索工具（框架原生 `Capability`）
pub struct SearchTool {
    provider: Arc<ToolVdfs>,
}

impl SearchTool {
    pub fn new(provider: Arc<ToolVdfs>) -> Self {
        Self { provider }
    }
}

#[async_trait]
impl Capability for SearchTool {
    fn meta(&self) -> CapabilityMeta {
        tool(
            "vdfs_search",
            "文件名模式搜索（Glob，与原生 glob_search 一致）。pattern 为文件名 Glob（如 '**/*.rs'、'src/**/*.toml'）；path 为可选搜索基目录（相对工作目录，缺省工作目录根）。仅返回普通文件，目录不返回。",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "可选搜索基目录（相对工作目录），缺省为工作目录根" },
                    "pattern": { "type": "string", "description": "Glob 模式，如 '**/*.rs'" }
                },
                "required": ["pattern"]
            }),
            vec!["{\"pattern\":\"**/*.rs\"}", "{\"pattern\":\"src/**/*.toml\"}"],
            None,
        )
    }

    async fn execute(
        &self,
        args: Value,
        _env: &ExecEnv,
        ctx: Arc<dyn InvokeRequest>,
    ) -> Result<Value, PluginError> {
        let req: super::super::protocol::VdfsSearchRequest = request_of(&args);
        let data = self.provider.search(&ctx, &req.path, &req.pattern).await?;

        let mut results = data.results;
        let mut truncated = data.truncated;
        if results.len() > MAX_RESULTS {
            truncated = true;
            results.truncate(MAX_RESULTS);
        }
        results.sort();

        // 与原生 glob_search 一致的摘要文案
        let message = if results.is_empty() {
            "未找到匹配的文件。".to_string()
        } else {
            let mut msg = results.join("\n");
            if truncated {
                msg.push_str(&format!("\n\n[结果已截断：显示前 {MAX_RESULTS} 个匹配]"));
            }
            msg.push_str(&format!("\n\n总计: {} 个文件", results.len()));
            msg
        };

        Ok(json!({
            "results": results,
            "truncated": truncated,
            "message": message,
        }))
    }
}
