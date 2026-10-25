//! **单轮收尾**：推理产物并入上下文 → 落库 → 流向判定。
//!
//! `close_turn` 是"单轮里发生了什么"的唯一实现；它的出口只有
//! [`TurnFlow`]，**不做终态收尾**（那是 `finish_turn` 的职责）。

use super::*;

/// 输出被 max_tokens 截断时的**自动续写**上限。
const MAX_CONTINUE_ROUNDS: u32 = 3;

/// 推理收尾：定格 assistant 子节点状态 → 校准 token 估算 → 把本轮产出并入上下文。
///
/// 自 `run_chat_loop` 原样搬移（拆函数不拆行为）：语句、注释、调用顺序与搬移前
/// 逐字一致，仅把"提前取出的字段"返回给调用方。
pub(crate) async fn settle_reasoning(
    orchestrator: &ChatOrchestrator,
    context: &mut SessionContext,
    sink: &ExecEventSink,
    root_id: &str,
    out: TurnOutput,
) -> ModelFinishReason {
    // 提前取出本轮的结束原因 / 用量 / 文本子节点 id，
    // 因为 `out.into_messages` 会按值消费 out，之后无法再读这些字段。
    let finish = out.finish.clone();
    let usage = out.usage;
    let rtid = out.response_text_child_id.clone();
    let rrid = out.reasoning_child_id.clone();

    orchestrator
        .finalize_assistant_turn(root_id, &out, sink)
        .await;

    // 用 provider 返回的真实用量滚动校准 token 估算。
    feedback_estimate(usage, &out);

    let mut new_msgs = out.into_messages(root_id, &orchestrator.session.principal());
    // 建树策略（[]）：flat 下本轮响应骨架的直接子节点升根 + 打 turn_root 记号，
    // 与流式层（`finalize_assistant_turn`）逐字同规则，保证 live↔存储↔WAL 三视图一致。
    for n in new_msgs.iter_mut() {
        detach_if_flat(n, root_id, orchestrator.presentation_tree);
    }
    // 工具上下文保留策略：策略不 Stamp 到节点 meta 持久化，
    // 由 run_chat_loop 在构建 LLM 请求前从 Registry 动态解析，
    // 节点 name 即 LLM 可见工具名，与声明名直接匹配。
    context.messages.extend(new_msgs);

    // 被长度截断的 Turn 打标（供前端「继续」按钮与回溯），不得静默结束。
    if finish.is_length() {
        for m in context.messages.iter_mut() {
            if m.id == rtid || m.id == rrid {
                let mut meta = m.meta.clone().unwrap_or_else(|| serde_json::json!({}));
                meta["finish_reason"] = serde_json::json!("length");
                m.meta = Some(meta);
            }
        }
    }

    finish
}

/// 本轮收尾阶段：截断续写 / 落库 / 流向判定。
///
/// **不承担终态收尾**：`fire_stop_hook` 与最终 `persist_messages` 由 [`finish_turn`]
/// 统一执行（本函数返回 `Finish` 前会把 [`TurnState::last_saved`] 推进到最新，
/// 使 `finish_turn` 的落库成为 no-op）。
///
/// ## 工具执行与主动压缩拦截不在这里（已随 DE-014 步骤 5 退役）
///
/// 工具由 v2 运行器在 `turn_exec::execute_turn` 内部经 `SessionDispatchPort` 跑完、
/// 副作用直落事实。v1 外壳在这里有一段「工具分发 + `context_compact` 拦截」分支，
/// 其进入条件（`TurnOutput.tool_calls` 非空）在 v2 恒不成立——生产与测试都不可达，
/// 随外壳删除（[DE-014 步骤 5a](../../../../../docs/003-design/014-单环一表.md "DE-014-单环一表")，
/// 其中 `context_compact` 那一条链的裁决见 [D-072](../../session/README.md#d-072-context_compact-工具与-55-水位提醒一并退役压缩只剩一个入口 "symbio/src/plugins/session/README")）；
/// 真实时序见 `turn_exec` / `turn_runner` 的模块文档。
pub(crate) async fn close_turn(
    sink: &ExecEventSink,
    context: &mut SessionContext,
    turn: &mut TurnState,
    finish: ModelFinishReason,
) -> TurnFlow {
    // 本轮无工具调用（既定事实：工具在 execute_turn 内跑完，回到这里只可能是纯答话）。
    // 「工具参数被截断」不在此处处理：那条路径由 `tools/tool_executor.rs` 的
    // `parse_error` 分支承担（拒绝执行 + 以协议错误回报模型），比这里补一句
    // 面向用户的告警更靠前、信息量更大。
    if finish.is_length() {
        // 纯文本被 max_tokens 截断 → 自动续写：
        // 已产出的（截断）文本已作为 assistant 消息进入上下文，下一轮请求时模型会
        // 自然从断点继续。最多续写 MAX_CONTINUE_ROUNDS 次，避免失控死循环。
        if turn.continuation_count < MAX_CONTINUE_ROUNDS {
            turn.continuation_count += 1;
            plugin_info!(
                "session",
                "[Turn] finish=Length，自动续写 ({}/{})",
                turn.continuation_count,
                MAX_CONTINUE_ROUNDS
            );
            persist_messages(context, turn.last_saved, sink).await;
            turn.last_saved = context.messages.len();
            return TurnFlow::NextTurn;
        }
        // 续写次数耗尽：明确告知，绝不静默结束（会话级告警状态，随下一轮请求清除）。
        sink.warn(Some(format!(
            "输出因达到长度上限而中断（已自动续写 {} 次仍超出）。请提高单次输出预算或缩小任务范围。",
            MAX_CONTINUE_ROUNDS
        )))
        .await;
    }
    plugin_info!(
        "session",
        "[Turn] 结束（正常收尾，无工具调用）finish={:?}",
        finish
    );
    persist_messages(context, turn.last_saved, sink).await;
    turn.last_saved = context.messages.len();
    TurnFlow::Finish(TurnExit::Completed)
}

/// 用 provider 返回的真实用量滚动校准 token 估算。
///
/// 中文/代码场景收益最大；估算长期偏低会直接导致 400 而非过早压缩。
///
/// 两条不变式（破坏任一条都会静默劣化水位判定，故单列为纯函数并配测试）：
/// 1. 反馈必须用**原始启发式估算**（`count_raw`）；用校准后的 `count()` 自反馈
///    会让校准系数收敛到 √(真实比值)（见 `CalibratedTokenizer::feedback` 文档）。
/// 2. 分母必须覆盖 provider 计入 `output_tokens` 的**全部**内容：文本 + 思考 +
///    工具调用名 + 参数 JSON。漏掉任一部分都会系统性低估估算值 → 校准比偏高
///    → 水位提前越过阈值 → 压缩被频繁触发。
fn feedback_estimate(usage: Option<ModelUsage>, out: &TurnOutput) {
    let Some(u) = usage else { return };
    let tok = super::super::tokenizer::default_tokenizer();
    let mut estimated = tok.count_raw(&out.text) + tok.count_raw(&out.reasoning);
    for tc in &out.tool_calls {
        if let Some(name) = tc.name.as_ref().filter(|n| !n.is_empty()) {
            estimated += tok.count_raw(name);
        }
        estimated += tok.count_raw(&tc.arguments.to_string());
    }
    super::super::tokenizer::report_provider_usage(estimated, u.output);
}
