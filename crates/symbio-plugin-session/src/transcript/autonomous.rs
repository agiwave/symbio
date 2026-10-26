//! 自主触发落格 —— 「这次开轮是系统自己发起的」这件事在事实源里的样子
//!
//! ## 为什么这段代码不住在心跳里
//!
//! 心跳回答的是「**什么时候**该开一轮」（间隔、抖动、空闲基线）；本文件回答的是
//! 「这一次开轮是一次**自主触发**，先把四格事实落进这个会话的事实源，落成了才准开」。
//! 后者不是定时器的属性——它回答的是**谁有资格开这个会话的事实源、在什么时刻开**，
//! 那是存储所有者的写窗口纪律，不是某个触发器的私事（归属裁决见
//! [D-076](../../heartbeat/README.md#d-076-心跳片的八个宿主触点逐条落位调度随片走触发事实落进收件箱写臂 "crates/symbio-plugin-heartbeat/src/README") 裁决一 #5/#6）。
//!
//! 于是「谁来发起」在这里不重要：今天唯一的入口是收件箱写臂（`write(<会话地址>/inbox)`，
//! 写体带 `meta.trigger` 触发声明，识别在 `plugin/nodes.rs::trigger_declaration`），
//! 它经 [`SessionPlugin::request_autonomous_round`] 开这一轮。任何触发器（心跳、巡检、
//! 外部调度、网关）声明同一件事，落的都是这同四格——**发起者的身份不是这段代码的输入**。
//!
//! ## 失败落在队列之外，不落队首
//!
//! 「落格没成就别开轮」这条链子必须在**入队之前**判：收件箱是一支 FIFO
//! （见 `inbox` 模块头），一条永远跑不起来的队首会把它后面的用户消息一起堵死。
//! 所以 [`SessionPlugin::record_autonomous_trigger`] 返回 `false` 时，调用方连队列都不碰，
//! 重试回到调度侧（一个 tick 之后），而不是压在队列里等一个永不到来的条件。

use super::super::active::{InboxItem, PreemptPending};
use super::super::plugin::SessionPlugin;
use crate::store::EventWalStore;
use symbio_core::chat_node as cm;
use symbio_core::session_chat;
use symbio_core::{conation, Event, Seq, Store};
use symbio_core::{EffectiveMatrix, PRINCIPAL_AUTONOMOUS};

impl SessionPlugin {
    /// 为一次自主触发**开一轮**：先落事实，落成了才带着服务端推导的运行参数入队。
    ///
    /// 返回 `None` = 这一趟没开（原因在 [`Self::record_autonomous_trigger`] 里逐条记了日志），
    /// 调用方据此**不要**推进任何「已触发」的簿记；返回 `Some(条目)` = 已入队，
    /// 什么时候真跑由收件箱消费者决定。
    ///
    /// 运行参数（`mode` / `include_history`）在这里由**服务端按会话自己的数据**推导，
    /// 不从调用方随身带（地址写入面不承载运行选项，见
    /// [D-066](../../../../../docs/003-design/012-会话输入与地址.md#d-066-输入面退役的前置条件解除地址写入不承载运行选项三项运行参数已有会话-metadata-回退协议面不必变宽 "DE-012-会话输入与地址") 决策 4）：
    /// 会话 metadata 是这两格唯一的 owner，与 `resolve_session_params` 的元数据回退同一口径
    /// （`orchestrator/entry.rs` 的 `req > metadata > 默认`）。
    ///
    /// `iid` 透传给 [`Self::enqueue_inbox`]：收件箱的两种地址形态（写目录自身 = 条目名
    /// 由本插件生成；写具名条目 = 名字由调用方给）在这里**不因触发声明而改变语义**。
    /// 落格失败时返回 `None` 而**不产生任何条目**，所以具名调用方也不会看到一个「名字占了
    /// 但内容是空的」条目。
    pub(crate) async fn request_autonomous_round(
        &self,
        session_id: &str,
        iid: Option<String>,
        metadata: &serde_json::Value,
        message: cm::ChatNode,
        idle_ms: i64,
    ) -> Option<InboxItem> {
        // 「欲」的目标 = 这一轮**实际要发出去**的那条正文，不是配置里的第二份副本。
        let goal = message
            .content
            .as_ref()
            .map(|c| c.to_text())
            .unwrap_or_default();
        if !self
            .record_autonomous_trigger(session_id, idle_ms, &goal)
            .await
        {
            return None;
        }

        let include_history = metadata
            .get("heartbeat")
            .and_then(|v| v.get("include_history"))
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let params = session_chat::Request {
            session_id: Some(session_id.to_string()),
            include_history: Some(include_history),
            // 自主轮没有人在屏幕前：需交互的工具产卡会把这一轮永远挂在等待上，
            // 所以 `mode` 不跟随会话设置，恒 `auto`（无人值守）。
            mode: Some("auto".to_string()),
            // 其余字段留空：`agent_id` / `risk_level` / `provider_id` 由
            // `resolve_session_params` 回退到会话 metadata，工作目录同（`start_turn` 的
            // `ctx > metadata > 报错` 第一档之前没有更早的来源）。自主触发是**本机**路径，
            // 无「来源声明」——轮末承诺的 `to` 由取值点落本机用户（那条不一致的归属见 D-076 后果）。
            ..Default::default()
        };
        Some(
            self.enqueue_inbox(session_id, iid, message, params, None)
                .await,
        )
    }

    /// 把一次定时触发落成事实（S9 第 21 步，[roadmap/S12](../../../../../docs/003-design/042-自主层与长期目标.md "DE-042-自主层与长期目标")）。
    ///
    /// 返回 `true` = 触发事实已入格；`false` = 这一趟不触发（原因已记日志）。
    ///
    /// ## 三种不触发，各说各的
    ///
    /// 1. **有写方在写**——事实源是同一会话的唯一写入口（`WalStore::open` 各自
    ///    replay，两个写方并存会重复 `head` ⇒ 盘上出现重复 seq、`seq_monotonic`
    ///    直接变红）。见下「谁有资格写」；
    /// 2. **没有事实源 / 事实源为空**——溯源锚是"上一格的 seq"，一格都没有就无处
    ///    可指。I2 要求触发事件带 `produced_by`（S12 §5 第 3 行：自主发起的工作
    ///    必须可追溯），**宁可不触发也不写一条来路不明的触发**；新会话收到第一条
    ///    消息后事实源自然就有了，下一趟就成；
    /// 3. **开档失败**——存储层问题，记错误。
    ///
    /// ## 谁有资格写
    ///
    /// 会话事实源只有两个写窗口，本函数把两条互斥条件都检查掉：
    ///
    /// - 收束期写方（v2 运行器的轮末收束）跑在 `is_working` 复位**之前** ⇒ 要求
    ///   `is_working == false`；
    /// - 收件箱的空闲写方（挂起落格 / 恢复落格）**只在 `preempt != None` 时**才
    ///   落格 ⇒ 要求 `preempt == PreemptPending::None`。
    ///
    /// 两条同时成立时没有第二个写方能开档。残余窗口是"检查之后一毫秒内跑完一整轮
    /// 插话"——那是既有设计已接受的一类风险（同款说明见
    /// `transcript/inbox.rs::append_preemption`），本函数不放大它：检查紧跟开档、
    /// 格子逐条落、失败逐条记日志。
    pub(crate) async fn record_autonomous_trigger(
        &self,
        session_id: &str,
        idle_ms: i64,
        goal: &str,
    ) -> bool {
        {
            let sessions = self.active_mgr.sessions.read().await;
            if let Some(state) = sessions.get(session_id) {
                let inner = state.inner.read().await;
                if inner.is_working() || inner.preempt != PreemptPending::None {
                    symbio_core::plugin_info!(
                        "session",
                        "[Autonomous] 会话 {} 有写方在写，本次触发不落格（下个 tick 重试）",
                        session_id
                    );
                    return false;
                }
            }
        }

        // 事实源句柄由存储层构造一次并按会话缓存（D-084 §2）——本函数不再拼路径开档。
        let store = match self.get_store().await {
            Ok(store) => store,
            Err(e) => {
                symbio_core::plugin_error!(
                    "session",
                    "[Autonomous] 会话 {} 存储句柄不可用：{}",
                    session_id,
                    e
                );
                return false;
            }
        };
        // 溯源锚 = 上一格的 seq：触发是"从当时的状态出发"的，指过去的事件而不是
        // 指自己。`head == 0` 既包含"事实源不存在"也包含"空文件"——两种都没有上一格。
        // 会话目录不存在 ⇒ `Ok(None)`：同样没有事实源，不触发。
        let store = match store.facts(session_id) {
            Ok(Some(store)) => store,
            Ok(None) => {
                symbio_core::plugin_info!(
                    "session",
                    "[Autonomous] 会话 {} 还没有事实源，本次不触发（无可指的溯源锚）",
                    session_id
                );
                return false;
            }
            Err(e) => {
                symbio_core::plugin_error!(
                    "session",
                    "[Autonomous] 会话 {} 事实源开档失败：{}",
                    session_id,
                    e
                );
                return false;
            }
        };
        let head = store.head().value();
        if head == 0 {
            symbio_core::plugin_info!(
                "session",
                "[Autonomous] 会话 {} 还没有事实源，本次不触发（无可指的溯源锚）",
                session_id
            );
            return false;
        }
        let anchor = head - 1;

        let initiator = AutonomousInitiator;
        let triggered_seq = match store.append(initiator.trigger(anchor)) {
            Ok(seq) => seq.value(),
            Err(e) => {
                symbio_core::plugin_error!(
                    "session",
                    "[Autonomous] 触发事实未入格（会话 {}）：{:?}",
                    session_id,
                    e
                );
                return false;
            }
        };
        // 自检紧跟触发：触发是主事实，自检是这次触发的观测附录（`system × progressed`）。
        if let Err(e) = store.append(initiator.health_event(triggered_seq, idle_ms)) {
            symbio_core::plugin_warn!(
                "session",
                "[Autonomous] 健康自检未入格（会话 {}）：{:?}",
                session_id,
                e
            );
        }

        // 「欲」先入格（E1：欲是数据），闸门再决定它要不要升格成任务。
        let conation_seq = match store.append(initiator.express_intent(goal, triggered_seq)) {
            Ok(seq) => seq.value(),
            Err(e) => {
                symbio_core::plugin_warn!(
                    "session",
                    "[Autonomous] 意图未入格（会话 {}）：{:?}",
                    session_id,
                    e
                );
                // 触发已经落格 ⇒ 这一轮照常开跑（意图没写成不该连累触发）。
                return true;
            }
        };
        // 回读刚落格的那一条：候选的唯一构造路径要读 `seq`，而本地副本还是 pending。
        let source = store.range(Seq::new(conation_seq)).into_iter().next();
        let Some(source) = source else {
            symbio_core::plugin_warn!(
                "session",
                "[Autonomous] 意图回读不到（会话 {}，seq {}）",
                session_id,
                conation_seq
            );
            return true;
        };
        let Some(mut candidate) = ConationCandidate::from_event(&source) else {
            symbio_core::plugin_warn!(
                "session",
                "[Autonomous] 意图构造不出候选（会话 {}）",
                session_id
            );
            return true;
        };

        let policy = {
            let cfg = self.config.read().await;
            ConationPolicy {
                enabled: cfg.conation_enabled,
                ..ConationPolicy::default()
            }
        };
        // 授权与本体同一把尺：自主主体在 `authz::ROWS` 里只持 `define.work`。
        let grants = symbio_core::production_matrix();
        let principal = symbio_core::PRINCIPAL_AUTONOMOUS;
        match IntentGate::approve(&mut candidate, &policy, grants, principal) {
            Ok(approved) => {
                // E2 的**动作点**：`task.opened` 只从闸门批准过的候选长出来
                //（`ApprovedIntent` 字段私有 = 编译期唯一出口），这一行验的是升格本身
                // ——少了它，「闸门与自主层同批成对」就退化成一句约定。
                if candidate.is_approved() && !long_goal_declared(&store, approved.goal()) {
                    let task_id = format!("hb-{triggered_seq}");
                    let opened =
                        initiator.open_long_goal(&task_id, approved.goal(), approved.source_seq());
                    if let Err(e) = store.append(opened) {
                        symbio_core::plugin_error!(
                            "session",
                            "[Autonomous] 长目标未入格（会话 {}）：{:?}",
                            session_id,
                            e
                        );
                    }
                }
            }
            Err(reason) => {
                symbio_core::plugin_info!(
                    "session",
                    "[Autonomous] 意图闸门拒绝（会话 {}）：{}",
                    session_id,
                    reason
                );
            }
        }
        true
    }
}

/// 事实源里有没有**已经声明过的**同一个长目标。
///
/// 自主触发按间隔周期性表达同一条「欲」，每次都开格会让就绪集无界增长——而 `readyset`
/// 是模型唯一的调度候选来源，那就等于用自己刷爆自己的提示词。重新发起是 S03 返工机制
/// （`{id}-r{n}`）的事，不由触发器每 tick 重开；「欲」本身照样每次入格（它是流，不是状态）。
///
/// 判定住 ③ [`conation`]（core），这里只**消费**——插件不再自己扫事实源拼一个布尔。
// doc-link-allow D-004: 「曾经声明过吗」问的是事实网格里有无该条记录，属数据语义
/// as-of 取全量读数口径 `read_all`：问的是「这条目标曾经声明过吗」，与 `session/stats`
/// 的复算同一写法。升格靠**溯源**认（`task.opened` 的 `produced_by` 指回欲 seq），
/// 不靠 goal 字符串相等，见 `conation` 模块头的边界一节。
///
// doc-link-allow D-004: 「曾经声明过」是累计口径的表述，非版本变更史
/// 读**完整史**（`range(0)`，S2.7-b2）：「曾经声明过」是累计口径——被快照取代而移入
/// 归档段的那次声明也在里面 ⇒ 压缩前声明过的目标不会被读成「没声明过」（每 tick 重开
/// 一条目标，静默的重复开工）。
fn long_goal_declared(store: &EventWalStore, goal: &str) -> bool {
    conation()
        .read_all(&store.range(Seq::new(0)))
        .value
        .is_declared(goal)
}

// ==================== 自主发起者与意欲闸门（D-189：唯一生产消费方在本文件） ====================

/// 自主发起者（S9 第 21 步，[roadmap/S12](../../../../../docs/003-design/042-自主层与长期目标.md "DE-042-自主层与长期目标")）。
///
/// 形状 = `pattern: decider`（规则驱动，零 LLM）、能力只 `define.work`、
/// `budget_ms` = 自主档 [`LatencyTier::Autonomic`]（四层时延的第四个取值——自主层不是
/// 新架构层，只是同一个参数的第四个取值）。
///
/// ## 两处「单点定义」，本类型各引用一次
///
/// - **主体名**：`PRINCIPAL_AUTONOMOUS`——表里那一行与这里四个事件的 `actor` 是同一个
///   常量。名字分两处写，授权表那行就成了一条谁也管不到的死记录（见该常量的文档）。
/// - **预算**：`LatencyTier::Autonomic`——`86400000` 不在本文件出现第二次。
///
/// 能力那一半**不在这里**：`capabilities = [define.work]` 的权威是授权表
/// （`authz::ROWS` 里 `PRINCIPAL_AUTONOMOUS` 那一行），本类型只负责按那个主体名写事件。
/// 两处各写一份能力清单必然漂移成「表授予 A、闸门判 B」——两边都是合法能力名。
struct AutonomousInitiator;

impl AutonomousInitiator {
    /// 定时触发：**触发器产出事件，不是旁路**——自主行为同样走 I1 单通道。
    fn trigger(&self, source_seq: u64) -> Event {
        Event::pending(
            format!("sys-{source_seq}"),
            symbio_core::EVENT_SYSTEM_TRIGGERED,
            symbio_core::Entity::System,
            symbio_core::Verb::Opened,
            0,
            PRINCIPAL_AUTONOMOUS,
        )
        .with_produced_by(source_seq)
        .with_payload(serde_json::json!({
            "kind": symbio_core::EVENT_TRIGGER_KIND_SCHEDULED
        }))
    }

    /// 表达一条「欲」（E1：欲是数据，必带溯源——否则过不了 IntentGate）。
    fn express_intent(&self, goal: &str, source_seq: u64) -> Event {
        Event::pending(
            format!("want-{source_seq}"),
            symbio_core::EVENT_CONATION_EXPRESSED,
            symbio_core::Entity::Conation,
            symbio_core::Verb::Opened,
            0,
            PRINCIPAL_AUTONOMOUS,
        )
        .with_produced_by(source_seq)
        .with_payload(serde_json::json!({ "goal": goal }))
    }

    /// 把闸门批准的意图落成长目标任务（自主层 `budget_ms` 的完整取值）。
    fn open_long_goal(&self, task_id: &str, goal: &str, source_seq: u64) -> Event {
        Event::pending(
            format!("o-{task_id}"),
            symbio_core::EVENT_TASK_OPENED,
            symbio_core::Entity::Task,
            symbio_core::Verb::Opened,
            0,
            PRINCIPAL_AUTONOMOUS,
        )
        .with_produced_by(source_seq)
        .with_payload(serde_json::json!({
            "task_id": task_id,
            "depends_on": [],
            "goal": goal,
            "budget_ms": symbio_core::LatencyTier::Autonomic.budget_ms(),
        }))
    }

    /// 健康自检：触发时刻测得的状态入格（`system × progressed`）。
    ///
    /// [`Self::trigger`] 说"触发了"，本条说"触发时测得什么"——`system` 实体的两个
    /// 格子一个开一个进（S12 §2 / [plan/01 §6](../../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")
    /// 的 `system` 行）。缺了它，健康自检永远没有事实，而"名字是数据、单点定义"
    /// （D-043）又不允许插件自己拼 `"system.health"` 字面量——写入口只能在这里。
    ///
    /// `source_seq` 取触发事实的 seq：自检是**这一次触发**的观测附录，不是独立事件。
    fn health_event(&self, source_seq: u64, idle_ms: i64) -> Event {
        Event::pending(
            format!("health-{source_seq}"),
            symbio_core::EVENT_SYSTEM_HEALTH,
            symbio_core::Entity::System,
            symbio_core::Verb::Progressed,
            0,
            PRINCIPAL_AUTONOMOUS,
        )
        .with_produced_by(source_seq)
        .with_payload(serde_json::json!({ "idle_ms": idle_ms }))
    }
}

/// 「欲」的治理策略（E3：平凡值 `enabled = false`——关掉后系统退化为纯响应式
/// 且仍完整运行；两层开关独立，这是生产环境最需要的开关）。
#[derive(Debug, Clone)]
struct ConationPolicy {
    /// 是否允许「欲」升格为任务。
    enabled: bool,
    /// 目标长度上界（过宽的目标 = 不可评估的意图，拒绝）。
    max_goal_len: usize,
}

impl Default for ConationPolicy {
    fn default() -> Self {
        ConationPolicy {
            enabled: false,
            max_goal_len: 200,
        }
    }
}

/// 候选意图：从 `conation.expressed` 事件**唯一**构造路径读出，
/// 造出来一定**未被批准**——「欲」不得直接变成「行」（E2）。
#[derive(Debug, Clone, PartialEq, Eq)]
struct ConationCandidate {
    /// 源事件 seq（溯源锚）。
    seq: u64,
    goal: String,
    /// 私有：只有闸门能把它置真。
    approved: bool,
}

impl ConationCandidate {
    /// 唯一构造路径：从欲事件读出候选。**无溯源的欲构造不出候选**（I2 强化）。
    fn from_event(e: &Event) -> Option<Self> {
        if e.kind != symbio_core::EVENT_CONATION_EXPRESSED {
            return None;
        }
        // 来路不明的意图混不进来（I2 强化：无溯源即 None）。
        e.produced_by?;
        let goal = e
            .payload
            .get("goal")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        Some(ConationCandidate {
            seq: e.seq.map(|s| s.value()).unwrap_or(u64::MAX),
            goal,
            approved: false,
        })
    }

    fn is_approved(&self) -> bool {
        self.approved
    }
}

/// 闸门评估结论。
#[derive(Debug, Clone, PartialEq, Eq)]
enum IntentDecision {
    Approved,
    Rejected(&'static str),
}

/// 经闸门批准后的可执行任务（**只有** [`IntentGate::approve`] 能造出来：字段私有，
/// 模块外连结构体字面量都写不出——这是 E2「欲不得直接变成行」的编译期兑现）。
#[derive(Debug, Clone, PartialEq, Eq)]
struct ApprovedIntent {
    from_seq: u64,
    goal: String,
}

impl ApprovedIntent {
    /// 溯源锚：升格自哪一条 `conation.expressed`。
    fn source_seq(&self) -> u64 {
        self.from_seq
    }

    /// 被批准的目标文本。
    fn goal(&self) -> &str {
        &self.goal
    }
}

/// 意图闸门：**「欲」与「行」之间唯一的一道门**（02 §2.3 E2）。
struct IntentGate;

impl IntentGate {
    /// 评估：关停开关优先，其次授权（升格就是「定义工作」），最后目标宽度。
    ///
    /// 授权**不是**一句声明：判据取 [`EffectiveMatrix::can_write`] 那一份
    /// （`define.work`），与本仓任务表写侧闸同一把尺。预算与时延不在这里判——
    /// 它们是**执行面**的闸（`CircuitBreaker` 在真动作上判），把意图升格成任务
    /// 本身不花预算。
    fn evaluate(
        candidate: &ConationCandidate,
        policy: &ConationPolicy,
        grants: &EffectiveMatrix,
        principal: &str,
    ) -> IntentDecision {
        if !policy.enabled {
            return IntentDecision::Rejected("conation disabled");
        }
        if !grants.can_write_name(principal, symbio_core::CAP_DEFINE_WORK) {
            return IntentDecision::Rejected("unauthorized: define.work");
        }
        if candidate.goal.len() > policy.max_goal_len {
            return IntentDecision::Rejected("goal too broad");
        }
        IntentDecision::Approved
    }

    /// 升格：**唯一**能把 [`ConationCandidate`] 变成 [`ApprovedIntent`] 的函数
    /// （`ApprovedIntent` 字段私有，绕过它造不出可执行任务）。
    fn approve(
        candidate: &mut ConationCandidate,
        policy: &ConationPolicy,
        grants: &EffectiveMatrix,
        principal: &str,
    ) -> Result<ApprovedIntent, &'static str> {
        match Self::evaluate(candidate, policy, grants, principal) {
            IntentDecision::Approved => {
                candidate.approved = true;
                Ok(ApprovedIntent {
                    from_seq: candidate.seq,
                    goal: candidate.goal.clone(),
                })
            }
            IntentDecision::Rejected(r) => Err(r),
        }
    }
}

#[cfg(test)]
#[path = "autonomous.test.rs"]
mod tests;
