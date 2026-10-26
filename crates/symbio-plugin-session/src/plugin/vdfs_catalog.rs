//! 会话地址面的**目录域**：根清单、单个会话条目、记忆、子会话、工作目录。
//!
//! 从 `vdfs_provider` 分出来的判据是「这一段错了谁会红」：本文件出错，红的是
//! **目录与清单那几面**（前端列不出会话、子会话下不去、工作目录读不到、
//! 记忆格空了），而不是转写内容，也不是路由本身。域内怎么读写由这里自己决定，
//! 「路径 → 域」的映射只在 `vdfs_provider` 那一处。
//!
//! 与 `vdfs_provider` 的分工是形状上的：那边只回答「这个地址归哪个域」，
//! 这边回答「这个域对这个请求给出什么值」——值一律转发既有会话能力，不新造协议。

use super::*;
use symbio_core::{
    vdfs_from_plugin_error, VdfsAccess, VdfsActionResult, VdfsChange, VdfsContent, VdfsContext,
    VdfsError, VdfsNewType, VdfsNode, VdfsRequest, VdfsResponse, VdfsResult, VdfsWriteResponse,
    VDFS_ACTION_ABORT,
};

impl SessionPlugin {
    /// 挂载根：会话清单 / 根节点 / 新建（名字由 provider 生成）。
    pub(super) async fn root_at(
        &self,
        ctx: &VdfsContext,
        path: &str,
        req: VdfsRequest,
    ) -> VdfsResult<VdfsResponse> {
        match req {
            VdfsRequest::List { .. } => {
                let (limit, before) = window_params(ctx);
                // 清单里**只有会话**——配置文件是文档，它进设置菜单走的是
                // CONFIGURABLE_VISITOR 那条通道，不该在会话清单里再出现一次
                // （否则列表底部会多一个「设置」项）。
                let sessions = self
                    .list_sessions_window(limit, before)
                    .await
                    .map_err(vdfs_from_plugin_error)?;
                // 选项定义与「是哪个会话」无关 ⇒ 一次算好，清单里逐项复用
                let schema = self.session_schema().await;
                Ok(VdfsResponse::list(
                    self.nodes_of_sessions(&sessions, &schema).await,
                ))
            }
            VdfsRequest::Stat => {
                // 自身根：**名字留空**——provider 不知道自己的挂载名，由使用方回填。
                //
                // 根的自述里带着「可新建类型」：根下可新建「会话」，类型定义带**选项
                // schema**（运行期汇流，见 [`Self::session_schema`]）。它是本插件自述中
                // 唯一动态的部分，因此随**根节点自述**一起给出（容器合成挂载点节点时
                // 向本 provider 发一次 `Stat("")` 取走 `new_type`），而不是 trait 上的
                // 另一个方法——见 D-030。
                //
                // ⚠️ 代价要认：`session_schema()` 会做一次全项目广播（options 收集，
                // 含 agent 目录扫描），因此**列 / stat 会话挂载根**比 stat 某个会话贵。
                // 这与从前相同（容器合成根节点时就要取这份自述），只是取法统一了。
                Ok(VdfsResponse::Stat(
                    VdfsNode::dir("", "会话", VdfsAccess::LIST).with_new_type(Some(
                        VdfsNewType::new(EXT_SESSION, "会话")
                            .with_description("新建会话")
                            .with_schema_opt(self.session_schema().await),
                    )),
                ))
            }
            VdfsRequest::Read => Err(VdfsError::invalid(format!("该路径不可读取内容：{path}"))),
            VdfsRequest::Write { content } => {
                // 挂载根 = 「新建一个会话，名字由 provider 生成」。
                //
                // 写目录自身没有可覆盖的目标，因此 `create` 是唯一合法意图
                // （见 `VdfsRequest::Write` 的两种目标形态）；缺它即报错，
                // 不静默落成「一次无意义的写」。
                if !content.create {
                    return Err(VdfsError::invalid(
                        "写会话挂载根需要 create 意图：目录自身没有可覆盖的目标",
                    ));
                }
                self.session_upsert(path, &content).await
            }
            VdfsRequest::Delete { .. } => {
                Err(VdfsError::Forbidden("会话挂载根不可删除".to_string()))
            }
            VdfsRequest::Watch { sink } => {
                self.watch_at_path(path, sink).await?;
                Ok(VdfsResponse::Unit)
            }
            VdfsRequest::Unwatch => {
                self.unwatch_at_path(path).await?;
                Ok(VdfsResponse::Unit)
            }
            _ => Err(VdfsError::NotImplemented),
        }
    }

    /// 会话本体（`<id>`）：清单里是**叶子**，被当目录访问时是**会话内部**视图。
    pub(super) async fn session_at(
        &self,
        path: &str,
        id: &str,
        req: VdfsRequest,
    ) -> VdfsResult<VdfsResponse> {
        match req {
            VdfsRequest::List { .. } => {
                // 会话内部：三个虚拟子目录 + 记忆文件（会话存在性校验由 `session_of` 承担）
                let session = self.session_of(id).await?;
                Ok(VdfsResponse::list(internal_dirs(
                    super::super::workdir::workdir_of(&session).is_some(),
                    self.memory_node_of(id).await,
                )))
            }
            VdfsRequest::Stat => {
                let session = self.session_of(id).await?;
                // 呈现与清单**同源**（`session_node`）：status / ext / 更新时间 /
                // 摘要都随节点给出。缺了 `status`，运行态在这条链路上永远读不到
                // ——`stat` 正是角标的取值点（变更通知 → 重读 `stat` →
                // `status == working` ⇒ 运行中），缺它会把本地乐观置的工作态
                // 在下一次变更时立刻改回空闲。
                // 只有访问位按**目录视图**回答（只给 `l`）：stat 的结果被分发层
                // 当作「当前目录节点」，其访问位决定是否给出新建入口；
                // 会话内部不支持新建 / 建目录。
                let mut n = session_node(&SessionSummary::of(&session), &self.runtime_of(id).await);
                n.access = VdfsAccess::LIST;
                Ok(VdfsResponse::Stat(n))
            }
            VdfsRequest::Read => {
                let session = self.session_of(id).await?;
                // 含在途：叶子与转写列表必须是同一份消息集合，否则前端
                // `loadMessages`（读叶子）会在流式期间看不到正在跑的那一轮。
                // 详见 `session_content` 的文档。
                let live = self.live_messages_of(id).await;
                let mode = self.presentation_mode().await;
                Ok(VdfsResponse::Read(session_content(
                    &session, live, id, mode,
                )?))
            }
            // 会话本身（`path` 即会话 id —— 具名新建时它就是**身份**，见
            // `session_upsert` 的 `create` 分支；`id` 由地址末段给出，不再由
            // provider 另生成一个）
            VdfsRequest::Write { content } => self.session_upsert(path, &content).await,
            VdfsRequest::Delete { .. } => {
                // 存在性校验：删除不存在的会话应报 NotFound 而非静默成功
                self.session_of(id).await?;
                self.delete_session_internal(id)
                    .await
                    .map_err(vdfs_from_plugin_error)?;
                Ok(VdfsResponse::Unit)
            }
            VdfsRequest::Watch { sink } => {
                self.watch_at_path(path, sink).await?;
                Ok(VdfsResponse::Unit)
            }
            VdfsRequest::Unwatch => {
                self.unwatch_at_path(path).await?;
                Ok(VdfsResponse::Unit)
            }
            // 节点动作：**中止**正在跑的那一轮。
            //
            // 落点是**会话节点自身**（`<sid>`），而不是 `<sid>/inbox`：在途那一轮
            // 早已出队，它现在是转写里的消息；队列上的动作只该管「还没被消费的」
            // （取消 = 删条目，清空 = `clear`）。两个动词作用在**两个不同的对象**上
            // ，因此是两个地址——与 D-026 已确立的分界同一条。
            VdfsRequest::Action { action, .. } => match action.as_str() {
                VDFS_ACTION_ABORT => {
                    // 存在性校验：会话不在就没有「它的一轮」可谈
                    self.session_of(id).await?;
                    let stopped = self.turn.abort_turn(id).await;
                    Ok(VdfsResponse::Action(VdfsActionResult {
                        action: action.clone(),
                        ok: stopped,
                        // 「没在跑」不是错误，但也不报成功——报成功会让调用方以为
                        // 停下来了（这正是 `chat/abort` 最坏的一面）。
                        message: if stopped {
                            "已中止当前轮次".to_string()
                        } else {
                            format!("该会话没有正在进行的轮次，无需中止：{path}")
                        },
                        data: Some(json!({ "session_id": id })),
                    }))
                }
                _ => Err(VdfsError::NotImplemented),
            },
            _ => Err(VdfsError::NotImplemented),
        }
    }

    /// 会话记忆（`<id>/MEMORY.md`）：单个文件，形状由共享实现产出。
    pub(super) async fn memory_at(
        &self,
        path: &str,
        id: &str,
        req: VdfsRequest,
    ) -> VdfsResult<VdfsResponse> {
        match req {
            // 记忆是**单个文件**：没有子项
            VdfsRequest::List { .. } => Err(VdfsError::not_found(format!(
                "记忆是文件，没有子项：{path}"
            ))),
            VdfsRequest::Stat => {
                // 存在性校验：会话不在，就没有「它的记忆」可谈
                self.session_of(id).await?;
                Ok(VdfsResponse::Stat(self.memory_node_of(id).await))
            }
            VdfsRequest::Read => {
                // 会话记忆（`<根>/session/<id>/MEMORY.md`）：正文即文件全文。
                // 文件不存在 → 空串（不是错误）——「还没写过」是记忆的正常状态。
                self.session_of(id).await?;
                let text = self
                    .memory_store(id)
                    .await
                    .read()
                    .map_err(VdfsError::internal)?;
                Ok(VdfsResponse::Read(VdfsContent::text(text)))
            }
            VdfsRequest::Write { content } => {
                // 会话记忆：**纯文本**写入（容量闸门在共享实现 `MemoryFile::write`，本插件不重复实现）
                if content.binary {
                    return Err(VdfsError::invalid("会话记忆是文本文件，不接受二进制内容"));
                }
                // 存在性校验：会话不在，就没有「它的记忆」可写
                self.session_of(id).await?;
                let store = self.memory_store(id).await;
                let existed = store.exists();
                let text = content.text.as_deref().unwrap_or_default();
                store.write(text).map_err(VdfsError::invalid)?;
                // 变更路径与 `list` 返回的节点地址同源（provider 子树口径，
                // 挂载名由容器的 watch 包装补上——见 `watch_at_path` 的说明）
                self.change_subs
                    .notify(&VdfsChange::bare(super::super::memory::memory_rel_path(id)));
                Ok(VdfsResponse::Write(VdfsWriteResponse {
                    name: None,
                    created: !existed,
                }))
            }
            VdfsRequest::Delete { .. } => {
                // 记忆**不可删除**（与 work / agent 的记忆层同一共享实现约定）：
                // 删除即丢失本会话的长期约定，且没有东西能把它找回来。要清空就写入空内容
                // ——那是一次可读、可审、可撤销的显式动作。
                Err(VdfsError::Forbidden(format!(
                    "会话记忆不可删除（删除即丢失本会话的长期约定）。\
                     如需清空，请向 `{}` 写入空内容。",
                    crate::memory::SESSION_MEMORY_FILE
                )))
            }
            VdfsRequest::Watch { sink } => {
                self.watch_at_path(path, sink).await?;
                Ok(VdfsResponse::Unit)
            }
            VdfsRequest::Unwatch => {
                self.unwatch_at_path(path).await?;
                Ok(VdfsResponse::Unit)
            }
            _ => Err(VdfsError::NotImplemented),
        }
    }

    /// 子会话清单目录（`<id>/sub`）。
    pub(super) async fn sub_sessions_at(
        &self,
        path: &str,
        id: &str,
        req: VdfsRequest,
    ) -> VdfsResult<VdfsResponse> {
        match req {
            VdfsRequest::List { .. } => {
                self.session_of(id).await?;
                let store = self.get_store().await.map_err(vdfs_from_plugin_error)?;
                let subs = store
                    .list_sub_sessions(id)
                    .await
                    .map_err(vdfs_from_plugin_error)?;
                // 子会话也是会话：同一份选项定义
                let schema = self.session_schema().await;
                Ok(VdfsResponse::list(
                    self.nodes_of_sessions(&subs, &schema).await,
                ))
            }
            VdfsRequest::Stat => {
                self.session_of(id).await?;
                Ok(VdfsResponse::Stat(
                    super::super::workdir::sub_sessions_dir_node(),
                ))
            }
            VdfsRequest::Read => Err(VdfsError::invalid(format!("该路径不可读取内容：{path}"))),
            VdfsRequest::Write { .. } => Err(VdfsError::invalid(format!("该路径不可写入：{path}"))),
            VdfsRequest::Delete { .. } => {
                Err(VdfsError::Forbidden(format!("该路径不可删除：{path}")))
            }
            VdfsRequest::Watch { sink } => {
                self.watch_at_path(path, sink).await?;
                Ok(VdfsResponse::Unit)
            }
            VdfsRequest::Unwatch => {
                self.unwatch_at_path(path).await?;
                Ok(VdfsResponse::Unit)
            }
            _ => Err(VdfsError::NotImplemented),
        }
    }

    /// 子会话本体（`<id>/sub/<sub>`）：独立会话，有自己的 id 与活跃状态。
    pub(super) async fn sub_session_at(
        &self,
        path: &str,
        id: &str,
        sub: &str,
        req: VdfsRequest,
    ) -> VdfsResult<VdfsResponse> {
        match req {
            VdfsRequest::List { .. } => Err(VdfsError::not_found(format!(
                "子会话是叶子节点，没有子项：{path}"
            ))),
            VdfsRequest::Stat => {
                let session = self.sub_session_of(id, sub).await?;
                Ok(VdfsResponse::Stat(session_node(
                    &SessionSummary::of(&session),
                    &self.runtime_of(sub).await,
                )))
            }
            VdfsRequest::Read => {
                let session = self.sub_session_of(id, sub).await?;
                // 子会话是**独立会话**（有自己的 id 与活跃状态），因此叠加的是它
                // 自己的在途缓冲，而不是父会话的——父会话的在途消息不属于它。
                let live = self.live_messages_of(&session.id).await;
                let mode = self.presentation_mode().await;
                Ok(VdfsResponse::Read(session_content(
                    &session, live, id, mode,
                )?))
            }
            VdfsRequest::Write { .. } => Err(VdfsError::invalid(format!("该路径不可写入：{path}"))),
            VdfsRequest::Delete { .. } => {
                // 子会话：先 abort 活跃任务再删（`delete_session_internal` 同语义）
                self.sub_session_of(id, sub).await?;
                self.delete_session_internal(sub)
                    .await
                    .map_err(vdfs_from_plugin_error)?;
                Ok(VdfsResponse::Unit)
            }
            VdfsRequest::Watch { sink } => {
                self.watch_at_path(path, sink).await?;
                Ok(VdfsResponse::Unit)
            }
            VdfsRequest::Unwatch => {
                self.unwatch_at_path(path).await?;
                Ok(VdfsResponse::Unit)
            }
            _ => Err(VdfsError::NotImplemented),
        }
    }

    /// 会话工作目录（`<id>/workdir[/rel]`）：目录树与文件读写。
    pub(super) async fn workdir_at(
        &self,
        path: &str,
        id: &str,
        rel: &str,
        req: VdfsRequest,
    ) -> VdfsResult<VdfsResponse> {
        match req {
            VdfsRequest::List { .. } => {
                let workdir = self.workdir_of(id).await?;
                // 目录树场景的实时监听（按会话 id 引用计数）
                self.workdir_watches.ensure_watch(&workdir, id);
                super::super::workdir::list_children(&workdir, Some(rel))
                    .await
                    .map_err(vdfs_from_plugin_error)
                    .map(VdfsResponse::list)
            }
            VdfsRequest::Stat => {
                let workdir = self.workdir_of(id).await?;
                if rel.is_empty() {
                    return Ok(VdfsResponse::Stat(super::super::workdir::workdir_dir_node()));
                }
                super::super::workdir::read_node(&workdir, rel)
                    .await
                    .map_err(vdfs_from_plugin_error)
                    .map(VdfsResponse::Stat)
            }
            VdfsRequest::Read => {
                if rel.is_empty() {
                    return Err(VdfsError::invalid(format!("该路径不可读取内容：{path}")));
                }
                let workdir = self.workdir_of(id).await?;
                let text = super::super::workdir::read_content(&workdir, rel)
                    .await
                    .map_err(vdfs_from_plugin_error)?;
                Ok(VdfsResponse::Read(VdfsContent::text(text)))
            }
            VdfsRequest::Write { content } => {
                if rel.is_empty() {
                    return Err(VdfsError::invalid(format!("该路径不可写入：{path}")));
                }
                // 工作目录分支：文件写回（与列读同一份实现——路径越界校验 + 落盘）
                let workdir = self.workdir_of(id).await?;
                let text = content.text.as_deref().unwrap_or("");
                super::super::workdir::write_node(&workdir, rel, text)
                    .await
                    .map_err(vdfs_from_plugin_error)?;
                self.workdir_watches.ensure_watch(&workdir, id);
                Ok(VdfsResponse::Write(VdfsWriteResponse {
                    name: None,
                    created: false,
                }))
            }
            VdfsRequest::Delete { .. } => {
                if rel.is_empty() {
                    return Err(VdfsError::Forbidden(format!("该路径不可删除：{path}")));
                }
                let workdir = self.workdir_of(id).await?;
                super::super::workdir::delete_node(&workdir, rel)
                    .await
                    .map_err(vdfs_from_plugin_error)?;
                Ok(VdfsResponse::Unit)
            }
            VdfsRequest::Watch { sink } => {
                // 工作目录子树：接入文件系统监听（文件变化经**同一张订阅表**到达本
                // sink，见 `SessionPlugin::new` 注入的 `set_vdfs_subs`）
                if let Ok(workdir) = self.workdir_of(id).await {
                    self.workdir_watches.ensure_watch(&workdir, id);
                }
                self.change_subs.watch(path, sink);
                Ok(VdfsResponse::Unit)
            }
            VdfsRequest::Unwatch => {
                // 与 `watch` 严格配对：引用计数归零才真正摘掉
                if let Ok(workdir) = self.workdir_of(id).await {
                    self.workdir_watches.release_watch(&workdir, id);
                }
                self.change_subs.unwatch(path);
                Ok(VdfsResponse::Unit)
            }
            _ => Err(VdfsError::NotImplemented),
        }
    }
}
