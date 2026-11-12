//! 自主触发落格 —— 「这次开轮是系统自己发起的」这件事在事实源里的样子
//!
//! ## 为什么这段代码不住在心跳里
//!
//! 心跳回答的是「**什么时候**该开一轮」（间隔、抖动、空闲基线）；本文件回答的是
//! 「这一次开轮是一次**自主触发**，先把三格事实（触发 / 健康 / 欲）落进这个会话的事实源，
//! 落成了才准开」。升格不在本文件：那是由 conation 申报的债与执行体（`D-386`）。
//! 后者不是定时器的属性——它回答的是**谁有资格开这个会话的事实源、在什么时刻开**，
//! 那是存储所有者的写窗口纪律，不是某个触发器的私事（归属裁决见
//! [D-076](../../../symbio-plugin-heartbeat/README.md#d-076-心跳片的八个宿主触点逐条落位调度随片走触发事实落进收件箱写臂 "crates/symbio-plugin-heartbeat/src/README") 裁决一 #5/#6）。
//!
//! 于是「谁来发起」在这里不重要：今天唯一的入口是收件箱写臂（`write(<会话地址>/inbox)`，
//! 写体带 `meta.trigger` 触发声明，识别在 `plugin/nodes.rs::trigger_declaration`），
//! 它经 [`SessionPlugin::request_autonomous_round`] 开这一轮。任何触发器（心跳、巡检、
//! 外部调度、网关）声明同一件事，落的都是这同三格——**发起者的身份不是这段代码的输入**。
//!
//! ## 失败落在队列之外，不落队首
//!
//! 「落格没成就别开轮」这条链子必须在**入队之前**判：收件箱是一支 FIFO
//! （见 `inbox` 模块头），一条永远跑不起来的队首会把它后面的用户消息一起堵死。
//! 所以 [`SessionPlugin::record_autonomous_trigger`] 返回 `false` 时，调用方连队列都不碰，
//! 重试回到调度侧（一个 tick 之后），而不是压在队列里等一个永不到来的条件。

use super::super::active::InboxItem;
use super::super::plugin::SessionPlugin;
use crate::declarations::{EVENT_SYSTEM_HEALTH, EVENT_SYSTEM_TRIGGERED};
use symbio_kernel::chat::chat_node as cm;
use symbio_kernel::event::Event;
use symbio_kernel::store::Store;
use symbio_plugin_api::PRINCIPAL_AUTONOMOUS;
use symbio_sdk::chat::session_chat;

impl SessionPlugin {
    /// 为一次自主触发**开一轮**：先落事实，落成了才带着服务端推导的运行参数入队。
    ///
    /// 返回 `None` = 这一趟没开（原因在 [`Self::record_autonomous_trigger`] 里逐条记了日志），
    /// 调用方据此**不要**推进任何「已触发」的簿记；返回 `Some(条目)` = 已入队，
    /// 什么时候真跑由收件箱消费者决定。
    ///
    /// 运行参数（`mode` / `include_history`）在这里由**服务端按会话自己的数据**推导，
    /// 不从调用方随身带（地址写入面不承载运行选项，见
    /// [D-066](../../../../docs/003-design/012-会话输入与地址.md#d-066-输入面退役的前置条件解除地址写入不承载运行选项三项运行参数已有会话-metadata-回退协议面不必变宽 "DE-012-会话输入与地址") 决策 4）：
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

    /// 把一次定时触发落成事实（[DE-042](../../../../docs/003-design/042-自主层与长期目标.md "DE-042-自主层与长期目标")）。
    ///
    /// 返回 `true` = 触发事实已入格；`false` = 这一趟不触发（原因已记日志）。
    ///
    /// ## 三种不触发，各说各的
    ///
    /// 1. **有写方在写**——事实源是同一会话的唯一写入口（`WalStore::open` 各自
    ///    replay，两个写方并存会重复 `head` ⇒ 盘上出现重复 seq、`seq_monotonic`
    ///    直接变红）。见下「谁有资格写」；
    /// 2. **没有事实源 / 事实源为空**——溯源锚是"上一格的 seq"，一格都没有就无处
    ///    可指。I2 要求触发事件带 `produced_by`（DE-042 §5 第 3 行：自主发起的工作
    ///    必须可追溯），**宁可不触发也不写一条来路不明的触发**；新会话收到第一条
    ///    消息后事实源自然就有了，下一趟就成；
    /// 3. **开档失败**——存储层问题，记错误。
    ///
    /// ## 谁有资格写
    ///
    /// 会话事实源在空闲分支只剩这一个写窗口，本函数把互斥条件检查掉：
    ///
    /// - 收束期写方（v2 运行器的轮末收束）跑在 `is_working` 复位**之前** ⇒ 要求
    ///   `is_working == false`。本函数只判这一条：抢占面已删（`D-388`），收件箱侧
    ///   没有第二个会在空闲分支写事实源的写方。
    ///
    /// 残余窗口是"检查之后一毫秒内跑完一整轮插话"——那是既有设计已接受的一类风险，
    /// 本函数不放大它：检查紧跟开档、格子逐条落、失败逐条记日志。
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
                if inner.is_working() {
                    symbio_plugin_api::plugin_info!(
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
                symbio_plugin_api::plugin_error!(
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
                symbio_plugin_api::plugin_info!(
                    "session",
                    "[Autonomous] 会话 {} 还没有事实源，本次不触发（无可指的溯源锚）",
                    session_id
                );
                return false;
            }
            Err(e) => {
                symbio_plugin_api::plugin_error!(
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
            symbio_plugin_api::plugin_info!(
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
                symbio_plugin_api::plugin_error!(
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
            symbio_plugin_api::plugin_warn!(
                "session",
                "[Autonomous] 健康自检未入格（会话 {}）：{:?}",
                session_id,
                e
            );
        }

        // 「欲」先入格（E1：欲是数据），闸门再决定它要不要升格成任务。
        if let Err(e) = store.append(initiator.express_intent(goal, triggered_seq)) {
            symbio_plugin_api::plugin_warn!(
                "session",
                "[Autonomous] 意图未入格（会话 {}）：{:?}",
                session_id,
                e
            );
            // 触发已经落格 ⇒ 这一轮照常开跑（意图没写成不该连累触发）。
            return true;
        }
        // 升格不在本插件：`conation.expressed` 落格后，由 conation 自己申报的债
        // （`conation.promote`）在环里判开关 / 授权 / 是否已声明，并落 `task.opened`。
        true
    }
}

// ==================== 自主发起者（D-189：唯一生产消费方在本文件） ====================

/// 自主发起者（[DE-042](../../../../docs/003-design/042-自主层与长期目标.md "DE-042-自主层与长期目标")）。
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
            EVENT_SYSTEM_TRIGGERED,
            symbio_kernel::event::Entity::System,
            symbio_kernel::event::Verb::Opened,
            PRINCIPAL_AUTONOMOUS,
        )
        .with_produced_by(source_seq)
        .with_payload(serde_json::json!({
            "kind": symbio_kernel::event::EVENT_TRIGGER_KIND_SCHEDULED
        }))
    }

    /// 表达一条「欲」（E1：欲是数据，必带溯源——升格债按这条 seq 认它，不按 goal 猜）。
    fn express_intent(&self, goal: &str, source_seq: u64) -> Event {
        Event::pending(
            format!("want-{source_seq}"),
            symbio_plugin_api::events::EVENT_CONATION_EXPRESSED,
            symbio_kernel::event::Entity::Conation,
            symbio_kernel::event::Verb::Opened,
            PRINCIPAL_AUTONOMOUS,
        )
        .with_produced_by(source_seq)
        .with_payload(serde_json::json!({ "goal": goal, "source_seq": source_seq }))
    }

    /// 健康自检：触发时刻测得的状态入格（`system × progressed`）。
    ///
    /// [`Self::trigger`] 说"触发了"，本条说"触发时测得什么"——`system` 实体的两个
    /// 格子一个开一个进（DE-042 §2 / [DE-001 §6](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")
    /// 的 `system` 行）。缺了它，健康自检永远没有事实，而"名字是数据、单点定义"
    /// （D-043）又不允许插件自己拼 `"system.health"` 字面量——写入口只能在这里。
    ///
    /// `source_seq` 取触发事实的 seq：自检是**这一次触发**的观测附录，不是独立事件。
    fn health_event(&self, source_seq: u64, idle_ms: i64) -> Event {
        Event::pending(
            format!("health-{source_seq}"),
            EVENT_SYSTEM_HEALTH,
            symbio_kernel::event::Entity::System,
            symbio_kernel::event::Verb::Progressed,
            PRINCIPAL_AUTONOMOUS,
        )
        .with_produced_by(source_seq)
        .with_payload(serde_json::json!({ "idle_ms": idle_ms }))
    }
}

#[cfg(test)]
#[path = "autonomous.test.rs"]
mod tests;
