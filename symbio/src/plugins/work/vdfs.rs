//! VDFS 挂载点（`<根>/work`）—— 本插件**直接实现 `VdfsProvider`**。
//!
//! ## 挂载根只有一样东西
//!
//! ```text
//! <根>/work/AGENTS.md    工作区记忆（rw；界面与模型共用这一份）
//! ```
//!
//! 物理落位是**工作区根目录的 `AGENTS.md`**（行业通行约定，见 [`super::memory`]
//! 的模块文档）；这里的挂载点是它的**稳定地址**——不随会话变、不随「工作目录」
//! 那个会话级视图变。
//!
//! 根下**只列记忆文件**：插件配置文档（`PLUGIN.yml`）不并列——它与其它插件同一口径，
//! 进设置走 `ConfigurableVisitor` 那条通道；但按**真实文件名**仍然可达
//! （`stat` / `read` / `write` 都认 `PLUGIN.yml`），隐藏的是「列表里的位置」，
//! 不是可达性。
//!
//! ## 节点的形状由内核产出
//!
//! `list` 与 `stat` 共用 [`MemoryFile::node`] 这一份形状（名称 / 标题 / kind /
//! 大小 / mtime / 描述），因此两条链路不会分叉——从前「列表里的和点开的不是同一个
//! 东西」这类 bug 在类型上就写不出来。
//!
//! ## 为什么删不掉
//!
//! 记忆是挂载点的唯一内容，也是**累积型**资源：`delete` 一次就抹掉全部长期事实，
//! 而抹掉之后没有任何东西能把它找回来。要清空就写入空内容——那是一次可读、可审、
//! 可撤销（有版本控制时）的显式动作。因此 `delete` 明确拒绝，而不是「允许但危险」。
//! （这条与 agent / session 的记忆层**完全一致**，是内核级的约定。）

use super::memory::{MEMORY_DESCRIPTION, SEGMENT_TITLE};
use super::plugin::WorkPlugin;
use crate::symbio_core::vdfs::{host_ctx, notify_change, unwatch_changes, watch_changes};
use crate::symbio_core::vdfs_provider::{
    VdfsAccess, VdfsChangeSink, VdfsContent, VdfsContext, VdfsError, VdfsNode, VdfsProvider,
    VdfsResult, VdfsWriteResponse,
};
use crate::symbio_core::{MemoryFile, NodeSpec, AGENTS_FILE, PLUGIN_FILE, PLUGIN_WORK};
use async_trait::async_trait;

const LABEL: &str = SEGMENT_TITLE;

/// 记忆文件 → 节点（`list` 与 `stat` 共用同一份形状，两条链路不会分叉）
fn memory_node(store: &MemoryFile) -> VdfsNode {
    store.node(&NodeSpec {
        title: LABEL,
        kind: PLUGIN_WORK,
        description: MEMORY_DESCRIPTION,
    })
}

impl WorkPlugin {
    /// 依宿主上下文构造记忆门面（工作目录经 `ctx[WORKDIR]`，两道闸门取自配置）
    async fn store_from_ctx(&self, ctx: &VdfsContext) -> VdfsResult<MemoryFile> {
        let host = host_ctx(ctx)?;
        Ok(self.store_of(&host).await)
    }
}

#[async_trait]
impl VdfsProvider for WorkPlugin {
    fn label(&self) -> Option<&str> {
        Some(LABEL)
    }

    fn description(&self) -> Option<&str> {
        Some("工作区记忆：跨会话保留的长期事实与约定，模型可读写。")
    }

    fn order(&self) -> i32 {
        // 在既有挂载点之后（session 1 / model 2 / agent 3 / skill 4 / mcp 5 /
        // setting 6 / local 7 / web 8 / gateway 9 / telegram 10）
        11
    }

    fn icon(&self) -> Option<&str> {
        Some(PLUGIN_WORK)
    }

    /// 根可列举 + 可递归遍历（记忆文件在根下，树视图要能走到它）
    fn root_access(&self) -> VdfsAccess {
        VdfsAccess::LIST_TRAVERSE
    }

    fn root_hidden(&self) -> bool {
        true
    }

    /// 根下不可新建：记忆文件是**唯一且恒在**的那一个，没有第二种东西可建
    async fn root_new_types(&self) -> Vec<crate::symbio_core::vdfs_provider::VdfsNewType> {
        Vec::new()
    }

    async fn list(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<Vec<VdfsNode>> {
        if !path.is_empty() {
            return Err(VdfsError::not_found(format!(
                "{LABEL}根下没有子目录：{path}"
            )));
        }
        let store = self.store_from_ctx(ctx).await?;
        // 无工作区：没有记忆可列（空表而非报错——「没有工作区」是正常状态）
        if !store.has_scope() {
            return Ok(Vec::new());
        }
        Ok(vec![memory_node(&store)])
    }

    async fn stat(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<VdfsNode> {
        let store = self.store_from_ctx(ctx).await?;
        match path {
            // 自身根：**名字留空**——provider 不知道自己的挂载名，由使用方回填
            "" => Ok(VdfsNode::dir("", LABEL, self.root_access())),
            AGENTS_FILE => {
                if !store.has_scope() {
                    return Err(VdfsError::not_found(format!(
                        "{LABEL}不可用：当前没有工作区"
                    )));
                }
                Ok(memory_node(&store))
            }
            // 配置文档按真实文件名可达（列表里不并列，见模块文档）
            PLUGIN_FILE => Ok(self.config_file().node()),
            other => Err(VdfsError::not_found(format!("未知路径：{other}"))),
        }
    }

    async fn read(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<VdfsContent> {
        let store = self.store_from_ctx(ctx).await?;
        match path {
            AGENTS_FILE => {
                let text = store.read().map_err(VdfsError::internal)?;
                Ok(VdfsContent::text(path, text))
            }
            PLUGIN_FILE => self.config_file().read(self.config_slot()).await,
            other => Err(VdfsError::not_found(format!("未知路径：{other}"))),
        }
    }

    async fn write(
        &self,
        ctx: &VdfsContext,
        path: &str,
        content: &VdfsContent,
    ) -> VdfsResult<VdfsWriteResponse> {
        let store = self.store_from_ctx(ctx).await?;
        match path {
            AGENTS_FILE => {
                if content.binary {
                    return Err(VdfsError::invalid("工作区记忆是文本文件，不接受二进制内容"));
                }
                let text = content.text.as_deref().unwrap_or_default();
                let existed = store.exists();
                // 容量闸门在内核里（`MemoryFile::write`）——本插件不重复实现
                store.write(text).map_err(VdfsError::invalid)?;
                notify_change(PLUGIN_WORK, AGENTS_FILE);
                Ok(VdfsWriteResponse {
                    path: path.to_string(),
                    created: !existed,
                    etag: None,
                })
            }
            PLUGIN_FILE => self.config_file().apply(self.config_slot(), content).await,
            other => Err(VdfsError::not_found(format!("未知路径：{other}"))),
        }
    }

    /// 记忆**不可删除**（见模块文档）：要清空就写入空内容
    async fn delete(&self, _ctx: &VdfsContext, path: &str, _recursive: bool) -> VdfsResult<()> {
        if path.is_empty() {
            return Err(VdfsError::Forbidden(format!("不可删除挂载点：{path}")));
        }
        Err(VdfsError::Forbidden(format!(
            "{LABEL}不可删除（删除即丢失全部长期事实）。\
             如需清空，请向 `{AGENTS_FILE}` 写入空内容。"
        )))
    }

    async fn watch(&self, _ctx: &VdfsContext, path: &str, sink: VdfsChangeSink) -> VdfsResult<()> {
        watch_changes(PLUGIN_WORK, path, sink).await
    }

    async fn unwatch(&self, _ctx: &VdfsContext, path: &str) -> VdfsResult<()> {
        unwatch_changes(PLUGIN_WORK, path).await
    }
}

#[cfg(test)]
#[path = "vdfs.test.rs"]
mod tests;
