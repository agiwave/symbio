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
//!    收尾再取下一条——不合并、不中断、不并发；**排队的是插话，不是在跑的任务**：
//!    忙窗里 core 的抢占判定者（`PreemptionDecider`）会评估当前任务要不要挂起
//!    （S8 第 19 步），结论经 [`super::super::active::PreemptPending`] 跨过忙窗、
//!    在空闲分支落成事实。挂起 / 恢复**都不开新轮、都不动队列**，所以与「不抢占」
//!    不冲突（判定顺序见 [04 §2.1](../../../../docs/plan/04-工程落地.md)）；
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

use super::super::active::{ActiveSessionState, InboxItem, PreemptPending};
use super::super::plugin::{inbox_item_node, inbox_item_path, SessionPlugin};
use super::supplements;
use crate::symbio_core::{chat_message as cm, session_chat, VdfsChange};
use crate::symbio_core::{
    Event, EventWalStore, LatencyTier, PluginError, PluginInvokeRequest, PluginInvokeRequestExt,
    Preemption, PreemptionDecider, Seq, Store, SESSION_ID, WORKDIR,
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
/// ⇒ 几十次空转；换来的是「无论哪条路径把回合收敛掉，下一条都会被取走」。队列
/// **空**时不吃轮询——那时挂在 `inbox_wake` 上，入队即被唤醒。
///
/// 第二个吃轮询的窗口：**抢占判定待结算**（插话轮已开跑、等它结束后写恢复事件）。
/// 没有这个窗口，插话轮收尾后没人来收，任务就永久停在 `held` 上、再也不进
/// `readyset`——而 readyset 是模型唯一的调度候选来源，于是任务凭空消失且没有一条
/// 消息说得出为什么（S07 §5 的静默失效）。代价同样是**有界的**：只在挂起未结清期间
/// 存在，恢复落格即停止。
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
        self.change_subs.notify(&VdfsChange::with_data(
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
                .notify(&VdfsChange::bare(inbox_item_path(session_id, iid)));
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
                .notify(&VdfsChange::bare(inbox_item_path(session_id, id)));
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
                // 没有可启动的：有活（排队被占着 / 挂起待收）就短轮询，否则等入队唤醒
                if self.has_pending_work().await {
                    tokio::time::sleep(BUSY_POLL).await;
                } else {
                    self.inbox_wake.notified().await;
                }
            }
        });
    }

    /// 还有活没干完吗（决定"空转"还是"等唤醒"）——两个来源，缺一个都会睡死：
    ///
    /// 1. 有会话排着队（都被占着，等这一轮收尾）；
    /// 2. 有会话挂着**未结清的抢占判定**（`preempt != None`）。挂起在插话轮**之前**
    ///    落格，恢复必须在它**之后**——中间隔着插话轮，而那段时间队列恰好是空的
    ///    （消息已出队）。只看队列就会在这里睡过去，恢复事件永不落格 ⇒ 任务永久
    ///    停在 `held` 上（S07 §5 的静默失效）。
    async fn has_pending_work(&self) -> bool {
        let sessions = self.active_mgr.sessions.read().await;
        let states: Vec<_> = sessions.values().cloned().collect();
        drop(sessions);
        for state in states {
            let inner = state.inner.read().await;
            if !inner.inbox.is_empty() || inner.preempt != PreemptPending::None {
                return true;
            }
        }
        false
    }

    /// 事实源的只读切片（抢占判定的输入）。开**只读档**：读方不创建文件、不截尾，
    /// 与 `session/stats` 同一条口径；读不开 ⇒ 空切片（没有事实就没有在跑任务）。
    ///
    /// 判据里只有两族事件被 [`PreemptionDecider::decide`] 消费——`task.*` 与
    /// `chat.assistant.final`，两者都只在**收束转写**时入格。这就是「一忙窗一判」
    /// 成立的前提：忙窗内本切片不会变，判过一次不等于漏判。
    fn preemption_facts(&self, session_id: &str) -> Vec<Event> {
        let path = super::super::paths::session_dir(&self.storage_dir(), session_id)
            .join(super::super::paths::V2_WAL_FILE);
        let Ok(store) = EventWalStore::open_readonly(&path) else {
            return Vec::new();
        };
        store.range(Seq::new(0))
    }

    /// 抢占事实的唯一写入口：开写档 → 取锚 → 按 `build(anchor)` 落格。
    ///
    /// 返回值三分，三种结局各不相同（调用方据此决定状态清不清）：
    ///
    /// - `Ok(Some(锚))` = 落格完成；
    /// - `Ok(None)` = **没有事实源**（路径不存在）⇒ 没有任务可挂起、也没有要收的，
    ///   状态**该清就清**——否则消费者会为一个没人认领的挂起轮询到天荒地老；
    /// - `Err` = 开档失败（日志已记）⇒ 状态**不清**，下一趟重试。开档失败是暂时的，
    ///   而清掉状态等于把「只挂不收」钉成永久——那正是 J3。
    ///
    /// ## 为什么写点只在空闲分支
    ///
    /// `WalStore::open` 各自 replay，两个写方并存会重复 `head` ⇒ 本轮为止唯一的
    /// 写方是收束期的 `record_to_wal`。它跑在 `is_working` 复位**之前**
    /// （`emit_session_state(Finished)` 才复位），所以空闲分支落笔与它不重叠。
    ///
    /// ## 锚 = 落格前的 `head`，不是任务开格 seq
    ///
    /// 04 §2.2 步 1「记录当前 seq 为 T」。它同时是幂等键的后半段，因此必须**每次
    /// 都不同**：同一任务可以挂起多次（挂起 → 恢复 → 再挂起），拿开格 seq 当锚会让
    /// 第二次撞 `Duplicate` 而**静默丢掉**那次挂起——任务因此照常出现在就绪集里，
    /// 挂起等于没发生（J3）。
    fn append_preemption(
        &self,
        session_id: &str,
        build: impl FnOnce(u64) -> Vec<Event>,
    ) -> Result<Option<u64>, String> {
        let path = super::super::paths::session_dir(&self.storage_dir(), session_id)
            .join(super::super::paths::V2_WAL_FILE);
        if !path.exists() {
            return Ok(None);
        }
        let store = EventWalStore::open(&path).map_err(|e| {
            crate::plugin_error!(
                "session",
                "[preempt] 事实源开档失败（会话 {}）：{}",
                session_id,
                e
            );
            format!("开档失败：{e}")
        })?;
        let anchor = store.head().value();
        for event in build(anchor) {
            let id = event.event_id.clone();
            if let Err(e) = store.append(event) {
                crate::plugin_warn!(
                    "session",
                    "[preempt] 事件 {id} 未入格（会话 {session_id}）：{e:?}"
                );
            }
        }
        Ok(Some(anchor))
    }

    /// 忙窗内的抢占判定（S8 第 19 步，[04 §2.1](../../../../docs/plan/04-工程落地.md)）。
    ///
    /// 三条边界，少一条这个函数就会变成噪声或变成第二个时延源：
    ///
    /// 1. **只在队列非空时判**——没有插话就没有"要保护的对象"，而判据是整条 WAL
    ///    重放，不该在每 50ms 的空转里白做；
    /// 2. **一忙窗一判**——标记由空闲分支复位，理由见 [`Self::preemption_facts`]；
    /// 3. **只判定不落格**——此刻收束期写方随时可能落笔，并发 `open` 会重复 `head`。
    ///    结论进 `PreemptPending`，由空闲分支结算（[`Self::commit_preemption`] /
    ///    [`Self::settle_preemption`]）。
    ///
    /// `elapsed_ms` 计的是**读判据本身**的耗时：判定者要读它的输入，这段时延就是
    /// 判定的时延（S07 的 `budget_ms = 80` 说的是这件事，不是纯函数自身）。
    async fn judge_preemption(&self, state: &Arc<ActiveSessionState>) {
        {
            let inner = state.inner.read().await;
            let judged = inner.preempt != PreemptPending::None || inner.preempt_judged;
            if inner.inbox.is_empty() || judged {
                return;
            }
        }
        let started_at = std::time::Instant::now();
        let snapshot = self.preemption_facts(&state.session_id);
        let elapsed_ms = started_at.elapsed().as_millis() as u64;

        let mut inner = state.inner.write().await;
        inner.preempt_judged = true;
        if let Preemption::Suspend { task_id, as_of_seq } =
            PreemptionDecider.decide(&snapshot, elapsed_ms, LatencyTier::Reflex.budget_ms())
        {
            inner.preempt = PreemptPending::Hold { task_id, as_of_seq };
        }
    }

    /// 把「判出了挂起」落成事实（04 §2.2 步 1–2），再把状态推到待收。
    ///
    /// 在空闲分支、**取到插话批之后**调用：没有插话轮要保护就什么都不写——
    /// `task.held` 会让任务退出 `readyset`，而 readyset 是模型唯一的调度候选来源，
    /// 写了却没人来收就是任务凭空消失（S07 §5 的静默失效）。
    async fn commit_preemption(&self, state: &Arc<ActiveSessionState>) {
        let hold = {
            let inner = state.inner.read().await;
            match &inner.preempt {
                PreemptPending::Hold { task_id, as_of_seq } => Some((task_id.clone(), *as_of_seq)),
                _ => None,
            }
        };
        let Some((task_id, as_of_seq)) = hold else {
            return;
        };
        let written = self.append_preemption(&state.session_id, |anchor| {
            let decider = PreemptionDecider;
            vec![
                decider.held_event(&task_id, anchor, as_of_seq),
                decider.control_event("interrupt-suspend", anchor),
            ]
        });
        let mut inner = state.inner.write().await;
        match written {
            // 落格了 → 等插话轮结束后收（`Resume` ⇒ `settle_preemption` 写恢复）。
            Ok(Some(_)) => inner.preempt = PreemptPending::Resume { task_id },
            // 没有事实源 ⇒ 没有任务可挂起、也没有要收的，判定作废。
            // 状态**必须清**：留着 `Resume` 会让消费者为一个没人认领的挂起空转到天荒地老。
            Ok(None) => inner.preempt = PreemptPending::None,
            // 开档失败（日志已记）⇒ **保留 `Hold`**：插话批还扣在队首，下一趟重试；
            // 若插话就此消失，`settle_preemption` 会把 `Hold` 作废掉。
            Err(_) => {}
        }
    }

    /// 结清抢占：写恢复事件，或把没有插话可处理的那次判定作废（04 §2.2 步 6）。
    ///
    /// 在空闲分支、**取到空批之后**调用。三种去向：
    ///
    /// - `Resume` → 写 `task.progress`。**必须写**：`held` 一旦落格就一直生效，
    ///   而没有任何消息会让它失效——任务从此不进 `readyset`，且说得出原因的
    ///   日志一条都没有（J3）；
    /// - `Hold` → 插话没了（被取消、或被轮边界抽走）⇒ 判定作废，不落格：写了也得
    ///   马上撤销；
    /// - `None` → 无事。
    ///
    /// 结不清的只有**开档失败**：留着 `Resume` 下一趟空闲再收（挂起不能只挂不收）。
    /// 「没有事实源」不算结不清——那说明没有挂起可收，清掉才是对的。
    async fn settle_preemption(&self, state: &Arc<ActiveSessionState>) {
        let pending = {
            let inner = state.inner.read().await;
            inner.preempt.clone()
        };
        match pending {
            PreemptPending::None => {}
            PreemptPending::Hold { .. } => {
                state.inner.write().await.preempt = PreemptPending::None;
            }
            PreemptPending::Resume { task_id } => {
                let settled = self.append_preemption(&state.session_id, |anchor| {
                    vec![PreemptionDecider.resume_event(&task_id, anchor)]
                });
                if settled.is_err() {
                    // 开档失败（日志已记）⇒ 留着 `Resume` 下一趟再收：清掉就等于把
                    // 「只挂不收」钉成永久（J3）。
                    return;
                }
                // 落成了 / 没有事实源（没有挂起可收）⇒ 结清。
                state.inner.write().await.preempt = PreemptPending::None;
            }
        }
    }

    /// 扫一遍所有会话，为**空闲**且队列非空的会话各抽一批开跑。
    ///
    /// 返回"这一趟是否启动了至少一轮"——调用方据此决定是立刻再扫（可能还有别的
    /// 会话排着）还是转入等待。
    ///
    /// ## 抽干整队，而不是取一条（补充整合）
    ///
    /// 一次抽干**整队**（至多 `supplements_max_per_drain` 条）并合并成**一条**
    /// 用户消息（见 [`supplements`]）。于是"用户连发三条"不再各占一轮，而是作为
    /// **同一轮**的一条输入被整体处理。
    ///
    /// 只抽一条（今天的行为）会让 n 条补充吃掉 n 份存储裁剪预算——裁剪按
    /// `role = User` 的消息数算分水岭（`chat_session/write.rs::prune_historical_tool_calls`），
    /// 用户多说的两句话会让历史悄悄少两轮。
    ///
    /// `supplements_enabled = false`（平凡值）或 `max_per_drain = 1` 时退化为
    /// "取一条"，与今天逐字一致。
    pub(crate) async fn drain_inbox_once(self: Arc<Self>) -> bool {
        let sessions = self.active_mgr.sessions.read().await;
        let states: Vec<_> = sessions.values().cloned().collect();
        drop(sessions);

        let mut started = false;
        for state in states {
            // 忙则不抢占新轮：留在队里，等本轮收尾后的下一趟。
            // 但忙窗正是判定的时刻——判一次，结论背着跨过忙窗（见 `PreemptPending`）。
            if state.inner.read().await.is_working {
                self.judge_preemption(&state).await;
                continue;
            }
            // 空闲 = 上一忙窗到此为止：复位「判过」标记，下一轮忙窗重新判。
            state.inner.write().await.preempt_judged = false;
            // 出队。写完即释放锁——`start_turn` 还会去写同一把锁。
            //
            // **出队要逐条发变更**：条目已经不在队列里了，而不通知就等于告诉订阅方
            // 「它还在」。变更是**无载荷**的（`bare`）——删掉的节点本就没有视图可带，
            // 消费方按「载荷缺失 + 回读 NotFound」收敛（ADR-025 的删除表达）。
            // 与 `cancel_inbox_item` 同一手法：两种「没了」对订阅方是同一件事。
            let batch = self.take_inbox_batch(&state).await;
            if batch.is_empty() {
                // 没有插话要处理 ⇒ 上一次判定到此作废 / 未收的挂起立刻收。
                // **先取批再结算**：没有插话就没有要保护的对象，这时候不写事实
                // （写了也得马上撤销，白留一条必须有人来收的状态）。
                self.settle_preemption(&state).await;
                continue;
            }
            // 有插话要开跑：先把挂起落成事实（04 §2.2 步 1–2），再开轮——顺序反了
            // 就是「插话轮开跑时任务还在就绪集里」，判定白做。
            self.commit_preemption(&state).await;

            match self.clone().run_inbox_turn(&state.session_id, &batch).await {
                Ok(()) => started = true,
                Err(e) => {
                    crate::plugin_error!(
                        "session",
                        "收件箱条目 {} 未能启动（会话 {}）：{}",
                        batch[0].id,
                        state.session_id,
                        e
                    );
                    // 启动失败 = 这一批没被消费。**按原顺序整体放回队首**：
                    // 倒序 push_front 才是原顺序——顺序是这个队列唯一的不变量
                    // （FIFO，ADR-026），放错就再也对不回来。
                    let mut inner = state.inner.write().await;
                    for item in batch.into_iter().rev() {
                        inner.inbox.push_front(item);
                    }
                }
            }
        }
        started
    }

    /// 取走该会话队首的一批条目（至多 `supplements_max_per_drain` 条），并逐条发删除变更。
    ///
    /// 抽干与变更通知必须成对：分开写就会出现"抽了没通知"（前端以为条目还在）
    /// 或"通知了没抽"（条目凭空消失）两种静默不一致。
    ///
    /// `pub(crate)`：轮边界抽干（[`super::super::chat_loop::SupplementDrain`]）经它取批，
    /// 再交给 [`super::supplements::merge_supplements`] 合并。两个抽干点共用同一个取批
    /// 实现——各写一遍必然在"上界取谁""变更发几次"上漂移。
    pub(crate) async fn take_inbox_batch(&self, state: &Arc<ActiveSessionState>) -> Vec<InboxItem> {
        let limit = {
            let cfg = self.config.read().await;
            if cfg.supplements_enabled {
                cfg.supplements_max_per_drain.max(1)
            } else {
                1
            }
        };

        let batch: Vec<InboxItem> = {
            let mut inner = state.inner.write().await;
            let n = inner.inbox.len().min(limit);
            inner.inbox.drain(..n).collect()
        };

        for item in &batch {
            self.change_subs.notify(&VdfsChange::bare(inbox_item_path(
                &state.session_id,
                &item.id,
            )));
        }
        batch
    }

    /// 用收件箱条目驱动一轮（消费者调用；单测也直接调它以绕开后台任务）。
    ///
    /// 上下文**自己造**：只带目标会话 id 与工作目录。发起者的请求上下文刻意不沿用
    /// ——它的 `SESSION_ID` 头是发起者自己的会话（跨空间写入时二者不同），沿用会
    /// 把消息投错会话（见 [`InboxItem`] 的说明）。
    ///
    /// 入参是一**批**条目（`drain_inbox_once` 抽干所得）：合并成一条用户消息后开跑。
    /// 批内条目的 `params` / `workdir` **一律忽略**，只贡献正文——本轮参数取自
    /// 批内第一条（它是这一轮的开端，见 [`supplements::merge_supplements`]）。
    pub(crate) async fn run_inbox_turn(
        self: Arc<Self>,
        session_id: &str,
        batch: &[InboxItem],
    ) -> Result<(), PluginError> {
        let Some(merged) = supplements::merge_supplements(batch) else {
            return Ok(());
        };
        let first = &batch[0];

        let ctx: Arc<dyn PluginInvokeRequest> =
            Arc::new(crate::symbio_core::PluginSimpleRequest::new(None, None));
        ctx.set(SESSION_ID, session_id.to_string());
        if let Some(w) = &first.workdir {
            ctx.set(WORKDIR, w.clone());
        }
        let req = session_chat::Request {
            session_id: Some(session_id.to_string()),
            message: Some(merged),
            ..first.params.clone()
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
