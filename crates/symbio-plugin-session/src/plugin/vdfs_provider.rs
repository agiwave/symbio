//! 会话地址面的**路由、订阅与共享读值**。
//!
//! 职责：
//! - [`symbio_core::VdfsProvider::dispatch`] 是唯一入口：**先按 `path` 定位资源域，
//!   再按 `req` 执行操作**——每个域各有一个 `*_at` 方法，dispatch 只做路由，不碰任何域内
//!   读写。「这个地址归哪个域」的问题只在这一处有答案；域内怎么读写在各自的文件里：
//!   目录面 [`vdfs_catalog`]、转写面 [`vdfs_messages`]、收件箱面 [`vdfs_inbox`]
//!   （分文件的判据是「那一段错了谁会红」，各自的模块头写着它红的那一面）；
//! - 本文件另带**跨域共用的读值**（`transcript_of` / `runtime_of` / `nodes_of_sessions` …）
//!   与订阅表（`watch` / `unwatch`）：它们服务的是「同一个值在几个域里长一样」，
//!   红的是**几面同时**，因此留在路由这一处，不跟着域分散；
//! - 写侧：**本文件不实现任何写语义**——各域方法把 `Write` / `Delete` / `Action` 翻译成
//!   对 `commands` 的调用（消息改写 / 截断 / 清空、会话新建 /
//!   覆盖 / 删除），再把变更投进这里的订阅表。「谁能改会话」的答案在 `commands` 一处。

use super::*;
use crate::stats_reading::present_message;
use symbio_core::chat_node as cm;
use symbio_core::{
    vdfs_from_plugin_error, VdfsChangeSink, VdfsContext, VdfsError, VdfsNode, VdfsProvider,
    VdfsRequest, VdfsResponse, VdfsResult,
};

#[async_trait]
impl VdfsProvider for SessionPlugin {
    /// 唯一入口：**先按 `path` 定位资源域，再按 `req` 执行操作**。
    ///
    /// 配置文件（`PLUGIN.yml`）是文档不是会话——先判路径再分流操作；其余全部经
    /// [`parse_session_path`] 定域，各域逻辑在按域分文件的 `*_at` 方法里（见模块头）。
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
            VdfsSessionPath::Run { id, op } => self.run_at(ctx, path, id, op, req).await,
        }
    }
}

impl SessionPlugin {
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
    pub(super) async fn watch_at_path(&self, path: &str, sink: VdfsChangeSink) -> VdfsResult<()> {
        self.change_subs.watch(path, sink);
        Ok(())
    }

    /// 取消订阅：与 [`Self::watch_at_path`] 严格配对——引用计数归零才真正摘掉。
    pub(super) async fn unwatch_at_path(&self, path: &str) -> VdfsResult<()> {
        self.change_subs.unwatch(path);
        Ok(())
    }

    /// 会话转写（**含在途消息**）——`<根>/session/<id>/message` 的唯一数据源。
    ///
    /// 落库转写 ∪ 本轮在途缓冲。之所以要并上后者：流式期间消息**还没落库**
    /// （`persist_messages` 只在每轮结束时写盘），只读存储的话列表在流式期间
    /// 就是空的——`created` / `appended` 变更到达时消费者去 `list` 会一无所获，
    /// 「转写即列表」当场失效。
    ///
    /// 存在性校验在此一并完成（`session_of`），因此调用方不必再查一次。
    ///
    /// ## 落库那份已由存储层交给真源
    ///
    /// [`Self::session_of`] 走 `load_session`，而存储层的 `load_messages` **已是
    /// WAL 重建优先**、`messages.json` 只作 v1 遗留兜底（见 `store::load_messages`）。
    /// 因此这里取 `session.messages` 拿到的就是真源那一份——本函数不再自己判断
    /// 「缓存在不在、要不要重建」：那是**存储层的职责**，在插件层再判一次就是同一条
    /// 规矩的第二份口径（早期「缓存不在才重建」的过渡形态， 收敛掉）。
    pub(super) async fn transcript_of(&self, id: &str) -> VdfsResult<Vec<cm::ChatNode>> {
        let session = self.session_of(id).await?;
        let live = self.live_messages_of(id).await;
        let mut messages = ordered(overlay_live(session.messages, live));
        // 呈现投影（[]）：转写列表与会话叶子是同一份数据的两个地址，必须给
        // 同一棵树——`session_content` 用同一个 `flatten_whole_tree`、同一个档位，
        // 这里跟着投一次，否则流式期间两条路径分叉（`grouped` 档下即时返回、零改动）。
        let mode = self.presentation_mode().await;
        symbio_core::node_build::flatten_whole_tree(&mut messages, mode);
        // 呈现派生：列表是前端读到的另一条交互出口（直播走 `Transcript::publish`，
        // 历史/切回走这里）。存储与事件里的节点不含 title/icon 也不含读数行，
        // 交付前按形状统一算出（读数口径见 [D-111](../README.md#d-111-消息读数由交付出口装配压缩与记忆的事实行离开前端 "crates/symbio-plugin-session/src/README")）。
        Ok(messages.into_iter().map(present_message).collect())
    }

    /// 该会话本轮的在途消息（无活跃状态 ⇒ 无在途消息）
    pub(super) async fn live_messages_of(&self, id: &str) -> Vec<cm::ChatNode> {
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

    /// 会话清单 → VDFS 节点（携带实时运行态）
    ///
    /// 逐项取名册的运行态（不再整段持名册读锁跨过存储读取）：每一项的合并规则
    /// 由 `ActiveSessionManager::runtime_of` 单点承担。
    pub(super) async fn nodes_of_sessions(
        &self,
        sessions: &[SessionSummary],
        schema: &Option<Value>,
    ) -> Vec<VdfsNode> {
        let mut out = Vec::with_capacity(sessions.len());
        for s in sessions {
            let rt = self.runtime_of(&s.id).await;
            let mut n = session_node(s, &rt);
            // 选项定义：清单一次取全（前端零额外请求，见设计文档 §7）
            n.schema = schema.clone();
            out.push(n);
        }
        out
    }

    /// 运行态的**唯一读取入口**：存储交事实投影，名册交进程内缓存并合并。
    ///
    /// 事实投影只是 `Option`（无 WAL 的 v1 会话 / 临时会话没有），名册的合并规则
    /// 决定「缓存优先、无缓存读投影」（见 `ActiveSessionManager::runtime_of`）。
    pub(super) async fn runtime_of(&self, id: &str) -> SessionRuntime {
        let facts = match self.get_store().await {
            Ok(store) => store.runtime_facts(id),
            Err(_) => None,
        };
        self.active_mgr.runtime_of(id, facts.as_ref()).await
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
                symbio_core::plugin_warn!("session", "选项定义序列化失败（选项栏将为空）: {e}");
                None
            }
        }
    }

    /// 会话的工作目录（未声明 workdir ⇒ 该会话无目录树能力）
    pub(super) async fn workdir_of(&self, id: &str) -> VdfsResult<String> {
        let session = self.session_of(id).await?;
        super::super::workdir::workdir_of(&session)
            .ok_or_else(|| VdfsError::not_found(format!("会话 {id} 没有工作目录")))
    }

    /// 会话记忆 → VDFS 节点。
    ///
    /// 形状由共享实现 `MemoryFile::node` 产出，
    /// `list`（经 `internal_dirs`）与 `stat`
    /// **共用同一份**——「列表里的和点开的不是同一个东西」这类 bug 因此写不出来。
    pub(super) async fn memory_node_of(&self, id: &str) -> VdfsNode {
        self.memory_store(id)
            .await
            .node(&symbio_providers::MemoryNodeSpec {
                title: super::super::memory::SEGMENT_TITLE,
                name: None,
                kind: PLUGIN_ID_SESSION,
                description: super::super::memory::MEMORY_DESCRIPTION,
            })
    }

    /// 取子会话（**归属校验**：必须确实挂在 `id` 之下，避免跨会话越权访问）
    pub(super) async fn sub_session_of(
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
