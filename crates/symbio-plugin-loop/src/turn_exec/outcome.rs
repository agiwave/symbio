//! 阶段③ 结局归类 + 一轮执行的出口类型（· B2，DE-021 §5 动作 B2）。
//!
//! 桥上取走未定格的正文/思考 → 桥归零（发射端退出的唯一信号）→ 发射任务收干 →
//! 按 `aborted / closure_denied / 正常` 三条结局归类：前两条直接现成收口
//! （[`Classified::Finished`]），正常收束则把落库动作交阶段④ [`super::persist`]。
//!
//! **纯结构拆分契约**：[`classify_turn`] 体内自 `turn_exec.rs` 的 `execute_turn`
//! 第三段逐字搬移，仅三处 `return` 改裹 [`Classified::Finished`]（早退语义不变）；
//! [`ExecTurnResult`] / [`TurnExecExit`] 同批搬入（本段的归类结果直接构造它们）。

use super::*;

/// 一轮 v2 原生产物的**全部出口**。
pub(crate) struct ExecTurnResult {
    /// 收口用产物（正文 / 节点 id）——`TurnOutput` 同形，收口步骤 5–7 只认这一份结构。
    pub output: TurnOutput,
    /// 本轮 v2 原生**额外产生**的消息（工具轮：助手中途正文 + ToolCall + 工具结果），
    /// 按产生顺序。chat_loop 把它们并入 `context.messages`——落库权威是那份
    /// 内存镜像（`persist_messages` 只写它的尾部增量），不并入就只看得见不落库。
    pub messages: Vec<ChatNode>,
}

/// [`super::execute_turn`] 的退出类型。
///
/// 为什么不是裸 `PluginError`：中止出口要携带**半截产出**（`Vec<ChatNode>`），
/// 而那批节点是 chat 域词汇。`PluginError` 是跨插件共享的错误词汇（`model`
/// 插件也构造 `Aborted`）——它不搬这个箱子：`to_frame` 只写错误码与文案，
/// 模型侧的中止在 `adapters` 边界转成 `AdapterError::Aborted`（载荷在那里
/// 被丢掉），loop 侧由 `execute_turn` 见到 `outcome.aborted` 后**重新构造**
/// 一份。半截产出的唯一生产方与唯一消费方都在 loop 内、是一次普通的进程内
/// Rust 调用，型在两侧编译期可见；塞进共享词汇再拆箱（无论用不用 `Any`）
/// 都是给一条不跨墙的数据付跨墙的税（[D-095]）。
///
/// 决策 D-095 见 [D-095](../../../../docs/003-design/005-协议规范.md#d-095-keys-域环拆解键与值同住aborted-载荷不跨域vdfs_change_of-归-event_bus "D-095")
#[derive(Debug)]
pub(crate) enum TurnExecExit {
    /// 用户中止且半截产出已在手（正文 / 思考 / 工具节点）——它们是事实，
    /// 不因没说完就不记（消费循环按 id 去重、整条覆盖、孤儿丢弃后入格）。
    Aborted(Vec<ChatNode>),
    /// 其余一切（模型失败 / 编排失败 / 内部错误）。
    Failed(PluginError),
}

impl From<PluginError> for TurnExecExit {
    fn from(e: PluginError) -> Self {
        // `Aborted` 一律走中止出口，不压成失败（D-044 同源纪律）：走到这里的
        // 中止没有半截产出可带（模型侧中止在 adapters 边界已转
        // `AdapterError::Aborted`，由 `outcome.aborted` 那条路重新构造）。
        match e {
            PluginError::Aborted => TurnExecExit::Aborted(Vec::new()),
            other => TurnExecExit::Failed(other),
        }
    }
}

/// 阶段③ 的出口：本轮已提前了结（[`Classified::Finished`]），或正常收束待落格
/// （[`Classified::Settled`]）。刻意收全三个提前出口于类型——落库段拿不到被跳过的
/// 结局，就不会出现「中止轮还去写派生事实」的顺序漂移。
pub(crate) enum Classified<'a> {
    Finished(Result<ExecTurnResult, TurnExecExit>),
    Settled(Settled<'a>),
}

/// 阶段③ ⇒ 阶段④ 的交付束：收束结局 + 桥上取下的节点身份 + 落库还要用的一切。
pub(crate) struct Settled<'a> {
    pub(crate) session: &'a LoopSession,
    pub(crate) store: Arc<dyn Store<Event = Event>>,
    pub(crate) outcome: super::super::turn_runner::TurnOutcome,
    pub(crate) produced: Vec<ChatNode>,
    pub(crate) derived: super::super::tool_dispatch::DerivedFacts,
    pub(crate) collected: Arc<std::sync::Mutex<Vec<ChatNode>>>,
    pub(crate) turn_no: u64,
    pub(crate) user_seq: u64,
    pub(crate) principal: String,
    pub(crate) to: String,
    pub(crate) user_text: &'a str,
    pub recalled: Option<&'a super::super::recall::RecallView>,
    pub(crate) skill_obs: &'a [(String, bool)],
    pub(crate) child_id: String,
    pub(crate) reasoning_child_id: String,
    pub(crate) reasoning_text: String,
}

/// 一轮的执行（阶段③）：结局归类——消费 [`Running`]，产 [`Classified`]。
pub(crate) async fn classify_turn<'a>(r: Running<'a>) -> Result<Classified<'a>, TurnExecExit> {
    let Running {
        session,
        store,
        bridge,
        emitter,
        outcome,
        produced,
        derived,
        collected,
        turn_no,
        user_seq,
        principal,
        to,
        user_text,
        recalled,
        skill_obs,
        root_id,
        presentation_tree,
    } = r;
    // 本轮**未定格**的正文与推理节点必须在桥 drop 之前取走：它们的收束帧由
    // chat_loop 的 `finalize_assistant_turn` 发出（用 `TurnOutput` 里的这两个 id），
    // 而**落库**则由 `TurnOutput::into_messages` 按同一批 id 建节点。
    //
    // 工具轮的那几轮在 `tool_dispatch::dispatch` 里已经定格并取走了 ⇒ 这里拿到的是
    // **最后一轮**（无工具调用那一轮）的节点；没有推理就是 `None`（`on_reasoning`
    // 一次都没被调过），不是空节点。
    let (child_id, child_text) = bridge.current_node().unwrap_or_default();
    let (reasoning_child_id, reasoning_text) = bridge.current_reasoning().unwrap_or_default();
    // 桥归零 ⇒ 通道关闭 ⇒ 发射端排空后退出。
    drop(bridge);
    emitter
        .await
        .map_err(|e| PluginError::InternalError(format!("流式发射任务失败：{e}")))?;

    // 中止：收束格已由运行器落格（`f-{turn}`，载荷 `status: "aborted"` + `terminated`），
    // 出口走 `TurnExecExit::Aborted`——由 chat_loop/消费循环落库为 `NodeStatus::Aborted`
    // + 会话结局 `aborted`（**不是** `failed`）。
    // 记忆与学习也**不写**——中止的轮次没有定稿产出，没有可固化的东西。
    //
    // **半截产出随出口带走**：末轮正文 / 思考此刻还只攒在桥上，丢了它们
    // 用户中断前看到的那半句话就消失。消费循环把它收尾入格
    // （按 id 去重、整条覆盖、孤儿丢弃，见 `persist_failure`）；工具轮已定格的节点
    // （`produced`）同走这条通道——中止出口不再回灌 `ExecTurnResult`，不带上就没了。
    if outcome.aborted {
        let session_principal = symbio_core::principal::Principal::from_legacy(&principal);
        let mut partial = produced;
        if !child_text.is_empty() {
            let mut n = symbio_core::node_build::llm_build_text_node(
                root_id,
                &child_text,
                None,
                child_id,
                &session_principal,
            );
            symbio_core::node_build::detach_if_flat(&mut n, root_id, presentation_tree);
            partial.push(n);
        }
        if !reasoning_text.trim().is_empty() {
            let mut n = ChatNode {
                id: reasoning_child_id,
                parent_id: Some(root_id.to_string()),
                principal: Some(session_principal.clone()),
                msg_type: Some(NodeKind::Notice),
                content: Some(MessageContent::Text(reasoning_text.clone())),
                status: Some(NodeStatus::Completed),
                timestamp: Some(symbio_core::clock_now_ms()),
                ..Default::default()
            }
            .with_style_tag(CHAT_STYLE_TAG_REASONING);
            symbio_core::node_build::detach_if_flat(&mut n, root_id, presentation_tree);
            partial.push(n);
        }
        return Ok(Classified::Finished(Err(TurnExecExit::Aborted(partial))));
    }

    // ── 收束派生事实（两半）：承诺 / 任务表 / 熔断 + 记忆与学习 ────────────────
    //
    // **本步的存在理由**：轮次事实由运行器原生入格，这两半都**不是轮次事实**，而是
    // 本轮的**派生副作用**——运行器一处都不写，写方就是这里的 `record_derived` /
    // `record_learning`（溯源锚 = 运行器原生写的 `u-{turn}` 格）。不在这里补，
    // 代际立约（S08）/ 任务表（S7）/ 熔断（S9 §6 验收 2）/ 长期记忆（S06）/
    // 技能自我改进（S11）就整体失效。
    //
    // 兜底收束的 `response` 为 `None`：兜底说明这条路没走通，固化它等于把失败写成
    // 套路。写侧闸拒（`closure_denied`，见运行器的 `closure_gate`）：收束格
    // **未入格** ⇒ 本轮的派生事实**也不写**（承诺 / 任务 / 熔断 / 记忆全不写）。
    // 用户的答案照旧经 `produced` 回给调用方——收束格是**记录**，不是呈现。
    if outcome.closure_denied {
        symbio_core::plugin_warn!(
            "session",
            "[turn-exec] 收束被授权拒绝，本轮派生事实不入格（turn={turn_no}）"
        );
        // 被拒的兜底仍是**失败**（I3）：呈现走 Failed 出口，与未拒时同形。
        if outcome.fell_back {
            return Ok(Classified::Finished(Err(TurnExecExit::Failed(
                PluginError::InternalError(outcome.text),
            ))));
        }
        return Ok(Classified::Finished(Ok(ExecTurnResult {
            output: TurnOutput {
                text: outcome.text,
                reasoning: reasoning_text,
                response_text_child_id: child_id,
                reasoning_child_id,
                // 实测用量直通：校准反馈的唯一数据源（缺口 6）。这一格若丢，
                // 校准比冻结在初值 1.0，压缩预检会把本可成功的摘要请求误判成
                // 「注定超限」。
                usage: outcome.usage,
                // 末轮原因直通：`..Default::default` 会把 finish 静默填成
                // Stop，chat_loop 的长度戳与自动续写判据因此读不到真值。反射档 /
                // 兜底 / 中止没有模型轮 ⇒ Stop 就是诚实值。
                finish: outcome.finish,
                ..Default::default()
            },
            messages: produced,
        })));
    }
    Ok(Classified::Settled(Settled {
        session,
        store,
        outcome,
        produced,
        derived,
        collected,
        turn_no,
        user_seq,
        principal,
        to,
        user_text,
        recalled,
        skill_obs,
        child_id,
        reasoning_child_id,
        reasoning_text,
    }))
}
