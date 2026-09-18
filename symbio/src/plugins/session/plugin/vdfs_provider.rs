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
            // 会话记忆（`.vdfs/session/<id>/AGENTS.md`）：正文即文件全文。
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

    /// 写入：`create` → 新建会话；否则 → 合并会话 metadata（`session/update` 语义）。
    ///
    /// **转写列表（`<id>/消息`）只读**——发言不是一次文件写入，而是一次**动作**
    /// （触发一整轮编排：模型调用 → 工具执行 → 流式落库），因此入口仍是聊天协议。
    /// 这不是「两套写路径」：VDFS 侧根本没有消息的写路径，只有读路径——
    /// 前端与 LLM 在**同一个地址**上读同一份数据，写入的唯一入口依旧只有一处。
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
            VdfsSessionPath::Messages { .. } => {
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
            let id = uuid::Uuid::new_v4().to_string();
            let mut session = Session::new(&id);
            let mut meta = serde_json::Map::new();
            // 使用方给的 metadata（草稿态选择的 workdir / agent / model / mode…）：
            // 与 `session/update` 同一浅合并语义，字段名由使用方与插件约定，这里不解释
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
        if let Some(incoming) = obj.get("metadata").and_then(Value::as_object) {
            let target = session
                .metadata
                .as_object_mut()
                .ok_or_else(|| vdfs::VdfsError::invalid("会话 metadata 不是对象"))?;
            for (k, v) in incoming {
                target.insert(k.clone(), v.clone());
            }
        }
        if let Some(title) = obj.get("title").and_then(Value::as_str) {
            if let Some(m) = session.metadata.as_object_mut() {
                m.insert("title".to_string(), Value::String(title.to_string()));
            }
        }
        session.updated_at = now_ms();
        self.save_session(&session)
            .await
            .map_err(vdfs::from_plugin_error)?;
        // 带节点视图：标题 / 元数据的变更就地收敛（消费方零回读）
        self.notify_session_state(path).await;
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
            _ => Err(vdfs::VdfsError::Forbidden(format!(
                "该路径不可删除：{path}"
            ))),
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
    /// 会话转写（**含在途消息**）——`.vdfs/session/<id>/消息` 的唯一数据源。
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
            Some(st) => st.live_messages.lock().await.clone(),
            None => Vec::new(),
        }
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
            return SessionRuntime::idle();
        };
        // `try_read` 失败（正被写者持有）时按"运行中"回答：运行态的写入都是
        // 瞬时操作，读不到就说明此刻正在变更——宁可多报一次运行中（前端会
        // 在下一次变更收敛），也不要凭空把正在跑的会话报成空闲（那会让停止
        // 按钮消失、用户无法中止）。
        let Ok(inner) = st.inner.try_read() else {
            return SessionRuntime::working(None);
        };
        SessionRuntime::from_state(
            inner.is_working,
            inner.last_outcome.clone(),
            inner.last_error.clone(),
            inner.phase.clone(),
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
                            inner.phase.clone(),
                        ),
                        Err(_) => SessionRuntime::working(None),
                    },
                    None => SessionRuntime::idle(),
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
