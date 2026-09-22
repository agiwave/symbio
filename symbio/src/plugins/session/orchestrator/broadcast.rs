//! 广播出口：把「状态收敛 + 变更投递」集中到一处。
//!
//! 只有一个出口，只做**投递**，不含业务判定：
//! - `emit_session_state`：**会话运行态的唯一出口**——写运行态 → 发带节点视图的
//!   VDFS 变更（→ 一切消费者）；
//! - `broadcast_error_with_idle`：可恢复错误路径的唯一出口（收敛为「失败」结局）。
//!
//! ## 为什么运行态要经 VDFS 变更而不是事件
//!
//! 见 `session/docs/node-state-streaming.md`：状态是节点的属性，变更携带**全量
//! 节点视图**，因此幂等、可交换、丢一次不影响正确性；而事件是增量的、有顺序的
//! （`busy` 与 `idle` 谁先到决定 UI 对错）。
//!
//! ## `kind = "session"` 事件频道已废除
//!
//! 会话域原先还往事件总线发一整套 `StreamEvent` 帧（`Status` / `Error` / `Abort`
//! / 消息 `Update`），供**进程内**消费者（子智能体转播、CLI）使用。那两个消费者
//! 现已改订阅 VDFS 变更（`vdfs/watch` + `kind = "vdfs"`），端口随之关闭：
//! 后端只发 VDFS 一条频道，`event_bus` 退化为「VDFS 变更的传输层」。
//! 漏掉这条的人会重新长出一条与 VDFS 并行的第二真相——那正是本次要废除的东西。
//!
//! 可见性：`broadcast_error_with_idle` 被 `consume.rs` 调用，标 `pub(super)`。

use super::*;

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
    /// 告警期间会话照常运行（持久化失败 / 长度截断 / 工具轮次上限）。
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
    /// `handle_abort`。曾经两边各写各的（一边 `completed`、一边 `aborted`），
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

impl SessionPlugin {
    /// 集中收敛"业务错误 + 状态收敛"：收敛为「失败」结局。
    ///
    /// **目的**：所有可恢复错误路径都必须保证运行态收敛到"不在跑"，
    /// 否则前端会一直显示"AI 处理中"且 30 分钟内无任何信号能清；
    /// 同时后端 `is_working` 不复位会导致后续 resume 请求被 `session_busy` 守卫静默拒绝。
    ///
    /// 错误的**呈现**不是另一条通道：它随会话节点视图（`status = failed` +
    /// `attributes.error`）下发，与状态走同一个出口——这正是「一处判据」的含义。
    pub(super) async fn broadcast_error_with_idle(
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
    /// 1. **写运行态**——节点视图的数据源（`list` / `stat` 与本函数因此同源）；
    /// 2. **发转写流帧**（带全量节点视图）——一切消费者据此渲染，**零回读**。
    ///
    /// 漏掉第 2 步，UI 会永久停在旧状态（角标不转、停止按钮不出现/不消失），
    /// 且没有任何机制会纠正它。
    ///
    /// ## 为什么发到**转写流**而不是 VDFS 变更流
    ///
    /// 会话运行态必须与它那一轮的消息**共用 `seq` 空间**，否则「会话报不忙」推不出
    /// 「本轮消息节点都已收到终态帧」——两条独立 `mpsc` + 两个泵任务的到达顺序
    /// 只是调度巧合（原先正是如此，前端被迫挂一条宽限复查兜底）。
    /// 现在 `seq` 从 `Transcript` 取号，单通道保序即给出这条推理。
    ///
    /// 调用时机（由两条收尾路径保证，都在本轮**最后一条**消息帧之后）：
    /// 正常收尾「清在途 → 复位 `is_working` → 本函数」，
    /// 中止收尾「`converge_inflight` 广播节点终态 → 本函数」。
    pub async fn emit_session_state(
        &self,
        state: &Arc<ActiveSessionState>,
        change: SessionStateChange,
    ) {
        let id = state.request_id_str();
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

        // 会话已被删（运行态无处可挂）时静默返回：删除本身另有 `deleted` 出口。
        let Some(node) = self.session_node_of(&id).await else {
            return;
        };
        state.transcript.lock().await.emit_session_state(node);
    }
}
