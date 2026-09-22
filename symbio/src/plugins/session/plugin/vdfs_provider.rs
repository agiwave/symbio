//! `impl VdfsProvider for SessionPlugin` 及其**私有辅助**。
//!
//! 自 `plugin.rs` 原样搬移（拆文件不拆行为）。职责：
//! - 实现 VDFS provider 的读写删改查（`list` / `stat` / `read` / `write` / `delete` /
//!   `create` / `watch` / `unwatch`）——全部**转发既有会话能力**
//!   （SessionStore + 会话 metadata 合并），不新造协议；
//! - 只读辅助：转写（含在途消息）、存在性校验、实时工作状态、工作目录、子会话。

use super::*;
use crate::symbio_core::now_ms;

#[async_trait]
impl vdfs::VdfsProvider for SessionPlugin {
    fn label(&self) -> Option<&str> {
        // 标签 / 顺序由本 provider 自持（实体注册表已随 VDFS 收敛下线）
        Some("会话")
    }

    fn description(&self) -> Option<&str> {
        Some("会话清单。每个会话是一份独立对话记录，可读 / 写 / 删；新建即创建一份新会话。")
    }

    fn order(&self) -> i32 {
        1
    }

    fn icon(&self) -> Option<&str> {
        Some("session")
    }

    /// 会话是叶子文档：可列，不参与树遍历（会话内部的子结构另有容器语义）
    fn root_access(&self) -> vdfs::VdfsAccess {
        vdfs::VdfsAccess::LIST
    }

    /// 根下可新建「会话」（新建语义由 provider 自持，见 `write`）
    fn root_new_types(&self) -> Vec<vdfs::VdfsNewType> {
        vec![vdfs::VdfsNewType::new(vdfs::VDFS_EXT_SESSION, "会话").with_description("新建会话")]
    }

    async fn list(
        &self,
        ctx: &vdfs::VdfsContext,
        path: &str,
    ) -> vdfs::VdfsResult<Vec<vdfs::VdfsNode>> {
        // 配置文件优先：`PLUGIN.yml` 是文档，不是会话
        if path == PLUGIN_FILE {
            return Err(vdfs::VdfsError::not_found(format!(
                "配置文件是文档，没有子项：{path}"
            )));
        }
        let (limit, before) = window_params(ctx);
        match parse_session_path(path)? {
            VdfsSessionPath::Root => {
                // 清单里**只有会话**——配置文件是文档，它进设置菜单走的是
                // ConfigurableVisitor 那条通道，不该在会话清单里再出现一次
                // （否则列表底部会多一个「设置」项）。
                let sessions = self
                    .list_sessions_window(limit, before)
                    .await
                    .map_err(vdfs::from_plugin_error)?;
                Ok(self.nodes_of_sessions(&sessions).await)
            }
            // 会话内部：三个虚拟子目录 + 记忆文件（会话存在性校验由 `session_of` 承担）
            VdfsSessionPath::Session(id) => {
                let session = self.session_of(id).await?;
                Ok(internal_dirs(
                    super::super::workdir::workdir_of(&session).is_some(),
                    self.memory_node_of(id).await,
                ))
            }
            // 记忆是**单个文件**：没有子项
            VdfsSessionPath::Memory(_) => Err(vdfs::VdfsError::not_found(format!(
                "记忆是文件，没有子项：{path}"
            ))),
            // 转写列表：每一条消息是一个列表项，顺序由 `seq` 决定。
            // 含**在途**消息——流式期间列表就是活的，不必等落库。
            VdfsSessionPath::Messages { id, mid: None } => {
                let msgs = self.transcript_of(id).await?;
                // 有界窗口：**只在调用方显式给参数时**生效。不给参数 = 全量，
                // 与从前逐字节一致（流式期间前端要的是完整列表）。
                Ok(transcript_window(&msgs, limit, before)
                    .iter()
                    .map(message_node)
                    .collect())
            }
            VdfsSessionPath::Messages { mid: Some(_), .. } => Err(vdfs::VdfsError::not_found(
                format!("消息是列表项，没有子项：{path}"),
            )),
            VdfsSessionPath::SubSessions(id) => {
                self.session_of(id).await?;
                let store = self.get_store().await.map_err(vdfs::from_plugin_error)?;
                let subs = store
                    .list_sub_sessions(id)
                    .await
                    .map_err(vdfs::from_plugin_error)?;
                Ok(self.nodes_of_sessions(&subs).await)
            }
            VdfsSessionPath::SubSession { .. } => Err(vdfs::VdfsError::not_found(format!(
                "子会话是叶子节点，没有子项：{path}"
            ))),
            VdfsSessionPath::Workdir { id, rel } => {
                let workdir = self.workdir_of(id).await?;
                // 目录树场景的实时监听（按会话 id 引用计数）
                self.workdir_watches.ensure_watch(&workdir, id);
                super::super::workdir::list_children(&workdir, Some(rel))
                    .await
                    .map_err(vdfs::from_plugin_error)
            }
        }
    }

    async fn stat(&self, _ctx: &vdfs::VdfsContext, path: &str) -> vdfs::VdfsResult<vdfs::VdfsNode> {
        // 配置文件优先：`PLUGIN.yml` 是文档，不是会话
        if path == PLUGIN_FILE {
            return Ok(self.config_file.node());
        }
        match parse_session_path(path)? {
            VdfsSessionPath::Root => {
                // 自身根：**名字留空**——provider 不知道自己的挂载名，由使用方回填
                Ok(vdfs::VdfsNode::dir("", "会话", self.root_access()))
            }
            // `<id>` 有二重身份：清单里是**叶子**（`session_node`，`rw`，
            // 点开 = 聊天详情），被当作目录访问时则是**会话内部**的目录视图。
            // 这里按后者回答——`stat` 的结果由分发层用作「当前目录节点」，
            // 其访问位直接决定页面是否给出新建入口；会话内部不支持
            // 新建 / 建目录，故只声明 `l`。
            VdfsSessionPath::Session(id) => {
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
                Ok(n)
            }
            // 记忆：单个文件，形状由内核产出（`list` 与 `stat` 同源）
            VdfsSessionPath::Memory(id) => {
                // 存在性校验：会话不在，就没有「它的记忆」可谈
                self.session_of(id).await?;
                Ok(self.memory_node_of(id).await)
            }
            VdfsSessionPath::Messages { id, mid } => match mid {
                // 列表本身是个目录（`l` 位：可列，不参与树遍历——会话整体是叶子）。
                // 只需存在性校验，不必取全量转写。
                None => {
                    self.session_of(id).await?;
                    Ok(vdfs::VdfsNode::dir(
                        SEG_MESSAGES,
                        SEG_MESSAGES,
                        vdfs::VdfsAccess::LIST,
                    ))
                }
                Some(mid) => Ok(message_node(message_of(
                    &self.transcript_of(id).await?,
                    mid,
                )?)),
            },
            VdfsSessionPath::SubSessions(id) => {
                self.session_of(id).await?;
                Ok(vdfs::VdfsNode::dir(
                    super::super::workdir::SEG_SUB_SESSIONS,
                    super::super::workdir::SEG_SUB_SESSIONS,
                    vdfs::VdfsAccess::LIST,
                ))
            }
            VdfsSessionPath::SubSession { id, sub } => {
                let session = self.sub_session_of(id, sub).await?;
                Ok(session_node(
                    &SessionSummary::of(&session),
                    &self.session_runtime(sub).await,
                ))
            }
            VdfsSessionPath::Workdir { id, rel } => {
                let workdir = self.workdir_of(id).await?;
                if rel.is_empty() {
                    return Ok(vdfs::VdfsNode::dir(
                        super::super::workdir::SEG_WORKDIR,
                        super::super::workdir::SEG_WORKDIR,
                        vdfs::VdfsAccess::LIST,
                    ));
                }
                super::super::workdir::read_node(&workdir, rel)
                    .await
                    .map_err(vdfs::from_plugin_error)
            }
        }
    }

    /// 读取会话内容（转写全文 + 元数据，JSON）——转发既有存储，不新造协议
    async fn read(
        &self,
        _ctx: &vdfs::VdfsContext,
        path: &str,
    ) -> vdfs::VdfsResult<vdfs::VdfsContent> {
        // 配置文件优先：`PLUGIN.yml` 是文档，不是会话
        if path == PLUGIN_FILE {
            return self.config_file.read(&self.config).await;
        }
        match parse_session_path(path)? {
            VdfsSessionPath::Session(id) => {
                let session = self.session_of(id).await?;
                // 含在途：叶子与转写列表必须是同一份消息集合，否则前端
                // `loadMessages`（读叶子）会在流式期间看不到正在跑的那一轮。
                // 详见 `session_content` 的文档。
                let live = self.live_messages_of(id).await;
                session_content(&session, live)
            }
            // 会话记忆（`<根>/session/<id>/AGENTS.md`）：正文即文件全文。
            // 文件不存在 → 空串（不是错误）——「还没写过」是记忆的正常状态。
            VdfsSessionPath::Memory(id) => {
                self.session_of(id).await?;
                let text = self
                    .memory_store(id)
                    .await
                    .read()
                    .map_err(vdfs::VdfsError::internal)?;
                Ok(vdfs::VdfsContent::text("", text))
            }
            // 单条消息的正文（列表项内容）——流式追加的正是它。
            // 同样取含在途的转写：追加型变更的消费者读到的必须是**已含该增量**的正文。
            VdfsSessionPath::Messages { id, mid: Some(mid) } => {
                let msgs = self.transcript_of(id).await?;
                Ok(vdfs::VdfsContent::text(
                    "",
                    message_text(message_of(&msgs, mid)?),
                ))
            }
            VdfsSessionPath::SubSession { id, sub } => {
                let session = self.sub_session_of(id, sub).await?;
                // 子会话是**独立会话**（有自己的 id 与活跃状态），因此叠加的是它
                // 自己的在途缓冲，而不是父会话的——父会话的在途消息不属于它。
                let live = self.live_messages_of(&session.id).await;
                session_content(&session, live)
            }
            VdfsSessionPath::Workdir { id, rel } if !rel.is_empty() => {
                let workdir = self.workdir_of(id).await?;
                let text = super::super::workdir::read_content(&workdir, rel)
                    .await
                    .map_err(vdfs::from_plugin_error)?;
                Ok(vdfs::VdfsContent::text("", text))
            }
            // 目录（挂载根 / 会话内部区段 / 工作目录子目录）无「内容」语义
            _ => Err(vdfs::VdfsError::invalid(format!(
                "该路径不可读取内容：{path}"
            ))),
        }
    }

    /// 写入：`create` → 新建会话；`<id>/消息/<mid>` → 改写该条消息；否则 → 合并会话
    /// metadata（`session/update` 语义）。
    ///
    /// 覆盖分支的浅合并与 `session/update` 路由**共用**
    /// [`Session::merge_metadata_object`]（不是"语义相同"，是同一份代码）。
    /// 新建分支另有一层优先级（路径名 → 显式 `title`），见下方注释。
    ///
    /// ## 消息节点可写，但**转写列表不可写**（这条区分是刻意的）
    ///
    /// - `write(<id>/消息/<mid>)` —— 改**既有**消息的字段。它不是一次发言：不触发
    ///   任何编排，只是对既有节点的一次存储改写，与会话 metadata 的写入同类
    ///   （两者都只是"把内容存到那个地址"）。地址语义在这里完全成立：消息节点的
    ///   内容就是这条消息。
    /// - `write(<id>/消息)` —— **拒绝**。往列表里放一条新消息 = 发言 = 一次**动作**
    ///   （触发一整轮编排：模型调用 → 工具执行 → 流式落库），不是一次写入。
    ///   入口仍然只有聊天协议一处。
    /// - `create` 意图在消息路径上**一律拒绝**：新建消息就是发言，静默接受会把
    ///   「追加消息不经 VDFS」这条不变量悄悄破掉。
    async fn write(
        &self,
        _ctx: &vdfs::VdfsContext,
        path: &str,
        content: &vdfs::VdfsContent,
    ) -> vdfs::VdfsResult<vdfs::VdfsWriteResponse> {
        // 配置文件优先：`PLUGIN.yml` 是文档，不是会话
        if path == PLUGIN_FILE {
            return self.config_file.apply(&self.config, content).await;
        }
        // 工作目录分支：文件写回（与列读同一份实现——路径越界校验 + 落盘）
        match parse_session_path(path)? {
            VdfsSessionPath::Workdir { id, rel } if !rel.is_empty() => {
                let workdir = self.workdir_of(id).await?;
                let text = content.text.as_deref().unwrap_or("");
                super::super::workdir::write_node(&workdir, rel, text)
                    .await
                    .map_err(vdfs::from_plugin_error)?;
                self.workdir_watches.ensure_watch(&workdir, id);
                return Ok(vdfs::VdfsWriteResponse {
                    path: path.to_string(),
                    created: false,
                    etag: None,
                });
            }
            // 会话记忆：**纯文本**写入（容量闸门在内核 `MemoryFile::write`，本插件不重复实现）
            VdfsSessionPath::Memory(id) => {
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
                // 挂载名由容器的 watch 包装补上——见本文件 `watch` 的说明）
                self.change_subs.notify(&vdfs::VdfsChange::new(
                    super::super::memory::memory_rel_path(id),
                    if existed {
                        vdfs::VDFS_CHANGE_UPDATED
                    } else {
                        vdfs::VDFS_CHANGE_CREATED
                    },
                ));
                return Ok(vdfs::VdfsWriteResponse {
                    path: path.to_string(),
                    created: !existed,
                    etag: None,
                });
            }
            // 会话本身（`path` 即会话 id；新建时是 `<标题>.session`）
            VdfsSessionPath::Session(_) => {}
            // 挂载根 = 「新建一个会话，名字由 provider 生成」。
            //
            // 写目录自身没有可覆盖的目标，因此 `create` 是唯一合法意图
            // （见 `VdfsProvider::write` 的两种目标形态）；缺它即报错，
            // 不静默落成「一次无意义的写」。
            VdfsSessionPath::Root => {
                if !content.create {
                    return Err(vdfs::VdfsError::invalid(
                        "写会话挂载根需要 create 意图：目录自身没有可覆盖的目标",
                    ));
                }
            }
            // 单条消息：改写既有消息的字段（**不是发言**——不触发编排）
            VdfsSessionPath::Messages { id, mid: Some(mid) } => {
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
                    .map_err(vdfs::from_plugin_error)?;
                return Ok(vdfs::VdfsWriteResponse {
                    path: path.to_string(),
                    created: false,
                    etag: None,
                });
            }
            // 转写列表本身：**仍只读**。往里放一条 = 发言 = 动作，不是写入。
            VdfsSessionPath::Messages { mid: None, .. } => {
                return Err(vdfs::VdfsError::Forbidden(
                    "转写列表只读：发言请走聊天协议（一次发言触发一整轮编排）".to_string(),
                ));
            }
            _ => return Err(vdfs::VdfsError::invalid(format!("该路径不可写入：{path}"))),
        }

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

        // 新建：**id 由 provider 生成**（它是存储细节，不属于使用方的知识）。
        // 名字（若有）在路径末段里——写挂载根时没有名字，标题留给
        // `display_title` 从首条消息派生（那条规则只有一处实现，使用方不预造）。
        if content.create {
            let id = self.new_session_id().await;
            let mut session = Session::new(&id);
            let mut meta = serde_json::Map::new();
            // 使用方给的 metadata（草稿态选择的 workdir / agent / model / mode…）。
            //
            // 这里**不**走 `merge_metadata_object`：新建是"建立初始 metadata"，
            // 与"往既有 metadata 上浅合并"不是同一件事——前者还要处理路径名与
            // 显式 title 的优先级、`created_via` 缺省填充。而 `merge_metadata_object`
            // 服务的是**两条**路径（`session/update` 与 `write` 的覆盖分支）之间
            // 的一致性，那才是会漂移的一对。
            if let Some(incoming) = obj.get("metadata").and_then(Value::as_object) {
                for (k, v) in incoming {
                    meta.insert(k.clone(), v.clone());
                }
            }
            // 具名新建（`<名字>.session`）→ 名字作标题；写目录自身（无名字）→ 不写标题
            if !path.trim_matches('/').is_empty() {
                meta.insert(
                    "title".to_string(),
                    Value::String(title_from_new_path(path)),
                );
            }
            // 显式 title 优先于路径名（使用方可以只给 title，不给 metadata）
            if let Some(t) = obj.get("title").and_then(Value::as_str) {
                if !t.trim().is_empty() {
                    meta.insert("title".to_string(), Value::String(t.trim().to_string()));
                }
            }
            meta.entry("created_via".to_string())
                .or_insert_with(|| Value::String("vdfs".to_string()));
            session.metadata = Value::Object(meta);
            session.updated_at = now_ms();
            self.save_session(&session)
                .await
                .map_err(vdfs::from_plugin_error)?;
            self.notify_change(&id, vdfs::VDFS_CHANGE_CREATED);
            return Ok(vdfs::VdfsWriteResponse {
                path: id,
                created: true,
                etag: None,
            });
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
        let mut session = self.session_of(path).await?;
        // 浅合并 —— 与 `session/update` 路由（CLI 用）**同一份实现**。
        // 两处各写一遍的话，「前端改名生效 / CLI 改名不生效」这类只在一条路径上
        // 出现的行为差异迟早会发生，而没有任何测试会覆盖两条路径的一致性。
        session.merge_metadata_object(&value);
        session.updated_at = now_ms();
        self.save_session(&session)
            .await
            .map_err(vdfs::from_plugin_error)?;
        // 资源变更（标题 / metadata）走粗粒度信号：消费方重拉清单收敛。
        // 不带节点视图——会话叶子的节点快照只有转写流（有序）与 `list` / `stat`
        // （回读）两个来源，见 `plugin::notify_change`。
        self.notify_change(path, vdfs::VDFS_CHANGE_UPDATED);
        Ok(vdfs::VdfsWriteResponse {
            path: path.to_string(),
            created: false,
            etag: None,
        })
    }

    async fn delete(
        &self,
        _ctx: &vdfs::VdfsContext,
        path: &str,
        _recursive: bool,
    ) -> vdfs::VdfsResult<()> {
        // 配置文件恒在，不可删
        if path == PLUGIN_FILE {
            return Err(vdfs::VdfsError::Forbidden("配置文件不可删除".to_string()));
        }
        match parse_session_path(path)? {
            VdfsSessionPath::Root => {
                Err(vdfs::VdfsError::Forbidden("会话挂载根不可删除".to_string()))
            }
            VdfsSessionPath::Session(id) => {
                // 存在性校验：删除不存在的会话应报 NotFound 而非静默成功
                self.session_of(id).await?;
                self.delete_session_internal(id)
                    .await
                    .map_err(vdfs::from_plugin_error)
            }
            // 子会话：先 abort 活跃任务再删（`delete_session_internal` 同语义）
            VdfsSessionPath::SubSession { id, sub } => {
                self.sub_session_of(id, sub).await?;
                self.delete_session_internal(sub)
                    .await
                    .map_err(vdfs::from_plugin_error)
            }
            VdfsSessionPath::Workdir { id, rel } if !rel.is_empty() => {
                let workdir = self.workdir_of(id).await?;
                super::super::workdir::delete_node(&workdir, rel)
                    .await
                    .map_err(vdfs::from_plugin_error)
            }
            // 记忆**不可删除**（与 work / agent 的记忆层同一内核约定）：
            // 删除即丢失本会话的长期约定，且没有东西能把它找回来。要清空就写入空内容
            // ——那是一次可读、可审、可撤销的显式动作。
            VdfsSessionPath::Memory(_) => Err(vdfs::VdfsError::Forbidden(format!(
                "会话记忆不可删除（删除即丢失本会话的长期约定）。\
                 如需清空，请向 `{}` 写入空内容。",
                crate::symbio_core::AGENTS_FILE
            ))),
            // 转写区段（列表与单条）**不可 delete**：它的两种删除语义各有自己的动作，
            // 而 `delete` 的全局语义是「**这一个**节点没了」（→ `deleted` 变更）。
            // 「从这里到末尾全没了」是 `truncated`（落在起始消息上）、「整个列表空了」
            // 是 `deleted`（落在列表目录上）——动作不共用一个动词，
            // 见 `VDFS_ACTION_TRUNCATE` 的文档。
            VdfsSessionPath::Messages { .. } => Err(vdfs::VdfsError::Forbidden(format!(
                "转写区段不可 delete：清空列表请用 action(\"{clear}\")，\
                 删除某条及其之后请用 action(\"{truncate}\")：{path}",
                clear = vdfs::VDFS_ACTION_CLEAR,
                truncate = vdfs::VDFS_ACTION_TRUNCATE,
            ))),
            _ => Err(vdfs::VdfsError::Forbidden(format!(
                "该路径不可删除：{path}"
            ))),
        }
    }

    /// 节点动作：转写区段的两种**集合操作**。
    ///
    /// | 路径 | 动作 | 语义 | 变更 | `data` |
    /// |---|---|---|---|---|
    /// | `<id>/消息/<mid>` | [`VDFS_ACTION_TRUNCATE`] | 该条**及其之后**全部没了 | 转写流逐条删除帧（`status = removed`） | 被删 id 列表 |
    /// | `<id>/消息` | [`VDFS_ACTION_CLEAR`] | 列表清空（会话本体保留） | 转写流逐条删除帧 | 无 |
    ///
    /// ## 变更为什么落在转写流上，而不是 VDFS 变更
    ///
    /// 消息的变更面只有一条通道（`session/stream` 的消息帧，见
    /// `symbio_core::transcript_stream`）；VDFS 侧只剩会话节点运行态与记忆文件。
    /// 两种集合操作都逐条发删除帧——删除帧是**元数据**
    /// （id + 状态，每条几十字节），而一次 `reset`（清空 + 从存储整份重读）
    /// 会把所有**保留的**消息都重传一遍：对「删几条」这个动作，逐条通知
    /// 恰恰是更便宜的形态。**权威的被删 id 列表走回执 `data`**：调用方据此
    /// 幂等对齐，不依赖任何推送。
    ///
    /// 为什么是动作而不是 `delete`：见 [`VDFS_ACTION_TRUNCATE`] 的文档
    /// （`delete` 是**逐节点**语义，表达不了"删一个节点却删掉了它后面所有"）。
    ///
    /// 其它路径 / 未实现的动作一律 [`VdfsError::NotImplemented`]——消费方据此
    /// 不给出入口，而不是收到一个"成功但什么都没做"。
    async fn action(
        &self,
        _ctx: &vdfs::VdfsContext,
        path: &str,
        action: &str,
        _payload: Option<&Value>,
    ) -> vdfs::VdfsResult<vdfs::VdfsActionResult> {
        match parse_session_path(path)? {
            VdfsSessionPath::Messages { id, mid: Some(mid) }
                if action == vdfs::VDFS_ACTION_TRUNCATE =>
            {
                let deleted_ids = self
                    .truncate_messages(id, mid)
                    .await
                    .map_err(vdfs::from_plugin_error)?;
                // 回执带**权威**的被删 id 列表：消费方本地若因锚点缺失而删窄了，
                // 据它补齐（`vdfs/delete` 只回 `{path}`，带不回这个）。
                let data = serde_json::to_value(&deleted_ids)
                    .map_err(|e| vdfs::VdfsError::internal(format!("截断回执序列化失败：{e}")))?;
                let message = if deleted_ids.is_empty() {
                    // 目标不存在 ⇒ 什么都没删。这是**结果**不是错误，但也不发变更
                    // ——「什么都没删」不该在 VDFS 上留下痕迹（发了 `truncated`
                    // 会让消费者从一条并不存在的节点起截断，把整个列表清空）。
                    format!("目标消息不存在，未做任何修改：{mid}")
                } else {
                    format!("已从 {mid} 起截断 {} 条消息", deleted_ids.len())
                };
                Ok(vdfs::VdfsActionResult {
                    action: action.to_string(),
                    ok: true,
                    message,
                    data: Some(data),
                })
            }
            VdfsSessionPath::Messages { id, mid: None } if action == vdfs::VDFS_ACTION_CLEAR => {
                self.clear_messages(id)
                    .await
                    .map_err(vdfs::from_plugin_error)?;
                Ok(vdfs::VdfsActionResult {
                    action: action.to_string(),
                    ok: true,
                    message: "已清空会话历史（会话本身与元数据保留）".to_string(),
                    data: None,
                })
            }
            _ => Err(vdfs::VdfsError::NotImplemented),
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
    /// 同一坐标系：`<id>`、`<id>/消息/<mid>`），本方法**原样登记**、不做任何
    /// 前缀处理。
    ///
    /// 看起来「应该」把它收敛成相对被订阅 `path` 的路径，但那样反而会错：
    /// 容器的 `watch` 包装器（`CompositeVdfs::watch`）只补**挂载名**（首段），
    /// 不是补被订阅的整条路径——它把 `(dir, rel)` 拆开，`rel` 传给了 provider，
    /// 回填时只加 `dir`。因此 provider 报出的路径必须仍是 provider 根口径，
    /// 容器补上挂载名后才是树内全路径：
    ///
    /// ```text
    /// watch("session/abc/消息") → dir = "session", rel = "abc/消息"
    /// provider 报 "abc/消息/m1" → 容器补成 "session/abc/消息/m1"  ✓
    /// 若这里先剥成 "m1"        → 容器补成 "session/m1"          ✗
    /// ```
    ///
    /// 代价是订阅者会收到兄弟子树的变更（订阅一个会话会看到别的会话的事件）；
    /// 消费者按路径前缀自行过滤即可，正确性不受影响。
    async fn watch(
        &self,
        _ctx: &vdfs::VdfsContext,
        path: &str,
        sink: vdfs::VdfsChangeSink,
    ) -> vdfs::VdfsResult<()> {
        // 工作目录子树：接入文件系统监听（文件变化经**同一张订阅表**到达本
        // sink，见 `SessionPlugin::new` 注入的 `set_vdfs_subs`）
        if let Ok(VdfsSessionPath::Workdir { id, .. }) = parse_session_path(path) {
            if let Ok(workdir) = self.workdir_of(id).await {
                self.workdir_watches.ensure_watch(&workdir, id);
            }
        }
        self.change_subs.watch(path, sink);
        Ok(())
    }

    async fn unwatch(&self, _ctx: &vdfs::VdfsContext, path: &str) -> vdfs::VdfsResult<()> {
        // 与 `watch` 严格配对：引用计数归零才真正摘掉
        if let Ok(VdfsSessionPath::Workdir { id, .. }) = parse_session_path(path) {
            if let Ok(workdir) = self.workdir_of(id).await {
                self.workdir_watches.release_watch(&workdir, id);
            }
        }
        self.change_subs.unwatch(path);
        Ok(())
    }
}

impl SessionPlugin {
    /// 新会话 id —— 短 GUID（8 位十六进制）。
    ///
    /// ## 为什么是短 id
    ///
    /// 会话 id 会**直接出现在用户视野里**：它是 VDFS 的目录名（`.symbio/session/<id>`），
    /// 会话列表、地址栏、分享时都要读它。此前用 `Uuid::new_v4().to_string()`（36 字符
    /// 带连字符），既难读也难抄。
    ///
    /// 项目早已有一致的短 id 约定，这里只是不再例外：
    /// - `symbio_core::turn::short_id()`（消息节点 id）
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
            let id = crate::symbio_core::turn::short_id();
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

    /// 会话转写（**含在途消息**）——`<根>/session/<id>/消息` 的唯一数据源。
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
    // 这三个方法是 `write(<id>/消息/<mid>)` / `action(truncate)` /
    // `action(clear)` 的**唯一实现**。它们曾经各有一个专用路由
    // （`chat/update_message` / `chat/delete_message` / `chat/clear_messages`），
    // 逻辑就在那三个 invoke 里——迁到 VDFS 时整体搬过来，不是重写一遍：
    // 「同一个操作两份实现」正是本轮要消灭的东西。
    //
    // 三者的**变更发射都在这里**（不留给调用方）：漏发任何一条，VDFS 视图都会
    // 残留一个已不存在的节点且永不纠正。

    /// 改写单条消息（`write(<id>/消息/<mid>)` 的实现）。
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
        self.transcript_apply(
            session_id,
            crate::symbio_core::turn::message_frame(&updated),
        )
        .await;
        Ok(updated)
    }

    /// 从某条消息起截断（`action(<id>/消息/<mid>, "truncate")` 的实现）。
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
                .map(|id| crate::symbio_core::turn::removed_frame(id))
                .collect();
            self.transcript_apply_all(session_id, frames).await;
        }
        Ok(deleted_ids)
    }

    /// 清空会话消息（`action(<id>/消息, "clear")` 的实现）。
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
            .map(|id| crate::symbio_core::turn::removed_frame(id))
            .collect();
        self.transcript_apply_all(session_id, frames).await;
        Ok(())
    }

    /// 按 id 取会话（**存在性校验**：`load_session` 对未命中会返回空会话，
    /// 因此这里走带存在性判据的 `load_session_checked`）
    pub(crate) async fn session_of(&self, id: &str) -> vdfs::VdfsResult<Session> {
        self.get_store()
            .await
            .map_err(vdfs::from_plugin_error)?
            .load_session_checked(id)
            .await
            .map_err(vdfs::from_plugin_error)?
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
    async fn nodes_of_sessions(&self, sessions: &[SessionSummary]) -> Vec<vdfs::VdfsNode> {
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
                session_node(s, &rt)
            })
            .collect()
    }

    /// 会话的工作目录（未声明 workdir ⇒ 该会话无目录树能力）
    async fn workdir_of(&self, id: &str) -> vdfs::VdfsResult<String> {
        let session = self.session_of(id).await?;
        super::super::workdir::workdir_of(&session)
            .ok_or_else(|| vdfs::VdfsError::not_found(format!("会话 {id} 没有工作目录")))
    }

    /// 会话记忆 → VDFS 节点。
    ///
    /// 形状由内核 [`MemoryFile::node`] 产出，`list`（经 `internal_dirs`）与 `stat`
    /// **共用同一份**——「列表里的和点开的不是同一个东西」这类 bug 因此写不出来。
    async fn memory_node_of(&self, id: &str) -> vdfs::VdfsNode {
        self.memory_store(id)
            .await
            .node(&crate::symbio_core::NodeSpec {
                title: super::super::memory::SEGMENT_TITLE,
                kind: PLUGIN_SESSION,
                description: super::super::memory::MEMORY_DESCRIPTION,
            })
    }

    /// 取子会话（**归属校验**：必须确实挂在 `id` 之下，避免跨会话越权访问）
    async fn sub_session_of(
        &self,
        id: &str,
        sub: &str,
    ) -> vdfs::VdfsResult<super::super::types::Session> {
        let store = self.get_store().await.map_err(vdfs::from_plugin_error)?;
        let session = store
            .load_session(sub)
            .await
            .map_err(vdfs::from_plugin_error)?;
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
