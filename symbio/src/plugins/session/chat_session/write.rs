//! 写路径：消息列表变更的**唯一写入口**（[`PersistentChatSession::apply`]）与
//! 两个存储原语（[`PersistentChatSession::append_messages`] /
//! [`PersistentChatSession::replace_all`]）。
//!
//! 顺序反转（ADR-054 决策 2）在这里落地：
//! `apply` **先落事实**、再折列表、再写缓存——内存列表是事实的一个渲染，
//! 不是「正在被编辑的真相」。事实与缓存从此不可能分开（决策 3 的类型强制）。
//!
//! 序号分配、时间戳回填、写入不变量与存储期裁剪都在这里——它们是**存储写入
//! 边界**的策略，与写入路径同处一文件便于对照；读侧的视图策略见
//! [`super::read`]。

use super::*;

/// 为缺失 `timestamp` 的消息回填 `now`，保持数组内的相对顺序不被排序打乱。
fn backfill_timestamps(messages: Vec<ChatNode>, now: i64) -> Vec<ChatNode> {
    messages
        .into_iter()
        .map(|mut m| {
            if m.timestamp.unwrap_or(0) == 0 {
                m.timestamp = Some(now);
            }
            m
        })
        .collect()
}

/// 取一批消息中已有的最大 `seq`，作为后续分配的起点（Lamport 计数器的当前水位）。
pub(super) fn max_seq(messages: &[ChatNode]) -> i64 {
    messages.iter().filter_map(|m| m.seq).max().unwrap_or(0)
}

/// 按切片顺序为消息补发 `seq`（**只补缺号**），返回分配后的新水位。
///
/// 自 `symbio_core::chat_node` 迁入。`seq` **字段**是跨栈 schema
/// （前端逐字段镜像，必须留 core，见该字段文档）；而「怎么补号」是会话存储的实现策略
/// ——调用点只有本文件的 `replace_all` 一处，故与它的兄弟 `append_messages`
/// （另一条分配路径）同处一文件，两条路径可以直接对照着读。
///
/// # 三条不变式（前两条由本函数保证，第三条由调用方保证）
///
/// 1. **顺序**：`seq` 沿数组严格递增——调用方的契约是「数组顺序即权威顺序」，
///    `get_messages` 的排序靠 `seq` 复现它；
/// 2. **稳定**：**已带 `seq` 的消息一律原样保留**，绝不因为一次整表重写而改号。
///    第 2 条不是洁癖：`seq` 是**消费者（前端）手里的顺序锚点**。前端在流式阶段就按
///    本地游标排好了序，整表重写若把既有消息改成新号，两边就各持一套互不相容的序号
///    ——同一条消息会排到列表的两个位置，压缩后顺序看起来就乱了；
/// 3. **前置条件**：无号项只出现在**开头**或**末尾**，不夹在两个既有序号之间。
///    `replace_all` 的全部调用点都满足：压缩 → 快照在**开头**（且已带槽位号）；
///    追加 / 补写 / 恢复 / 清空 → 新节点在**末尾**。违反会被函数末尾的
///    `debug_assert!` 当场拦下。
///
/// # 填号方向：无号项朝「不越过既有序号」的方向让位
///
/// `base` 只在**新列表完全没有既有序号**时才用得上（整批新节点 / 清空后重建）。
/// 只要列表里已经有序号：
///
/// | 位置 | 取值 | 理由 |
/// |---|---|---|
/// | 开头（压缩快照接替被压掉的历史） | `首号 − k … 首号 − 1`（**往下**） | 往上找会越过首号，快照反而排到保留区**之后** |
/// | 末尾（在途追加 / 恢复时新建的子节点） | `末号 + 1 …`（往上） | 与既有历史接续，不抢前面的号 |
///
/// **为什么开头那一段必须整体往下、且一次算好**：逐个「往上找空位」会先占用首号
/// 本身，把既有序号顶成下一个号——正是「seq 随压缩改变」的直接来源。由 `首号` 是最小
/// 序号可知 `首号 − k … 首号 − 1` 全部落在既有序号**之外**，故这一段天然不可能撞号，
/// 也不需要逐个探测。
///
/// 旧实现从 `base` 起向上无条件填号，于是 L2 压缩这种「前缀重写」必然被改号：新列表是
/// `[快照, 保留区…]`，快照无 seq 拿到 `base+1`（成了**最大**），保留区沿用旧**低**序号
/// 却因 `existing > cursor` 不成立而被逐个改号——实测会话 `mtmae8j2wxam4dhrei` 的保留区
/// 因此从 `631..642` 被抬到 `870..881`。顺序虽然被「修」对了，代价却是整段历史的序号
/// 全部重排。
///
/// 真正的修法也在调用方：压缩时给快照显式指定**被压缩内容的槽位序号**
/// （`keep[0].seq - 1`，见 `context::snapshot_slot_seq`），使新列表**本来就单调**。
/// 于是本函数退化为「只补缺号」，既有序号一个不动。
///
/// **已删除的兜底**：旧版还有一段「夹缝容不下无号项时整表按数组顺序重排」，它只在
/// 「无号项夹在两个既有序号之间」时触发——那正是上面的前置条件所排除的输入（该分支
/// 自带的文档也写着「该输入在真实路径上不可达」）。且它的「修法」是**整表改号**，
/// 本身就破坏第 2 条不变式：为一个不可达输入保留一条有害分支，不如把前置条件写成断言。
pub(super) fn assign_seq(messages: &mut [ChatNode], base: i64) -> i64 {
    // 全列表无号（整批新节点 / 清空后重建）：接在调用方水位之后——`base` 的唯一用途
    let Some(first) = messages.iter().filter_map(|m| m.seq).min() else {
        let mut cursor = base;
        for m in messages.iter_mut() {
            cursor += 1;
            m.seq = Some(cursor);
        }
        return cursor;
    };

    // ① 开头连续的无号项：整体落在 `首号` 之前（`首号` 是最小号 ⇒ 必不撞号）
    let head = messages.iter().take_while(|m| m.seq.is_none()).count();
    for (cursor, m) in (first - head as i64..).zip(messages.iter_mut().take(head)) {
        m.seq = Some(cursor);
    }

    // ② 其余无号项：接在**前驱**之后。前置条件保证这里只会遇到末尾连续段，而既有序号
    //    全部在它之前 ⇒ `前驱 + 1` 必未被占用，无需探测空位（旧版的 `taken` 集合与
    //    「往上找空位」循环正是为夹缝场景写的，已随兜底一起删除）。
    //    前驱水位初值取 `base` 只有形式意义：循环首项必是既有序号（`head` 是**最大**
    //    连续无号前缀），故它在第一轮就被覆盖。
    let mut cursor = base;
    for m in messages.iter_mut() {
        match m.seq {
            // 既有序号：原样保留（**绝不改号**），并作为新的前驱水位
            Some(existing) => cursor = existing,
            None => {
                cursor += 1;
                m.seq = Some(cursor);
            }
        }
    }

    debug_assert!(
        messages.windows(2).all(|w| w[0].seq < w[1].seq),
        "assign_seq 的前置条件被破坏：无号项只允许出现在开头或末尾（实得 {:?}）",
        messages.iter().map(|m| m.seq).collect::<Vec<_>>()
    );

    cursor
}

/// 一次**消息列表变更**（ADR-054 决策 2 + 3）。
///
/// 这是 ADR-054 的**闭集原语**落到类型上：
/// 消息列表的变更只有「诞生 / 断言 / 关闭」三种节点级原语（第四种「取代」是列表级的
/// 压缩快照，见 [`PersistentChatSession::replace_all`]——它的事实 owner 不同）。
///
/// ## 为什么它是枚举而不是「一份列表 + 一个说明」
///
/// 旧形态是 `replace_messages(整份列表)`：调用方自己折好列表再整体写入，事实
/// **事后补**——于是「改了列表却没落事实」在编译期**表达得出来**（实测 10 个
/// 写入口里 6 个没落）。收成枚举之后，`apply` 在**同一个函数里**落事实、折列表、
/// 写缓存，那句话在类型上**表达不出来**——这就是 ADR-054 决策 3 要的类型强制。
pub(crate) enum MessageChange {
    /// **诞生**：一个此前不存在的节点出现。
    Born {
        nodes: Vec<ChatNode>,
        /// 它的**承载事实**由谁落——一个事实只能有一个 owner（[ADR-049] 理由 1）。
        carrier: Carrier,
    },
    /// **断言**：一个已存在的节点现在读作 Y ⇒ 落 `message.edited`。
    Edited(Vec<ChatNode>),
    /// **关闭**：这些节点没了 ⇒ 落 `message.removed`。
    Removed(Vec<String>),
}

/// 诞生的节点**由谁落事实**。
///
/// 这是 [`MessageChange::Born`] 唯一需要调用方回答的问题，而它**不是实现细节**：
/// 「这条节点诞生了」的 owner 必须唯一，写错方向的两半代价不对称——
/// - 该 `Runner` 写成 `Here` ⇒ 多一条 `message.added`，重建按「同 id 只认首次」
///   去重，**无害**；
/// - 该 `Here` 写成 `Runner` ⇒ 节点在事实源里**没有出生记录**，切主读后整条消失，
///   **是数据丢失**。
///
/// 所以拿不准时选 `Here`——宁可多一条被去重的事实，不可少一条出生记录。
pub(crate) enum Carrier {
    /// 由**运行器**落：用户发言归 `turn × opened`、助手正文归 `turn × closed|progressed`、
    /// 工具结果归 `artifact × asserted`。`apply` **不另落事实**。
    Runner,
    /// **没有别的格可搭车**（压缩记录节点）⇒ `apply` 落 `message.added`。
    Here,
}

impl PersistentChatSession {
    /// 一次消息列表变更的**唯一写入口**（ADR-054
    /// 决策 2 的「顺序反转」）。
    ///
    /// 顺序是**先落事实、再折列表、再写缓存**。这与旧形态（`读列表 → 改 Vec →
    /// replace_messages → 事后补一条事实`）的区别不是风格：旧形态里事实是**事后补的**，
    /// 所以它天然是可选项——忘了补，代码照样编译、测试照样绿。本函数把「落事实」
    /// 收进写入路径本身，**没有「补」这一步**，也就没有「忘了补」这个失败。
    ///
    /// ## 失败口径：事实失败即本次变更失败（T-164）
    ///
    /// 旧口径是「附加事实失败不该把成功的动作说成失败」——那在 b3 之前成立：缓存是
    /// 读侧真源、事实是影子。b3 把主读切到 WAL 之后**事实就是本体**，放过它等于
    /// 「回执说改好了、重建后一看没改」，而且用户无感。
    ///
    /// 步骤 ① 在写缓存**之前**，所以这里返回错误时什么都已经没发生——报失败是如实。
    /// 两处例外各自有理由：拿不到写者令牌 ⇒ 寄存（T-055，稍后由持令牌方补落）；
    /// 无锚可指 ⇒ v1 遗留会话，读侧本就回落 `messages.json`（ADR-049），缓存仍是真源。
    ///
    /// ## 返回值
    ///
    /// **受这次变更影响、且仍留在存储里的节点的权威副本**：`Born` 回追加进去的那批
    /// （按追加顺序，含存储分配的 `seq`——落库回包靠它），`Edited` 回改写后的那批，
    /// `Removed` 回空（被删的节点不在存储里，交回它们等于让前端对齐幻影）。
    pub(crate) async fn apply(&self, change: MessageChange) -> Result<Vec<ChatNode>, PluginError> {
        // ① 落事实（**先于**写缓存）。
        if let Some(dir) = self.store.session_dir(&self.session_id) {
            use super::super::message_events::AppendOutcome;
            let mut failure: Option<String> = None;
            match &change {
                MessageChange::Born {
                    nodes,
                    carrier: Carrier::Here,
                } => {
                    if let AppendOutcome::Failed(why) =
                        super::super::message_events::added(&dir, nodes)
                    {
                        failure = Some(why);
                    }
                }
                MessageChange::Edited(msgs) => {
                    for m in msgs {
                        // 第一条失败即停：后续的更没理由成功，而一次变更只报一个错。
                        if let AppendOutcome::Failed(why) =
                            super::super::message_events::edited(&dir, m)
                        {
                            failure = Some(why);
                            break;
                        }
                    }
                }
                MessageChange::Removed(ids) => {
                    if let AppendOutcome::Failed(why) =
                        super::super::message_events::removed(&dir, ids)
                    {
                        failure = Some(why);
                    }
                }
                // 承载事实由运行器落（用户发言 / 助手正文 / 工具结果）——此处再落
                // 一条 `message.added` 就是第二个 owner。
                MessageChange::Born {
                    carrier: Carrier::Runner,
                    ..
                } => {}
            }
            if let Some(why) = failure {
                crate::plugin_error!("session", "[message] 变更未能入格，本次变更未生效：{}", why);
                return Err(PluginError::InternalError(format!(
                    "消息变更未能入格，本次变更未生效：{why}"
                )));
            }
        }

        // ② 折列表 + ③ 写缓存。
        match change {
            MessageChange::Born { nodes, .. } => {
                let (persisted, removed) = self.append_messages(nodes).await?;
                // 写入期裁剪（`prune_tool_history` / `max_messages`）是**列表变更**，
                // 必须入格（T-054 · ADR-054 决策 1 的「关闭」原语）。裁剪发生在
                // `append_messages` 的存储临界区内（它才拿得到权威列表与配置），
                // 故由本处补落 `message.removed`——**事实仍由 `apply` 这一个入口落**，
                // 没有开出第二个写入口。
                //
                // 不落格的后果是**静默的**：WAL（唯一真源）里那些节点还在，只是
                // `messages.json` 缓存里少了。切主读后重建照 WAL 折回 ⇒ **被裁节点
                // 复活**——用户会看到早已淘汰的旧工具链重新出现在历史里。
                //
                // **这一步不能升级为错误**（T-164 的另一半）：它发生在写缓存**之后**
                // （裁剪决策在 `append_messages` 的临界区内产生），此时追加已经成功；
                // 报失败会让调用方重试，而重试等于把同一条用户消息再追加一遍。代价
                // 因此取「出声 + 有界」：被裁节点在下次重建后复活（列表变长，不是
                // 数据丢失），日志把这次失败指出来。
                if !removed.is_empty() {
                    if let Some(dir) = self.store.session_dir(&self.session_id) {
                        use super::super::message_events::AppendOutcome;
                        if let AppendOutcome::Failed(why) =
                            super::super::message_events::removed(&dir, &removed)
                        {
                            crate::plugin_error!(
                                "session",
                                "[message] 写入期裁剪的删除事实未入格：{why}\
                                 （被裁的 {} 条在下次重建后会复活）",
                                removed.len()
                            );
                        }
                    }
                }
                Ok(persisted)
            }
            MessageChange::Edited(msgs) => {
                let mut list = self.get_messages().await?;
                for m in &msgs {
                    if let Some(existing) = list.iter_mut().find(|e| e.id == m.id) {
                        *existing = m.clone();
                    }
                }
                self.replace_all(list).await?;
                Ok(msgs)
            }
            MessageChange::Removed(ids) => {
                let mut list = self.get_messages().await?;
                list.retain(|m| !ids.contains(&m.id));
                self.replace_all(list).await?;
                Ok(Vec::new())
            }
        }
    }

    /// 追加消息并落库，返回 `(落库后的权威副本, 被写入期裁剪删掉的 id)`。
    ///
    /// ## 为什么回消息，而不是回条数
    ///
    /// `seq` 只在存储写入时分配（见 `symbio/src/plugins/session/docs/022-vdfs-session-messages.md` §3.4），调用方
    /// 手里那条**没有号**——所以「每一条被落库的消息都必须发一次带存储 `seq` 的
    /// 变更」这条不变量，只有在调用方能拿到权威副本时才成立。回条数等于让调用方
    /// 拿不到可下发的载荷，于是"落库"与"换号"之间必然出现断口：前端留下转写分配的
    /// 在途号（`1 << 50`），与存储号并存 ⇒ 排序错位（只有整份回读才恢复）。
    ///
    /// 返回的条目 = **实际留在存储里**的那些（按追加顺序）：被轮次窗口淘汰、或
    /// 被写入期工具链裁剪动过的，以存储里的最终形态为准——交回一条存储里并不
    /// 存在的消息，等于让前端"对齐"到幻影。
    ///
    /// 第二个返回值（**T-054**）是本次写入期裁剪删掉的 id：调用方（`apply`）
    /// 据此落 `message.removed`。为什么由调用方落而不是本函数落——本函数是
    /// **存储原语**，事实落格收在 `apply` 一个入口（ADR-054 决策 2/3 的类型强制）；
    /// 本函数只**报告**它动了哪些节点。空 `Vec` = 没裁任何节点。
    ///
    /// 顺序与入参一致（不含被淘汰项）；落库失败则整体失败，不返回部分结果。
    pub(super) async fn append_messages(
        &self,
        messages: Vec<ChatNode>,
    ) -> Result<(Vec<ChatNode>, Vec<String>), PluginError> {
        // 持久层写入不变量（见 `ensure_durable_states`）：瞬态状态不得落盘
        ensure_durable_states(&messages, "append_messages")?;
        // 临界区：整段「读 → 改 → 整份写回」必须串行。会话写入是整份覆盖，
        // 两次并发追加各自读到同一份旧数据、各自写回，后写的会整份覆盖先写的。
        let _write = self.store.lock_writes(&self.session_id).await;
        let mut session = self.load_session().await?;
        let now = clock_now_ms();

        let cfg = self.config.read().await;

        // 分配单调序号：起点取当前会话已有最大 seq，保证追加的消息严格排在其后。
        let mut seq_cursor = max_seq(&session.messages);

        // 本次追加的 id（顺序 = 追加顺序）：末尾据此从落库结果里取回**权威副本**，
        // 交给调用方做落库回包（见 trait 上 `append_messages` 的说明）。
        let mut appended_ids: Vec<String> = Vec::with_capacity(messages.len());

        // 存储保持**完整原文**（架构原则，见 chat_loop「存储保持完整历史，视图逐轮裁剪」）：
        // 一切压缩均发生在"发给大模型之前"——L0 工具结果守卫在工具执行生产时刻、
        // L2 上下文摘要在 Turn 开始、内容节点淡化在请求视图组装（build_request_view）。
        // 落库前不做任何内容改写：压缩原料不被污染，UI（reason 面板等）读到的也是全文。
        for mut chat_msg in messages {
            // WAL 投影已含该节点（事实先落——`Born{Here}` 在进本函数前已入格，
            // 运行器承载格同理）⇒ **不重复追加**：投影就是权威，回包取投影里那份。
            // 少了这一段，apply 落完事实再 append 会把同一节点写成两条
            // （一条重建号、一条本地补号）——切主读后同一句话在列表里出现两次。
            if session.messages.iter().any(|m| m.id == chat_msg.id) {
                appended_ids.push(chat_msg.id);
                continue;
            }
            if chat_msg.timestamp.unwrap_or(0) == 0 {
                chat_msg.timestamp = Some(now);
            }
            // 存储边界：**在途占位号一律摘掉**，交给下面统一分配（见
            // `transcript::INFLIGHT_SEQ_BASE`）。不摘的后果是静默的：存储水位被抬进
            // 在途号段，此后存储计数器与在途计数器在同一区间各自递增 ⇒ 撞号。
            if chat_msg
                .seq
                .is_some_and(super::super::transcript::is_inflight_seq)
            {
                chat_msg.seq = None;
            }
            if chat_msg.seq.is_none() {
                seq_cursor += 1;
                chat_msg.seq = Some(seq_cursor);
            } else if let Some(s) = chat_msg.seq {
                if s > seq_cursor {
                    seq_cursor = s;
                }
            }

            appended_ids.push(chat_msg.id.clone());
            session.messages.push(chat_msg);
        }

        let context_messages = cfg.context_messages;
        // 内存临时会话（`prune_on_write = false`）跳过物理裁剪：其历史本就不回看，
        // 裁剪只会让同一轮内可复用的工具链凭空消失。
        let prune_tool_history = cfg.prune_tool_history && self.prune_on_write;
        // `max_messages` 直接生效，0 = 不限制（与 `context_messages` / `sliding_window` 语义一致）。
        // 历史上此处曾硬编码 `.max(500)`，导致设置面板中小于 500 的值静默失效。
        let max_turns = cfg.max_messages;
        drop(cfg);

        // 写入期裁剪删掉的全部 id（两类裁剪的并集，按被删顺序累积）——回给调用方
        // 落 `message.removed`（T-054）。空 = 没裁任何节点。
        let mut removed_ids: Vec<String> = Vec::new();

        // 写入期工具链裁剪：架构原则（存储保持完整原文）的唯一例外，默认开启以
        // 控制磁盘与节点树体积；置 `prune_tool_history: false` 后存储严格保留全文，
        // 工具链裁剪完全交给请求视图层（build_request_view 的骨架化/淡化）。
        if prune_tool_history {
            removed_ids.extend(prune_historical_tool_calls(
                &mut session.messages,
                context_messages,
            ));
        }

        let mut user_indices = Vec::new();
        for (idx, msg) in session.messages.iter().enumerate() {
            if msg.node_role() == Some(NodeRole::User) {
                user_indices.push(idx);
            }
        }

        if max_turns > 0 && user_indices.len() > max_turns {
            let start_idx = user_indices[user_indices.len() - max_turns];
            // 轮次 FIFO 淘汰只删消息节点，**不动** `tool_archives/` 归档文件（审计 A3）。
            // 理由：归档的磁盘生命周期已有专职策略——`tool_result_guard` 按修改时间
            // 保留每会话最新 [`TOOL_ARCHIVE_KEEP`] 个文件（滚动淘汰）。写入路径上再删一遍
            // 既与"淘汰旧轮"无关（旧轮引用的往往是最新的那几个存档），又让配置项
            // `max_messages` 变成静默删文件的破坏性操作。
            let dropped = start_idx;
            removed_ids.extend(session.messages[..start_idx].iter().map(|m| m.id.clone()));
            session.messages.drain(0..start_idx);
            if dropped > 0 {
                plugin_info!(
                    "session",
                    "append_messages: 轮次窗口淘汰 {} 条消息（归档文件保留，由 L0 滚动策略回收）",
                    dropped
                );
            }
        }

        session.updated_at = now;
        self.save_session(&session).await?;

        // 落库回包（见 `symbio/src/plugins/session/docs/022-vdfs-session-messages.md` §3.4）：取**存储里的最终形态**
        // 交回调用方下发。从 `session.messages` 取而非回传入参副本——上面两道写入期
        // 变换（轮次窗口淘汰、工具链裁剪）可能已经动过它们，回执必须与磁盘逐字段一致。
        // 已被淘汰的（批量追加超过窗口时可能发生）自然被滤掉：交回一条存储里并不
        // 存在的消息，等于让前端"对齐"到一个幻影。
        let appended: HashSet<&str> = appended_ids.iter().map(|s| s.as_str()).collect();
        let persisted = session
            .messages
            .iter()
            .filter(|m| appended.contains(m.id.as_str()))
            .cloned()
            .collect();

        Ok((persisted, removed_ids))
    }

    pub(crate) async fn replace_all(&self, messages: Vec<ChatNode>) -> Result<(), PluginError> {
        // 持久层写入不变量（见 `ensure_durable_states`）：瞬态状态不得落盘
        ensure_durable_states(&messages, "replace_all")?;
        // 临界区：与 append / update 共用同一把 per-session 写锁（整份覆盖语义）
        let _write = self.store.lock_writes(&self.session_id).await;
        let mut session = self.load_session().await?;
        let now = clock_now_ms();
        // 回填缺失的 timestamp 与 seq：replace 会整体重写消息列表，若保留 `None`，
        // `get_messages` 只能靠"哨兵 + 稳定排序"兜底，容易打乱"父先于子"的顺序。
        // seq 的分配规则见 `assign_seq`：**既有序号一律原样保留**，只补缺号；
        // `max_seq(旧列表)` 仅作为"新列表全无序号"时的填号起点（整批新节点的场景）。
        // 因此 L2 压缩（前缀重写）不会改写保留区任何一条消息的 seq——压缩只改变
        // 列表的**构成**，不改变既有消息的身份锚点。
        let mut messages = backfill_timestamps(messages, now);
        // 存储边界：在途占位号**必须在 `assign_seq` 之前**摘掉——`assign_seq` 对
        // 既有序号是"原样保留"，放它进去就等于把在途号当成权威号写进存储
        // （`converge_inflight` 的补写路径正是带在途号进来的）。见
        // `transcript::INFLIGHT_SEQ_BASE`。
        for m in messages.iter_mut() {
            if m.seq.is_some_and(super::super::transcript::is_inflight_seq) {
                m.seq = None;
            }
        }
        assign_seq(&mut messages, max_seq(&session.messages));
        // 孤儿存档：`replace_all` 整体重写消息列表（L2 语义压缩 /
        // Streaming 清理），被丢弃消息引用的 L0 `tool_archives/` 存档随之失去引用。
        // 此处**只统计不删除**（审计 A3）：归档的磁盘生命周期由 `tool_result_guard`
        // 的每会话滚动保留（最新 `TOOL_ARCHIVE_KEEP` 个）统一负责，写入路径不再
        // 承担删文件职责——历史上这里需要在删除前做"拒绝 `..` + 限定 archives 根"
        // 的双重路径校验，正是因为把不可信输入变成了删除动作。
        if let Some(dir) = self.store.session_dir(&self.session_id) {
            let kept: HashSet<&str> = messages
                .iter()
                .filter_map(|m| m.meta.as_ref())
                .filter_map(|m| m.get("archive_path"))
                .filter_map(|v| v.as_str())
                .collect();
            let orphans = session
                .messages
                .iter()
                .filter_map(|m| {
                    m.meta
                        .as_ref()
                        .and_then(|m| m.get("archive_path"))
                        .and_then(|v| v.as_str())
                })
                .filter(|p| !kept.contains(*p))
                .count();
            if orphans > 0 {
                plugin_info!(
                    "session",
                    "replace_all: {} 个工具存档失去引用（保留于 {}，等待 L0 滚动回收）",
                    orphans,
                    dir.display()
                );
            }
        }
        session.messages = messages;
        session.updated_at = now;
        self.save_session(&session).await
    }
}

/// 持久层**写入不变量**：`Streaming` 是瞬态状态，只存在于在途缓冲（`live_messages`）
/// 与广播通道（VDFS 变更帧），**永不落盘**。
///
/// 持久层允许的状态域 = 终态集合：`completed` / `failed` / `aborted` /
/// `waiting_user_action`（`None` / `Pending` 等未标注按已结束处理，见
/// `plugin/nodes.rs::message_status`）。崩溃恢复因此不需要任何修复器：
/// 磁盘上只有终态，「上次没跑完的轮次」压根不在存储里，自然不会残留
/// 永久转圈的节点（`WaitingUserAction` 是合法的可恢复状态，保留）。
///
/// 违反即**程序错误**——某个写入点把瞬态状态当成了可持久化状态。在写入当场报错，
/// 让错误指向真实肇事者；而不是落盘之后靠启动期清理器静默改写（修复器存在的
/// 每一天，都在给新的违例写入点发通行证）。
pub(super) fn ensure_durable_states(
    messages: &[cm::ChatNode],
    op: &str,
) -> Result<(), PluginError> {
    for m in messages {
        if m.status == Some(cm::NodeStatus::Streaming) {
            return Err(PluginError::InternalError(format!(
                "{op}: 消息 {} 携带瞬态状态 `streaming`，持久层只接受终态；\
                 瞬态状态应经在途缓冲与广播通道下发，不得写入存储",
                m.id
            )));
        }
        // 增量是**不完整的片段**：它是帧的形态，不是消息的形态。存储里只有
        // `content`（累积后的正文），一条带 `delta` 的消息落盘意味着某处把
        // 「这一段」当成了「全部」——存进去的就是半截消息。
        if m.delta.is_some() {
            return Err(PluginError::InternalError(format!(
                "{op}: 消息 {} 携带流式增量 `delta`，持久层只接受完整正文；\
                 增量只存在于出方向的帧上，累积后的正文应写入 `content`",
                m.id
            )));
        }
    }
    Ok(())
}

/// 写入期工具链物理裁剪（原 `context.rs` 并入，体检备注 audit-5）。
///
/// 「存储保持完整原文、压缩只发生在请求视图」架构原则的**唯一例外**：在落库时
/// 物理删除 `context_messages` 轮之前的 Tool / ToolCall / Reasoning 节点。
/// 由 `SessionConfig::prune_tool_history` 控制（默认 `true` 保持历史行为）；
/// 置 `false` 后工具链裁剪完全由请求视图层（`build_request_view`）承担，
/// 存储与前端节点树可回看全部历史。
///
/// **只删节点、不删文件**（审计 A3）：`meta.archive_path` 指向的 L0 归档由
/// `tool_result_guard` 的每会话滚动保留（最新 `TOOL_ARCHIVE_KEEP` 个，按修改时间）
/// 统一回收。写入路径此前把不可信的消息 meta 当删除依据（需额外做 `..` 与根目录
/// 双重校验），且"淘汰旧轮"删掉的常是最新存档——职责错位，故收敛为纯节点裁剪。
///
/// 返回**被删节点 id**（顺序 = 原列表顺序）——调用方据此落 `message.removed` 事实。
/// 这是 T-054 的承重面：裁剪是**列表变更**，
/// 而列表变更必须入格（ADR-054
/// 决策 1 的「关闭」原语）。裁剪改了列表却不落事实 ⇒ 切主读后**被裁节点复活**
/// ——WAL 里它们还在，只是缓存里少了（而缓存已经不再是真源）。
pub fn prune_historical_tool_calls(messages: &mut Vec<ChatNode>, keep_turns: usize) -> Vec<String> {
    // keep_turns == 0 语义为「不限制」（与 `SessionConfig::context_messages` 的 0 值语义、
    // `sliding_window` 保持一致）。此处缺失守卫时，下面 `user_indices[len - keep_turns]`
    // 会取到下标 `len`（越界）并 panic，导致整次 `append_messages` 失败。
    if keep_turns == 0 {
        return Vec::new();
    }
    let mut user_indices = Vec::new();
    for (idx, msg) in messages.iter().enumerate() {
        if msg.node_role() == Some(NodeRole::User) {
            user_indices.push(idx);
        }
    }

    if user_indices.len() <= keep_turns {
        return Vec::new();
    }

    let limit_idx = user_indices[user_indices.len() - keep_turns];
    let mut to_remove = HashSet::new();

    for msg in &messages[..limit_idx] {
        if msg.node_role() == Some(NodeRole::Tool) || msg.is_tool_call() || msg.is_reasoning() {
            to_remove.insert(msg.id.clone());
        }
    }

    // 同时移除被剪除 ToolCall 的直接子节点（请求 Text / 响应 Text）
    let extra: HashSet<String> = messages[..limit_idx]
        .iter()
        .filter(|m| {
            m.parent_id
                .as_ref()
                .map(|p| to_remove.contains(p))
                .unwrap_or(false)
        })
        .map(|m| m.id.clone())
        .collect();
    to_remove.extend(extra);

    for msg in &messages[..limit_idx] {
        if msg.node_role() == Some(NodeRole::Assistant) {
            let content_text = msg
                .content
                .as_ref()
                .map(|c| c.to_text())
                .unwrap_or_default();
            let mut has_retained_children = false;
            for child in &messages[..limit_idx] {
                if child.parent_id.as_ref() == Some(&msg.id) && !to_remove.contains(&child.id) {
                    has_retained_children = true;
                    break;
                }
            }
            if content_text.trim().is_empty() && !has_retained_children {
                to_remove.insert(msg.id.clone());
            }
        }
    }

    // 被删的那批，**按原列表顺序**取出（`to_remove` 是 `HashSet`，顺序不定；
    // 事实与回执里的 id 序列必须稳定——与 `truncate_messages` 的回执同一条纪律）。
    let removed: Vec<String> = messages
        .iter()
        .filter(|m| to_remove.contains(&m.id))
        .map(|m| m.id.clone())
        .collect();
    messages.retain(|msg| !to_remove.contains(&msg.id));
    removed
}
