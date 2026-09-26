//! `impl VdfsProvider for SessionPlugin` 及其**私有辅助**。
//!
//! 自 `plugin.rs` 原样搬移（拆文件不拆行为）。职责：
//! - [`vdfs::VdfsProvider::dispatch`] 是唯一入口：**先按 `path` 定位资源域，再按
//!   `req` 执行操作**——`parse_session_path` 的每个域各有一个 `*_at` 私有方法，
//!   dispatch 只做路由，域内怎么读写是各方法自己的事；
//! - 域方法全部**转发既有会话能力**（SessionStore + 会话 metadata 合并），
//!   不新造协议；
//! - 只读辅助：转写（含在途消息）、存在性校验、实时工作状态、工作目录、子会话。

use super::*;
use crate::symbio_core::clock_now_ms;

#[async_trait]
impl vdfs::VdfsProvider for SessionPlugin {
    /// 唯一入口：**先按 `path` 定位资源域，再按 `req` 执行操作**。
    ///
    /// 配置文件（`PLUGIN.yml`）是文档不是会话——先判路径再分流操作；其余全部经
    /// [`parse_session_path`] 定域，各域逻辑收敛在下方 `*_at` 方法里。
    async fn dispatch(
        &self,
        ctx: &vdfs::VdfsContext,
        path: &str,
        req: vdfs::VdfsRequest,
    ) -> vdfs::VdfsResult<vdfs::VdfsResponse> {
        // 配置文件优先：`PLUGIN.yml` 是文档，不是会话
        if path == PLUGIN_FILE {
            return match req {
                vdfs::VdfsRequest::Stat => Ok(vdfs::VdfsResponse::Stat(self.config_file.node())),
                vdfs::VdfsRequest::Read => Ok(vdfs::VdfsResponse::Read(
                    self.config_file.read(&self.config).await?,
                )),
                vdfs::VdfsRequest::Write { content } => Ok(vdfs::VdfsResponse::Write(
                    self.config_file.apply(&self.config, &content).await?,
                )),
                vdfs::VdfsRequest::List { .. } => Err(vdfs::VdfsError::not_found(format!(
                    "配置文件是文档，没有子项：{path}"
                ))),
                vdfs::VdfsRequest::Delete { .. } => {
                    Err(vdfs::VdfsError::Forbidden("配置文件不可删除".to_string()))
                }
                _ => Err(vdfs::VdfsError::invalid(format!("该路径不可操作：{path}"))),
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
        ctx: &vdfs::VdfsContext,
        path: &str,
        req: vdfs::VdfsRequest,
    ) -> vdfs::VdfsResult<vdfs::VdfsResponse> {
        match req {
            vdfs::VdfsRequest::List { .. } => {
                let (limit, before) = window_params(ctx);
                // 清单里**只有会话**——配置文件是文档，它进设置菜单走的是
                // ConfigurableVisitor 那条通道，不该在会话清单里再出现一次
                // （否则列表底部会多一个「设置」项）。
                let sessions = self
                    .list_sessions_window(limit, before)
                    .await
                    .map_err(vdfs::vdfs_from_plugin_error)?;
                // 选项定义与「是哪个会话」无关 ⇒ 一次算好，清单里逐项复用
                let schema = self.session_schema().await;
                Ok(vdfs::VdfsResponse::list(
                    self.nodes_of_sessions(&sessions, &schema).await,
                ))
            }
            vdfs::VdfsRequest::Stat => {
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
                Ok(vdfs::VdfsResponse::Stat(
                    vdfs::VdfsNode::dir("", "会话", vdfs::VdfsAccess::LIST).with_new_type(Some(
                        vdfs::VdfsNewType::new(EXT_SESSION, "会话")
                            .with_description("新建会话")
                            .with_schema_opt(self.session_schema().await),
                    )),
                ))
            }
            vdfs::VdfsRequest::Read => Err(vdfs::VdfsError::invalid(format!(
                "该路径不可读取内容：{path}"
            ))),
            vdfs::VdfsRequest::Write { content } => {
                // 挂载根 = 「新建一个会话，名字由 provider 生成」。
                //
                // 写目录自身没有可覆盖的目标，因此 `create` 是唯一合法意图
                // （见 `VdfsRequest::Write` 的两种目标形态）；缺它即报错，
                // 不静默落成「一次无意义的写」。
                if !content.create {
                    return Err(vdfs::VdfsError::invalid(
                        "写会话挂载根需要 create 意图：目录自身没有可覆盖的目标",
                    ));
                }
                self.session_upsert(path, &content).await
            }
            vdfs::VdfsRequest::Delete { .. } => {
                Err(vdfs::VdfsError::Forbidden("会话挂载根不可删除".to_string()))
            }
            vdfs::VdfsRequest::Watch { sink } => {
                self.watch_at_path(path, sink).await?;
                Ok(vdfs::VdfsResponse::Unit)
            }
            vdfs::VdfsRequest::Unwatch => {
                self.unwatch_at_path(path).await?;
                Ok(vdfs::VdfsResponse::Unit)
            }
            _ => Err(vdfs::VdfsError::NotImplemented),
        }
    }

    /// 会话本体（`<id>`）：清单里是**叶子**，被当目录访问时是**会话内部**视图。
    async fn session_at(
        &self,
        path: &str,
        id: &str,
        req: vdfs::VdfsRequest,
    ) -> vdfs::VdfsResult<vdfs::VdfsResponse> {
        match req {
            vdfs::VdfsRequest::List { .. } => {
                // 会话内部：三个虚拟子目录 + 记忆文件（会话存在性校验由 `session_of` 承担）
                let session = self.session_of(id).await?;
                Ok(vdfs::VdfsResponse::list(internal_dirs(
                    super::super::workdir::workdir_of(&session).is_some(),
                    self.memory_node_of(id).await,
                )))
            }
            vdfs::VdfsRequest::Stat => {
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
                n.access = vdfs::VdfsAccess::LIST;
                Ok(vdfs::VdfsResponse::Stat(n))
            }
            vdfs::VdfsRequest::Read => {
                let session = self.session_of(id).await?;
                // 含在途：叶子与转写列表必须是同一份消息集合，否则前端
                // `loadMessages`（读叶子）会在流式期间看不到正在跑的那一轮。
                // 详见 `session_content` 的文档。
                let live = self.live_messages_of(id).await;
                Ok(vdfs::VdfsResponse::Read(session_content(&session, live)?))
            }
            // 会话本身（`path` 即会话 id —— 具名新建时它就是**身份**，见
            // `session_upsert` 的 `create` 分支；`id` 由地址末段给出，不再由
            // provider 另生成一个）
            vdfs::VdfsRequest::Write { content } => self.session_upsert(path, &content).await,
            vdfs::VdfsRequest::Delete { .. } => {
                // 存在性校验：删除不存在的会话应报 NotFound 而非静默成功
                self.session_of(id).await?;
                self.delete_session_internal(id)
                    .await
                    .map_err(vdfs::vdfs_from_plugin_error)?;
                Ok(vdfs::VdfsResponse::Unit)
            }
            vdfs::VdfsRequest::Watch { sink } => {
                self.watch_at_path(path, sink).await?;
                Ok(vdfs::VdfsResponse::Unit)
            }
            vdfs::VdfsRequest::Unwatch => {
                self.unwatch_at_path(path).await?;
                Ok(vdfs::VdfsResponse::Unit)
            }
            // 节点动作：**中止**正在跑的那一轮。
            //
            // 落点是**会话节点自身**（`<sid>`），而不是 `<sid>/inbox`：在途那一轮
            // 早已出队，它现在是转写里的消息；队列上的动作只该管「还没被消费的」
            // （取消 = 删条目，清空 = `clear`）。两个动词作用在**两个不同的对象**上
            // ，因此是两个地址——与 ADR-026 已确立的分界同一条。
            vdfs::VdfsRequest::Action { action, .. } => match action.as_str() {
                vdfs::VDFS_ACTION_ABORT => {
                    // 存在性校验：会话不在就没有「它的一轮」可谈
                    self.session_of(id).await?;
                    let stopped = self.abort_turn(id).await;
                    Ok(vdfs::VdfsResponse::Action(vdfs::VdfsActionResult {
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
                _ => Err(vdfs::VdfsError::NotImplemented),
            },
            _ => Err(vdfs::VdfsError::NotImplemented),
        }
    }

    /// 会话记忆（`<id>/MEMORY.md`）：单个文件，形状由共享实现产出。
    async fn memory_at(
        &self,
        path: &str,
        id: &str,
        req: vdfs::VdfsRequest,
    ) -> vdfs::VdfsResult<vdfs::VdfsResponse> {
        match req {
            // 记忆是**单个文件**：没有子项
            vdfs::VdfsRequest::List { .. } => Err(vdfs::VdfsError::not_found(format!(
                "记忆是文件，没有子项：{path}"
            ))),
            vdfs::VdfsRequest::Stat => {
                // 存在性校验：会话不在，就没有「它的记忆」可谈
                self.session_of(id).await?;
                Ok(vdfs::VdfsResponse::Stat(self.memory_node_of(id).await))
            }
            vdfs::VdfsRequest::Read => {
                // 会话记忆（`<根>/session/<id>/MEMORY.md`）：正文即文件全文。
                // 文件不存在 → 空串（不是错误）——「还没写过」是记忆的正常状态。
                self.session_of(id).await?;
                let text = self
                    .memory_store(id)
                    .await
                    .read()
                    .map_err(vdfs::VdfsError::internal)?;
                Ok(vdfs::VdfsResponse::Read(vdfs::VdfsContent::text(text)))
            }
            vdfs::VdfsRequest::Write { content } => {
                // 会话记忆：**纯文本**写入（容量闸门在共享实现 `MemoryFile::write`，本插件不重复实现）
                if content.binary {
                    return Err(vdfs::VdfsError::invalid(
                        "会话记忆是文本文件，不接受二进制内容",
                    ));
                }
                // 存在性校验：会话不在，就没有「它的记忆」可写
                self.session_of(id).await?;
                let store = self.memory_store(id).await;
                let existed = store.exists();
                let text = content.text.as_deref().unwrap_or_default();
                store.write(text).map_err(vdfs::VdfsError::invalid)?;
                // 变更路径与 `list` 返回的节点地址同源（provider 子树口径，
                // 挂载名由容器的 watch 包装补上——见 `watch_at_path` 的说明）
                self.change_subs.notify(&vdfs::VdfsChange::bare(
                    super::super::memory::memory_rel_path(id),
                ));
                Ok(vdfs::VdfsResponse::Write(vdfs::VdfsWriteResponse {
                    name: None,
                    created: !existed,
                    etag: None,
                }))
            }
            vdfs::VdfsRequest::Delete { .. } => {
                // 记忆**不可删除**（与 work / agent 的记忆层同一共享实现约定）：
                // 删除即丢失本会话的长期约定，且没有东西能把它找回来。要清空就写入空内容
                // ——那是一次可读、可审、可撤销的显式动作。
                Err(vdfs::VdfsError::Forbidden(format!(
                    "会话记忆不可删除（删除即丢失本会话的长期约定）。\
                     如需清空，请向 `{}` 写入空内容。",
                    crate::plugins::session::memory::SESSION_MEMORY_FILE
                )))
            }
            vdfs::VdfsRequest::Watch { sink } => {
                self.watch_at_path(path, sink).await?;
                Ok(vdfs::VdfsResponse::Unit)
            }
            vdfs::VdfsRequest::Unwatch => {
                self.unwatch_at_path(path).await?;
                Ok(vdfs::VdfsResponse::Unit)
            }
            _ => Err(vdfs::VdfsError::NotImplemented),
        }
    }

    /// 转写区段（`<id>/message[/mid]`）：列表与单条消息。
    async fn messages_at(
        &self,
        ctx: &vdfs::VdfsContext,
        path: &str,
        id: &str,
        mid: Option<&str>,
        req: vdfs::VdfsRequest,
    ) -> vdfs::VdfsResult<vdfs::VdfsResponse> {
        match req {
            vdfs::VdfsRequest::List { .. } => match mid {
                // 转写列表：每一条消息是一个列表项，顺序由 `seq` 决定。
                // 含**在途**消息——流式期间列表就是活的，不必等落库。
                None => {
                    let msgs = self.transcript_of(id).await?;
                    let (limit, before) = window_params(ctx);
                    // 有界窗口：**只在调用方显式给参数时**生效。不给参数 = 全量，
                    // 与从前逐字节一致（流式期间前端要的是完整列表）。
                    Ok(vdfs::VdfsResponse::list(
                        transcript_window(&msgs, limit, before)
                            .iter()
                            .map(message_node)
                            .collect::<Vec<vdfs::VdfsNode>>(),
                    ))
                }
                Some(_) => Err(vdfs::VdfsError::not_found(format!(
                    "消息是列表项，没有子项：{path}"
                ))),
            },
            vdfs::VdfsRequest::Stat => match mid {
                // 列表本身是个目录（`l` 位：可列，不参与树遍历——会话整体是叶子）。
                // 只需存在性校验，不必取全量转写。
                None => {
                    self.session_of(id).await?;
                    Ok(vdfs::VdfsResponse::Stat(messages_dir_node()))
                }
                Some(mid) => Ok(vdfs::VdfsResponse::Stat(message_node(message_of(
                    &self.transcript_of(id).await?,
                    mid,
                )?))),
            },
            vdfs::VdfsRequest::Read => match mid {
                // 单条消息的正文（列表项内容）——流式追加的正是它。
                // 同样取含在途的转写：追加型变更的消费者读到的必须是**已含该增量**的正文。
                Some(mid) => {
                    let msgs = self.transcript_of(id).await?;
                    Ok(vdfs::VdfsResponse::Read(vdfs::VdfsContent::text(
                        message_text(message_of(&msgs, mid)?),
                    )))
                }
                None => Err(vdfs::VdfsError::invalid(format!(
                    "该路径不可读取内容：{path}"
                ))),
            },
            vdfs::VdfsRequest::Write { content } => match mid {
                Some(mid) => {
                    if content.create {
                        return Err(vdfs::VdfsError::invalid(
                            "消息不支持 create 意图：新增消息即发言，请走聊天协议\
                             （一次发言触发一整轮编排）",
                        ));
                    }
                    if content.binary {
                        return Err(vdfs::VdfsError::invalid(
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
                        vdfs::VdfsError::invalid(format!(
                            "消息补丁需要合法 JSON（ChatMessage 字段子集）：{e}"
                        ))
                    })?;
                    let obj = value.as_object_mut().ok_or_else(|| {
                        vdfs::VdfsError::invalid("消息补丁需要 JSON 对象（ChatMessage 字段子集）")
                    })?;
                    if !obj.contains_key("id") {
                        obj.insert("id".to_string(), Value::String(mid.to_string()));
                    }
                    let patch: cm::ChatMessage = serde_json::from_value(value).map_err(|e| {
                        vdfs::VdfsError::invalid(format!(
                            "消息补丁需要合法 JSON（ChatMessage 字段子集）：{e}"
                        ))
                    })?;
                    self.patch_message(id, mid, &patch)
                        .await
                        .map_err(vdfs::vdfs_from_plugin_error)?;
                    Ok(vdfs::VdfsResponse::Write(vdfs::VdfsWriteResponse {
                        name: None,
                        created: false,
                        etag: None,
                    }))
                }
                // 转写列表本身：**仍只读**。往里放一条 = 发言 = 动作，不是写入。
                None => Err(vdfs::VdfsError::Forbidden(
                    "转写列表只读：发言请走聊天协议（一次发言触发一整轮编排）".to_string(),
                )),
            },
            vdfs::VdfsRequest::Delete { .. } => {
                // 转写区段（列表与单条）**不可 delete**：`delete` 的全局语义是
                // 「**这一个**节点没了」，而转写有两种**集合**删除语义，各有自己的动作：
                // 「从这里到末尾全没了」是 [`vdfs::VDFS_ACTION_TRUNCATE`]（落在起始消息上）、
                // 「整个列表空了」是 [`vdfs::VDFS_ACTION_CLEAR`]（落在列表目录上）。
                // 动作不共用一个动词——理由见 `VDFS_ACTION_TRUNCATE` 的文档。
                Err(vdfs::VdfsError::Forbidden(format!(
                    "转写区段不可 delete：清空列表请用 action(\"{clear}\")，\
                     删除某条及其之后请用 action(\"{truncate}\")：{path}",
                    clear = vdfs::VDFS_ACTION_CLEAR,
                    truncate = vdfs::VDFS_ACTION_TRUNCATE,
                )))
            }
            // 节点动作：转写区段上有两类动词——两种**集合操作**（逐条下发移除帧的
            // 理由见各分支文档）与**恢复**（落在单条消息上，见下）。
            vdfs::VdfsRequest::Action { action, payload } => {
                match (mid, action.as_str(), payload.as_ref()) {
                    (Some(mid), vdfs::VDFS_ACTION_TRUNCATE, _) => {
                        let deleted_ids = self
                            .truncate_messages(id, mid)
                            .await
                            .map_err(vdfs::vdfs_from_plugin_error)?;
                        // 回执带**权威**的被删 id 列表：消费方本地若因锚点缺失而删窄了，
                        // 据它补齐（`vdfs/delete` 只回 `{path}`，带不回这个）。
                        let data = serde_json::to_value(&deleted_ids).map_err(|e| {
                            vdfs::VdfsError::internal(format!("截断回执序列化失败：{e}"))
                        })?;
                        let message = if deleted_ids.is_empty() {
                            // 目标不存在 ⇒ 什么都没删。这是**结果**不是错误，但也不发变更
                            // ——「什么都没删」不该留下痕迹：发一条「从 <mid> 起截断」的
                            // 通知会让消费者从一条并不存在的节点起截断，把整个列表清空。
                            format!("目标消息不存在，未做任何修改：{mid}")
                        } else {
                            format!("已从 {mid} 起截断 {} 条消息", deleted_ids.len())
                        };
                        Ok(vdfs::VdfsResponse::Action(vdfs::VdfsActionResult {
                            action: action.clone(),
                            ok: true,
                            message,
                            data: Some(data),
                        }))
                    }
                    (None, vdfs::VDFS_ACTION_CLEAR, _) => {
                        self.clear_messages(id)
                            .await
                            .map_err(vdfs::vdfs_from_plugin_error)?;
                        Ok(vdfs::VdfsResponse::Action(vdfs::VdfsActionResult {
                            action: action.clone(),
                            ok: true,
                            message: "已清空会话历史（会话本身与元数据保留）".to_string(),
                            data: None,
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
                            return Err(vdfs::VdfsError::NotImplemented);
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
                        let host = vdfs::vdfs_host_ctx(ctx)?;
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
                            .map_err(vdfs::vdfs_from_plugin_error)?
                            .start_turn(req_ctx)
                            .await
                            .map_err(vdfs::vdfs_from_plugin_error)?;
                        let status = resp
                            .get::<Value>()
                            .ok()
                            .and_then(|v| v.get("status").and_then(Value::as_str).map(String::from))
                            .unwrap_or_default();
                        Ok(vdfs::VdfsResponse::Action(vdfs::VdfsActionResult {
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
                    _ => Err(vdfs::VdfsError::NotImplemented),
                }
            }
            vdfs::VdfsRequest::Watch { sink } => {
                self.watch_at_path(path, sink).await?;
                Ok(vdfs::VdfsResponse::Unit)
            }
            vdfs::VdfsRequest::Unwatch => {
                self.unwatch_at_path(path).await?;
                Ok(vdfs::VdfsResponse::Unit)
            }
            _ => Err(vdfs::VdfsError::NotImplemented),
        }
    }

    /// 收件箱（`<id>/inbox[/iid]`）：**待消费**的用户消息。
    async fn inbox_at(
        &self,
        ctx: &vdfs::VdfsContext,
        path: &str,
        id: &str,
        iid: Option<&str>,
        req: vdfs::VdfsRequest,
    ) -> vdfs::VdfsResult<vdfs::VdfsResponse> {
        match req {
            vdfs::VdfsRequest::List { .. } => match iid {
                // 收件箱：**待消费**的用户消息（已消费的那些在转写里，不在这）
                None => {
                    self.session_of(id).await?;
                    Ok(vdfs::VdfsResponse::list(
                        self.inbox_items(id)
                            .await
                            .iter()
                            .map(inbox_item_node)
                            .collect::<Vec<vdfs::VdfsNode>>(),
                    ))
                }
                Some(_) => Err(vdfs::VdfsError::not_found(format!(
                    "收件箱条目是列表项，没有子项：{path}"
                ))),
            },
            vdfs::VdfsRequest::Stat => match iid {
                // 收件箱目录：形状与 `list` 同源；条目：队列里那一条（出队即消失）
                None => {
                    self.session_of(id).await?;
                    Ok(vdfs::VdfsResponse::Stat(inbox_dir_node()))
                }
                Some(iid) => {
                    self.session_of(id).await?;
                    self.inbox_items(id)
                        .await
                        .iter()
                        .find(|i| i.id == iid)
                        .map(inbox_item_node)
                        .map(vdfs::VdfsResponse::Stat)
                        .ok_or_else(|| {
                            vdfs::VdfsError::not_found(format!(
                                "收件箱里没有待消费条目：{iid}（已出队的那条在转写里）"
                            ))
                        })
                }
            },
            vdfs::VdfsRequest::Read => match iid {
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
                            vdfs::VdfsError::not_found(format!("收件箱里没有待消费条目：{iid}"))
                        })?;
                    Ok(vdfs::VdfsResponse::Read(vdfs::VdfsContent::text(
                        message_text(&message),
                    )))
                }
                None => Err(vdfs::VdfsError::invalid(format!(
                    "该路径不可读取内容：{path}"
                ))),
            },
            vdfs::VdfsRequest::Write { content } => {
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
                    return Err(vdfs::VdfsError::invalid(
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
                let workdir = vdfs::vdfs_host_ctx(ctx)
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
                Ok(vdfs::VdfsResponse::Write(vdfs::VdfsWriteResponse {
                    name: anonymous.then(|| item.id.clone()),
                    created: true,
                    etag: None,
                }))
            }
            vdfs::VdfsRequest::Delete { .. } => match iid {
                // 收件箱条目：**取消排队**（尚未被消费的那条）。
                //
                // 找不到就是"已出队"（它已经变成一条真消息了）或本来就没有——两种
                // 都报 `NotFound`：`delete` 的语义是"这条队列项没了"，不能报成成功。
                Some(iid) => {
                    self.session_of(id).await?;
                    if self.cancel_inbox_item(id, iid).await {
                        Ok(vdfs::VdfsResponse::Unit)
                    } else {
                        Err(vdfs::VdfsError::not_found(format!(
                            "收件箱里没有待消费条目 {iid}\
                             （已出队的那条已是会话里的消息，中止请走 \
                             action(会话地址, \"{abort}\")）：{path}",
                            abort = vdfs::VDFS_ACTION_ABORT
                        )))
                    }
                }
                None => Err(vdfs::VdfsError::Forbidden(format!(
                    "收件箱不可整体删除：清空待消费队列请用 action(\"{clear}\")：{path}",
                    clear = vdfs::VDFS_ACTION_CLEAR,
                ))),
            },
            // 收件箱的 `clear`：**只取消还没被消费的**。已出队的那条已经是会话里的
            // 消息，中止它走 `action(<会话地址>, "abort")`（与删除条目同款分界）。
            vdfs::VdfsRequest::Action { action, .. }
                if iid.is_none() && action == vdfs::VDFS_ACTION_CLEAR =>
            {
                self.session_of(id).await?;
                let ids = self.clear_inbox(id).await;
                Ok(vdfs::VdfsResponse::Action(vdfs::VdfsActionResult {
                    action: action.clone(),
                    ok: true,
                    message: format!("已取消 {} 条待消费消息", ids.len()),
                    data: Some(json!(ids)),
                }))
            }
            vdfs::VdfsRequest::Watch { sink } => {
                self.watch_at_path(path, sink).await?;
                Ok(vdfs::VdfsResponse::Unit)
            }
            vdfs::VdfsRequest::Unwatch => {
                self.unwatch_at_path(path).await?;
                Ok(vdfs::VdfsResponse::Unit)
            }
            _ => Err(vdfs::VdfsError::NotImplemented),
        }
    }

    /// 子会话清单目录（`<id>/sub`）。
    async fn sub_sessions_at(
        &self,
        path: &str,
        id: &str,
        req: vdfs::VdfsRequest,
    ) -> vdfs::VdfsResult<vdfs::VdfsResponse> {
        match req {
            vdfs::VdfsRequest::List { .. } => {
                self.session_of(id).await?;
                let store = self
                    .get_store()
                    .await
                    .map_err(vdfs::vdfs_from_plugin_error)?;
                let subs = store
                    .list_sub_sessions(id)
                    .await
                    .map_err(vdfs::vdfs_from_plugin_error)?;
                // 子会话也是会话：同一份选项定义
                let schema = self.session_schema().await;
                Ok(vdfs::VdfsResponse::list(
                    self.nodes_of_sessions(&subs, &schema).await,
                ))
            }
            vdfs::VdfsRequest::Stat => {
                self.session_of(id).await?;
                Ok(vdfs::VdfsResponse::Stat(
                    super::super::workdir::sub_sessions_dir_node(),
                ))
            }
            vdfs::VdfsRequest::Read => Err(vdfs::VdfsError::invalid(format!(
                "该路径不可读取内容：{path}"
            ))),
            vdfs::VdfsRequest::Write { .. } => {
                Err(vdfs::VdfsError::invalid(format!("该路径不可写入：{path}")))
            }
            vdfs::VdfsRequest::Delete { .. } => Err(vdfs::VdfsError::Forbidden(format!(
                "该路径不可删除：{path}"
            ))),
            vdfs::VdfsRequest::Watch { sink } => {
                self.watch_at_path(path, sink).await?;
                Ok(vdfs::VdfsResponse::Unit)
            }
            vdfs::VdfsRequest::Unwatch => {
                self.unwatch_at_path(path).await?;
                Ok(vdfs::VdfsResponse::Unit)
            }
            _ => Err(vdfs::VdfsError::NotImplemented),
        }
    }

    /// 子会话本体（`<id>/sub/<sub>`）：独立会话，有自己的 id 与活跃状态。
    async fn sub_session_at(
        &self,
        path: &str,
        id: &str,
        sub: &str,
        req: vdfs::VdfsRequest,
    ) -> vdfs::VdfsResult<vdfs::VdfsResponse> {
        match req {
            vdfs::VdfsRequest::List { .. } => Err(vdfs::VdfsError::not_found(format!(
                "子会话是叶子节点，没有子项：{path}"
            ))),
            vdfs::VdfsRequest::Stat => {
                let session = self.sub_session_of(id, sub).await?;
                Ok(vdfs::VdfsResponse::Stat(session_node(
                    &SessionSummary::of(&session),
                    &self.session_runtime(sub).await,
                )))
            }
            vdfs::VdfsRequest::Read => {
                let session = self.sub_session_of(id, sub).await?;
                // 子会话是**独立会话**（有自己的 id 与活跃状态），因此叠加的是它
                // 自己的在途缓冲，而不是父会话的——父会话的在途消息不属于它。
                let live = self.live_messages_of(&session.id).await;
                Ok(vdfs::VdfsResponse::Read(session_content(&session, live)?))
            }
            vdfs::VdfsRequest::Write { .. } => {
                Err(vdfs::VdfsError::invalid(format!("该路径不可写入：{path}")))
            }
            vdfs::VdfsRequest::Delete { .. } => {
                // 子会话：先 abort 活跃任务再删（`delete_session_internal` 同语义）
                self.sub_session_of(id, sub).await?;
                self.delete_session_internal(sub)
                    .await
                    .map_err(vdfs::vdfs_from_plugin_error)?;
                Ok(vdfs::VdfsResponse::Unit)
            }
            vdfs::VdfsRequest::Watch { sink } => {
                self.watch_at_path(path, sink).await?;
                Ok(vdfs::VdfsResponse::Unit)
            }
            vdfs::VdfsRequest::Unwatch => {
                self.unwatch_at_path(path).await?;
                Ok(vdfs::VdfsResponse::Unit)
            }
            _ => Err(vdfs::VdfsError::NotImplemented),
        }
    }

    /// 会话工作目录（`<id>/workdir[/rel]`）：目录树与文件读写。
    async fn workdir_at(
        &self,
        path: &str,
        id: &str,
        rel: &str,
        req: vdfs::VdfsRequest,
    ) -> vdfs::VdfsResult<vdfs::VdfsResponse> {
        match req {
            vdfs::VdfsRequest::List { .. } => {
                let workdir = self.workdir_of(id).await?;
                // 目录树场景的实时监听（按会话 id 引用计数）
                self.workdir_watches.ensure_watch(&workdir, id);
                super::super::workdir::list_children(&workdir, Some(rel))
                    .await
                    .map_err(vdfs::vdfs_from_plugin_error)
                    .map(vdfs::VdfsResponse::list)
            }
            vdfs::VdfsRequest::Stat => {
                let workdir = self.workdir_of(id).await?;
                if rel.is_empty() {
                    return Ok(vdfs::VdfsResponse::Stat(
                        super::super::workdir::workdir_dir_node(),
                    ));
                }
                super::super::workdir::read_node(&workdir, rel)
                    .await
                    .map_err(vdfs::vdfs_from_plugin_error)
                    .map(vdfs::VdfsResponse::Stat)
            }
            vdfs::VdfsRequest::Read => {
                if rel.is_empty() {
                    return Err(vdfs::VdfsError::invalid(format!(
                        "该路径不可读取内容：{path}"
                    )));
                }
                let workdir = self.workdir_of(id).await?;
                let text = super::super::workdir::read_content(&workdir, rel)
                    .await
                    .map_err(vdfs::vdfs_from_plugin_error)?;
                Ok(vdfs::VdfsResponse::Read(vdfs::VdfsContent::text(text)))
            }
            vdfs::VdfsRequest::Write { content } => {
                if rel.is_empty() {
                    return Err(vdfs::VdfsError::invalid(format!("该路径不可写入：{path}")));
                }
                // 工作目录分支：文件写回（与列读同一份实现——路径越界校验 + 落盘）
                let workdir = self.workdir_of(id).await?;
                let text = content.text.as_deref().unwrap_or("");
                super::super::workdir::write_node(&workdir, rel, text)
                    .await
                    .map_err(vdfs::vdfs_from_plugin_error)?;
                self.workdir_watches.ensure_watch(&workdir, id);
                Ok(vdfs::VdfsResponse::Write(vdfs::VdfsWriteResponse {
                    name: None,
                    created: false,
                    etag: None,
                }))
            }
            vdfs::VdfsRequest::Delete { .. } => {
                if rel.is_empty() {
                    return Err(vdfs::VdfsError::Forbidden(format!(
                        "该路径不可删除：{path}"
                    )));
                }
                let workdir = self.workdir_of(id).await?;
                super::super::workdir::delete_node(&workdir, rel)
                    .await
                    .map_err(vdfs::vdfs_from_plugin_error)?;
                Ok(vdfs::VdfsResponse::Unit)
            }
            vdfs::VdfsRequest::Watch { sink } => {
                // 工作目录子树：接入文件系统监听（文件变化经**同一张订阅表**到达本
                // sink，见 `SessionPlugin::new` 注入的 `set_vdfs_subs`）
                if let Ok(workdir) = self.workdir_of(id).await {
                    self.workdir_watches.ensure_watch(&workdir, id);
                }
                self.change_subs.watch(path, sink);
                Ok(vdfs::VdfsResponse::Unit)
            }
            vdfs::VdfsRequest::Unwatch => {
                // 与 `watch` 严格配对：引用计数归零才真正摘掉
                if let Ok(workdir) = self.workdir_of(id).await {
                    self.workdir_watches.release_watch(&workdir, id);
                }
                self.change_subs.unwatch(path);
                Ok(vdfs::VdfsResponse::Unit)
            }
            _ => Err(vdfs::VdfsError::NotImplemented),
        }
    }

    /// 会话写入（`<id>` 具名覆盖 / 挂载根新建）——**同一份 upsert 实现**。
    ///
    /// 覆盖分支的浅合并与 `session/update` 路由**共用**
    /// [`Session::merge_metadata_object`]（不是"语义相同"，是同一份代码）。
    /// 新建分支另有一层优先级（路径名 → 显式 `title`）。
    async fn session_upsert(
        &self,
        path: &str,
        content: &vdfs::VdfsContent,
    ) -> vdfs::VdfsResult<vdfs::VdfsResponse> {
        let text = content.text.as_deref().unwrap_or("");
        let value: Value = if text.trim().is_empty() {
            json!({})
        } else {
            serde_json::from_str(text)
                .map_err(|e| vdfs::VdfsError::invalid(format!("会话写入需要合法 JSON：{e}")))?
        };
        let obj = value
            .as_object()
            .ok_or_else(|| vdfs::VdfsError::invalid("会话写入需要 JSON 对象"))?;

        // 寻址：**具名目标的地址末段就是会话 id**；写挂载根（无名目标）没有 id。
        // 会话 id 从来就是路径末段原样，这里只是把「怎么从地址得到 id」收成一处
        // ——两处各写一遍，改地址方案时必漏一边。
        let named = session_id_from_new_path(path);
        // 目标已存在吗？判据就用 `session_of`——磁盘 / 内存 / 嵌套三种驻留它都认，
        // 不另立一份「存在性探测」（那会与寻址逻辑分叉，且漏掉内存驻留）。
        // `NotFound` 之外的错误照实上抛：一次 IO 抖动不该被讲成「会话不存在」。
        let existing = match named.as_deref() {
            Some(id) => match self.session_of(id).await {
                Ok(session) => Some(session),
                Err(vdfs::VdfsError::NotFound(_)) => None,
                Err(e) => return Err(e),
            },
            None => None,
        };

        // 新建：`create` 意图 + 目标不存在。
        //
        // `create` **只回答「不存在时怎么办」**（`VdfsRequest::Write` 的 `create`
        // 位表）：目标已存在时它是**覆盖**，控制流因此落到下面的覆盖分支。
        // CLI 的「新建 / 改元数据」于是是同一次调用，正是旧 `session/update`
        // 的 upsert 语义。
        //
        // id 从哪来取决于**目标形态**：
        //
        // - **具名目标**（`<根>/session/<名字>`）：**就地创建**，名字就是 id
        //   ——这是 VDFS 的通用规则（`vdfs_service::entry::id_of` 同义：
        //   「有名字时 id 来自地址，没名字时 id 由 provider 生成」）。CLI 的
        //   「客户端指定会话 id」正是靠它表达，不再需要一条专用路由。
        // - **目录自身**（写挂载根，无名字）：名字由 provider 生成。这是前端的
        //   「新建会话」——它只说建在哪个目录，不说叫什么。
        if existing.is_none() && content.create {
            // 名字是不是**本插件生成的**——决定回执里要不要交回它
            // （见 [`vdfs::VdfsWriteResponse::name`]）。
            let anonymous = named.is_none();
            let id = match named {
                Some(id) => id,
                None => self.new_session_id().await,
            };
            let mut session = Session::new(&id);
            let mut meta = serde_json::Map::new();
            // 使用方给的 metadata（草稿态选择的 workdir / agent / model / mode…）。
            //
            // 这里**不**走 `merge_metadata_object`：新建是"建立初始 metadata"，
            // 与"往既有 metadata 上浅合并"不是同一件事——前者还要处理
            // `created_via` 缺省填充。而 `merge_metadata_object` 服务的是
            // **覆盖**分支的浅合并语义。
            if let Some(incoming) = obj.get("metadata").and_then(Value::as_object) {
                for (k, v) in incoming {
                    meta.insert(k.clone(), v.clone());
                }
            }
            // 标题只认**显式** `title`（或 metadata 里的 `title` 键）：
            // 名字已经是 id，拿它当标题会让 `--session cli18f3a2` 这类机器生成的
            // id 直接变成侧栏标题。无标题时由 `display_title` 从首条消息派生
            // （那条规则只有一处实现，使用方不预造）。
            if let Some(t) = obj.get("title").and_then(Value::as_str) {
                if !t.trim().is_empty() {
                    meta.insert("title".to_string(), Value::String(t.trim().to_string()));
                }
            }
            meta.entry("created_via".to_string())
                .or_insert_with(|| Value::String("vdfs".to_string()));
            session.metadata = Value::Object(meta);
            session.updated_at = clock_now_ms();
            self.save_session(&session)
                .await
                .map_err(vdfs::vdfs_from_plugin_error)?;
            self.notify_change(&id);
            return Ok(vdfs::VdfsResponse::Write(vdfs::VdfsWriteResponse {
                name: anonymous.then_some(id),
                created: true,
                etag: None,
            }));
        }

        // 覆盖：只接受 metadata / title 两类字段（其余字段无写入语义，明确拒绝，
        // 不静默丢弃使用方的意图）
        let has_title = obj.contains_key("title");
        let has_meta = obj.contains_key("metadata");
        if !has_title && !has_meta {
            return Err(vdfs::VdfsError::invalid(
                "会话写入支持 metadata / title 字段；消息请走聊天协议",
            ));
        }
        let id = named.ok_or_else(|| {
            vdfs::VdfsError::invalid("写会话挂载根需要 create 意图：目录自身没有可覆盖的目标")
        })?;
        // 前面已经取过一次（存在性判据），这里复用同一份，不重复读盘
        let mut session = match existing {
            Some(session) => session,
            None => self.session_of(&id).await?,
        };
        // 浅合并 —— `Session::merge_metadata_object` 是 metadata 写入的**唯一**实现。
        session.merge_metadata_object(&value);
        session.updated_at = clock_now_ms();
        self.save_session(&session)
            .await
            .map_err(vdfs::vdfs_from_plugin_error)?;
        // 资源变更（标题 / metadata）走粗粒度信号：消费方重拉清单收敛。
        // 不带节点视图，见 `symbio_core::vdfs_notify_change`。
        self.notify_change(&id);
        Ok(vdfs::VdfsResponse::Write(vdfs::VdfsWriteResponse {
            name: None,
            created: false,
            etag: None,
        }))
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
    async fn watch_at_path(&self, path: &str, sink: vdfs::VdfsChangeSink) -> vdfs::VdfsResult<()> {
        self.change_subs.watch(path, sink);
        Ok(())
    }

    /// 取消订阅：与 [`Self::watch_at_path`] 严格配对——引用计数归零才真正摘掉。
    async fn unwatch_at_path(&self, path: &str) -> vdfs::VdfsResult<()> {
        self.change_subs.unwatch(path);
        Ok(())
    }
}

impl SessionPlugin {
    /// 新会话 id —— 短 GUID（8 位十六进制）。
    ///
    /// **只在「目录自身」新建时用**（写挂载根，使用方没给名字）：具名目标的名字
    /// 就是身份，由地址给出，不走这里（见 `write` 的 `create` 分支）。
    ///
    /// ## 为什么是短 id
    ///
    /// 会话 id 会**直接出现在用户视野里**：它是 VDFS 的目录名（`.symbio/session/<id>`），
    /// 会话列表、地址栏、分享时都要读它。此前用 `Uuid::new_v4().to_string()`（36 字符
    /// 带连字符），既难读也难抄。
    ///
    /// 项目早已有一致的短 id 约定，这里只是不再例外：
    /// - `symbio_core::turn::llm_short_id()`（消息节点 id）
    /// - `vdfs_service::entry::auto_id()`（无名字新建的条目 id，`<kind>-<8位>`）
    ///
    /// 后端**没有任何地方** `Uuid::parse_str` 会话 id（已全仓核对），因此改格式安全；
    /// 已存在的长 id 会话照旧按原 id 寻址，不受影响。
    ///
    /// ## 碰撞
    ///
    /// 8 位十六进制 = 32 bit。桌面应用的会话量级（数百）碰撞概率可忽略，但 id
    /// 同时是**磁盘目录名**——撞上就是新建失败，属于用户可见错误。故生成后查一次
    /// 目录，命中则重摇（上限内），把概率问题变成确定性检查。
    async fn new_session_id(&self) -> String {
        let store = self.get_store().await.ok();
        for _ in 0..8 {
            let id = crate::symbio_core::llm_short_id();
            let taken = match &store {
                Some(s) => s.session_dir(&id).is_some(),
                None => false,
            };
            if !taken {
                return id;
            }
        }
        // 兜底：连续 8 次都撞（概率约 1e-67）时宁可长一点也要保证唯一
        uuid::Uuid::new_v4().to_string()
    }

    /// 会话转写（**含在途消息**）——`<根>/session/<id>/message` 的唯一数据源。
    ///
    /// 落库转写 ∪ 本轮在途缓冲。之所以要并上后者：流式期间消息**还没落库**
    /// （`persist_messages` 只在每轮结束时写盘），只读存储的话列表在流式期间
    /// 就是空的——`created` / `appended` 变更到达时消费者去 `list` 会一无所获，
    /// 「转写即列表」当场失效。
    ///
    /// 存在性校验在此一并完成（`session_of`），因此调用方不必再查一次。
    async fn transcript_of(&self, id: &str) -> vdfs::VdfsResult<Vec<cm::ChatMessage>> {
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

    // ==================== 消息的三个变更操作（VDFS 入口的实现） ====================
    //
    // 这三个方法是 `write(<id>/message/<mid>)` / `action(truncate)` /
    // `action(clear)` 的**唯一实现**。它们曾经各有一个专用路由
    // （`chat/update_message` / `chat/delete_message` / `chat/clear_messages`），
    // 逻辑就在那三个 invoke 里——迁到 VDFS 时整体搬过来，不是重写一遍：
    // 「同一个操作两份实现」正是本轮要消灭的东西。
    //
    // 三者的**变更发射都在这里**（不留给调用方）：漏发任何一条，VDFS 视图都会
    // 残留一个已不存在的节点且永不纠正。

    /// 改写单条消息（`write(<id>/message/<mid>)` 的实现）。
    ///
    /// 按**地址**定位（`mid` 即消息 id），只覆盖补丁里**提供**的字段
    /// （content / status / error / meta 等），未提供的保持不变。
    ///
    /// 补丁里的 `id` 若与地址不符即报错，不静默按地址写——调用方把消息发错了
    /// 节点是**它自己不知道的 bug**，静默写下去会把它埋掉。
    ///
    /// 补丁**不带** `id` 是合法用法：调用方 `write` 时已按地址补齐（见那里的注释），
    /// 因此到这里 `patch.id` 必然等于 `mid`，除非调用方**明确**写了一个别的 id。
    ///
    /// ## `content` 是**整体替换**，不是追加
    ///
    /// 本方法的调用方是在 VDFS 上**编辑一条已有消息**，期望的是整体替换。
    /// 流式逐帧累积走的是另一条路（帧上的 `delta` 窄增量），两者在**协议层**
    /// 就已分开，
    /// 因此这里不需要任何「合并模式」开关。
    ///
    /// （保存走 `replace_messages` 是安全的：其内部的 `assign_seq` 对**已带且单调**
    /// 的序号是原样沿用的，不会重排既有消息。）
    pub(crate) async fn patch_message(
        &self,
        session_id: &str,
        mid: &str,
        patch: &cm::ChatMessage,
    ) -> Result<cm::ChatMessage, PluginError> {
        if patch.id != mid {
            return Err(PluginError::ValidationError(format!(
                "消息补丁的 id（{}）与地址不符（{mid}）：地址是消息身份的唯一权威",
                patch.id
            )));
        }
        let chat_session = self.open_chat_session(session_id).await?;
        let mut messages = chat_session.get_messages().await?;

        let Some(existing) = messages.iter_mut().find(|m| m.id == mid) else {
            return Err(PluginError::NotFound(format!("消息不存在: {mid}")));
        };

        if let Some(role) = &patch.role {
            existing.role = Some(role.clone());
        }
        if let Some(t) = &patch.msg_type {
            existing.msg_type = Some(t.clone());
        }
        if let Some(n) = &patch.name {
            existing.name = Some(n.clone());
        }
        if let Some(p) = &patch.parent_id {
            existing.parent_id = Some(p.clone());
        }
        if let Some(c) = &patch.content {
            existing.content = Some(c.clone());
        }
        if let Some(s) = &patch.status {
            existing.status = Some(s.clone());
        }
        if let Some(e) = &patch.error {
            existing.error = Some(e.clone());
        } else if patch
            .status
            .as_ref()
            .map(|s| *s != cm::MessageStatus::Failed)
            .unwrap_or(false)
        {
            // 状态不再是 Failed 时，顺带清掉旧的 error，避免残留误导。
            existing.error = None;
        }
        if let Some(ts) = patch.timestamp {
            existing.timestamp = Some(ts);
        }
        if let Some(rid) = &patch.response_id {
            existing.response_id = Some(rid.clone());
        }
        if let Some(new_meta) = &patch.meta {
            match &mut existing.meta {
                Some(existing_meta) => {
                    if let (Some(a), Some(b)) =
                        (existing_meta.as_object_mut(), new_meta.as_object())
                    {
                        for (k, v) in b {
                            a.insert(k.clone(), v.clone());
                        }
                    } else {
                        existing.meta = Some(new_meta.clone());
                    }
                }
                None => {
                    existing.meta = Some(new_meta.clone());
                }
            }
        }

        let updated = existing.clone();
        chat_session.replace_messages(messages).await?;
        // 变更：一条**完整消息**帧——`content` 的语义是整条替换，正是"这次编辑"
        // 要说的事（不需要先删再建：删除帧表达的是"这个节点没了"，而编辑后它还在）。
        self.transcript_apply(session_id, crate::symbio_core::llm_message_frame(&updated))
            .await;
        Ok(updated)
    }

    /// 从某条消息起截断（`action(<id>/message/<mid>, "truncate")` 的实现）。
    ///
    /// 消息列表已按时间 / 顺序排好序，因此只需按列表顺序定位目标，然后把
    /// 「它及其之后的所有消息」整段 `drain` 掉——无需任何 `parent_id` 级联逻辑。
    /// 这样既保证会话消息的连续性（不会出现孤立的后半截助手回复），又足够简单直接。
    ///
    /// 目标**不存在**时返回空列表且**不发任何变更**：「什么都没删」不该在 VDFS 上
    /// 留下痕迹（发了 `truncated` 会让消费者从一条并不存在的节点起截断，把整个列表清空）。
    pub(crate) async fn truncate_messages(
        &self,
        session_id: &str,
        mid: &str,
    ) -> Result<Vec<String>, PluginError> {
        let chat_session = self.open_chat_session(session_id).await?;
        let mut messages = chat_session.get_messages().await?;

        let deleted_ids: Vec<String> = match messages.iter().position(|m| m.id == mid) {
            Some(i) => {
                let removed: Vec<String> = messages[i..].iter().map(|m| m.id.clone()).collect();
                messages.drain(i..);
                removed
            }
            None => Vec::new(),
        };

        chat_session.replace_messages(messages).await?;
        // 变更：转写的**尾部区间**没了——逐条删除帧（`status = removed`）。协议里
        // 没有「清空重读」这种形态，而让消费端整份重读会把**保留的**消息也重传
        // 一遍：对"删掉若干条"这个动作，逐条通知恰好是更便宜的形态。回执里的
        // `deleted_ids` 是权威列表：调用方据此幂等对齐本地视图，不依赖推送。
        if !deleted_ids.is_empty() {
            let frames: Vec<cm::ChatMessage> = deleted_ids
                .iter()
                .map(|id| crate::symbio_core::llm_removed_frame(id))
                .collect();
            self.transcript_apply_all(session_id, frames).await;
        }
        Ok(deleted_ids)
    }

    /// 清空会话消息（`action(<id>/message, "clear")` 的实现）。
    ///
    /// 与 `delete(<id>)`（删除整个会话）不同：这里只把 `session.messages` 整体替换为
    /// 空，会话本体 / 元数据 / 工作目录 / 标题继续存在。UI 的「清空历史」走此路径。
    pub(crate) async fn clear_messages(&self, session_id: &str) -> Result<(), PluginError> {
        let chat_session = self.open_chat_session(session_id).await?;
        // 先取 id 再清空（清空后读回的是空列表，顺序反了就删无可发）。
        let ids: Vec<String> = chat_session
            .get_messages()
            .await?
            .iter()
            .map(|m| m.id.clone())
            .collect();
        chat_session.replace_messages(Vec::new()).await?;
        // 变更：清空 = 逐条删除帧（理由见 truncate：清空重读会连保留的一起重传）。
        let frames: Vec<cm::ChatMessage> = ids
            .iter()
            .map(|id| crate::symbio_core::llm_removed_frame(id))
            .collect();
        self.transcript_apply_all(session_id, frames).await;
        Ok(())
    }

    /// 按 id 取会话（**存在性校验**：`load_session` 对未命中会返回空会话，
    /// 因此这里走带存在性判据的 `load_session_checked`）
    pub(crate) async fn session_of(&self, id: &str) -> vdfs::VdfsResult<Session> {
        self.get_store()
            .await
            .map_err(vdfs::vdfs_from_plugin_error)?
            .load_session_checked(id)
            .await
            .map_err(vdfs::vdfs_from_plugin_error)?
            .ok_or_else(|| vdfs::VdfsError::not_found(format!("会话不存在：{id}")))
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
    ) -> Vec<vdfs::VdfsNode> {
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
    async fn workdir_of(&self, id: &str) -> vdfs::VdfsResult<String> {
        let session = self.session_of(id).await?;
        super::super::workdir::workdir_of(&session)
            .ok_or_else(|| vdfs::VdfsError::not_found(format!("会话 {id} 没有工作目录")))
    }

    /// 会话记忆 → VDFS 节点。
    ///
    /// 形状由共享实现 [`MemoryFile::node`](crate::providers::MemoryFile::node) 产出，
    /// `list`（经 `internal_dirs`）与 `stat`
    /// **共用同一份**——「列表里的和点开的不是同一个东西」这类 bug 因此写不出来。
    async fn memory_node_of(&self, id: &str) -> vdfs::VdfsNode {
        self.memory_store(id)
            .await
            .node(&crate::providers::MemoryNodeSpec {
                title: super::super::memory::SEGMENT_TITLE,
                kind: PLUGIN_ID_SESSION,
                description: super::super::memory::MEMORY_DESCRIPTION,
            })
    }

    /// 取子会话（**归属校验**：必须确实挂在 `id` 之下，避免跨会话越权访问）
    async fn sub_session_of(
        &self,
        id: &str,
        sub: &str,
    ) -> vdfs::VdfsResult<super::super::types::Session> {
        let store = self
            .get_store()
            .await
            .map_err(vdfs::vdfs_from_plugin_error)?;
        let session = store
            .load_session(sub)
            .await
            .map_err(vdfs::vdfs_from_plugin_error)?;
        if session.id == sub && session.parent_session_id() == Some(id) {
            Ok(session)
        } else {
            Err(vdfs::VdfsError::not_found(format!(
                "会话 {id} 下不存在子会话 {sub}"
            )))
        }
    }
}

#[cfg(test)]
#[path = "vdfs_provider.test.rs"]
mod tests;
