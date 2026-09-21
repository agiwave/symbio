//! `vdfs_tree` —— 递归展开一棵子树（只下钻访问位含 t 的目录）。
//!
//! 树遍历 = 对封装 provider 的多次 `list`（`t` 位控制下钻）；深度与数量上限防爆炸，
//! 单分支失败跳过不拖垮整体。节点的 `path` 用**大模型地址空间**的相对路径回填
//! （如 `src/main.rs`），与其余工具的入参口径一致。

use super::{request_of, tool, ToolVdfs};
use crate::symbio_core::{Capability, CapabilityMeta, ExecEnv, InvokeRequest, PluginError};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::Arc;

/// 树遍历工具（框架原生 `Capability`）
pub struct TreeTool {
    provider: Arc<ToolVdfs>,
}

impl TreeTool {
    pub fn new(provider: Arc<ToolVdfs>) -> Self {
        Self { provider }
    }
}

#[async_trait]
impl Capability for TreeTool {
    fn meta(&self) -> CapabilityMeta {
        tool(
            "vdfs_tree",
            "递归展开一棵子树（只下钻访问位含 t 的目录）。一次拿到层级结构，适合先整体了解工作目录布局。",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": super::PATH_DESC },
                    "depth": { "type": "integer", "description": "最大深度，缺省 3；0 表示不限" },
                    "limit": { "type": "integer", "description": "最多返回节点数，缺省 500" }
                },
                "required": ["path"]
            }),
            vec!["{\"path\":\"/\"}", "{\"path\":\"src\",\"depth\":2}"],
            None,
        )
    }

    async fn execute(
        &self,
        args: Value,
        _env: &ExecEnv,
        ctx: Arc<dyn InvokeRequest>,
    ) -> Result<Value, PluginError> {
        let req: super::super::protocol::VdfsTreeRequest = request_of(&args);
        let depth_limit = req.depth.unwrap_or(3); // 0 = 不限
        let count_limit = req.limit.unwrap_or(500).max(1) as usize;

        let mut out: Vec<crate::symbio_core::vdfs_provider::VdfsNode> = Vec::new();
        let mut truncated = false;

        // 队列元素 = (目录地址, 深度)；地址始终是大模型地址空间，由封装 provider 翻译
        let mut queue: VecDeque<(String, u32)> = VecDeque::new();
        queue.push_back((req.path.clone(), 0));

        while let Some((dir, depth)) = queue.pop_front() {
            let children = match self.provider.list(&ctx, &dir).await {
                Ok(c) => c,
                // 单分支失败降级：不让一棵子树拖垮整个遍历
                Err(e) => {
                    crate::plugin_warn!("vdfs", "vdfs_tree: 列出 {dir} 失败，已跳过: {e}");
                    continue;
                }
            };

            // 子地址 = 父目录 + 子名（`/` 与 `""` 都视为根，子地址直接用子名）
            let base = dir.trim_end_matches('/').to_string();
            for child in children {
                if out.len() >= count_limit {
                    truncated = true;
                    break;
                }
                let mut child = child;
                let child_addr = if base.is_empty() {
                    child.name.clone()
                } else {
                    format!("{base}/{}", child.name)
                };
                if child.path.is_empty() {
                    child.path = child_addr.clone();
                }
                let descend =
                    child.access.traverse && (depth_limit == 0 || depth + 1 < depth_limit);
                out.push(child);
                if descend {
                    queue.push_back((child_addr, depth + 1));
                }
            }
            if truncated {
                break;
            }
        }

        Ok(serde_json::to_value(
            &super::super::protocol::VdfsTreeResponse {
                path: req.path,
                nodes: out,
                truncated,
            },
        )?)
    }
}
