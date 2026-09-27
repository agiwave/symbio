//! 会话与消息的**变更入口**（session 域内的写语义）
//!
//! 「改写一条消息 / 截断 / 清空 / 新建或覆盖一个会话 / 删除会话」这五件事
//! ——**它们的实现住在这里**，是「谁能改会话」这个问题的唯一答案。
//!
//! ## 它不是什么
//!
//! 不是第二条协议：VDFS（`<根>/session` 的 `write` / `delete` / `action`）与
//! 编排入口（`chat/send` 的落库）都只是**调用方**，最终都落到本文件的五个方法上。
//! 从前它们住在 `plugin/vdfs_provider.rs`（随旧路由整体迁到 VDFS 时的落点），
//! 于是**适配层持有写语义**：改一条消息的合法边界要去协议适配文件里找，
//! 写路径的守卫也没法在一处收口。本域把它们收回来，provider 只留
//! 「按 path 定位资源域 → 翻译请求 → 委托本域 → 投递变更」。
//!
//! ## 为什么领域层里会出现 `impl SessionPlugin`
//!
//! 五个方法都是 `SessionPlugin` 的方法（它们要握手头的存储单例、活跃状态表与
//! 转写出口）。Rust 允许 inherent impl 分散在 crate 内任意模块，本仓一贯如此
//! （`orchestrator/` / `chat_loop/` / `plugin/vdfs_provider.rs` 都各有自己的块）。
//! 因此本域**唯一**允许引用汇编层的符号是 `SessionPlugin` **类型本身**：
//! 不得引用 `plugin` 域的函数 / 常量（那条边界由 `scripts/session-layout-audit.mjs`
//! 的 S-001 判定）。领域对「插件状态容器」的读取经它自己的 `pub(crate)` 字段与方法，
//! 而不是经汇编层的入口。
//!
//! ## 测试在哪
//!
//! 五个方法目前经 **VDFS 边界**被覆盖（`plugin/vdfs_provider.test.rs` 的
//! `truncate_*` / `clear_*` / `named_create_*` / `write_merges_*` 等用例）：
//! 这些操作在线上本来就是**按地址寻址**的，把它们放到 provider 那一侧观察
//! （地址 → 回执 + 变更帧）比直接调本域方法更接近真实调用面。故本域不另立
//! 测试文件，避免同一段语义被两处用例各自钉一遍（见 `docs/module-layout.md` §2）。

use super::paths::session_id_from_new_path;
use super::plugin::SessionPlugin;
use super::types::Session;
use crate::symbio_core::clock_now_ms;
use crate::symbio_core::schemas::session::chat_message as cm;
use crate::symbio_core::vdfs;
use crate::symbio_core::PluginError;
use serde_json::{json, Value};

impl SessionPlugin {
    // ==================== 会话本体：新建 / 覆盖 ====================

    /// 会话写入（`<id>` 具名覆盖 / 挂载根新建）——**同一份 upsert 实现**。
    ///
    /// 覆盖分支的浅合并与 `session/update` 路由**共用**
    /// [`Session::merge_metadata_object`]（不是"语义相同"，是同一份代码）。
    /// 新建分支另有一层优先级（路径名 → 显式 `title`）。
    pub(crate) async fn session_upsert(
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
    pub(crate) async fn new_session_id(&self) -> String {
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
    //
    // 位置：本域成立后它们不再住在 `plugin/vdfs_provider.rs`（那里只留请求翻译），
    // 理由见模块头——适配层不该持有写语义。

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

    // ==================== 会话本体的删除（会话本体与子会话两条 Delete 分支共用） ====================

    /// 删除会话的统一内部实现（abort 活跃任务 → 清活跃条目 → 存储删除）。
    ///
    /// 唯一入口是 [`vdfs::VdfsProvider::dispatch`] 的两条 `Delete` 分支（会话本体
    /// 与 `sub_session_at` 的子会话），两者语义相同，共用这一份实现。
    ///
    /// ## 为什么没有第二条删除路径
    ///
    /// 曾经的 `session/clear` 路由是它的第二个消费方，已退役——两个入口对同一件事
    /// 就是两条会各自漂移的实现，VDFS 侧本来就已经完整具备这个能力。
    pub(crate) async fn delete_session_internal(
        &self,
        session_id: &str,
    ) -> Result<(), PluginError> {
        // 删除前先 abort 该会话的活跃任务
        let state = self.active_mgr.get_or_create(session_id).await;
        {
            // 置位即中止（无帧、无 await）：与 `handle_abort` 走同一个原语。
            let mut inner = state.inner.write().await;
            if let Some(signal) = inner.abort_signal.take() {
                signal.abort();
            }
        }
        // 清理活跃条目
        self.active_mgr.sessions.write().await.remove(session_id);

        let store = self.get_store().await?;
        store.delete_session(session_id).await?;

        // VDFS 实时链路（provider 侧变更广播 → watch 的 sink → 总线 kind="vdfs"）：
        // 前端据此把该会话从清单移除。session/clear 与 VDFS 删除两条删除路径共用此处。
        // 作用域按**路径前缀**分流（子会话落在 `<sid>/subsession/…` 之下），因此这里
        // 不再需要实体时代的 `parent_id` 载荷——也不必为发事件多读一次盘。
        self.notify_change(session_id);

        Ok(())
    }
}
