//! 主循环的**副作用出口**：落库、广播、流式占位。
//!
//! 集中在此的目的：让主循环与单轮逻辑只表达"做什么"，把"写到哪里"收在一处。
//!
//! 生命周期钩子（`UserPromptSubmit` / `Stop`）不在这里：它们是**会话生命周期**的
//! 一部分，随 `StopSignal` 留在 session（[D-086] §6）。本插件只把末条正文经
//! `TurnReport.last_message` 交回。
//!
//! 决策 D-086 见 [D-086](../../../../../docs/003-design/014-单环一表.md#d-086-loopsession-切片契约turndispatch-进程内交付turnspec-纯值面内部动作走地址面 "D-086")

use super::*;
use crate::symbio_core::chat_node::CHAT_STYLE_TAG_RUN;

/// 封根 Turn：广播本轮组合节点的终态（**唯一**发射点）。
///
/// 调用位置固定在 `close_turn` 返回之后——Turn 是组合节点（仅分组、无正文），
/// 终态必须**跟随子树**，而 `close_turn` 归来恰好意味着本轮全部子节点都已收敛
/// （ToolCall 由执行方定格、结果子节点已就位）。放在 `finalize_assistant_turn`
/// （LLM 流结束那一刻）会早整整一个执行窗口，前端于是看到"轮次已完成、其中的
/// 工具调用仍在运行"。
///
/// 发的是**权威转写里的那条节点**（`llm_build_assistant_messages` 的产物，也是
/// `persist_messages` 即将落库的那份），不是手拼的半截快照：帧里带的身份、`meta`
/// 与状态就是权威副本上那份，手拼会漏掉先前帧写下的场景字段。
/// 节点不在转写里 = 本轮根本没建立过（异常路径），静默返回。
pub(crate) async fn finalize_turn_root(
    sink: &ExecEventSink,
    context: &SessionContext,
    root_id: &str,
) {
    let Some(mut node) = context.messages.iter().find(|m| m.id == root_id).cloned() else {
        return;
    };
    node.status = Some(NodeStatus::Completed);
    // Turn 是组合节点（仅分组、无正文）⇒ 状态帧。
    llm_emit_state(sink, node).await;
}

/// 增量落库 + **落库回包**（把存储分配的权威 `seq` 交回实时面）。
///
/// 写口是 session 的 `add-by-runner` 动作（[D-086] §4）：落库、补身份（`principal`）、
/// 给消息补权威 `seq`、把权威副本投进本会话转写——两件事在 session 侧是**一个**出口
/// （`transcript::publish::append_and_publish`），不在这里拆成两半。
///
/// 发的是 `append_and_publish` 交回的**存储权威副本**（`content` 整条替换），不是
/// 本插件内存镜像里那份：后者没有号——存储是在临界区内给自己的私有副本补号的。
///
/// 决策 D-086 见 [D-086](../../../../../docs/003-design/014-单环一表.md#d-086-loopsession-切片契约turndispatch-进程内交付turnspec-纯值面内部动作走地址面 "D-086")
pub(crate) async fn persist_messages(
    context: &SessionContext,
    last_saved: usize,
    sink: &ExecEventSink,
) {
    let new_messages = &context.messages[last_saved..];
    if new_messages.is_empty() {
        return;
    }

    // `Carrier::Runner` 的语义由动作名承载：在途助手 / 工具节点的**承载事实由运行器
    // 落**（助手正文走 `turn_exec` 的 `assistant.progressed`、工具结果走
    // `turn_runner` 的 `artifact.added`），此处再落一条 `message.added` 就是第二个
    // owner。本处只做「落库 + 回包」。
    if let Err(e) = context
        .session
        .run_op::<Vec<ChatNode>>(
            crate::symbio_core::run_ops::ADD_BY_RUNNER,
            crate::symbio_core::AddNodesRequest {
                messages: new_messages.to_vec(),
            },
        )
        .await
    {
        // 持久化失败（可恢复）：不静默吃错误，也不中断对话（消息仍在内存中，
        // chat_loop 继续）。错误是**状态**不是事件——发 `Warn` 由消费循环写入
        // 会话节点 `attributes.warning`，前端按状态渲染；新一轮请求开始时清除。
        let msg = format!("消息持久化失败（消息仍在内存中）: {}", e);
        plugin_warn!("session", "[Session] {}", msg);
        // 告警是会话节点状态（VDFS watch 域），经出口的告警通道下发，不是消息帧。
        sink.warn(Some(msg)).await;
    }
}

pub(crate) async fn emit_streaming_start(sink: &ExecEventSink, root_id: &str, turn: Option<usize>) {
    let meta = turn.map(|t| serde_json::json!({"turn": t}));
    // 骨架（一整轮响应）节点无正文：身份 + 状态一帧到位。
    sink.emit(
        ChatNode {
            id: root_id.to_string(),
            // 骨架是根级节点，与 User 互为兄弟（请求/响应由 NodeRole 区分）
            parent_id: None,
            role: Some(NodeRole::Assistant),
            msg_type: Some(NodeKind::Skeleton),
            status: Some(NodeStatus::Streaming),
            meta,
            ..Default::default()
        }
        .with_style_tag(CHAT_STYLE_TAG_RUN),
    )
    .await;
}
