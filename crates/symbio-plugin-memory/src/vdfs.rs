//! VDFS 挂载点（`<根>/memory`）—— 本插件**直接实现 `VdfsProvider`**。
//!
//! ## 挂载根有三份记忆
//!
//! ```text
//! <根>/memory/MEMORY.md      智能体自身的记忆（rw；物理文件 = {agent_dir}/MEMORY.md）
//! <根>/memory/WORKSPACE.md   工作区记忆（rw；物理文件 = {work_dir}/MEMORY.md）
//! <根>/memory/SESSION.md     会话记忆（rw；物理文件 = {session_dir}/MEMORY.md）
//! ```
//!
//! 三者的作用域分别来自装配位置、`ctx[WORKDIR]` 与 `ctx[SESSION_DIR]`，
//! 物理文件名统一 `MEMORY.md`机制一致、可预测。挂载名则各自点明作用域，
//! 列表里不会撞名。三者都归本插件——稳定指令 `AGENTS.md` 归 `setting`，与本挂载点
//! 无关。智能体 / 工作区两份地址不随会话变；会话那份跟着当前会话的作用域走。
//!
//! 根下**只列记忆文件**：插件配置文档（`PLUGIN.yml`）不并列——它与其它插件同一
//! 口径，进设置走 `CONFIGURABLE_VISITOR` 那条通道；但按**真实文件名**仍然可达
//! （`stat` / `read` / `write` 都认 `PLUGIN.yml`），隐藏的是「列表里的位置」，
//! 不是可达性。
//!
//! ## 挂载点标了 `hidden`：入口在设置页
//!
//! 记忆没有「列表内容」：用户要改的是闸门，不是浏览一个资源集合。
//! 因此本挂载点（[`MemoryPlugin::metadata`]）声明 `hidden`，不进左侧主导航；
//! 入口是设置页（`plugin_manager` 的 memory 条目）。地址本身照常可用——模型与
//! 深链仍走 `<根>/memory/MEMORY.md`。
//!
//! 设置页编辑的只是 `PLUGIN.yml` 这份配置文档（三层的开关与闸门）；**记忆正文不在
//! 配置里**，一律经 `<根>/memory/MEMORY.md` / `WORKSPACE.md` / `SESSION.md` 读写。
//! 稳定指令 `AGENTS.md` 归 `setting` 插件，与本挂载点无关。
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
use super::session::{self, MOUNT_FILE as SESSION_FILE, SEGMENT_TITLE as SESSION_LABEL};
use super::workspace::{self, MOUNT_FILE as WORKSPACE_FILE, SEGMENT_TITLE as WORKSPACE_LABEL};
use crate::memory_file::MemoryFile;
use async_trait::async_trait;
use symbio_plugin_api::PLUGIN_FILE;
use symbio_plugin_api::PLUGIN_ID_MEMORY;
use symbio_plugin_api::{
    vdfs_host_ctx, vdfs_notify_change, vdfs_unwatch_changes, vdfs_watch_changes,
};
use symbio_plugin_api::{
    VdfsAccess, VdfsContent, VdfsContext, VdfsError, VdfsNode, VdfsProvider, VdfsRequest,
    VdfsResponse, VdfsResult, VdfsWriteResponse,
};

const LABEL: &str = PLUGIN_TITLE;

/// 智能体记忆 → 节点（`list` 与 `stat` 共用同一份形状，两条链路不会分叉）
async fn agent_memory_node(store: &MemoryFile) -> VdfsNode {
    store.node(&memory::node_spec()).await
}

/// 工作区记忆 → 节点（同上，节点名 = 挂载名 `WORKSPACE.md`）
async fn workspace_memory_node(store: &MemoryFile) -> VdfsNode {
    store.node(&workspace::node_spec()).await
}

/// 会话记忆 → 节点（同上，节点名 = 挂载名 `SESSION.md`）
async fn session_memory_node(store: &MemoryFile) -> VdfsNode {
    store.node(&session::node_spec()).await
}

impl MemoryPlugin {
    /// 智能体记忆门面：作用域来自请求 ctx 的 `AGENT_VDFS` provider。
    async fn agent_store(&self, vctx: &VdfsContext) -> VdfsResult<MemoryFile> {
        let host = vdfs_host_ctx(vctx)?;
        Ok(self.store(&host).await)
    }

    /// 工作区记忆门面：作用域来自**请求上下文**（`ctx[WORKDIR]`，经宿主请求透传）。
    ///
    /// 宿主句柄取不到是**机制级故障**（调用方没按契约构造上下文），如实报错——
    /// 与 agent / composite 的取法同一口径。
    async fn workspace_store(&self, ctx: &VdfsContext) -> VdfsResult<MemoryFile> {
        let host = vdfs_host_ctx(ctx)?;
        Ok(self.workspace_store_of(&host).await)
    }

    /// 会话记忆门面：作用域来自**本轮上下文**（`ctx[SESSION_DIR]`）。
    ///
    /// 宿主句柄取不到是机制级故障；句柄在但没有会话目录是正常状态（无作用域），
    /// 由 [`MemoryFile::has_scope`] 表达，不在这里报错。
    async fn session_store(&self, ctx: &VdfsContext) -> VdfsResult<MemoryFile> {
        let host = vdfs_host_ctx(ctx)?;
        Ok(self.session_store_of(&host).await)
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
        let agent = self.agent_store(ctx).await?;
        let ws = self.workspace_store(ctx).await?;
        let sess = self.session_store(ctx).await?;
        match req {
            VdfsRequest::List { .. } => {
                if !path.is_empty() {
                    return Err(VdfsError::not_found(format!(
                        "{LABEL}根下没有子目录：{path}"
                    )));
                }
                // 智能体记忆恒在（装配作用域）；另两腿有作用域才列
                let mut items = vec![agent_memory_node(&agent).await];
                if ws.has_scope() {
                    items.push(workspace_memory_node(&ws).await);
                }
                if sess.has_scope() {
                    items.push(session_memory_node(&sess).await);
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
                MEMORY_FILE => Ok(VdfsResponse::Stat(agent_memory_node(&agent).await)),
                WORKSPACE_FILE => {
                    if !ws.has_scope() {
                        return Err(VdfsError::not_found(format!(
                            "{WORKSPACE_LABEL}不可用：当前没有工作区"
                        )));
                    }
                    Ok(VdfsResponse::Stat(workspace_memory_node(&ws).await))
                }
                SESSION_FILE => {
                    if !sess.has_scope() {
                        return Err(VdfsError::not_found(format!(
                            "{SESSION_LABEL}不可用：当前没有会话作用域"
                        )));
                    }
                    Ok(VdfsResponse::Stat(session_memory_node(&sess).await))
                }
                // 配置文档按真实文件名可达（列表里不并列，见模块文档）
                PLUGIN_FILE => Ok(VdfsResponse::Stat(self.config_file().node())),
                other => Err(VdfsError::not_found(format!("未知路径：{other}"))),
            },
            VdfsRequest::Read => match path {
                MEMORY_FILE => {
                    let text = agent.read().await.map_err(VdfsError::internal)?;
                    Ok(VdfsResponse::Read(VdfsContent::text(text)))
                }
                WORKSPACE_FILE => {
                    let text = ws.read().await.map_err(VdfsError::internal)?;
                    Ok(VdfsResponse::Read(VdfsContent::text(text)))
                }
                SESSION_FILE => {
                    let text = sess.read().await.map_err(VdfsError::internal)?;
                    Ok(VdfsResponse::Read(VdfsContent::text(text)))
                }
                // 配置文档（设置页只存三层开关与闸门；记忆正文经各自的 MEMORY.md 读写）
                PLUGIN_FILE => {
                    let content = self.config_file().read(self.config_slot()).await?;
                    Ok(VdfsResponse::Read(content))
                }
                other => Err(VdfsError::not_found(format!("未知路径：{other}"))),
            },
            VdfsRequest::Write { content } => match path {
                MEMORY_FILE => {
                    if content.binary {
                        return Err(VdfsError::invalid("智能体记忆是文本文件，不接受二进制内容"));
                    }
                    let text = content.text.as_deref().unwrap_or_default();
                    let existed = agent.exists().await;
                    // 容量闸门在共享实现里（`MemoryFile::write`）——本插件不重复实现
                    agent.write(text).await.map_err(VdfsError::invalid)?;
                    vdfs_notify_change(PLUGIN_ID_MEMORY, MEMORY_FILE);
                    Ok(VdfsResponse::Write(VdfsWriteResponse {
                        name: None,
                        created: !existed,
                    }))
                }
                WORKSPACE_FILE => {
                    if content.binary {
                        return Err(VdfsError::invalid("工作区记忆是文本文件，不接受二进制内容"));
                    }
                    let text = content.text.as_deref().unwrap_or_default();
                    let existed = ws.exists().await;
                    // 容量闸门在共享实现里（`MemoryFile::write`）——本插件不重复实现
                    ws.write(text).await.map_err(VdfsError::invalid)?;
                    vdfs_notify_change(PLUGIN_ID_MEMORY, WORKSPACE_FILE);
                    Ok(VdfsResponse::Write(VdfsWriteResponse {
                        name: None,
                        created: !existed,
                    }))
                }
                SESSION_FILE => {
                    if content.binary {
                        return Err(VdfsError::invalid("会话记忆是文本文件，不接受二进制内容"));
                    }
                    let text = content.text.as_deref().unwrap_or_default();
                    let existed = sess.exists().await;
                    // 容量闸门在共享实现里（`MemoryFile::write`）——本插件不重复实现
                    sess.write(text).await.map_err(VdfsError::invalid)?;
                    vdfs_notify_change(PLUGIN_ID_MEMORY, SESSION_FILE);
                    Ok(VdfsResponse::Write(VdfsWriteResponse {
                        name: None,
                        created: !existed,
                    }))
                }
                // 配置文档（只存三层开关与闸门；记忆正文经各自的 MEMORY.md 读写）
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
                     如需清空，请向 `{MEMORY_FILE}` / `{WORKSPACE_FILE}` / `{SESSION_FILE}` 写入空内容。"
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
