//! 失败降级持久化：把"仍在飞行中的那一个 Turn"标为 `Failed`。
//!
//! - `persist_failure`：作用域**收窄到失败 Turn 及其后代**（M-002）——历史轮次的
//!   消息不参与合并，避免 session 层拿流式快照去"补写"一份 Model 插件已落库的节点；
//! - `subtree_of`：计算 `root` 的传递闭包子树，供上面的作用域判定使用。
//!
//! 可见性：`persist_failure` 被 `consume.rs` 的 `fail_before_loop` 与根文件的
//! `WorkingGuard::drop` 调用，故标 `pub(super)`。

use super::*;

/// 「这个节点还在飞行中」——**唯一**判据。
///
/// 收口类操作（失败降级 / 中止收口）都必须用同一份定义：漏掉一个状态，
/// 表现就是前端一个**永远转下去**的「运行中」，而它不会报错、只会一直转。
/// `WaitingUserAction` **不在此列**：它不是「正在跑」，而是「等用户回答」——
/// 用户仍可从审批卡片回答或忽略，抹掉它等于把入口删了。
///
/// 可见性 `pub(super)`：`orchestrator.test.rs` 直接锁定这个集合，
/// 避免它被悄悄改小（少一个状态 = 前端一个永远转下去的「运行中」）。
pub(super) fn is_inflight(status: &Option<cm::MessageStatus>) -> bool {
    matches!(
        status,
        None | Some(cm::MessageStatus::Pending) | Some(cm::MessageStatus::Streaming)
    )
}

/// 中止时这个节点该落到哪个终态。
///
/// **根级 Turn → `Aborted`，其余 → `Completed`。**
///
/// 这不是措辞讲究，而是「能不能重试」的分界：前端的重试入口挂在 Turn 的终态上，
/// 定稿成 `Completed` 就等于宣布这一轮正常结束，入口随之消失——而用户明明是按了
/// 停止，那一轮是半截的。子节点（正文 / 思考 / 工具调用）没有独立的重试语义，
/// 它们的重试归组级 Turn，因此按现状定稿 `Completed` 结束流式动画即可。
///
/// 可见性 `pub(super)`：`orchestrator.test.rs` 直接锁定这个映射。
pub(super) fn abort_terminal_of(m: &cm::ChatMessage) -> cm::MessageStatus {
    if m.msg_type == Some(cm::MessageType::Turn) && m.parent_id.is_none() {
        cm::MessageStatus::Aborted
    } else {
        cm::MessageStatus::Completed
    }
}

impl SessionPlugin {
    /// 把"仍在进行中"的 AI 响应消息持久化为 `Failed` + 错误原因。
    ///
    /// ## 触发场景
    ///
    /// - 业务错误路径（Model 插件返回 Err / 透传 PluginFrame::Error）
    /// - 后台任务 **panic 崩溃**（由 `WorkingGuard::drop` 调用）
    ///
    /// ## 作用域：只有「真正在飞行中的那一个 Turn」会被降级（M-002）
    ///
    /// `collected` 是**整个 `run_chat_loop` 调用期间**累积的全部流式消息快照。
    /// 一次调用最多可跑 `max_tool_rounds` 轮（默认 0 = 不限制），因此它往往包含几十上百个
    /// **早已成功定稿**的历史 Turn——那些轮次的工具早已执行完毕、结果也早已由
    /// Model 插件在每轮结束时落库。
    ///
    /// 若不加区分地把 `collected` 里每一个根级 Turn 都回滚成 `Failed`，那么末尾一次
    /// 断流就会把**此前全部成功轮次追溯标记为失败**，并把同一条错误写进每一轮。
    /// 实测：一次上游 502 断流 → 189 个 Turn 全部 `Failed` + 同一条错误刷屏 189 次。
    ///
    /// 因此这里先定位失败 Turn（`collected` 中**最后一个**根级 Turn，即错误发生时
    /// 正在进行的那一个，因为 `collected` 按到达顺序累积），再只对**它及其子树**
    /// 做收尾；历史 Turn 一律保持存储中已有的终态，不动、不广播。
    ///
    /// ## 持久化策略
    ///
    /// 直接 `replace_messages(collected)` 会**覆盖掉整段历史**（用户之前的消息也会被清掉），
    /// 所以这里走"载入整会话 → 按 id 合并 → 整体替换"：
    /// - 失败 Turn（根级）：标 `terminal` —— 业务错误是 `Failed`（+ `error`，这是前端唯一
    ///   渲染 ⚠ 错误条 + 重试入口的节点），用户中止是 `Aborted`（**不挂 error**：中止不是故障）
    /// - 失败 Turn 的子节点（text / reasoning / tool_call / 工具结果）：仅把仍在进行中的
    ///   (None / Streaming / Pending) 定稿为 Completed，**绝不挂 error**
    /// - 尚未落库的子节点：补写，前提是其父节点存在（避免写入悬空孤儿节点）
    ///
    /// 这样切回会话时（`get_messages`）能看到上次的失败终态与原因（CHAT_FLOW_ANALYSIS 目标 3）。
    ///
    /// ## 为什么 `terminal` 是参数而不是「调用方事后改状态」
    ///
    /// 中止与失败是**两个终态**，不是「同一种降级 + 一个分类标志位」。
    /// 中止路径（`handle_abort` → [`Self::converge_inflight`]）与这里都可能在同一次
    /// 中止里跑到（谁先谁后取决于 abort 信号落在哪个检查点）；若两处各自判定，
    /// 同一轮就有了两个真相。收成参数后两条路写**同一个值**，竞态因此无害。
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn persist_failure(
        &self,
        session_id: &str,
        transcript: &Arc<tokio::sync::Mutex<crate::plugins::session::transcript::Transcript>>,
        error: &str,
        terminal: cm::MessageStatus,
    ) {
        // 1. 定位失败 Turn。
        //    在途图为空表示错误发生在任何 Turn 节点创建之前（例如能力收集失败、
        //    Model 插件路由直接报错）——此时没有任何节点可降级，错误属于会话级，
        //    由调用方广播的 Error 事件承载，这里直接返回。
        let mut mirror = transcript.lock().await.snapshot();
        let failing_turn_id = mirror
            .iter()
            .rev()
            .find(|m| m.msg_type == Some(cm::MessageType::Turn) && m.parent_id.is_none())
            .map(|m| m.id.clone());
        let failing_turn_id = match failing_turn_id {
            Some(id) => id,
            None => {
                crate::plugin_info!(
                    "session",
                    "persist_failure: 无在途 Turn（错误发生在 Turn 创建之前），仅广播会话级错误"
                );
                return;
            }
        };

        // 2. 收窄作用域：只取失败 Turn 及其**传递闭包**子树
        //    （Turn → text/reasoning/tool_call → 工具结果）。
        //    历史轮次的消息不在此列：它们已由 Model 插件按自己的规则落库，session 层
        //    不应再拿流式快照去"补写"一份——否则同一个 Turn 下会出现两份内容相同的
        //    文本节点（流式 id 与落库 id 不一致时的典型表现）。
        let subtree_ids = subtree_of(&mirror, &failing_turn_id);

        // 3. 在转写镜像里定稿失败 Turn 的子树。
        //
        //    关键修复（原始 Bug：错误刷屏 + 工具节点出现服务器错误）：
        //    - 失败 Turn（根级）：Failed + error，这是前端唯一渲染 ⚠ 错误条 + 重试入口的节点。
        //    - 其余子节点（text / reasoning / tool_call）：仅把仍在进行中的
        //      (None / Streaming / Pending) 定稿为 Completed，**绝不挂 error**。
        //      理由：工具调用在本地执行、不请求 LLM 服务器，不可能产生「API 错误」；
        //      把同一个 429 刷到每条文本/工具节点既不符合逻辑，又造成错误刷屏。
        for m in mirror.iter_mut() {
            if !subtree_ids.contains(&m.id) {
                continue;
            }
            if m.id == failing_turn_id {
                m.status = Some(terminal.clone());
                // 中止不挂 error：它是用户自己的操作，不是故障；前端按状态渲染
                // 「已中止」，不需要一条错误文案来凑。
                m.error = (terminal == cm::MessageStatus::Failed).then(|| error.to_string());
            } else if matches!(
                m.status,
                None | Some(cm::MessageStatus::Streaming) | Some(cm::MessageStatus::Pending)
            ) {
                m.status = Some(cm::MessageStatus::Completed);
                m.error = None;
            }
        }

        // 4. 载入整会话并合并（合并语义，不覆盖历史）。
        let chat_session = match self.open_chat_session(session_id).await {
            Ok(cs) => cs,
            Err(e) => {
                crate::plugin_error!("session", "persist_failure: open session failed: {}", e);
                return;
            }
        };
        let mut all = match chat_session.get_messages().await {
            Ok(m) => m,
            Err(e) => {
                crate::plugin_error!("session", "persist_failure: get_messages failed: {}", e);
                return;
            }
        };

        // 仅记录本次真正发生状态变化的消息，最后只广播这些，
        // 避免对未变化的消息重复推送（也避免把错误刷屏到每条文本）。
        let mut changed: Vec<cm::ChatMessage> = Vec::new();

        // 5. 合并失败 Turn 的子树（作用域外的一律跳过：保持存储中已有的终态）。
        for cm_msg in &mirror {
            if !subtree_ids.contains(&cm_msg.id) {
                continue;
            }
            // 失败 Turn 本身必须回滚为 Failed：即便它已被 finalize 为 Completed
            //（例如 turn 循环在 finalize 之后、工具阶段才抛出 PluginFrame::Error，
            // 或 panic 发生在 finalize 之后），也应强制标 Failed 并持久化。否则前端实时
            // 看到的「失败 + 重试」在会话重载后变回 Completed，错误显示与重试入口消失。
            let is_failing_turn = cm_msg.id == failing_turn_id;
            match all.iter_mut().find(|m| m.id == cm_msg.id) {
                Some(existing) => {
                    if is_failing_turn {
                        // 仅失败 Turn 承载错误 + 重试入口；中止（`Aborted`）同样可重试，
                        // 但不挂 error 文案——状态本身已经说清了发生了什么。
                        let want_error = if terminal == cm::MessageStatus::Failed {
                            Some(error.to_string())
                        } else {
                            None
                        };
                        if existing.status != Some(terminal.clone())
                            || (want_error.is_some() && existing.error.is_none())
                        {
                            existing.status = Some(terminal.clone());
                            if existing.error.is_none() {
                                existing.error = want_error;
                            }
                            changed.push(existing.clone());
                        }
                    } else if is_inflight(&existing.status) {
                        // 进行中的子节点定稿为 Completed（结束前端流式动画），不挂 error。
                        existing.status = Some(cm::MessageStatus::Completed);
                        existing.error = None;
                        changed.push(existing.clone());
                    }
                    // 已 Completed/Failed 的节点：原样保留，不改动、不广播。
                }
                None => {
                    // 在途图中存在但存储里没有的消息（崩溃时刚流式出来、尚未落库）。
                    // - 失败 Turn 本身：补写为 Failed + error（承载重试入口）；
                    // - 子节点：定稿为 Completed 后补写，**丢弃父节点缺失的孤儿**
                    //   （避免写入无父的悬空节点污染前端渲染）。
                    let parent_exists = cm_msg
                        .parent_id
                        .as_ref()
                        .map(|pid| all.iter().any(|m| m.id == *pid))
                        .unwrap_or(true);
                    let mut new = cm_msg.clone();
                    if is_failing_turn {
                        new.status = Some(terminal.clone());
                        // 同上：只有失败才承载 error 文案，中止不挂。
                        new.error = if terminal == cm::MessageStatus::Failed {
                            Some(error.to_string())
                        } else {
                            None
                        };
                        all.push(new.clone());
                        changed.push(new);
                    } else if parent_exists {
                        new.status = Some(cm::MessageStatus::Completed);
                        new.error = None;
                        all.push(new.clone());
                        changed.push(new);
                    }
                    // 孤儿子节点：直接丢弃，不写入存储。
                }
            }
        }

        if let Err(e) = chat_session.replace_messages(all).await {
            crate::plugin_error!("session", "persist_failure: replace_messages failed: {}", e);
        }

        // 6. 仅发布本次状态真正变化的消息（服务端权威终态，前端只信服务端）：
        //    - 根 Turn 的 Failed + error → 错误条 + 重试入口；
        //    - 子节点的 Completed → 结束前端流式动画（不再渲染 ⚠）。
        //    推送发生在 Error 事件之前（调用方先 persist_failure 再 broadcast_error_with_idle），
        //    Error 事件仅承担 transport 级兜底语义。
        for m in changed {
            transcript
                .lock()
                .await
                .apply(crate::symbio_core::llm_state_frame(&m));
        }

        // 7. 本轮在途图随之作废：权威副本已由上面的 `replace_messages` 回到存储，
        //    继续叠加只会让同一条消息在转写列表里出现两次。
        transcript.lock().await.clear();
    }

    /// 中止收口：把该会话**仍在飞行中的消息节点**定稿，使「用户按下停止」与
    /// 「界面不再显示运行中」之间没有窗口。返回被定稿的节点数。
    ///
    /// 终态分工：根级 Turn → [`cm::MessageStatus::Aborted`]（可重试），
    /// 子节点 → `Completed`（结束流式动画）。判据见 [`abort_terminal_of`]。
    ///
    /// ## 为什么不能只靠 [`Self::persist_failure`]
    ///
    /// `persist_failure` 的触发点是**消费循环收到 Error 帧**。但中止信号是经
    /// `ai_control_tx` 投给 chat_loop 的，而 chat_loop 只在**检查点**才能看到它：
    /// 一旦它卡在某个不轮询通道的 await 里，帧就一直躺在通道里没人取
    /// （非流式工具执行就是典型——`route_fut` 是单次 await，最长硬超时 600s）。
    ///
    /// `handle_abort` 的 3s 兜底会把 `is_working` 复位，于是消费循环在下一帧醒来时
    /// 直接 `break`（`!is_working`），**跳过 `persist_failure`**；而工具节点早已被
    /// `emit_tool_running` 置为 `Streaming` 并广播出去。结果：会话显示「已中止」，
    /// 节点显示「运行中」，且没有任何机制会纠正它——违反
    /// `session/docs/node-state-streaming.md` §8 #11「不得有节点停在 Streaming」。
    ///
    /// 因此中止路径**自己**承担收口责任，不依赖 chat_loop 是否已退出：这是个
    /// 「谁宣称状态、谁负责收口」的划分，不是补丁。
    ///
    /// ## 只看在途缓冲
    ///
    /// 存储不需要扫描：持久层写入不变量（`chat_session.rs::ensure_durable_states`）
    /// 拒绝任何瞬态状态落盘，`Streaming` 只经广播通道存在——「存储里的在途节点」
    /// 按设计就不可能出现在。在途缓冲（`state.live_messages`）是唯一可能停在
    /// 瞬态状态的地方，也是本函数唯一的收敛对象。
    pub(super) async fn converge_inflight(
        &self,
        state: &Arc<ActiveSessionState>,
        session_id: &str,
        reason: &str,
    ) -> usize {
        let chat_session = match self.open_chat_session(session_id).await {
            Ok(cs) => cs,
            Err(e) => {
                crate::plugin_error!("session", "converge_inflight: open session failed: {}", e);
                return 0;
            }
        };
        let mut all = match chat_session.get_messages().await {
            Ok(m) => m,
            Err(e) => {
                crate::plugin_error!("session", "converge_inflight: get_messages failed: {}", e);
                return 0;
            }
        };
        let live = state.transcript.lock().await.snapshot();

        // 在途缓冲里**尚未落库**的节点：补写，前提是父节点存在
        //    （与 `persist_failure` 同一孤儿规则——写入无父的悬空节点会污染前端渲染）。
        // 存储侧无需扫描：写入不变量保证存储只有终态（见上 doc）。
        let mut changed: Vec<cm::ChatMessage> = Vec::new();
        for l in &live {
            if !is_inflight(&l.status) || all.iter().any(|m| m.id == l.id) {
                continue;
            }
            let parent_exists = l
                .parent_id
                .as_ref()
                .map(|pid| all.iter().any(|m| m.id == *pid))
                .unwrap_or(true);
            if !parent_exists {
                continue;
            }
            let mut new = l.clone();
            new.status = Some(abort_terminal_of(l));
            new.error = None;
            all.push(new.clone());
            changed.push(new);
        }

        if changed.is_empty() {
            // 在途图仍要作废：本轮要么已由 `persist_failure` 收口（权威副本已在
            // 存储里），要么根本没有在途节点。留着只会让陈旧副本继续参与叠加。
            state.transcript.lock().await.clear();
            return 0;
        }

        if let Err(e) = chat_session.replace_messages(all).await {
            crate::plugin_error!(
                "session",
                "converge_inflight: replace_messages failed: {}",
                e
            );
            return 0;
        }

        crate::plugin_info!(
            "session",
            "converge_inflight: {} 个节点定稿（根 Turn → Aborted，子节点 → Completed；原因：{}）",
            changed.len(),
            reason
        );
        let converged = changed.len();

        // 广播：这些终态同样是**消息变更**，VDFS 列表必须同步收敛
        // （列表读存储 + 在途叠加，而存储已被改写）。这些都是本次新落库的
        // 节点，以 `created` 报出——VDFS 列表里它们此前不存在。
        for m in changed {
            state
                .transcript
                .lock()
                .await
                .apply(crate::symbio_core::llm_state_frame(&m));
        }

        // 权威副本已回到存储，在途图作废（与 `persist_failure` 步骤 7 同一理由）。
        state.transcript.lock().await.clear();
        converged
    }
}

/// 计算 `root` 在 `messages` 中的传递闭包子树（含 `root` 自身）。
///
/// `persist_failure` 用它把「错误降级」的作用域收窄到失败 Turn 及其后代：
/// 历史轮次的消息不参与合并，避免 session 层拿流式快照去"补写"一份
/// Model 插件已落库的节点（同一 Turn 下出现两份同内容文本节点的根源）。
fn subtree_of(messages: &[cm::ChatMessage], root: &str) -> std::collections::HashSet<String> {
    let mut ids = std::collections::HashSet::new();
    ids.insert(root.to_string());
    let mut queue = vec![root.to_string()];
    while let Some(pid) = queue.pop() {
        for m in messages {
            if m.parent_id.as_deref() == Some(pid.as_str()) && ids.insert(m.id.clone()) {
                queue.push(m.id.clone());
            }
        }
    }
    ids
}
