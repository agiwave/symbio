//! 失败降级持久化：把"仍在飞行中的那一个 Turn"标为 `Failed`。
//!
//! - `persist_failure`：作用域**收窄到失败 Turn 及其后代**（M-002）——历史轮次的
//!   消息不参与合并，避免 session 层拿流式快照去"补写"一份 Model 插件已落库的节点；
//! - `subtree_of`：计算 `root` 的传递闭包子树，供上面的作用域判定使用。
//!
//! 可见性：`persist_failure` 被 `consume.rs` 的 `fail_before_loop` 与根文件的
//! `WorkingGuard::drop` 调用，故标 `pub(super)`。

use super::*;

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
    /// - 失败 Turn（根级）：标 `Failed` + `error`，这是前端唯一渲染 ⚠ 错误条 + 重试入口的节点
    /// - 失败 Turn 的子节点（text / reasoning / tool_call / 工具结果）：仅把仍在进行中的
    ///   (None / Streaming / Pending) 定稿为 Completed，**绝不挂 error**
    /// - 尚未落库的子节点：补写，前提是其父节点存在（避免写入悬空孤儿节点）
    ///
    /// 这样切回会话时（`get_messages`）能看到上次的失败终态与原因（CHAT_FLOW_ANALYSIS 目标 3）。
    pub(super) async fn persist_failure(
        &self,
        state: &Arc<ActiveSessionState>,
        session_id: &str,
        collected: &Arc<tokio::sync::Mutex<Vec<cm::ChatMessage>>>,
        error: &str,
    ) {
        // 1. 定位失败 Turn。
        //    `collected` 为空表示错误发生在任何 Turn 节点创建之前（例如能力收集失败、
        //    Model 插件路由直接报错）——此时没有任何节点可降级，错误属于会话级，
        //    由调用方广播的 Error 事件承载，这里直接返回。
        let failing_turn_id = {
            let c = collected.lock().await;
            c.iter()
                .rev()
                .find(|m| m.msg_type == Some(cm::MessageType::Turn) && m.parent_id.is_none())
                .map(|m| m.id.clone())
        };
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
        let subtree_ids = {
            let c = collected.lock().await;
            subtree_of(&c, &failing_turn_id)
        };

        // 3. 在本任务的内存镜像里定稿失败 Turn 的子树。
        //
        //    关键修复（原始 Bug：错误刷屏 + 工具节点出现服务器错误）：
        //    - 失败 Turn（根级）：Failed + error，这是前端唯一渲染 ⚠ 错误条 + 重试入口的节点。
        //    - 其余子节点（text / reasoning / tool_call）：仅把仍在进行中的
        //      (None / Streaming / Pending) 定稿为 Completed，**绝不挂 error**。
        //      理由：工具调用在本地执行、不请求 LLM 服务器，不可能产生「API 错误」；
        //      把同一个 429 刷到每条文本/工具节点既不符合逻辑，又造成错误刷屏。
        {
            let mut c = collected.lock().await;
            for m in c.iter_mut() {
                if !subtree_ids.contains(&m.id) {
                    continue;
                }
                if m.id == failing_turn_id {
                    m.status = Some(cm::MessageStatus::Failed);
                    if m.error.is_none() {
                        m.error = Some(error.to_string());
                    }
                } else if matches!(
                    m.status,
                    None | Some(cm::MessageStatus::Streaming) | Some(cm::MessageStatus::Pending)
                ) {
                    m.status = Some(cm::MessageStatus::Completed);
                    m.error = None;
                }
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
        let c = collected.lock().await;
        for cm_msg in c.iter() {
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
                        // 仅失败 Turn 承载错误 + 重试入口。
                        if existing.status != Some(cm::MessageStatus::Failed)
                            || existing.error.is_none()
                        {
                            existing.status = Some(cm::MessageStatus::Failed);
                            if existing.error.is_none() {
                                existing.error = Some(error.to_string());
                            }
                            changed.push(existing.clone());
                        }
                    } else if matches!(
                        existing.status,
                        None | Some(cm::MessageStatus::Streaming)
                            | Some(cm::MessageStatus::Pending)
                    ) {
                        // 进行中的子节点定稿为 Completed（结束前端流式动画），不挂 error。
                        existing.status = Some(cm::MessageStatus::Completed);
                        existing.error = None;
                        changed.push(existing.clone());
                    }
                    // 已 Completed/Failed 的节点：原样保留，不改动、不广播。
                }
                None => {
                    // collected 中存在但存储里没有的消息（崩溃时刚流式出来、尚未落库）。
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
                        new.status = Some(cm::MessageStatus::Failed);
                        if new.error.is_none() {
                            new.error = Some(error.to_string());
                        }
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
        drop(c);

        if let Err(e) = chat_session.replace_messages(all).await {
            crate::plugin_error!("session", "persist_failure: replace_messages failed: {}", e);
        }

        // 6. 仅广播本次状态真正变化的消息（服务端权威终态，前端只信服务端）：
        //    - 根 Turn 的 Failed + error → 错误条 + 重试入口；
        //    - 子节点的 Completed → 结束前端流式动画（不再渲染 ⚠）。
        //    推送发生在 Error 事件之前（调用方先 persist_failure 再 broadcast_error_with_idle），
        //    Error 事件仅承担 transport 级兜底语义。
        //
        //    走 `emit_message_patch`（而非直接 `broadcast_frame`）：这几条终态同样是
        //    **消息补丁**，VDFS 列表必须同步收敛，否则一次失败之后 VDFS 视图会永远
        //    停在 streaming——`replace_messages` 已把终态写进存储，而列表读的是存储。
        //    这些消息都已在存储中（`existed = true`），且这里是全量替换终态，
        //    故一律 `updated`——`patch` 与 `view` 同一条（全量帧本身就是合并结果）。
        for m in changed {
            self.emit_message_patch(state, session_id, m.clone(), &m, true, None)
                .await;
        }

        // 7. 本轮在途缓冲随之作废：权威副本已由上面的 `replace_messages` 回到存储，
        //    继续叠加只会让同一条消息在 VDFS 列表里出现两次。
        collected.lock().await.clear();
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
