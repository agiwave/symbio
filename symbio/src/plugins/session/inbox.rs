//! 会话**收件箱**：用户消息入队 + 进程内消费。
//!
//! ## 它解决什么
//!
//! `chat/send` 把「接收消息」与「跑一轮」绑在一起，并且要有一个**调用方连接**在
//! 那儿等结果。子智能体空间没有这样的调用方——它的会话照样要能完整地跑。于是
//! 用户消息改以**对空间的写入**表达：往 `<sid>/inbox` 写一条即入队，由空间自己
//! 消费（[ADR-026](docs/DECISIONS.md)）。顶层会话走同一条路，只是恰好也有人替它
//! 调 `chat/send`——`chat/send` 因此退化成「入队」。
//!
//! ## 一条消息的两个落点
//!
//! | 阶段 | 住在 | 谁写 |
//! |---|---|---|
//! | 已入队、未消费 | [`InboxItem`]（本模块的队列）→ VDFS `<sid>/inbox/<iid>` | [`SessionPlugin::enqueue_inbox`] |
//! | 已消费 | 转写（存储 + `Transcript`） | 统一发送链路（`handle_chat_send_oneoff`） |
//!
//! 条目**出队即消失**：它不是"被处理过的队列项"，而是"还没成为消息的消息"。
//! 「正在处理的那条」在转写里，不在队列里——所以取消它走中止（`chat/abort`），
//! 而不是删队列项。
//!
//! ## 消费规则（与 ADR-026 的三条决定一一对应）
//!
//! 1. **FIFO**：同一个会话严格按入队顺序，先到先跑；
//! 2. **忙则排队，不抢占**：会话正在跑（`is_working`）时只把条目留在队里，等这一轮
//!    收尾再取下一条——不合并、不中断、不并发；
//! 3. **机制统一**：顶层会话与子智能体空间共用本模块，没有"子智能体专用分支"。
//!
//! ## 为什么是**每插件实例一个**消费者
//!
//! 消费者在 [`SessionPlugin::build`] 里起一次，扫过本实例名下所有会话的队列。
//! 不按会话起任务，是因为"按会话起"必须由写入方触发，而写入方（provider 的
//! `write`）手上只有 `&self`——拿不到 `Arc<SessionPlugin>` 就 spawn 不出消费者，
//! 只能靠"等某个恰好持有 Arc 的时机"来补，于是"第一次写入能不能被消费"取决于
//! 装配顺序（脆弱且不可测）。实例级消费者在构造点起，**早于任何写入**，与谁写、
//! 什么时候写无关。

use super::active::InboxItem;
use super::plugin::{inbox_item_node, inbox_item_path, SessionPlugin};
use crate::symbio_core::schemas::{session::chat_message as cm, session::session_chat};
use crate::symbio_core::{
    vdfs, PluginError, PluginInvokeRequest, PluginInvokeRequestExt, SESSION_ID, WORKDIR,
};
use std::sync::Arc;

/// 忙等间隔：消费者发现会话在跑、或队列非空但本轮刚被拒时的轮询周期。
///
/// ## 为什么是轮询而不是等待一个"本轮结束"信号
///
/// `is_working` 的置位/复位散落在多条收尾路径上（正常收尾、中止收尾、失败收敛、
/// `WorkingGuard` 的 panic 兜底）。在其中**一处**加唤醒，漏掉任何一条都会让消费者
/// 永久卡住——而卡住的表现是"消息永远不发"，静默且致命。
///
/// 轮询的代价是有界的：只在「队列非空 **且** 会话正在跑」这个窗口内发生，一轮几秒
/// ⇒ 几十次空转；换来的是「无论哪条路径把回合收敛掉，下一条都会被取走」。
/// 队列**空**时不吃轮询——那时挂在 `inbox_wake` 上，入队即被唤醒。
const BUSY_POLL: std::time::Duration = std::time::Duration::from_millis(50);

impl SessionPlugin {
    /// **入队**一条用户消息（收件箱集合的唯一写入口）。
    ///
    /// 同时做两件事，顺序固定：入队 → 发变更 → 唤醒消费者。变更落在**条目自身**的
    /// 地址上（`<sid>/inbox/<iid>`）并带条目节点视图，因此订阅方零回读即可显示
    /// "有一条待发的消息"（与 ADR-025 的落点约定一致）。
    ///
    /// ## 条目 id 的三个来源（优先级从高到低）
    ///
    /// 1. **地址末段**（`write(<sid>/inbox/<iid>)` 的 `iid`）——地址即身份；
    /// 2. **消息自带 id**（`chat/send` 带的是客户端生成的 id）。保留它是**必须的**：
    ///    前端先按自己的 id 放了乐观副本，落库回包若换了 id，那条副本就永远等不到
    ///    替身，界面上会变成两条；
    /// 3. `turn::llm_short_id()`（都没有时生成）——与消息 id 同一套格式，地址末段、
    ///    日志、前端 key 都读它。
    ///
    /// 三条都收在这一个地方：分散到调用点就会出现"同一个地址取到两个 id"。
    pub(crate) async fn enqueue_inbox(
        &self,
        session_id: &str,
        id: Option<String>,
        message: cm::ChatMessage,
        params: session_chat::Request,
        workdir: Option<String>,
    ) -> InboxItem {
        let mut message = message;
        let id = id
            .filter(|s| !s.trim().is_empty())
            .or_else(|| (!message.id.trim().is_empty()).then(|| message.id.clone()))
            .unwrap_or_else(crate::symbio_core::llm_short_id);
        // 消息 id 与条目 id **是同一个值**：地址末段即身份，两处各生成一个会让
        // 「按地址取消息」在两套 id 之间对不上（见 `message_path` 的同款约定）。
        message.id = id.clone();
        let item = InboxItem {
            id: id.clone(),
            message,
            params,
            workdir,
        };

        let state = self.active_mgr.get_or_create(session_id).await;
        state.inner.write().await.inbox.push_back(item.clone());
        self.change_subs.notify(&vdfs::VdfsChange::with_data(
            inbox_item_path(session_id, &id),
            inbox_item_node(&item),
        ));
        self.inbox_wake.notify_waiters();
        item
    }

    /// 取消一条**仍在排队**的条目（`delete(<sid>/inbox/<iid>)` 的实现）。
    ///
    /// 已出队（正在处理）的条目**找不到**：它已经不是队列项了。取消正在跑的那条
    /// 是**中止**（`chat/abort`）——两种取消作用在不同对象上，因此是两个动作，
    /// 不硬塞进一个动词。返回 `false` 表示"队里没有这一条"，由调用方翻成
    /// `NotFound`（不静默成功）。
    pub(crate) async fn cancel_inbox_item(&self, session_id: &str, iid: &str) -> bool {
        let state = self.active_mgr.get_or_create(session_id).await;
        let removed = {
            let mut inner = state.inner.write().await;
            match inner.inbox.iter().position(|i| i.id == iid) {
                Some(pos) => inner.inbox.remove(pos).is_some(),
                None => false,
            }
        };
        if removed {
            self.change_subs
                .notify(&vdfs::VdfsChange::bare(inbox_item_path(session_id, iid)));
        }
        removed
    }

    /// 清空队列（`action(<sid>/inbox, "clear")` 的实现）——返回被取消的条目 id。
    ///
    /// 与会话转写的 `clear` 同款语义：容器（会话）保留，集合清空。两者都被表达成
    /// **逐条移除**（每条落在自己的地址上），因此消费端不必认识"集合级删除"。
    pub(crate) async fn clear_inbox(&self, session_id: &str) -> Vec<String> {
        let state = self.active_mgr.get_or_create(session_id).await;
        let ids: Vec<String> = {
            let mut inner = state.inner.write().await;
            inner.inbox.drain(..).map(|i| i.id).collect()
        };
        for id in &ids {
            self.change_subs
                .notify(&vdfs::VdfsChange::bare(inbox_item_path(session_id, id)));
        }
        ids
    }

    /// 该会话当前**待消费**的条目（VDFS `<sid>/inbox` 的数据源）
    pub(crate) async fn inbox_items(&self, session_id: &str) -> Vec<InboxItem> {
        let state = self.active_mgr.get_or_create(session_id).await;
        let inner = state.inner.read().await;
        inner.inbox.iter().cloned().collect()
    }

    /// 启动本实例的**收件箱消费者**（常驻，见模块头「为什么是每插件实例一个」）。
    ///
    /// 无 Tokio runtime 时静默跳过：那种场景（部分单测）没有真实会话要跑，调用方
    /// 不因此失败；单测要验证消费顺序时直接调 [`Self::drain_inbox_once`]。
    pub(crate) fn spawn_inbox_consumer(self: Arc<Self>) {
        if tokio::runtime::Handle::try_current().is_err() {
            return;
        }
        tokio::spawn(async move {
            loop {
                let started = self.clone().drain_inbox_once().await;
                if started {
                    continue;
                }
                // 没有可启动的：有排队（都被占着）就短轮询，否则等入队唤醒
                if self.has_pending_inbox().await {
                    tokio::time::sleep(BUSY_POLL).await;
                } else {
                    self.inbox_wake.notified().await;
                }
            }
        });
    }

    /// 还有会话排着队吗（决定"空转"还是"等唤醒"）
    async fn has_pending_inbox(&self) -> bool {
        let sessions = self.active_mgr.sessions.read().await;
        let states: Vec<_> = sessions.values().cloned().collect();
        drop(sessions);
        for state in states {
            if !state.inner.read().await.inbox.is_empty() {
                return true;
            }
        }
        false
    }

    /// 扫一遍所有会话，为**空闲**且队列非空的会话各取出一条开跑。
    ///
    /// 返回"这一趟是否启动了至少一轮"——调用方据此决定是立刻再扫（可能还有别的
    /// 会话排着）还是转入等待。
    pub(crate) async fn drain_inbox_once(self: Arc<Self>) -> bool {
        let sessions = self.active_mgr.sessions.read().await;
        let states: Vec<_> = sessions.values().cloned().collect();
        drop(sessions);

        let mut started = false;
        for state in states {
            // 忙则不抢占：留在队里，等本轮收尾后的下一趟
            if state.inner.read().await.is_working {
                continue;
            }
            // 出队。写完即释放锁——`start_turn` 还会去写同一把锁。
            //
            // **出队要发一条变更**：条目已经不在队列里了，而不通知就等于告诉订阅方
            // 「它还在」。变更是**无载荷**的（`bare`）——删掉的节点本就没有视图可带，
            // 消费方按「载荷缺失 + 回读 NotFound」收敛（ADR-025 的删除表达）。
            // 与 `cancel_inbox_item` 同一手法：两种「没了」对订阅方是同一件事。
            let item = {
                let mut inner = state.inner.write().await;
                inner.inbox.pop_front()
            };
            let Some(item) = item else { continue };
            self.change_subs
                .notify(&vdfs::VdfsChange::bare(inbox_item_path(
                    &state.session_id,
                    &item.id,
                )));

            match self.clone().run_inbox_turn(&state.session_id, &item).await {
                Ok(()) => started = true,
                Err(e) => {
                    crate::plugin_error!(
                        "session",
                        "收件箱条目 {} 未能启动（会话 {}）：{}",
                        item.id,
                        state.session_id,
                        e
                    );
                    // 启动失败 = 这一条没被消费。放回队首，等下一趟（用户也可删它）
                    state.inner.write().await.inbox.push_front(item);
                }
            }
        }
        started
    }

    /// 用收件箱条目驱动一轮（消费者调用；单测也直接调它以绕开后台任务）。
    ///
    /// 上下文**自己造**：只带目标会话 id 与工作目录。发起者的请求上下文刻意不沿用
    /// ——它的 `SESSION_ID` 头是发起者自己的会话（跨空间写入时二者不同），沿用会
    /// 把消息投错会话（见 [`InboxItem`] 的说明）。
    pub(crate) async fn run_inbox_turn(
        self: Arc<Self>,
        session_id: &str,
        item: &InboxItem,
    ) -> Result<(), PluginError> {
        let ctx: Arc<dyn PluginInvokeRequest> =
            Arc::new(crate::symbio_core::PluginSimpleRequest::new(None, None));
        ctx.set(SESSION_ID, session_id.to_string());
        if let Some(w) = &item.workdir {
            ctx.set(WORKDIR, w.clone());
        }
        let req = session_chat::Request {
            session_id: Some(session_id.to_string()),
            message: Some(item.message.clone()),
            ..item.params.clone()
        };
        ctx.set_payload(req)?;
        // 直呼**执行**而非入口：入口会把 message 再次入队（无限循环）
        self.start_turn(ctx).await?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "inbox.test.rs"]
mod tests;
