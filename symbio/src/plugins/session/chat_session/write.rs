//! 写路径：两条写入路径（[`PersistentChatSession::append_messages`] /
//! [`PersistentChatSession::replace_messages`]）与它们的存储边界策略纯函数。
//!
//! 序号分配、时间戳回填、写入不变量与存储期裁剪都在这里——它们是**存储写入
//! 边界**的策略，与两条写入路径同处一文件便于对照；读侧的视图策略见
//! [`super::read`]。

use super::*;

/// 为缺失 `timestamp` 的消息回填 `now`，保持数组内的相对顺序不被排序打乱。
fn backfill_timestamps(messages: Vec<ChatMessage>, now: i64) -> Vec<ChatMessage> {
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
pub(super) fn max_seq(messages: &[ChatMessage]) -> i64 {
    messages.iter().filter_map(|m| m.seq).max().unwrap_or(0)
}

/// 按切片顺序为消息补发 `seq`（**只补缺号**），返回分配后的新水位。
///
/// 自 `symbio_core::chat_message` 迁入。`seq` **字段**是跨栈 schema
/// （前端逐字段镜像，必须留 core，见该字段文档）；而「怎么补号」是会话存储的实现策略
/// ——调用点只有本文件的 `replace_messages` 一处，故与它的兄弟 `append_messages`
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
///    `replace_messages` 的全部调用点都满足：压缩 → 快照在**开头**（且已带槽位号）；
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
pub(super) fn assign_seq(messages: &mut [ChatMessage], base: i64) -> i64 {
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

impl PersistentChatSession {
    /// 追加消息并落库，返回**落库后的权威副本**（含存储分配的 `seq` / `timestamp`）。
    ///
    /// ## 为什么回消息，而不是回条数
    ///
    /// `seq` 只在存储写入时分配（见 `docs/vdfs-session-messages.md` §3.4），调用方
    /// 手里那条**没有号**——所以「每一条被落库的消息都必须发一次带存储 `seq` 的
    /// 变更」这条不变量，只有在调用方能拿到权威副本时才成立。回条数等于让调用方
    /// 拿不到可下发的载荷，于是"落库"与"换号"之间必然出现断口：前端留下转写分配的
    /// 在途号（`1 << 50`），与存储号并存 ⇒ 排序错位（只有整份回读才恢复）。
    ///
    /// 返回的条目 = **实际留在存储里**的那些（按追加顺序）：被轮次窗口淘汰、或
    /// 被写入期工具链裁剪动过的，以存储里的最终形态为准——交回一条存储里并不
    /// 存在的消息，等于让前端"对齐"到幻影。
    ///
    /// 顺序与入参一致（不含被淘汰项）；落库失败则整体失败，不返回部分结果。
    pub(crate) async fn append_messages(
        &self,
        messages: Vec<ChatMessage>,
    ) -> Result<Vec<ChatMessage>, PluginError> {
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

        // 写入期工具链裁剪：架构原则（存储保持完整原文）的唯一例外，默认开启以
        // 控制磁盘与节点树体积；置 `prune_tool_history: false` 后存储严格保留全文，
        // 工具链裁剪完全交给请求视图层（build_request_view 的骨架化/淡化）。
        if prune_tool_history {
            prune_historical_tool_calls(&mut session.messages, context_messages);
        }

        let mut user_indices = Vec::new();
        for (idx, msg) in session.messages.iter().enumerate() {
            if msg.role == Some(MessageRole::User) {
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

        // 落库回包（见 `docs/vdfs-session-messages.md` §3.4）：取**存储里的最终形态**
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

        Ok(persisted)
    }

    pub(crate) async fn replace_messages(
        &self,
        messages: Vec<ChatMessage>,
    ) -> Result<(), PluginError> {
        // 持久层写入不变量（见 `ensure_durable_states`）：瞬态状态不得落盘
        ensure_durable_states(&messages, "replace_messages")?;
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
        // 孤儿存档：`replace_messages` 整体重写消息列表（L2 语义压缩 /
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
                    "replace_messages: {} 个工具存档失去引用（保留于 {}，等待 L0 滚动回收）",
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
    messages: &[cm::ChatMessage],
    op: &str,
) -> Result<(), PluginError> {
    for m in messages {
        if m.status == Some(cm::MessageStatus::Streaming) {
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
pub fn prune_historical_tool_calls(messages: &mut Vec<ChatMessage>, keep_turns: usize) {
    // keep_turns == 0 语义为「不限制」（与 `SessionConfig::context_messages` 的 0 值语义、
    // `sliding_window` 保持一致）。此处缺失守卫时，下面 `user_indices[len - keep_turns]`
    // 会取到下标 `len`（越界）并 panic，导致整次 `append_messages` 失败。
    if keep_turns == 0 {
        return;
    }
    let mut user_indices = Vec::new();
    for (idx, msg) in messages.iter().enumerate() {
        if msg.role == Some(MessageRole::User) {
            user_indices.push(idx);
        }
    }

    if user_indices.len() <= keep_turns {
        return;
    }

    let limit_idx = user_indices[user_indices.len() - keep_turns];
    let mut to_remove = HashSet::new();

    for msg in &messages[..limit_idx] {
        if msg.role == Some(MessageRole::Tool)
            || msg.msg_type == Some(MessageType::ToolCall)
            || msg.msg_type == Some(MessageType::Reasoning)
        {
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
        if msg.role == Some(MessageRole::Assistant) {
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

    messages.retain(|msg| !to_remove.contains(&msg.id));
}
