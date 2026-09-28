//! VDFS 挂载点（`<根>/memory`）—— 本插件**直接实现 `VdfsProvider`**。
//!
//! ## 挂载根有两份记忆
//!
//! ```text
//! <根>/memory/AGENTS.md      智能体自身的记忆（rw；物理文件 = 宿主目录的 AGENTS.md）
//! <根>/memory/WORKSPACE.md   工作区记忆（rw；物理文件 = {workdir}/AGENTS.md）
//! ```
//!
//! 智能体记忆的物理落位是**宿主目录的 `AGENTS.md`**（宿主目录 = 本插件目录的父
//! 目录，见 [`super::memory`]）；工作区记忆的物理落位是**工作区根的 `AGENTS.md`**
//! （行业通行约定，见 [`super::workspace`]）。两者的**物理文件同名**（同一族行业
//! 约定），因此挂载里的名字必须能区分归属：工作区那份叫 `WORKSPACE.md`。
//! 这里的挂载点是它们的**稳定地址**——不随会话变、不随「选了哪个智能体 / 哪个
//! 工作目录」那个会话级视图变（选中与否决定的是**注不注入**，不是文件在哪）。
//!
//! 根下**只列记忆文件**：插件配置文档（`PLUGIN.yml`）不并列——它与其它插件同一
//! 口径，进设置走 `ConfigurableVisitor` 那条通道；但按**真实文件名**仍然可达
//! （`stat` / `read` / `write` 都认 `PLUGIN.yml`），隐藏的是「列表里的位置」，
//! 不是可达性。
//!
//! ## 节点的形状由共享实现产出
//!
//! `list` 与 `stat` 共用 [`MemoryFile::node`] 这一份形状（名称 / 标题 / kind /
//! 大小 / mtime / 描述），因此两条链路不会分叉——从前「列表里的和点开的不是同一个
//! 东西」这类 bug 在类型上就写不出来。
//!
//! ## 无工作区时工作区条目缺席
//!
//! `ctx[WORKDIR]` 缺失 / 为空 = 没有工作区 → 列表里**没有** `WORKSPACE.md`，
//! 对它的 `stat` 如实 NotFound。「没有工作区」是正常状态，不是故障。
//!
//! ## 为什么删不掉
//!
//! 记忆是**累积型**资源：`delete` 一次就抹掉全部长期事实，而抹掉之后没有任何东西
//! 能把它找回来。要清空就写入空内容——那是一次可读、可审、可撤销（有版本控制时）
//! 的显式动作。因此 `delete` 对两份记忆都明确拒绝，而不是「允许但危险」。
//! （这条与 session 的记忆层**完全一致**，是共享实现级的约定。）

use super::memory::{self, MEMORY_FILE, PLUGIN_TITLE};
use super::plugin::MemoryPlugin;
use super::workspace::{self, MOUNT_FILE as WORKSPACE_FILE, SEGMENT_TITLE as WORKSPACE_LABEL};
use crate::providers::MemoryFile;
use crate::symbio_core::{
    vdfs_host_ctx, vdfs_notify_change, vdfs_unwatch_changes, vdfs_watch_changes,
};
use crate::symbio_core::{
    VdfsAccess, VdfsContent, VdfsContext, VdfsError, VdfsNode, VdfsProvider, VdfsRequest,
    VdfsResponse, VdfsResult, VdfsWriteResponse,
};
use crate::symbio_core::{PLUGIN_FILE, PLUGIN_ID_MEMORY};
use async_trait::async_trait;

const LABEL: &str = PLUGIN_TITLE;

/// 智能体记忆 → 节点（`list` 与 `stat` 共用同一份形状，两条链路不会分叉）
fn agent_memory_node(store: &MemoryFile) -> VdfsNode {
    store.node(&memory::node_spec())
}

/// 工作区记忆 → 节点（同上，节点名 = 挂载名 `WORKSPACE.md`）
fn workspace_memory_node(store: &MemoryFile) -> VdfsNode {
    store.node(&workspace::node_spec())
}

impl MemoryPlugin {
    /// 智能体记忆门面：作用域来自**装配位置**（本插件目录的父目录），与请求上下文
    /// 无关——插件被构造出来这件事本身就意味着这一层记忆存在。
    async fn agent_store(&self) -> MemoryFile {
        self.store().await
    }

    /// 工作区记忆门面：作用域来自**请求上下文**（`ctx[WORKDIR]`，经宿主请求透传）。
    ///
    /// 宿主句柄取不到是**机制级故障**（调用方没按契约构造上下文），如实报错——
    /// 与 agent / composite 的取法同一口径。
    async fn workspace_store(&self, ctx: &VdfsContext) -> VdfsResult<MemoryFile> {
        let host = vdfs_host_ctx(ctx)?;
        Ok(self.workspace_store_of(&host).await)
    }
}

#[async_trait]
impl VdfsProvider for MemoryPlugin {
    async fn dispatch(
        &self,
        ctx: &VdfsContext,
        path: &str,
        req: VdfsRequest,
    ) -> VdfsResult<VdfsResponse> {
        let agent = self.agent_store().await;
        let ws = self.workspace_store(ctx).await?;
        match req {
            VdfsRequest::List { .. } => {
                if !path.is_empty() {
                    return Err(VdfsError::not_found(format!(
                        "{LABEL}根下没有子目录：{path}"
                    )));
                }
                // 智能体记忆恒在（装配作用域）；工作区记忆有工作区才列
                let mut items = vec![agent_memory_node(&agent)];
                if ws.has_scope() {
                    items.push(workspace_memory_node(&ws));
                }
                Ok(VdfsResponse::list(items))
            }
            VdfsRequest::Stat => match path {
                // 自身根：**名字留空**——provider 不知道自己的挂载名，由使用方回填
                "" => Ok(VdfsResponse::Stat(VdfsNode::dir(
                    "",
                    LABEL,
                    VdfsAccess::LIST_TRAVERSE,
                ))),
                MEMORY_FILE => Ok(VdfsResponse::Stat(agent_memory_node(&agent))),
                WORKSPACE_FILE => {
                    if !ws.has_scope() {
                        return Err(VdfsError::not_found(format!(
                            "{WORKSPACE_LABEL}不可用：当前没有工作区"
                        )));
                    }
                    Ok(VdfsResponse::Stat(workspace_memory_node(&ws)))
                }
                // 配置文档按真实文件名可达（列表里不并列，见模块文档）
                PLUGIN_FILE => Ok(VdfsResponse::Stat(self.config_file().node())),
                other => Err(VdfsError::not_found(format!("未知路径：{other}"))),
            },
            VdfsRequest::Read => match path {
                MEMORY_FILE => {
                    let text = agent.read().map_err(VdfsError::internal)?;
                    Ok(VdfsResponse::Read(VdfsContent::text(text)))
                }
                WORKSPACE_FILE => {
                    let text = ws.read().map_err(VdfsError::internal)?;
                    Ok(VdfsResponse::Read(VdfsContent::text(text)))
                }
                PLUGIN_FILE => Ok(VdfsResponse::Read(
                    self.config_file().read(self.config_slot()).await?,
                )),
                other => Err(VdfsError::not_found(format!("未知路径：{other}"))),
            },
            VdfsRequest::Write { content } => match path {
                MEMORY_FILE => {
                    if content.binary {
                        return Err(VdfsError::invalid("智能体记忆是文本文件，不接受二进制内容"));
                    }
                    let text = content.text.as_deref().unwrap_or_default();
                    let existed = agent.exists();
                    // 容量闸门在共享实现里（`MemoryFile::write`）——本插件不重复实现
                    agent.write(text).map_err(VdfsError::invalid)?;
                    vdfs_notify_change(PLUGIN_ID_MEMORY, MEMORY_FILE);
                    Ok(VdfsResponse::Write(VdfsWriteResponse {
                        name: None,
                        created: !existed,
                        etag: None,
                    }))
                }
                WORKSPACE_FILE => {
                    if content.binary {
                        return Err(VdfsError::invalid("工作区记忆是文本文件，不接受二进制内容"));
                    }
                    let text = content.text.as_deref().unwrap_or_default();
                    let existed = ws.exists();
                    // 容量闸门在共享实现里（`MemoryFile::write`）——本插件不重复实现
                    ws.write(text).map_err(VdfsError::invalid)?;
                    vdfs_notify_change(PLUGIN_ID_MEMORY, WORKSPACE_FILE);
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
                    "{LABEL}不可删除（删除即丢失全部长期记忆）。\
                     如需清空，请向 `{MEMORY_FILE}` 或 `{WORKSPACE_FILE}` 写入空内容。"
                )))
            }
            VdfsRequest::Watch { sink } => {
                vdfs_watch_changes(PLUGIN_ID_MEMORY, path, sink).await?;
                Ok(VdfsResponse::Unit)
            }
            VdfsRequest::Unwatch => {
                vdfs_unwatch_changes(PLUGIN_ID_MEMORY, path).await?;
                Ok(VdfsResponse::Unit)
            }
            _ => Err(VdfsError::NotImplemented),
        }
    }
}

#[cfg(test)]
#[path = "vdfs.test.rs"]
mod tests;
