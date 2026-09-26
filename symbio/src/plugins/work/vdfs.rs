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
use crate::symbio_core::{host_ctx, notify_change, unwatch_changes, watch_changes};
use crate::symbio_core::{
    MemoryFile, MemoryNodeSpec, MEMORY_AGENTS_FILE, PLUGIN_FILE, PLUGIN_ID_WORK,
};
use crate::symbio_core::{
    VdfsAccess, VdfsContent, VdfsContext, VdfsError, VdfsNode, VdfsProvider, VdfsRequest,
    VdfsResponse, VdfsResult, VdfsWriteResponse,
};
use async_trait::async_trait;

const LABEL: &str = SEGMENT_TITLE;

/// 记忆文件 → 节点（`list` 与 `stat` 共用同一份形状，两条链路不会分叉）
fn memory_node(store: &MemoryFile) -> VdfsNode {
    store.node(&MemoryNodeSpec {
        title: LABEL,
        kind: PLUGIN_ID_WORK,
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
    async fn dispatch(
        &self,
        ctx: &VdfsContext,
        path: &str,
        req: VdfsRequest,
    ) -> VdfsResult<VdfsResponse> {
        let store = self.store_from_ctx(ctx).await?;
        match req {
            VdfsRequest::List { .. } => {
                if !path.is_empty() {
                    return Err(VdfsError::not_found(format!(
                        "{LABEL}根下没有子目录：{path}"
                    )));
                }
                // 无工作区：没有记忆可列（空表而非报错——「没有工作区」是正常状态）
                if !store.has_scope() {
                    return Ok(VdfsResponse::list(Vec::<VdfsNode>::new()));
                }
                Ok(VdfsResponse::list(vec![memory_node(&store)]))
            }
            VdfsRequest::Stat => match path {
                // 自身根：**名字留空**——provider 不知道自己的挂载名，由使用方回填
                "" => Ok(VdfsResponse::Stat(VdfsNode::dir(
                    "",
                    LABEL,
                    VdfsAccess::LIST_TRAVERSE,
                ))),
                MEMORY_AGENTS_FILE => {
                    if !store.has_scope() {
                        return Err(VdfsError::not_found(format!(
                            "{LABEL}不可用：当前没有工作区"
                        )));
                    }
                    Ok(VdfsResponse::Stat(memory_node(&store)))
                }
                // 配置文档按真实文件名可达（列表里不并列，见模块文档）
                PLUGIN_FILE => Ok(VdfsResponse::Stat(self.config_file().node())),
                other => Err(VdfsError::not_found(format!("未知路径：{other}"))),
            },
            VdfsRequest::Read => match path {
                MEMORY_AGENTS_FILE => {
                    let text = store.read().map_err(VdfsError::internal)?;
                    Ok(VdfsResponse::Read(VdfsContent::text(text)))
                }
                PLUGIN_FILE => Ok(VdfsResponse::Read(
                    self.config_file().read(self.config_slot()).await?,
                )),
                other => Err(VdfsError::not_found(format!("未知路径：{other}"))),
            },
            VdfsRequest::Write { content } => match path {
                MEMORY_AGENTS_FILE => {
                    if content.binary {
                        return Err(VdfsError::invalid("工作区记忆是文本文件，不接受二进制内容"));
                    }
                    let text = content.text.as_deref().unwrap_or_default();
                    let existed = store.exists();
                    // 容量闸门在内核里（`MemoryFile::write`）——本插件不重复实现
                    store.write(text).map_err(VdfsError::invalid)?;
                    notify_change(PLUGIN_ID_WORK, MEMORY_AGENTS_FILE);
                    Ok(VdfsResponse::Write(VdfsWriteResponse {
                        name: None,
                        created: !existed,
                        etag: None,
                    }))
                }
                PLUGIN_FILE => Ok(VdfsResponse::Write(
                    self.config_file()
                        .apply(self.config_slot(), &content)
                        .await?,
                )),
                other => Err(VdfsError::not_found(format!("未知路径：{other}"))),
            },
            // 记忆**不可删除**（见模块文档）：要清空就写入空内容
            VdfsRequest::Delete { .. } => {
                if path.is_empty() {
                    return Err(VdfsError::Forbidden(format!("不可删除挂载点：{path}")));
                }
                Err(VdfsError::Forbidden(format!(
                    "{LABEL}不可删除（删除即丢失全部长期事实）。\
                     如需清空，请向 `{MEMORY_AGENTS_FILE}` 写入空内容。"
                )))
            }
            VdfsRequest::Watch { sink } => {
                watch_changes(PLUGIN_ID_WORK, path, sink).await?;
                Ok(VdfsResponse::Unit)
            }
            VdfsRequest::Unwatch => {
                unwatch_changes(PLUGIN_ID_WORK, path).await?;
                Ok(VdfsResponse::Unit)
            }
            _ => Err(VdfsError::NotImplemented),
        }
    }
}

#[cfg(test)]
#[path = "vdfs.test.rs"]
mod tests;
