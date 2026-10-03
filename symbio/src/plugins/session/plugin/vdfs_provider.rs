//! `impl VdfsProvider for SessionPlugin` 及其**私有辅助**。
//!
//! 职责：
//! - [`crate::symbio_core::VdfsProvider::dispatch`] 是唯一入口：**先按 `path` 定位资源域，再按
//!   `req` 执行操作**——`parse_session_path` 的每个域各有一个 `*_at` 私有方法，
//!   dispatch 只做路由，域内怎么读写是各方法自己的事；
//! - 读侧：域方法**转发既有会话能力**（SessionStore + 会话 metadata 合并），不新造协议
//!   ——转写（含在途消息）、存在性校验、实时工作状态、工作目录、子会话；
//! - 写侧：**本文件不实现任何写语义**——它只把 `Write` / `Delete` / `Action` 翻译成
//!   对 [`commands`](super::commands) 域的调用（消息改写 / 截断 / 清空、会话新建 / 覆盖 /
//!   删除），再把变更投进订阅表。「谁能改会话」的答案在 `commands` 一处。

use super::*;
use crate::symbio_core::{
    vdfs_from_plugin_error, vdfs_host_ctx, VdfsAccess, VdfsActionResult, VdfsChange,
    VdfsChangeSink, VdfsContent, VdfsContext, VdfsError, VdfsNewType, VdfsNode, VdfsProvider,
    VdfsRequest, VdfsResponse, VdfsResult, VdfsWriteResponse, VDFS_ACTION_ABORT, VDFS_ACTION_CLEAR,
    VDFS_ACTION_TRUNCATE,
};

#[async_trait]
impl VdfsProvider for SessionPlugin {
    /// 唯一入口：**先按 `path` 定位资源域，再按 `req` 执行操作**。
    ///
    /// 配置文件（`PLUGIN.yml`）是文档不是会话——先判路径再分流操作；其余全部经
    /// [`parse_session_path`] 定域，各域逻辑收敛在下方 `*_at` 方法里。
    async fn dispatch(
        &self,
        ctx: &VdfsContext,
        path: &str,
        req: VdfsRequest,
    ) -> VdfsResult<VdfsResponse> {
        // 配置文件优先：`PLUGIN.yml` 是文档，不是会话
        if path == PLUGIN_FILE {
            return match req {
                VdfsRequest::Stat => Ok(VdfsResponse::Stat(self.config_file.node())),
                VdfsRequest::Read => Ok(VdfsResponse::Read(
                    self.config_file.read(&self.config).await?,
                )),
                VdfsRequest::Write { content } => Ok(VdfsResponse::Write(
                    self.config_file.apply(&self.config, &content).await?,
                )),
                VdfsRequest::List { .. } => Err(VdfsError::not_found(format!(
                    "配置文件是文档，没有子项：{path}"
                ))),
                VdfsRequest::Delete { .. } => {
                    Err(VdfsError::Forbidden("配置文件不可删除".to_string()))
                }
                _ => Err(VdfsError::invalid(format!("该路径不可操作：{path}"))),
            };
        }
        match parse_session_path(path)? {
            VdfsSessionPath::Root => self.root_at(ctx, path, req).await,
            VdfsSessionPath::Session(id) => self.session_at(path, id, req).await,
            VdfsSessionPath::Memory(id) => self.memory_at(path, id, req).await,
            VdfsSessionPath::Messages { id, mid } => {
                self.messages_at(ctx, path, id, mid, req).await
            }
            VdfsSessionPath::Inbox { id, iid } => self.inbox_at(ctx, path, id, iid, req).await,
            VdfsSessionPath::SubSessions(id) => self.sub_sessions_at(path, id, req).await,
            VdfsSessionPath::SubSession { id, sub } => {
                self.sub_session_at(path, id, sub, req).await
            }
            VdfsSessionPath::Workdir { id, rel } => self.workdir_at(path, id, rel, req).await,
        }
    }
}

impl SessionPlugin {
    // ==================== 按域实现（dispatch 只做路由，见上方） ====================

    /// 挂载根：会话清单 / 根节点 / 新建（名字由 provider 生成）。
    async fn root_at(
        &self,
        ctx: &VdfsContext,
        path: &str,
        req: VdfsRequest,
    ) -> VdfsResult<VdfsResponse> {
        match req {
            VdfsRequest::List { .. } => {
                let (limit, before) = window_params(ctx);
                // 清单里**只有会话**——配置文件是文档，它进设置菜单走的是
                // ConfigurableVisitor 那条通道，不该在会话清单里再出现一次
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
                // 另一个方法——见 ADR-030。
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
    async fn session_at(&self, path: &str, id: &str, req: VdfsRequest) -> VdfsResult<VdfsResponse> {
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
                let mut n = session_node(
                    &SessionSummary::of(&session),
                    &self.session_runtime(id).await,
                );
                n.access = VdfsAccess::LIST;
                Ok(VdfsResponse::Stat(n))
            }
            VdfsRequest::Read => {
                let session = self.session_of(id).await?;
                // 含在途：叶子与转写列表必须是同一份消息集合，否则前端
                // `loadMessages`（读叶子）会在流式期间看不到正在跑的那一轮。
                // 详见 `session_content` 的文档。
                let live = self.live_messages_of(id).await;
                Ok(VdfsResponse::Read(session_content(&session, live)?))
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
            // ，因此是两个地址——与 ADR-026 已确立的分界同一条。
            VdfsRequest::Action { action, .. } => match action.as_str() {
                VDFS_ACTION_ABORT => {
                    // 存在性校验：会话不在就没有「它的一轮」可谈
                    self.session_of(id).await?;
                    let stopped = self.abort_turn(id).await;
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
    async fn memory_at(&self, path: &str, id: &str, req: VdfsRequest) -> VdfsResult<VdfsResponse> {
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
                    etag: None,
                }))
            }
            VdfsRequest::Delete { .. } => {
                // 记忆**不可删除**（与 work / agent 的记忆层同一共享实现约定）：
                // 删除即丢失本会话的长期约定，且没有东西能把它找回来。要清空就写入空内容
                // ——那是一次可读、可审、可撤销的显式动作。
                Err(VdfsError::Forbidden(format!(
                    "会话记忆不可删除（删除即丢失本会话的长期约定）。\
                     如需清空，请向 `{}` 写入空内容。",
                    crate::plugins::session::memory::SESSION_MEMORY_FILE
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

    /// 转写区段（`<id>/message[/mid]`）：列表与单条消息。
    async fn messages_at(
        &self,
        ctx: &VdfsContext,
        path: &str,
        id: &str,
        mid: Option<&str>,
        req: VdfsRequest,
    ) -> VdfsResult<VdfsResponse> {
        match req {
            VdfsRequest::List { .. } => match mid {
                // 转写列表：每一条消息是一个列表项，顺序由 `seq` 决定。
                // 含**在途**消息——流式期间列表就是活的，不必等落库。
                None => {
                    let msgs = self.transcript_of(id).await?;
                    let (limit, before) = window_params(ctx);
                    // 有界窗口：**只在调用方显式给参数时**生效。不给参数 = 全量，
                    // 与从前逐字节一致（流式期间前端要的是完整列表）。
                    Ok(VdfsResponse::list(
                        transcript_window(&msgs, limit, before)
                            .iter()
                            .map(message_node)
                            .collect::<Vec<VdfsNode>>(),
                    ))
                }
                Some(_) => Err(VdfsError::not_found(format!(
                    "消息是列表项，没有子项：{path}"
                ))),
            },
            VdfsRequest::Stat => match mid {
                // 列表本身是个目录（`l` 位：可列，不参与树遍历——会话整体是叶子）。
                // 只需存在性校验，不必取全量转写。
                None => {
                    self.session_of(id).await?;
                    Ok(VdfsResponse::Stat(messages_dir_node()))
                }
                Some(mid) => Ok(VdfsResponse::Stat(message_node(message_of(
                    &self.transcript_of(id).await?,
                    mid,
                )?))),
            },
            VdfsRequest::Read => match mid {
                // 单条消息的正文（列表项内容）——流式追加的正是它。
                // 同样取含在途的转写：追加型变更的消费者读到的必须是**已含该增量**的正文。
                Some(mid) => {
                    let msgs = self.transcript_of(id).await?;
                    Ok(VdfsResponse::Read(VdfsContent::text(message_text(
                        message_of(&msgs, mid)?,
                    ))))
                }
                None => Err(VdfsError::invalid(format!("该路径不可读取内容：{path}"))),
            },
            VdfsRequest::Write { content } => match mid {
                Some(mid) => {
                    if content.create {
                        return Err(VdfsError::invalid(
                            "消息不支持 create 意图：新增消息即发言，请走聊天协议\
                             （一次发言触发一整轮编排）",
                        ));
                    }
                    if content.binary {
                        return Err(VdfsError::invalid(
                            "消息是文本（JSON 字段补丁），不接受二进制内容",
                        ));
                    }
                    // 补丁缺 `id` 时由**地址**补上：地址是消息身份的唯一权威，
                    // 因此「不给 id」（`{"content":"..."}`）是合法用法，而「给了别的
                    // id」是错误（由 `patch_message` 报出）。
                    //
                    // 必须在这里补而不是靠 `ChatMessage` 的 serde 默认值：`id` 在
                    // 该结构里是必填字段（它也是存储层的主键），反序列化会**先一步**
                    // 拒绝掉不带 id 的补丁——那样「补丁是字段子集、未提供的保持不变」
                    // 这条承诺在 `id` 上就是假的，调用方被迫把地址里已有的信息
                    // 再抄一遍。
                    let raw = content.text.as_deref().unwrap_or("").trim();
                    let mut value: Value = serde_json::from_str(raw).map_err(|e| {
                        VdfsError::invalid(format!(
                            "消息补丁需要合法 JSON（ChatMessage 字段子集）：{e}"
                        ))
                    })?;
                    let obj = value.as_object_mut().ok_or_else(|| {
                        VdfsError::invalid("消息补丁需要 JSON 对象（ChatMessage 字段子集）")
                    })?;
                    if !obj.contains_key("id") {
                        obj.insert("id".to_string(), Value::String(mid.to_string()));
                    }
                    let patch: cm::ChatMessage = serde_json::from_value(value).map_err(|e| {
                        VdfsError::invalid(format!(
                            "消息补丁需要合法 JSON（ChatMessage 字段子集）：{e}"
                        ))
                    })?;
                    self.patch_message(id, mid, &patch)
                        .await
                        .map_err(vdfs_from_plugin_error)?;
                    Ok(VdfsResponse::Write(VdfsWriteResponse {
                        name: None,
                        created: false,
                        etag: None,
                    }))
                }
                // 转写列表本身：**仍只读**。往里放一条 = 发言 = 动作，不是写入。
                None => Err(VdfsError::Forbidden(
                    "转写列表只读：发言请走聊天协议（一次发言触发一整轮编排）".to_string(),
                )),
            },
            VdfsRequest::Delete { .. } => {
                // 转写区段（列表与单条）**不可 delete**：`delete` 的全局语义是
                // 「**这一个**节点没了」，而「从这条删到末尾」是一种**区间**语义，
                // 有它自己的动作——[`crate::symbio_core::VDFS_ACTION_TRUNCATE`]（落在起始消息上）。
                // 动作承载这类集合操作——理由见 `VDFS_ACTION_TRUNCATE` 的文档。
                Err(VdfsError::Forbidden(format!(
                    "转写区段不可 delete：删除某条及其之后请用 action(\"{truncate}\")：{path}",
                    truncate = VDFS_ACTION_TRUNCATE,
                )))
            }
            // 节点动作：转写区段上只有两类动词——**区间删除**
            // （`truncate`，落在单条消息上）与**恢复**（见下）。
            VdfsRequest::Action { action, payload } => {
                match (mid, action.as_str(), payload.as_ref()) {
                    (Some(mid), VDFS_ACTION_TRUNCATE, _) => {
                        let deleted_ids = self
                            .truncate_messages(id, mid)
                            .await
                            .map_err(vdfs_from_plugin_error)?;
                        // 回执带**权威**的被删 id 列表：消费方本地若因锚点缺失而删窄了，
                        // 据它补齐（`vdfs/delete` 只回 `{path}`，带不回这个）。
                        let data = serde_json::to_value(&deleted_ids)
                            .map_err(|e| VdfsError::internal(format!("截断回执序列化失败：{e}")))?;
                        let message = if deleted_ids.is_empty() {
                            // 目标不存在 ⇒ 什么都没删。这是**结果**不是错误，但也不发变更
                            // ——「什么都没删」不该留下痕迹：发一条「从 <mid> 起截断」的
                            // 通知会让消费者从一条并不存在的节点起截断，把整个列表清空。
                            format!("目标消息不存在，未做任何修改：{mid}")
                        } else {
                            format!("已从 {mid} 起截断 {} 条消息", deleted_ids.len())
                        };
                        Ok(VdfsResponse::Action(VdfsActionResult {
                            action: action.clone(),
                            ok: true,
                            message,
                            data: Some(data),
                        }))
                    }
                    // ── 恢复（retry_turn / retry / approve / reject / supply /
                    //    answer / retry_compaction）──
                    //
                    // 落点是**目标消息自身**（`<sid>/message/<mid>`）——与 `truncate`
                    // 同一个位置：「对这条消息做点什么」的地址就是它自己。动作名直接
                    // 取 `ResumeAction` 的线上词形（snake_case，跨栈契约已由
                    // `resume_action_wire_words_are_snake_case` 钉住），**不另造一套**
                    // ——同一批语义多一套名字就多一处漂移。
                    //
                    // 为什么它是动作而不是"写一条消息"：恢复不产生新用户消息，它
                    // **落在当时那条消息上**（删除-重建）。入队会让它排到一堆新消息
                    // 之后，等到被消费时候选早已被后续轮次改写，恢复语义当场失效。
                    (Some(mid), word, payload) => {
                        let Ok(resume_action) = serde_json::from_value::<cm::ResumeAction>(
                            Value::String(word.to_string()),
                        ) else {
                            return Err(VdfsError::NotImplemented);
                        };
                        self.session_of(id).await?;
                        let p = payload.cloned().unwrap_or(Value::Null);
                        let obj = p.as_object();
                        let get_str = |k: &str| {
                            obj.and_then(|o| o.get(k))
                                .and_then(Value::as_str)
                                .filter(|s| !s.is_empty())
                                .map(str::to_string)
                        };
                        // 上下文**自己造**（与收件箱消费者的 `run_inbox_turn` 同款）：
                        // 只带目标会话 id 与恢复请求。发起者的请求上下文刻意不沿用——
                        // 它的 `SESSION_ID` 是发起者自己的会话（跨空间时二者不同）。
                        let host = vdfs_host_ctx(ctx)?;
                        let req_ctx = host.fork();
                        req_ctx.set(SESSION_ID, id.to_string());
                        let _ = req_ctx.set_payload(session_chat::Request {
                            session_id: Some(id.to_string()),
                            resume: Some(cm::ResumeRequest {
                                target_id: mid.to_string(),
                                action: resume_action,
                                args: obj.and_then(|o| o.get("args")).cloned(),
                                reason: get_str("reason"),
                                answer: obj.and_then(|o| o.get("answer")).cloned(),
                            }),
                            mode: get_str("mode"),
                            risk_level: get_str("risk_level"),
                            ..session_chat::Request::default()
                        });
                        let resp = self
                            .me()
                            .map_err(vdfs_from_plugin_error)?
                            .start_turn(req_ctx)
                            .await
                            .map_err(vdfs_from_plugin_error)?;
                        let status = resp
                            .get::<Value>()
                            .ok()
                            .and_then(|v| v.get("status").and_then(Value::as_str).map(String::from))
                            .unwrap_or_default();
                        Ok(VdfsResponse::Action(VdfsActionResult {
                            action: action.clone(),
                            // 忙碌时**明确回绝**：http 层是 200，只有 `ok` 能把这个
                            // 事实带给调用方（前端据此把乐观置的 working 复位）。
                            ok: status != "session_busy",
                            message: if status == "session_busy" {
                                format!("会话正在处理中，无法{action}：{path}")
                            } else {
                                format!("已在 {mid} 上执行 {action}")
                            },
                            data: Some(json!({ "session_id": id, "status": status })),
                        }))
                    }
                    _ => Err(VdfsError::NotImplemented),
                }
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

    /// 收件箱（`<id>/inbox[/iid]`）：**待消费**的用户消息。
    async fn inbox_at(
        &self,
        ctx: &VdfsContext,
        path: &str,
        id: &str,
        iid: Option<&str>,
        req: VdfsRequest,
    ) -> VdfsResult<VdfsResponse> {
        match req {
            VdfsRequest::List { .. } => match iid {
                // 收件箱：**待消费**的用户消息（已消费的那些在转写里，不在这）
                None => {
                    self.session_of(id).await?;
                    Ok(VdfsResponse::list(
                        self.inbox_items(id)
                            .await
                            .iter()
                            .map(inbox_item_node)
                            .collect::<Vec<VdfsNode>>(),
                    ))
                }
                Some(_) => Err(VdfsError::not_found(format!(
                    "收件箱条目是列表项，没有子项：{path}"
                ))),
            },
            VdfsRequest::Stat => match iid {
                // 收件箱目录：形状与 `list` 同源；条目：队列里那一条（出队即消失）
                None => {
                    self.session_of(id).await?;
                    Ok(VdfsResponse::Stat(inbox_dir_node()))
                }
                Some(iid) => {
                    self.session_of(id).await?;
                    self.inbox_items(id)
                        .await
                        .iter()
                        .find(|i| i.id == iid)
                        .map(inbox_item_node)
                        .map(VdfsResponse::Stat)
                        .ok_or_else(|| {
                            VdfsError::not_found(format!(
                                "收件箱里没有待消费条目：{iid}（已出队的那条在转写里）"
                            ))
                        })
                }
            },
            VdfsRequest::Read => match iid {
                // 收件箱条目：正文就是那条待发消息（与消息节点同一投影口径，
                // 它就是一条消息——差的只是"还没被消费"）
                Some(iid) => {
                    self.session_of(id).await?;
                    let message = self
                        .inbox_items(id)
                        .await
                        .iter()
                        .find(|i| i.id == iid)
                        .map(|i| i.message.clone())
                        .ok_or_else(|| {
                            VdfsError::not_found(format!("收件箱里没有待消费条目：{iid}"))
                        })?;
                    Ok(VdfsResponse::Read(VdfsContent::text(message_text(
                        &message,
                    ))))
                }
                None => Err(VdfsError::invalid(format!("该路径不可读取内容：{path}"))),
            },
            VdfsRequest::Write { content } => {
                // 收件箱：**写即入队**（`<id>/inbox` 与 `<id>/inbox/<iid>` 同义）。
                //
                // ## 为什么这里不存在"发言不是一次写入"那个叉
                //
                // 转写列表（`<id>/message`）拒绝写入，因为"往列表里放一条"= 发言 =
                // 一次**动作**；而收件箱要表达的不是"跑一轮"，而是"把这条消息交给
                // 这个空间"——它本来就是一次写入，跑不跑、何时跑由空间自己决定
                // （见 `inbox` 模块）。两个地址因此一个只读、一个可写，这不是不对称，
                // 而是它们本来在说两件事。
                //
                // `create` 在这里**没有区分度**：两种形态都是新建一条队列项，因此既不
                // 要求也不拒绝——回执统一 `created: true`（它确实是一条新条目）。
                if content.binary {
                    return Err(VdfsError::invalid(
                        "收件箱条目是文本（ChatMessage 字段子集或纯文本），不接受二进制内容",
                    ));
                }
                // 存在性校验：会话不在，就没有"它的收件箱"可写
                self.session_of(id).await?;
                let raw = content.text.as_deref().unwrap_or("");
                let message = parse_inbox_message(raw)?;
                // 工作目录取自**请求上下文**（与 `chat/send` 入队时同一来源）：
                // 它是 `start_turn` 回退链的**第一档**（ctx > 会话 metadata > 报错），
                // 丢了它就只能靠会话 metadata——而「会话还没绑过 workdir」正是新建
                // 会话那一刻的常态。
                let workdir = vdfs_host_ctx(ctx)
                    .ok()
                    .and_then(|h| h.get(crate::symbio_core::WORKDIR));
                let item = self
                    .enqueue_inbox(
                        id,
                        iid.map(str::to_string),
                        message,
                        session_chat::Request::default(),
                        workdir,
                    )
                    .await;
                // 回执的 `name` 是**条目自己的名字**（不是请求地址的末段）：写收件箱
                // 目录自身时 id 由 provider 生成，调用方只能从回执得知它落成了什么
                // （与新建会话回执返回生成的 id 同一手法）；写具名条目
                // （`<inbox>/<iid>`）时那个名字本来就是调用方给的，回传没有信息量。
                let anonymous = iid.is_none();
                Ok(VdfsResponse::Write(VdfsWriteResponse {
                    name: anonymous.then(|| item.id.clone()),
                    created: true,
                    etag: None,
                }))
            }
            VdfsRequest::Delete { .. } => match iid {
                // 收件箱条目：**取消排队**（尚未被消费的那条）。
                //
                // 找不到就是"已出队"（它已经变成一条真消息了）或本来就没有——两种
                // 都报 `NotFound`：`delete` 的语义是"这条队列项没了"，不能报成成功。
                Some(iid) => {
                    self.session_of(id).await?;
                    if self.cancel_inbox_item(id, iid).await {
                        Ok(VdfsResponse::Unit)
                    } else {
                        Err(VdfsError::not_found(format!(
                            "收件箱里没有待消费条目 {iid}\
                             （已出队的那条已是会话里的消息，中止请走 \
                             action(会话地址, \"{abort}\")）：{path}",
                            abort = VDFS_ACTION_ABORT
                        )))
                    }
                }
                None => Err(VdfsError::Forbidden(format!(
                    "收件箱不可整体删除：清空待消费队列请用 action(\"{clear}\")：{path}",
                    clear = VDFS_ACTION_CLEAR,
                ))),
            },
            // 收件箱的 `clear`：**只取消还没被消费的**。已出队的那条已经是会话里的
            // 消息，中止它走 `action(<会话地址>, "abort")`（与删除条目同款分界）。
            VdfsRequest::Action { action, .. } if iid.is_none() && action == VDFS_ACTION_CLEAR => {
                self.session_of(id).await?;
                let ids = self.clear_inbox(id).await;
                Ok(VdfsResponse::Action(VdfsActionResult {
                    action: action.clone(),
                    ok: true,
                    message: format!("已取消 {} 条待消费消息", ids.len()),
                    data: Some(json!(ids)),
                }))
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
    async fn sub_sessions_at(
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
    async fn sub_session_at(
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
                    &self.session_runtime(sub).await,
                )))
            }
            VdfsRequest::Read => {
                let session = self.sub_session_of(id, sub).await?;
                // 子会话是**独立会话**（有自己的 id 与活跃状态），因此叠加的是它
                // 自己的在途缓冲，而不是父会话的——父会话的在途消息不属于它。
                let live = self.live_messages_of(&session.id).await;
                Ok(VdfsResponse::Read(session_content(&session, live)?))
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
    async fn workdir_at(
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
                    etag: None,
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

    /// 订阅：把 sink 登记进本插件的变更订阅表（`unwatch` 时按引用计数摘除）。
    ///
    /// provider 是**变更源的持有者**，因此这里不需要轮询——写入 / 删除路径
    /// 直接投递（见 [`SessionPlugin::notify_change`]）。投递在机制层收敛为
    /// **恰好一次**：重叠订阅（清单订根 + 转写订子树）不会把同一条变更投两遍。
    ///
    /// ## 路径不在这里收敛（容易看错，特此写明）
    ///
    /// 表里的路径与投递出的路径都是 **provider 根口径**（与 `list` / `stat`
    /// 同一坐标系：`<id>`、`<id>/message/<mid>`），本方法**原样登记**、不做任何
    /// 前缀处理。
    ///
    /// 看起来「应该」把它收敛成相对被订阅 `path` 的路径，但那样反而会错：
    /// 容器的 `watch` 包装器（`CompositeVdfs`）只补**挂载名**（首段），
    /// 不是补被订阅的整条路径——它把 `(dir, rel)` 拆开，`rel` 传给了 provider，
    /// 回填时只加 `dir`。因此 provider 报出的路径必须仍是 provider 根口径，
    /// 容器补上挂载名后才是树内全路径：
    ///
    /// ```text
    /// watch("session/abc/message") → dir = "session", rel = "abc/message"
    /// provider 报 "abc/message/m1" → 容器补成 "session/abc/message/m1"  ✓
    /// 若这里先剥成 "m1"        → 容器补成 "session/m1"          ✗
    /// ```
    ///
    /// 代价是订阅者会收到兄弟子树的变更（订阅一个会话会看到别的会话的事件）；
    /// 消费者按路径前缀自行过滤即可，正确性不受影响。
    async fn watch_at_path(&self, path: &str, sink: VdfsChangeSink) -> VdfsResult<()> {
        self.change_subs.watch(path, sink);
        Ok(())
    }

    /// 取消订阅：与 [`Self::watch_at_path`] 严格配对——引用计数归零才真正摘掉。
    async fn unwatch_at_path(&self, path: &str) -> VdfsResult<()> {
        self.change_subs.unwatch(path);
        Ok(())
    }
}

impl SessionPlugin {
    /// 会话转写（**含在途消息**）——`<根>/session/<id>/message` 的唯一数据源。
    ///
    /// 落库转写 ∪ 本轮在途缓冲。之所以要并上后者：流式期间消息**还没落库**
    /// （`persist_messages` 只在每轮结束时写盘），只读存储的话列表在流式期间
    /// 就是空的——`created` / `appended` 变更到达时消费者去 `list` 会一无所获，
    /// 「转写即列表」当场失效。
    ///
    /// 存在性校验在此一并完成（`session_of`），因此调用方不必再查一次。
    async fn transcript_of(&self, id: &str) -> VdfsResult<Vec<cm::ChatMessage>> {
        let session = self.session_of(id).await?;
        let live = self.live_messages_of(id).await;
        Ok(ordered(overlay_live(session.messages, live)))
    }

    /// 该会话本轮的在途消息（无活跃状态 ⇒ 无在途消息）
    async fn live_messages_of(&self, id: &str) -> Vec<cm::ChatMessage> {
        let active = self.active_mgr.sessions.read().await;
        match active.get(id) {
            Some(st) => st.transcript.lock().await.snapshot(),
            None => Vec::new(),
        }
    }

    /// 按 id 取会话（**存在性校验**：`load_session` 对未命中会返回空会话，
    /// 因此这里走带存在性判据的 `load_session_checked`）
    pub(crate) async fn session_of(&self, id: &str) -> VdfsResult<Session> {
        self.get_store()
            .await
            .map_err(vdfs_from_plugin_error)?
            .load_session_checked(id)
            .await
            .map_err(vdfs_from_plugin_error)?
            .ok_or_else(|| VdfsError::not_found(format!("会话不存在：{id}")))
    }

    /// 单个会话的**运行态**（节点状态的唯一来源，见 [`SessionRuntime`]）。
    ///
    /// 无活跃状态（进程内从未跑过该会话）⇒ 空闲。读的是同一份
    /// `ActiveSessionStateInner`，因此 `list` / `stat` / 变更发射三处口径必然一致
    /// ——不存在"列表说空闲、变更说运行中"这种分叉。
    pub(crate) async fn session_runtime(&self, id: &str) -> SessionRuntime {
        let active = self.active_mgr.sessions.read().await;
        let Some(st) = active.get(id) else {
            return SessionRuntime::idle(None);
        };
        // `try_read` 失败（正被写者持有）时按"运行中"回答：运行态的写入都是
        // 瞬时操作，读不到就说明此刻正在变更——宁可多报一次运行中（前端会
        // 在下一次变更收敛），也不要凭空把正在跑的会话报成空闲（那会让停止
        // 按钮消失、用户无法中止）。
        let Ok(inner) = st.inner.try_read() else {
            return SessionRuntime::working();
        };
        SessionRuntime::from_state(
            inner.is_working,
            inner.last_outcome.clone(),
            inner.last_error.clone(),
            inner.last_warning.clone(),
        )
    }

    /// 会话清单 → VDFS 节点（携带实时运行态）
    async fn nodes_of_sessions(
        &self,
        sessions: &[SessionSummary],
        schema: &Option<Value>,
    ) -> Vec<VdfsNode> {
        let active = self.active_mgr.sessions.read().await;
        sessions
            .iter()
            .map(|s| {
                let rt = match active.get(&s.id) {
                    Some(st) => match st.inner.try_read() {
                        Ok(inner) => SessionRuntime::from_state(
                            inner.is_working,
                            inner.last_outcome.clone(),
                            inner.last_error.clone(),
                            inner.last_warning.clone(),
                        ),
                        Err(_) => SessionRuntime::working(),
                    },
                    None => SessionRuntime::idle(None),
                };
                let mut n = session_node(s, &rt);
                // 选项定义：清单一次取全（前端零额外请求，见设计文档 §7）
                n.schema = schema.clone();
                n
            })
            .collect()
    }

    /// 会话的选项定义（`node.schema` / `new_type.schema` 的取值）。
    ///
    /// 定义与「是哪个会话」无关（见 `options::SessionPlugin::build_option_definition`），
    /// 因此清单里每一项挂的是**同一份**——`VdfsNode::schema` 是 `Value`，逐项 clone
    /// 一份即可（设计文档 §7 已量化这份重复：量级 ~2 KB/项，换前端零额外请求）。
    /// 序列化失败（理论上不会：定义里没有非字符串键的 map）退化为**不挂 schema**，
    /// 前端按「无定义」处理——选项栏为空，会话本身照常可用。
    pub(crate) async fn session_schema(&self) -> Option<Value> {
        match serde_json::to_value(self.build_option_definition().await) {
            Ok(v) => Some(v),
            Err(e) => {
                crate::plugin_warn!("session", "选项定义序列化失败（选项栏将为空）: {e}");
                None
            }
        }
    }

    /// 会话的工作目录（未声明 workdir ⇒ 该会话无目录树能力）
    async fn workdir_of(&self, id: &str) -> VdfsResult<String> {
        let session = self.session_of(id).await?;
        super::super::workdir::workdir_of(&session)
            .ok_or_else(|| VdfsError::not_found(format!("会话 {id} 没有工作目录")))
    }

    /// 会话记忆 → VDFS 节点。
    ///
    /// 形状由共享实现 [`MemoryFile::node`](crate::providers::MemoryFile::node) 产出，
    /// `list`（经 `internal_dirs`）与 `stat`
    /// **共用同一份**——「列表里的和点开的不是同一个东西」这类 bug 因此写不出来。
    async fn memory_node_of(&self, id: &str) -> VdfsNode {
        self.memory_store(id)
            .await
            .node(&crate::providers::MemoryNodeSpec {
                title: super::super::memory::SEGMENT_TITLE,
                name: None,
                kind: PLUGIN_ID_SESSION,
                description: super::super::memory::MEMORY_DESCRIPTION,
            })
    }

    /// 取子会话（**归属校验**：必须确实挂在 `id` 之下，避免跨会话越权访问）
    async fn sub_session_of(
        &self,
        id: &str,
        sub: &str,
    ) -> VdfsResult<super::super::types::Session> {
        let store = self.get_store().await.map_err(vdfs_from_plugin_error)?;
        let session = store
            .load_session(sub)
            .await
            .map_err(vdfs_from_plugin_error)?;
        if session.id == sub && session.parent_session_id() == Some(id) {
            Ok(session)
        } else {
            Err(VdfsError::not_found(format!(
                "会话 {id} 下不存在子会话 {sub}"
            )))
        }
    }
}

#[cfg(test)]
#[path = "vdfs_provider.test.rs"]
mod tests;
