//! VDFS 挂载点（`<根>/memory`）—— 本插件**直接实现 `VdfsProvider`**。
//!
//! ## 挂载根有三份记忆
//!
//! ```text
//! <根>/memory/AGENTS.md      智能体自身的记忆（rw；物理文件 = {agent_dir}/AGENTS.md）
//! <根>/memory/WORKSPACE.md   工作区记忆（rw；物理文件 = {work_dir}/AGENTS.md）
//! <根>/memory/SESSION.md     会话记忆（rw；物理文件 = {session_dir}/MEMORY.md）
//! ```
//!
//! 三者的作用域分别来自装配位置、`ctx[WORKDIR]` 与 `ctx[SESSION_DIR]`（D-513）。
//! 智能体与工作区的物理文件同名（同一族行业约定），会话那份刻意叫 `MEMORY.md`：
//! 会话目录不是 agent 目录。挂载名则各自点明作用域，列表里不会撞名。
//! 智能体 / 工作区两份地址不随会话变；会话那份跟着当前会话的作用域走。
//!
//! 根下**只列记忆文件**：插件配置文档（`PLUGIN.yml`）不并列——它与其它插件同一
//! 口径，进设置走 `CONFIGURABLE_VISITOR` 那条通道；但按**真实文件名**仍然可达
//! （`stat` / `read` / `write` 都认 `PLUGIN.yml`），隐藏的是「列表里的位置」，
//! 不是可达性。
//!
//! ## 挂载点标了 `hidden`：入口在设置页
//!
//! 记忆没有「列表内容」：用户要改的是智能体记忆正文与闸门，不是浏览一个资源集合。
//! 因此本挂载点（[`MemoryPlugin::metadata`]）声明 `hidden`，不进左侧主导航；
//! 入口是设置页（`plugin_manager` 的 memory 条目）。地址本身照常可用——模型与
//! 深链仍走 `<根>/memory/AGENTS.md`。
//!
//! 设置页编辑的是 `PLUGIN.yml` 这份配置文档，但**智能体记忆的正文不在配置里**：
//! `read` / `write` 在配置模型上并入 / 拆出一个投影字段 [`AGENT_MEMORY_FIELD`]，
//! 正文的唯一落盘处仍是 `AGENTS.md`。工作区记忆不投影——它随工作区路径变，
//! 没有「一处设置」可言。
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
use symbio_providers::MemoryFile;

const LABEL: &str = PLUGIN_TITLE;

/// 设置页表单里承载**智能体记忆正文**的投影字段。
///
/// 它不是 `MemoryConfig` 的字段（配置文档只存闸门），读写 `PLUGIN.yml` 时在此
/// 并入 / 拆出：正文的唯一落盘处是 `AGENTS.md`，这个键只是设置页的视图。
pub(crate) const AGENT_MEMORY_FIELD: &str = "agent_memory";

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
    /// 智能体记忆门面：作用域来自请求 ctx 的 `AGENT_VDFS` provider（D-515）。
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
                // 配置文档的**设置页视图** = 配置模型 + 智能体记忆正文。正文从
                // `AGENTS.md` 现读，不写进配置（那份文件只存闸门）。
                PLUGIN_FILE => {
                    let content = self.config_file().read(self.config_slot()).await?;
                    let text = content.text.as_deref().unwrap_or("{}");
                    let mut model: serde_json::Value = serde_json::from_str(text)
                        .map_err(|e| VdfsError::internal(format!("配置序列化结果不可解析：{e}")))?;
                    if let Some(obj) = model.as_object_mut() {
                        let memory = agent.read().await.map_err(VdfsError::internal)?;
                        obj.insert(
                            AGENT_MEMORY_FIELD.to_string(),
                            serde_json::Value::String(memory),
                        );
                    }
                    let text = serde_json::to_string_pretty(&model)
                        .map_err(|e| VdfsError::internal(format!("设置视图序列化失败：{e}")))?;
                    Ok(VdfsResponse::Read(
                        VdfsContent::text(text).with_mime("application/json"),
                    ))
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
                // 设置页视图的写：正文与闸门**各回各的落盘处**。顺序上先校验配置、
                // 再写正文（容量闸门在 `MemoryFile::write` 里）、最后才落配置——
                // 正文超限时这份配置一个字节都不该动。
                PLUGIN_FILE => {
                    if content.binary {
                        return Err(VdfsError::invalid(
                            "记忆设置是文本（JSON），不接受二进制内容",
                        ));
                    }
                    let text = content.text.as_deref().unwrap_or_default();
                    let mut value: serde_json::Value = serde_json::from_str(text)
                        .map_err(|e| VdfsError::invalid(format!("配置不是合法 JSON：{e}")))?;
                    let Some(obj) = value.as_object_mut() else {
                        return Err(VdfsError::invalid("配置必须是 JSON 对象"));
                    };
                    // 没带这个键 = 「这次不改正文」（例如只调闸门的旧调用方）
                    let memory_text = obj.remove(AGENT_MEMORY_FIELD);
                    let config = VdfsContent::text(
                        serde_json::to_string_pretty(&value)
                            .map_err(|e| VdfsError::internal(format!("配置序列化失败：{e}")))?,
                    )
                    .with_mime("application/json");
                    // 先按拥有者的定义校验配置，再动正文
                    self.config_file().decode(&config)?;
                    if let Some(v) = memory_text {
                        let Some(text) = v.as_str() else {
                            return Err(VdfsError::invalid("智能体记忆必须是字符串"));
                        };
                        agent.write(text).await.map_err(VdfsError::invalid)?;
                        vdfs_notify_change(PLUGIN_ID_MEMORY, MEMORY_FILE);
                    }
                    Ok(VdfsResponse::Write(
                        self.config_file()
                            .apply(self.config_slot(), &config)
                            .await?,
                    ))
                }
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
