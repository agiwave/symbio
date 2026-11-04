//! 广播出口：把「状态收敛 + 变更投递」集中到一处。
//!
//! 只有一个出口，只做**投递**，不含业务判定：
//! [`Convergence::emit_session_state`]：**会话运行态的唯一出口**——写运行态 → 发一条
//! VDFS 变更（`<sid>` 上 `data` = 全量节点视图，与 `stat` 同源构造 → 消费端零回读）。
//!
//! 它住在 [`Convergence`]（收敛手柄）上而不是 `SessionPlugin` 上：调用方要的是收敛能力，
//! 不是整个插件。
//!
//! ## 为什么运行态要经 VDFS 变更而不是事件
//!
//! 见 `docs/105-node-state-streaming.md`：状态是**节点的属性**，因此幂等、
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

use super::active::ActiveSessionState;
use super::plugin::session_node;
use super::types::SessionSummary;
use std::sync::Arc;
use symbio_kernel::plugin::PluginError;

/// 会话运行态的一次变化。
///
/// 用枚举而不是 `(status: &str, error: Option<String>)` 两个参数：后者允许
/// "`busy` 还带着上次的错误"这种非法组合存在，而它的表现是**静默的**——
/// 前端会同时显示"处理中"和"上次失败"。
pub enum SessionStateChange {
    /// 新一轮开始（上一轮的结局随之作废）
    Working,
    /// 一轮结束：正常 / 用户中止 / 以错误结束
    Finished {
        outcome: &'static str,
        error: Option<String>,
    },
}

/// 会话**收敛**手柄：把一个会话的运行态与终态落到它身上所需的那四样。
///
/// ## 为什么是手柄而不是插件（[D-075](../../../docs/003-design/002-架构总览.md#d-075-卸载能力先于目录分离session-切分立三条前置会话记忆与存储留守 "DE-002-架构总览") 前置 P2）
///
/// 收敛这件事的调用方是会话动作（开轮 / 收轮 / 中止收口）——它们要的就是
/// 「打开会话（存储 + 配置）+ 取会话摘要（存储）+ 取运行态（名册）+ 写进这一份
/// 会话状态并发一帧」，把整个插件背进来既多于所需，又把 provider 注册、路由与
/// 订阅表一起钉住。
///
/// `store` 是**同一个** `OnceCell`（不是解析好的 store）：存储的懒初始化只有一条路，
/// 由 [`super::store::resolve_store`] 承载，插件与手柄共用这一个格子。
#[derive(Clone)]
pub(crate) struct Convergence {
    store: Arc<tokio::sync::OnceCell<Arc<super::store::SessionStore>>>,
    dir: std::path::PathBuf,
    config: Arc<tokio::sync::RwLock<super::config::SessionConfig>>,
    active: Arc<super::active::ActiveSessionManager>,
}

impl Convergence {
    pub(crate) fn new(
        store: Arc<tokio::sync::OnceCell<Arc<super::store::SessionStore>>>,
        dir: std::path::PathBuf,
        config: Arc<tokio::sync::RwLock<super::config::SessionConfig>>,
        active: Arc<super::active::ActiveSessionManager>,
    ) -> Self {
        Self {
            store,
            dir,
            config,
            active,
        }
    }

    pub(crate) async fn get_store(&self) -> Result<Arc<super::store::SessionStore>, PluginError> {
        super::store::resolve_store(&self.store, &self.dir).await
    }

    /// 打开会话引擎（存储 + 配置 + id）——`SessionPlugin::open_chat_session` 的实现就在这里，
    /// 那里只是门面。
    pub(crate) async fn open_chat_session(
        &self,
        session_id: &str,
    ) -> Result<Arc<super::chat_session::PersistentChatSession>, PluginError> {
        let store = self.get_store().await?;
        Ok(Arc::new(super::chat_session::PersistentChatSession::new(
            session_id.to_string(),
            self.config.clone(),
            store,
        )))
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
    /// 调用时机：本轮**最后一条**消息帧之后（收尾 / 中止的落点由动作层决定，
    /// 本函数只负责写运行态并投递）。
    pub(crate) async fn emit_session_state(
        &self,
        state: &Arc<ActiveSessionState>,
        change: SessionStateChange,
    ) {
        // 运行态写路径各只有一个方法（单写者）——见 `active.rs` 的
        // 「运行态的写路径」：写点散回调用方就会重新长出双写面。
        match &change {
            SessionStateChange::Working => state.begin_turn().await,
            SessionStateChange::Finished { outcome, error } => {
                state.finish_turn(outcome, error.clone()).await
            }
        }

        // 运行态变更挂在会话叶子 `<sid>` 上，`data` = 全量节点视图（与 `stat`
        // 同一构造点 `session_node`）。会话已被删时取不到视图——退化为无载荷
        // 变更，消费端回读 `stat` 得到 `NotFound` 即自然收敛。
        let node = match self.get_store().await {
            Ok(store) => match store.load_session_checked(&state.session_id).await {
                Ok(Some(session)) => {
                    let facts = store
                        .fact_source(&state.session_id)
                        .ok()
                        .flatten()
                        .map(|source| crate::runtime::project(&source.range(0)));
                    let rt = self
                        .active
                        .runtime_of(&state.session_id, facts.as_ref())
                        .await;
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
#[path = "convergence.test.rs"]
mod tests;
