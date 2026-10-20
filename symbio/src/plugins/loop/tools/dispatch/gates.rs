//! 阶段①「批首闸门读数」——`process_tool_calls_async` 三段拆分的第一段
//! （T-179 / [PL-015 §5 动作 B](../../../../../../docs/004-plan/015-可扩充性与插件切分.md "PL-015-可扩充性与插件切分")）。
//!
//! 判据（「这段错了谁会红」）：闸门判据**只在批首读一次**、整批复用的读数用例红
//! （`breaker` / `elimination` / `task_table` 的批首读场景与 `tool_executor.test.rs`
//! 的闸门组）。读数语句的顺序与拆分前逐字一致——`gate_started` 仍从读判据之前起算。

use super::*;

/// 批首一次读齐的全部闸门读数（阶段①的产出；阶段②③只按值消费，不再回读事实源）。
pub(crate) struct GateReadings {
    /// `MODE` 配置读数（`interactive` 下本批可在前序待用户时中止）。
    pub(crate) mode: String,
    /// 熔断 `gate-timeout` 的起算时刻（量「这份输入已经读了多久」，T-186）。
    pub(crate) gate_started: std::time::Instant,
    /// 外部执行闸门判据（S8 第 20 步）。
    pub(crate) breaker: BreakerInputs,
    /// 淘汰闸门判据（S13 §3，T-073）。
    pub(crate) elimination: EliminationInputs,
    /// 任务表写侧闸判据（S03 §3，T-107）。
    pub(crate) task_table: TaskTableInputs,
    /// 工具自声明的行为位（T-158）。
    pub(crate) decls: ToolDecls,
    /// 本批留底 `(id, name, args)`——阶段③「哪些没被执行」的收口依据。
    pub(crate) batch_meta: Vec<(String, String, serde_json::Value)>,
    /// 同参重复调用台账（T-140 步 2）；阶段②逐条登记，故为可变读数。
    pub(crate) seen_calls: Vec<String>,
}

/// 上下文里**本批之外**已出现过的工具调用签名（按出现顺序）。
///
/// 读的是**工具调用请求节点**（骨架 + tag `tool`，正文即定稿参数文本）：
/// 长跑里模型跨请求原样重发同一条调用时，前一次就在这些节点里。
/// `current_ids` 是本批调用的节点 id——转写在执行前**就已含**本轮这一批 ToolCall
/// 节点，不排除的话每条调用都会把自己数成「重复」。
fn prior_call_signatures(context_messages: &[ChatNode], current_ids: &[String]) -> Vec<String> {
    context_messages
        .iter()
        .filter(|m| m.is_tool_call() && !current_ids.contains(&m.id))
        .filter_map(|m| {
            let name = m.tool_name()?;
            let args = match m.content.as_ref()? {
                MessageContent::Text(s) => s.as_str(),
                MessageContent::Parts(_) => return None,
            };
            Some(call_signature(name, args))
        })
        .collect()
}

/// 批首闸门读数（阶段①）：以下语句逐字来自拆分前 `process_tool_calls_async` 的批首段。
pub(crate) async fn read_batch_gates(
    tool_calls: &[TurnToolCallInfo],
    ctx: Arc<dyn PluginInvokeRequest>,
    context_messages: &[ChatNode],
    facts: &dyn Store<Event = Event>,
    elimination_revoked: bool,
) -> GateReadings {
    let mode = ctx.get(crate::symbio_core::MODE).unwrap_or_default();

    plugin_info!(
        "session",
        "[Tool] 处理 {} 个工具调用（mode={}）",
        tool_calls.len(),
        mode
    );

    // 熔断闸门的判据批首读一次（见 `BreakerInputs`）：授权表与成本台账都是只读的，
    // 读一次给整批复用——每个调用点各重放一次事实源，闸门自己就变成时延源了。
    //
    // `gate_started` 从**读判据之前**起算：熔断的 `gate-timeout` 量的是「这份输入
    // 已经读了多久」（[T-186](../../../../../../docs/009-task/186-虚闸清偿批.md "T-186-虚闸清偿批")）。
    // 从前它在 `gate()` 前一刻才起算，`elapsed_ms` 恒为 0，判据永不触发。
    let gate_started = std::time::Instant::now();
    let session_snapshot = facts.range(Seq::new(0));
    let breaker = BreakerInputs::of(&session_snapshot, &ctx);
    // 淘汰闸门的判据同批首读一次（见 [`EliminationInputs`]，S13 §3）：部署级覆盖层
    // 由 session 折叠成**一个值**（它持有全部会话的事实源）；loop 只收结论。
    let elimination = EliminationInputs::of(elimination_revoked, &ctx);
    // 任务表写侧闸的判据同批首读一次（见 `TaskTableInputs`，S03 §3 / T-107）：
    // 只问本会话主体那一行——任务表是会话自己的工作跟踪，写它的资格随主体走。
    let task_table = TaskTableInputs::of(&ctx);
    // 工具自声明的行为位同批首读一次（见 `ToolDecls`，T-158）：淘汰闸 / 承诺
    // 记账 / 任务表三族的管辖对象从能力注册表读，不按名字判。没有 visitor ⇒
    // 三个集合皆空 ⇒ 三道闸都不命中（缺声明与漏声明同形，e2e 兜行为）。
    let decls = ToolDecls::of(&ctx).await;

    // 本批留底 `(id, name, args)`：循环按值消费 `tool_calls`，而「哪些没被执行」
    // 要在循环**之后**才知道（break 出口）——末尾收口既要补节点，也要把
    // **没跑到的 `agent_run`** 记成没履约的承诺（见 [`note_delegation`]）。
    // id 或 name 缺失的不进表：它们走协议失败分支，父节点已入 `parent_updates`。
    let batch_meta: Vec<(String, String, serde_json::Value)> = tool_calls
        .iter()
        .filter_map(|tc| {
            let id = tc.id.as_deref()?;
            let name = tc.name.as_deref()?;
            if id.trim().is_empty() || name.trim().is_empty() {
                return None;
            }
            Some((id.to_string(), name.to_string(), tc.arguments.clone()))
        })
        .collect();

    // 同参重复调用的**台账**（[T-140] 步 2）：批首收进本批之外已出现过的调用签名，
    // 之后每处理一条调用就登记一条。登记以**模型发出的调用**为准，被闸门拒掉的也算
    // ——「绕圈」是模型侧的事实，与本次执行成败无关。
    let current_call_ids: Vec<String> = tool_calls.iter().filter_map(|tc| tc.id.clone()).collect();
    let seen_calls: Vec<String> = prior_call_signatures(context_messages, &current_call_ids);
    GateReadings {
        mode,
        gate_started,
        breaker,
        elimination,
        task_table,
        decls,
        batch_meta,
        seen_calls,
    }
}
