//! 广播出口：把「状态收敛 + 变更投递」集中到一处。
//!
//! 只有一个出口，只做**投递**，不含业务判定：
//! - [`Convergence::emit_session_state`]：**会话运行态的唯一出口**——写运行态 → 发一条
//!   VDFS 变更（`<sid>` 上 `data` = 全量节点视图，与 `stat` 同源构造 → 消费端零回读）；
//! - [`Convergence::broadcast_error_with_idle`]：可恢复错误路径的唯一出口（收敛为「失败」结局）。
//!
//! 两者都住在 [`Convergence`]（收敛手柄）上而不是 `SessionPlugin` 上：调用方有写在
//! 后台任务里的（`TranscriptSink` / `WorkingGuard`），它们要的是收敛能力，不是整个插件。
//!
//! ## 为什么运行态要经 VDFS 变更而不是事件
//!
//! 见 `session/docs/018-node-state-streaming.md`：状态是**节点的属性**，因此幂等、
//! 可交换、丢一次不影响正确性——消费端回读 `stat` 即得当前值；而事件是增量的、
//! 有顺序的（`busy` 与 `idle` 谁先到决定 UI 对错）。随载荷下发的视图是**调用
//! 那一刻**从权威源构造的（与 `stat` 同一构造点），不是缓存副本——幂等不受影响。
//!
//! ## `kind = "session"` 事件频道已废除
//!
//! 会话域原先还往事件总线发一整套 `StreamEvent` 帧（`Status` / `Error` / `Abort`
//! / 消息 `Update`），供**进程内**消费者（子智能体转播、CLI）使用。那两个消费者
//! 现已改订阅 VDFS 变更（`vdfs/watch` + `kind = "vdfs"`），端口随之关闭：
//! 后端只发 VDFS 一条频道，`event_bus` 退化为「VDFS 变更的传输层」。
//! 漏掉这条的人会重新长出一条与 VDFS 并行的第二真相——那正是本次要废除的东西。
//!
//! 可见性：两个出口都在 [`Convergence`] 上，`pub(crate)`（宿主与执行期出口都是同一插件内
//! 的持有者）。

use super::*;
use crate::plugins::session::plugin::session_node;
use crate::plugins::session::types::SessionSummary;

/// 会话运行态的一次变化。
///
/// 用枚举而不是 `(status: &str, error: Option<String>)` 两个参数：后者允许
/// "`busy` 还带着上次的错误"这种非法组合存在，而它的表现是**静默的**——
/// 前端会同时显示"处理中"和"上次失败"。
pub enum SessionStateChange {
    /// 新一轮开始（上一轮的结局随之作废，上一轮的告警随之清除）
    Working,
    /// 一轮结束：正常 / 用户中止 / 以错误结束
    Finished {
        outcome: &'static str,
        error: Option<String>,
    },
    /// 会话级告警（可恢复）：只写 `attributes.warning`，**不改变运行态**。
    /// `Some(text)` 设置，`None` 清除。与 `Finished.error`（失败终态）的分界：
    /// 告警期间会话照常运行（持久化失败 / 长度截断）。
    Warning(Option<String>),
}

impl SessionStateChange {
    /// 一轮**正常结束**。
    pub(crate) fn completed() -> Self {
        Self::Finished {
            outcome: OUTCOME_COMPLETED,
            error: None,
        }
    }

    /// 一轮**被用户中止**。
    ///
    /// **唯一构造点**，因为中止有两个写者：消费循环的 ABORTED 出口与
    /// `handle_abort`。两边各写各的（一处 `completed`、一处 `aborted`），
    /// 最终显示哪个取决于「3s 轮询」与「循环收尾」谁先跑到——窗口期内 UI 会
    /// 短暂显示"已完成"，提示音也可能选错音色。收成一个构造函数之后，
    /// 两个写者写的是同一个值，竞态因此无害。
    pub(crate) fn aborted() -> Self {
        Self::Finished {
            outcome: OUTCOME_ABORTED,
            error: None,
        }
    }
}

/// 会话**收敛**手柄：把一个会话的运行态与终态落到它身上所需的那四样。
///
/// ## 为什么是手柄而不是插件（[ADR-075](../../../../../docs/010-adr/075.md "ADR-075") 前置 P2）
///
/// 收敛这件事有两个写在**后台任务**里的调用方：执行期出口的告警
/// （`TranscriptSink::warn`）与崩溃兜底（`WorkingGuard::drop`）。任务比插件活得久
/// 是卸载路径上的常态——它们要的就是「打开会话（存储 + 配置）+ 取会话摘要（存储）+
/// 取运行态（名册）+ 写进这一份会话状态并发一帧」，把整个插件背进来既多于所需，
/// 又把 provider 注册、路由与订阅表一起钉住。
///
/// `store` 是**同一个** `OnceCell`（不是解析好的 store）：存储的懒初始化只有一条路，
/// 由 [`super::super::store::resolve_store`] 承载，插件与手柄共用这一个格子。
#[derive(Clone)]
pub(crate) struct Convergence {
    store: Arc<tokio::sync::OnceCell<Arc<super::super::store::SessionStore>>>,
    dir: std::path::PathBuf,
    config: Arc<tokio::sync::RwLock<super::super::config::SessionConfig>>,
    active: Arc<super::super::active::ActiveSessionManager>,
}

impl Convergence {
    pub(crate) fn new(
        store: Arc<tokio::sync::OnceCell<Arc<super::super::store::SessionStore>>>,
        dir: std::path::PathBuf,
        config: Arc<tokio::sync::RwLock<super::super::config::SessionConfig>>,
        active: Arc<super::super::active::ActiveSessionManager>,
    ) -> Self {
        Self {
            store,
            dir,
            config,
            active,
        }
    }

    pub(crate) async fn get_store(
        &self,
    ) -> Result<Arc<super::super::store::SessionStore>, PluginError> {
        super::super::store::resolve_store(&self.store, &self.dir).await
    }

    /// 打开会话引擎（存储 + 配置 + id）——`SessionPlugin::open_chat_session` 的实现就在这里，
    /// 那里只是门面。
    pub(crate) async fn open_chat_session(
        &self,
        session_id: &str,
    ) -> Result<Arc<super::super::chat_session::PersistentChatSession>, PluginError> {
        let store = self.get_store().await?;
        Ok(Arc::new(
            super::super::chat_session::PersistentChatSession::new(
                session_id.to_string(),
                self.config.clone(),
                store,
            ),
        ))
    }

    /// 按 session_id 构造会话引擎实例——**三臂的唯一工厂口**
    /// （[ADR-081](../../../../../docs/010-adr/081.md "ADR-081") 裁决一：`PersistentChatSession`
    /// 的两个构造器在驱动器侧只许经此调用，豁免恰一处 = `chat_loop/io.rs` 的缺句柄回退）。
    ///
    /// - `_t_` 前缀 → 内存 ephemeral 会话；
    /// - 非空 id → 持久会话（走 [`Self::open_chat_session`]，构造点只有一处）；
    /// - None/空 → 内存 ephemeral 会话（固定 id `"ephemeral"`）。
    ///
    /// 两类会话共用同一引擎，差异只在存储后端（审计 B1）；ephemeral 会话的配置取
    /// 当前值的**快照**（内存会话不随 `session/config` 变更而变）。
    ///
    /// 消费方**只有一处**：`TurnLauncher::start_turn` 向 chat_ctx 交付会话句柄
    /// （`SESSION_HANDLE`，交付失败时 model 侧兜底内存会话）。没有对外 `session/open`
    /// 路由消费它（对外返回进程内句柄），已退役——进程内句柄不该有对外路由。
    pub(crate) async fn open_session_handle(
        &self,
        session_id: Option<String>,
    ) -> Result<Arc<super::super::chat_session::PersistentChatSession>, PluginError> {
        use super::super::chat_session::PersistentChatSession;

        let snapshot = {
            let cfg = self.config.read().await;
            cfg.clone()
        };

        match session_id {
            Some(sid) if !sid.is_empty() && !sid.starts_with("_t_") => {
                self.open_chat_session(&sid).await
            }
            // `_t_` 前缀与空/缺省 id：内存临时会话，配置取当前值快照。
            // 固定 id "ephemeral"（审计 B2）：随机 id 会让压缩前的 transcript
            // 转存落到永不复现的目录名下，成为无法关联的孤儿存档。
            Some(sid) => Ok(Arc::new(PersistentChatSession::detached(sid, snapshot))),
            None => Ok(Arc::new(PersistentChatSession::detached(
                "ephemeral",
                snapshot,
            ))),
        }
    }

    /// 集中收敛"业务错误 + 状态收敛"：收敛为「失败」结局。
    ///
    /// **目的**：所有可恢复错误路径都必须保证运行态收敛到"不在跑"，
    /// 否则前端会一直显示"AI 处理中"且 30 分钟内无任何信号能清；
    /// 同时后端 `is_working` 不复位会导致后续 resume 请求被 `session_busy` 守卫静默拒绝。
    ///
    /// 错误的**呈现**不是另一条通道：它随会话节点视图（`status = failed` +
    /// `attributes.error`）下发，与状态走同一个出口——这正是「一处判据」的含义。
    pub(crate) async fn broadcast_error_with_idle(
        &self,
        state: &Arc<ActiveSessionState>,
        error: impl Into<String>,
    ) {
        let err = error.into();
        self.emit_session_state(
            state,
            SessionStateChange::Finished {
                outcome: OUTCOME_FAILED,
                error: Some(err),
            },
        )
        .await;
    }

    /// 会话运行态变化的**唯一出口**。
    ///
    /// 两件事，顺序写死：
    /// 1. **写运行态**——`list` / `stat` 的数据源（与本函数因此同源）；
    /// 2. **发一条 VDFS 变更**（`<sid>` 上 `data` = 全量节点视图）——消费端
    ///    零回读就地收敛；视图与 `stat` 同源（`session_node`），口径必然一致。
    ///
    /// 漏掉第 2 步，UI 会永久停在旧状态（角标不转、停止按钮不出现/不消失），
    /// 且没有任何机制会纠正它。
    ///
    /// ## 视图从哪来
    ///
    /// 调用本函数前运行态刚写完，此时经 `session_node`（与 `list` / `stat` 同一
    /// 构造点）取视图——它读的是**同一份** `ActiveSessionStateInner`，因此发出去
    /// 的就是「此刻的状态」，不是过期副本。会话已被删时取不到视图，退化为
    /// 无载荷变更（回读 `NotFound` 即收敛）。
    ///
    /// ## 与消息变更的关系：同一张表，不靠顺序
    ///
    /// 两者都投到 session provider 的那张 `VdfsChangeSubscriptions`（消息投
    /// `<sid>/message/<mid>`、运行态投 `<sid>`）。到达顺序**没有**机制保证，也
    /// **不需要**：`ChatNode.seq` 决定显示顺序（节点属性），而「本轮是否结束」
    /// 由会话节点自己的 `status` 决定——消费端回读即得，不依赖谁先到。
    ///
    /// 调用时机（由两条收尾路径保证，都在本轮**最后一条**消息帧之后）：
    /// 正常收尾「清在途 → 复位 `is_working` → 本函数」，
    /// 中止收尾「`converge_inflight` 广播节点终态 → 本函数」。
    pub(crate) async fn emit_session_state(
        &self,
        state: &Arc<ActiveSessionState>,
        change: SessionStateChange,
    ) {
        {
            let mut inner = state.inner.write().await;
            match &change {
                SessionStateChange::Working => {
                    inner.is_working = true;
                    // 新一轮开始：上一轮的结局与告警一并作废，否则失败角标/告警条会残留
                    inner.last_outcome = None;
                    inner.last_error = None;
                    inner.last_warning = None;
                }
                SessionStateChange::Finished { outcome, error } => {
                    inner.is_working = false;
                    inner.last_outcome = Some((*outcome).to_string());
                    inner.last_error = error.clone();
                }
                // 告警只改自己的属性：is_working / 结局原样保留
                SessionStateChange::Warning(w) => {
                    inner.last_warning = w.clone();
                }
            }
        }

        // 运行态变更挂在会话叶子 `<sid>` 上，`data` = 全量节点视图（与 `stat`
        // 同一构造点 `session_node`）。会话已被删时取不到视图——退化为无载荷
        // 变更，消费端回读 `stat` 得到 `NotFound` 即自然收敛。
        let node = match self.get_store().await {
            Ok(store) => match store.load_session_checked(&state.session_id).await {
                Ok(Some(session)) => {
                    let rt = self.active.runtime_of(&state.session_id).await;
                    Some(session_node(&SessionSummary::of(&session), &rt))
                }
                _ => None,
            },
            Err(_) => None,
        };
        state.transcript.lock().await.emit_session_state(node);
    }
}

#[cfg(test)]
#[path = "broadcast.test.rs"]
mod tests;
