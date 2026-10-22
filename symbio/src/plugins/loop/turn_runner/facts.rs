//! 运行器的纯取值 / 载荷辅助（T-179 · B3 搬移）。
//!
//! 产出载荷、窗口视图、工具交换消息与 id 合成——都是无状态的纯函数，
//! 因此与阶段拆分层分开放：阶段文件只描述流程，这里只描述形状。

use super::*;

/// 取此刻的流式节点事实（无桥 ⇒ 空事实，不是错误）。
pub(crate) fn node_facts(nodes: &Option<NodeFactsFn>) -> StreamNodeFacts {
    nodes.as_ref().map(|f| f()).unwrap_or_default()
}

/// 产出部分的载荷（收束 / 进展 / 兜底格共用的形状）。
///
/// 一处定义的理由：三种格子记的是**同一批产出事实**（末轮正文 / 思考 / 本轮全部
/// 工具调用 / 工具轮的中途产出），差别只在状态字段——形状写三遍必然漂移，而重建
/// 只按一个形状读。半截正文 / 半截思考照原样进 `text` / `reasoning` / `rounds`：
/// **它们是事实**，不因「没说完」就不记。
pub(crate) fn produced_payload(
    text: &str,
    model: &str,
    turn_root_id: &str,
    facts: &StreamNodeFacts,
    tool_calls: &[ToolCallFact],
    rounds: &[StreamNodeFacts],
) -> serde_json::Value {
    serde_json::json!({
        "text": text,
        // 空串 = 这一轮没有模型参与（定稿答话 / 反射档），不是「忘了记」。
        "model": model,
        // 节点结构入格：重建按这批 id 复刻落库形状（载荷补全）。
        "turn_id": turn_root_id,
        "text_id": facts.text_id,
        "reasoning_id": facts.reasoning_id,
        "reasoning": facts.reasoning,
        "tool_calls": tool_calls_payload(tool_calls),
        "rounds": rounds_payload(rounds),
    })
}

/// `tool_calls` 载荷条目（形状见 [`ToolCallFact`]）。
pub(crate) fn tool_calls_payload(calls: &[ToolCallFact]) -> Vec<serde_json::Value> {
    calls
        .iter()
        .map(|c| {
            serde_json::json!({
                "id": c.id,
                "name": c.name,
                "args": c.args,
                "wire_id": c.wire_id,
                "result_id": c.result_id,
            })
        })
        .collect()
}

/// `rounds` 载荷条目：工具轮的**中途产出**（每轮正文 / 思考 + 节点 id）。
pub(crate) fn rounds_payload(rounds: &[StreamNodeFacts]) -> Vec<serde_json::Value> {
    rounds
        .iter()
        .map(|r| {
            serde_json::json!({
                "text_id": r.text_id,
                "text": r.text,
                "reasoning_id": r.reasoning_id,
                "reasoning": r.reasoning,
            })
        })
        .collect()
}

/// 一次工具调用 → 事实条目（结果配不上的记 `result_id: None`）。
pub(crate) fn tool_fact_of(
    tc: &crate::symbio_core::TurnToolCallInfo,
    outcome: Option<&crate::symbio_core::DispatchOutcome>,
) -> ToolCallFact {
    ToolCallFact {
        id: outcome
            .map(|o| o.call_id.clone())
            .or_else(|| tc.id.clone())
            .unwrap_or_default(),
        name: tc.name.clone().unwrap_or_default(),
        args: crate::symbio_core::node_build::tool_call_args_text(tc),
        wire_id: outcome
            .map(|o| o.wire_id.clone())
            .or_else(|| tc.wire_id.clone())
            .unwrap_or_default(),
        result_id: outcome
            .map(|o| o.result_id.clone())
            .filter(|s| !s.is_empty()),
    }
}

/// 从本轮结果里配对出 `tc` 的那条（按 ToolCall 节点 id / wire id；配不上 ⇒ `None`）。
pub(crate) fn outcome_of<'a>(
    outcomes: &'a [crate::symbio_core::DispatchOutcome],
    tc: &crate::symbio_core::TurnToolCallInfo,
) -> Option<&'a crate::symbio_core::DispatchOutcome> {
    outcomes.iter().find(|o| {
        tc.id.as_deref().map(|id| id == o.call_id).unwrap_or(false)
            || (!o.wire_id.is_empty() && tc.wire_id.as_deref() == Some(o.wire_id.as_str()))
    })
}

/// 给产出载荷补**状态**字段（完成 / 终止 / 等待是事实的状态，不进产出形状本身）。
pub(crate) fn with_status(mut produced: serde_json::Value, status: &str) -> serde_json::Value {
    produced["status"] = serde_json::Value::String(status.to_string());
    produced
}

/// 给收束格载荷补**停机原因**（`reason`，T-220）。
///
/// 与 [`with_status`] 是两个维度，缺一不可：
/// - `status` 说**结果**是哪一类（三值，读侧按它折出 `RuntimeOutcome`）；
/// - `reason` 说**为什么停**（闭集，可判、可统计）。
///
/// 合成一个字段会让「截断但答出来了」（`status: completed` + `reason: exhausted`）
/// 这类组合表达不出来——而那正是此前 `status: completed` 把「自然说完」与
/// 「被上限掐断」混在一起的原因。
pub(crate) fn with_reason(
    mut payload: serde_json::Value,
    reason: crate::symbio_core::ClosureReason,
) -> serde_json::Value {
    payload["reason"] = serde_json::Value::String(reason.as_str().to_string());
    payload
}

/// 从本轮结果折出**停机原因**：被上限掐断（`finish_reason = length`）记
/// [`ClosureReason::Exhausted`]，其余自然结束记 [`ClosureReason::Completed`]。
///
/// 只判 `Length`：内容安全过滤（`ContentFilter`）与未识别原因（`Other`）今天
/// 也走自然收束——把它们另立取值是发明闭集以外的值，而闭集的意义就在于
/// 可穷举。这一处残留的混淆记录在 T-220 的处理过程里。
pub(crate) fn reason_of_finish(
    finish: &crate::symbio_core::ModelFinishReason,
) -> crate::symbio_core::ClosureReason {
    match finish {
        crate::symbio_core::ModelFinishReason::Length => {
            crate::symbio_core::ClosureReason::Exhausted
        }
        _ => crate::symbio_core::ClosureReason::Completed,
    }
}

/// 把定稿答话的**对话面节点身份**并入收束格载荷（见 [`TurnInput::dialog`]）。
///
/// 缺席（`None` = 这一轮不是答话轮）⇒ 载荷原样返回：普通收束格的重建路径不受影响，
/// 「有对话面节点」因此是一个**由载荷显式声明**的事实，不靠重建去猜。
pub(crate) fn with_dialog(
    mut produced: serde_json::Value,
    dialog: Option<&DialogFacts>,
) -> serde_json::Value {
    if let Some(d) = dialog {
        produced["dialog"] = serde_json::json!({
            "id": d.id,
            "reason": d.reason,
            "exclude_from_context": d.exclude_from_context,
        });
    }
    produced
}

/// 终止信息（`terminated: { at }`，仅终止时有）：终止的事实被截到哪一刻——
/// 半截正文停在哪里由此可观测。
///
/// **`terminated.reason` 已退役**（T-220）：「为什么中止」由顶层 `reason`
/// （`interrupted`）说，这里只留时刻。两处各写一份原因，必然有一处先漂——
/// 而漂的时候两边都是合法拼写，谁也不报错。
pub(crate) fn with_terminated(mut payload: serde_json::Value) -> serde_json::Value {
    payload["terminated"] = serde_json::json!({
        "at": crate::symbio_core::clock_now_ms(),
    });
    payload
}

/// 按主体过滤一批事件（**可见域入口**，[plan/11 批 1](../../../../../docs/004-plan/009-多执行器与多主体加固实施方案.md "PL-009-多执行器与多主体加固实施方案")
/// ③）：`None` = 不过滤（与接线前逐字一致），`Some(v)` = 只交 `visible_to` 通过的事件。
///
/// 全部可见时**不复制**（借用原切片）——平凡值下（单主体会话）这条路径与
/// 接线前逐字一致，零拷贝。
pub(crate) fn filter_visible<'a>(events: &'a [Event], viewer: Option<&str>) -> Cow<'a, [Event]> {
    let Some(viewer) = viewer else {
        return Cow::Borrowed(events);
    };
    if events.iter().all(|e| visible_to(&e.actor, viewer)) {
        Cow::Borrowed(events)
    } else {
        Cow::Owned(
            events
                .iter()
                .filter(|e| visible_to(&e.actor, viewer))
                .cloned()
                .collect(),
        )
    }
}

/// 对话窗口：保留 turn 号落在「当前轮往前数 `keep` 个」之内的事件
/// （含当前轮）。事实是全量的，**视图**才是窗口——纯切片，不改数据。
///
/// `keep == 0` = **不截断**（全量）——与 `SessionConfig::context_messages` 的 `0`
/// 同口径。会话轮必填窗口值正是为了防「WAL 只增不减 ⇒ prompt 无界增长」，而
/// 「配 0 换不截断」是这个防护的**显式关闭开关**，不是笔误。
///
/// 第二层是**可见域**（`viewer`，见 [`filter_visible`]）：窗口先按轮切，
/// 再按主体滤——两层都只改视图，不改数据。
///
/// ## 压缩快照**不在窗口内也要带上**（S2.7-b2）
///
/// `session.consolidated` 是当前历史的**基线**（它的载荷就是压缩后的整份列表），
/// 窗口裁的是「更早的轮次」，不是基线本身。而归档之后没被取代的那段只剩
/// 「快照事件 + 压缩后的新事件」，且快照事件的 `turn` 恒为 0、不在窗口内 ⇒ 纯按轮切
/// 会把基线切掉，于是模型**既看不见摘要、也看不见刚聊过什么**（被保留的近轮只存在于
/// 快照载荷里——它们的原始事件已移入归档段）。
///
/// 故窗口 = `[最近一次快照] ++ 窗口内的事件`。只带**最近**一条：更早的快照连它
/// 自己的列表都已被后来的快照取代（与投影 [`crate::symbio_core::transcript`] 同一口径）。
pub(crate) fn window_by_turn<'a>(
    events: &'a [Event],
    current_turn: u64,
    keep: u64,
    viewer: Option<&str>,
) -> Cow<'a, [Event]> {
    if keep == 0 {
        // `keep == 0` = **不截断**（全量）——与 `SessionConfig::context_messages`
        // 的 `0` 同一条口径（那里 `0` 一直是「上下文窗口不按轮次截断」）。两处
        // 对同一个数必须同义：否则配 `0` 的实例会**静默**丢掉全部历史
        // ——事实照样入格、`session/stats` 照样有数，只有模型看不见。
        return filter_visible(events, viewer);
    }
    let oldest = current_turn.saturating_sub(keep - 1);
    let cut = events
        .iter()
        .position(|e| e.entity == Entity::Turn && e.turn >= oldest)
        .unwrap_or(events.len());
    match latest_snapshot_index(events) {
        // 快照在窗口外 ⇒ 单独把它拎到最前（见函数文档）。
        Some(ci) if ci < cut => {
            let mut w: Vec<Event> = Vec::with_capacity(1 + events.len() - cut);
            w.push(events[ci].clone());
            w.extend_from_slice(&events[cut..]);
            if let Some(v) = viewer {
                w.retain(|e| visible_to(&e.actor, v));
            }
            Cow::Owned(w)
        }
        _ => filter_visible(&events[cut..], viewer),
    }
}

/// 切片里**最近一次**压缩快照（`session.consolidated`）的下标。
pub(crate) fn latest_snapshot_index(events: &[Event]) -> Option<usize> {
    events.iter().rposition(|e| {
        e.entity == Entity::Message
            && e.verb == Verb::Progressed
            && e.kind == EVENT_SESSION_CONSOLIDATED
    })
}

/// 一轮工具交换 → **结构化**消息（ADR-048a）。
///
/// 出的是真消息：assistant（带 `tool_calls` 摘要）/ tool（带 `tool_call_id`）。
/// 拍成散文（`助手请求工具: … / 工具结果(x): …`）会同时坏三件事——把工具调用当成
/// **assistant 说的话**、把结果做成**没有角色的文本**、**丢掉 `tool_call_id`**
/// （provider 侧协议要求 `tool` 消息关联到具体那次调用，否则多工具并发时无法配对）。
///
/// ## `round` 参数的存在理由：这一批消息要**跨轮累加**
///
/// 交换是**累加**进下一次请求的（见运行器 2e：模型必须看见本回合此前每一次调用与
/// 结果，否则重复调用同一个工具）。于是 id 的唯一性域也从「一次请求」扩到「整个
/// 回合」——同一工具在两个工具轮各调一次，若两侧的 id 相同，adapter 的 `last_result_of`
/// 会把它当成**同一次调用的两条结果**而只留最后那条（第一次的结果凭空消失）。
/// id 因此优先取调用自身带得的身份（wire id / 节点 id，二者天然跨轮唯一），
/// 两者都缺才合成，且合成式里带上轮次（见 [`prompt_call_id`]）。
pub(crate) fn tool_exchange_messages(
    turn: &crate::symbio_core::LlmTurn,
    outcomes: &[crate::symbio_core::DispatchOutcome],
    round: usize,
) -> Vec<crate::symbio_core::PromptMessage> {
    let mut out: Vec<crate::symbio_core::PromptMessage> = Vec::new();
    if !turn.text.trim().is_empty() {
        out.push(crate::symbio_core::PromptMessage {
            role: "assistant".into(),
            text: turn.text.trim().to_string(),
            tool_call_id: None,
            tool: None,
            tool_calls: None,
        });
    }
    // 调用请求：作为 assistant 消息，**带真正的 `tool_calls` 结构**。
    //
    // ⚠️ `tool_calls` 字段不能省（实测踩过）：协议层的请求包清洗会丢弃「无对应
    // `tool_call` 的 tool 结果」，而「对应的 tool_call」是从 assistant 消息的
    // `tool_calls` **结构**里认的，不是从正文里认的。只在正文写「调用工具 x」的话，
    // 后面每一条 tool 结果都成孤儿被丢 ⇒ 模型永远收不到结果 ⇒ **无限工具循环**
    // （实测 9905 次请求、CLI 撞 120s 超时，症状完全指不到这一层）。
    //
    // 正文那行仍然保留：它是给人读的（诊断 / 日志 / 断言），不是给协议用的。
    for (i, call) in turn.tool_calls.iter().enumerate() {
        let name = call.name.as_deref().unwrap_or("<unnamed>");
        let id = prompt_call_id(call, round, i);
        out.push(crate::symbio_core::PromptMessage {
            role: "assistant".into(),
            text: format!("调用工具 {} {}", name, call.arguments),
            tool_call_id: Some(id.clone()),
            tool: Some(name.to_string()),
            tool_calls: Some(vec![PromptToolCall {
                id,
                name: name.to_string(),
                arguments: call.arguments.clone(),
            }]),
        });
    }
    // 结果：逐条 tool 消息，**按调用配对**取 id（不是按工具名）。
    //
    // ⚠️ 为什么不能按名字配：同一次响应里调用同一个工具两次时，按名配对会把**两条结果
    // 都挂到第一次调用**的 id 上。而 adapter 侧 `last_result_of` 按 id 取「同一次调用的
    // 最终那条结果」——两条同 id 的结果在模型眼里就是一次调用的两条，第一次的真实
    // 结果被吃掉，第二次拿到别人的正文。**结果与调用错配比缺一结果更坏**：模型据此
    // 改道，而它改的是凭据不足的道。
    //
    // 配不上调用时（无对照物）退回结果自己的 `call_id`：它在本次请求里没有对应的
    // 调用声明，adapter 会把它降级成上下文（`declared_here` 为假那条路）。
    for outcome in outcomes {
        let at = turn
            .tool_calls
            .iter()
            .position(|c| outcome_of(outcomes, c).is_some_and(|o| o.call_id == outcome.call_id));
        let id = match at {
            Some(i) => prompt_call_id(&turn.tool_calls[i], round, i),
            None => outcome.call_id.clone(),
        };
        out.push(crate::symbio_core::PromptMessage {
            role: "tool".into(),
            text: outcome.text.clone(),
            tool_call_id: Some(id),
            tool: Some(outcome.name.clone()),
            tool_calls: None,
        });
    }
    out
}

/// 续写轮的恢复交换 → 结构化消息。与 [`tool_exchange_messages`] 同形：
/// 「这一轮是续写」是**调度事实**，不该变成模型眼里的另一种协议。
pub(crate) fn exchange_messages(
    call: &crate::symbio_core::TurnToolCallInfo,
    text: &str,
) -> Vec<crate::symbio_core::PromptMessage> {
    let name = call.name.as_deref().unwrap_or("<unnamed>");
    // 恢复交换只有这一次调用 ⇒ 轮次记 0（id 仍优先取调用自带的身份，见
    // [`prompt_call_id`]）：恢复的调用与本轮其它调用共属同一次请求，撞名即错配。
    let id = prompt_call_id(call, 0, 0);
    vec![
        crate::symbio_core::PromptMessage {
            role: "assistant".into(),
            text: format!("调用工具 {} {}", name, call.arguments),
            tool_call_id: Some(id.clone()),
            tool: Some(name.to_string()),
            tool_calls: Some(vec![PromptToolCall {
                id: id.clone(),
                name: name.to_string(),
                arguments: call.arguments.clone(),
            }]),
        },
        crate::symbio_core::PromptMessage {
            role: "tool".into(),
            text: text.to_string(),
            tool_call_id: Some(id),
            tool: Some(name.to_string()),
            tool_calls: None,
        },
    ]
}

/// 合成 / 选取 `tool_call_id`（跨轮唯一，见 [`tool_exchange_messages`] 的 `round`）。
///
/// 优先级与 v1 的请求构建**同一口径**（`plugins/model/message_builder.rs`：wire id
/// 优先、历史数据回退节点 id）：
/// 1. `wire_id`——provider 自己签发的 `tool_call_id`，模型认得它，跨轮天然唯一；
/// 2. `id`——节点 id（会话内唯一），provider 没给 wire id 时的对照物；
/// 3. 合成 `call_<tool>_<轮>_<序号>`——两者都缺（桩 / 老数据）时兜底，
///    **带轮次**是为了跨轮累加后不与任何一次调用撞名。
///
/// 前缀 `call_` 是**故意的**：它让 id 一眼可辨是合成的（provider 侧回填时若
/// 与真实调用 id 空间混淆，冲突能被看见而不是静默）。
pub(crate) fn prompt_call_id(
    call: &crate::symbio_core::TurnToolCallInfo,
    round: usize,
    index: usize,
) -> String {
    fn present(id: &Option<String>) -> Option<String> {
        id.clone().filter(|s| !s.trim().is_empty())
    }
    present(&call.wire_id)
        .or_else(|| present(&call.id))
        .unwrap_or_else(|| {
            synthetic_call_id(call.name.as_deref().unwrap_or("<unnamed>"), round, index)
        })
}

pub(crate) fn synthetic_call_id(tool: &str, round: usize, index: usize) -> String {
    format!("call_{tool}_{round}_{index}")
}
