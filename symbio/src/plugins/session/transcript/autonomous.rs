//! 自主触发落格 —— 「这次开轮是系统自己发起的」这件事在事实源里的样子
//!
//! ## 为什么这段代码不住在心跳里
//!
//! 心跳回答的是「**什么时候**该开一轮」（间隔、抖动、空闲基线）；本文件回答的是
//! 「这一次开轮是一次**自主触发**，先把四格事实落进这个会话的事实源，落成了才准开」。
//! 后者不是定时器的属性——它回答的是**谁有资格开这个会话的事实源、在什么时刻开**，
//! 那是存储所有者的写窗口纪律，不是某个触发器的私事（归属裁决见
//! [ADR-076](../../../../../docs/010-adr/076.md "ADR-076") 裁决一 #5/#6）。
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
use crate::symbio_core::chat_node as cm;
use crate::symbio_core::session_chat;
use crate::symbio_core::{
    conation, AutonomousInitiator, Budget, ConationCandidate, ConationPolicy, EventWalStore,
    IntentGate, Seq, Store,
};

impl SessionPlugin {
    /// 为一次自主触发**开一轮**：先落事实，落成了才带着服务端推导的运行参数入队。
    ///
    /// 返回 `None` = 这一趟没开（原因在 [`Self::record_autonomous_trigger`] 里逐条记了日志），
    /// 调用方据此**不要**推进任何「已触发」的簿记；返回 `Some(条目)` = 已入队，
    /// 什么时候真跑由收件箱消费者决定。
    ///
    /// 运行参数（`mode` / `include_history`）在这里由**服务端按会话自己的数据**推导，
    /// 不从调用方随身带（地址写入面不承载运行选项，见
    /// [ADR-066](../../../../../docs/010-adr/066.md "ADR-066") 决策 4）：
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
            // 无「来源声明」——轮末承诺的 `to` 由取值点落本机用户（那条不一致的归属见 ADR-076 后果）。
            ..Default::default()
        };
        Some(
            self.enqueue_inbox(session_id, iid, message, params, None)
                .await,
        )
    }

    /// 把一次定时触发落成事实（S9 第 21 步，[roadmap/S12](../../../../../docs/009-task/132-自主层与长期目标.md "T-132-自主层与长期目标")）。
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
                if inner.is_working || inner.preempt != PreemptPending::None {
                    crate::plugin_info!(
                        "session",
                        "[Autonomous] 会话 {} 有写方在写，本次触发不落格（下个 tick 重试）",
                        session_id
                    );
                    return false;
                }
            }
        }

        // 溯源锚 = 上一格的 seq：触发是"从当时的状态出发"的，指过去的事件而不是
        // 指自己。`head == 0` 既包含"事实源不存在"也包含"空文件"——两种都没有上一格。
        // 顺带挡掉目录不存在的情形：再往下就会 `append`（写方入口对缺父目录是 panic）。
        let path = super::super::paths::session_dir(&self.storage_dir(), session_id)
            .join(super::super::paths::V2_WAL_FILE);
        let store = match EventWalStore::open(&path) {
            Ok(store) => store,
            Err(e) => {
                crate::plugin_error!(
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
            crate::plugin_info!(
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
                crate::plugin_error!(
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
            crate::plugin_warn!(
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
                crate::plugin_warn!(
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
            crate::plugin_warn!(
                "session",
                "[Autonomous] 意图回读不到（会话 {}，seq {}）",
                session_id,
                conation_seq
            );
            return true;
        };
        let Some(mut candidate) = ConationCandidate::from_event(&source) else {
            crate::plugin_warn!(
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
        match IntentGate::approve(&mut candidate, &policy, &IntentGate::issue_warrant()) {
            Ok(approved) => {
                // E2 的**动作点**：`task.opened` 只从盖过章的候选长出来。闸门是发牌口
                //（`issue_warrant` 是唯一一道），这一行验的是牌本身——少了它，「闸门与
                // 自主层同批成对」就退化成一句约定。
                if candidate.is_approved() && !long_goal_declared(&store, &approved.goal) {
                    let task_id = format!("hb-{triggered_seq}");
                    let opened =
                        initiator.open_long_goal(&task_id, &approved.goal, approved.from_seq);
                    if let Err(e) = store.append(opened) {
                        crate::plugin_error!(
                            "session",
                            "[Autonomous] 长目标未入格（会话 {}）：{:?}",
                            session_id,
                            e
                        );
                    }
                }
            }
            Err(reason) => {
                crate::plugin_info!(
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
/// as-of 取上界 `i64::MAX`：问的是「这条目标曾经声明过吗」，与 `session/stats`
/// 的复算同口径。升格靠**溯源**认（`task.opened` 的 `produced_by` 指回欲 seq），
/// 不靠 goal 字符串相等，见 `conation` 模块头的边界一节。
///
// doc-link-allow D-004: 「曾经声明过」是累计口径的表述，非版本变更史
/// 读**完整史**（`range(0)`，S2.7-b2）：「曾经声明过」是累计口径——被快照取代而移入
/// 归档段的那次声明也在里面 ⇒ 压缩前声明过的目标不会被读成「没声明过」（每 tick 重开
/// 一条目标，静默的重复开工）。
fn long_goal_declared(store: &EventWalStore, goal: &str) -> bool {
    conation()
        .apply(&store.range(Seq::new(0)), i64::MAX, Budget::generous())
        .value
        .is_declared(goal)
}

#[cfg(test)]
#[path = "autonomous.test.rs"]
mod tests;
