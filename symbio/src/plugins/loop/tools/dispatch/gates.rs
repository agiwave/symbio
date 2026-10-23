//! 阶段①「批首闸门读数」——`process_tool_calls_async` 三段拆分的第一段
//! （/ [DE-021 §5 动作 B](../../../../../../docs/003-design/021-横向扩充与插件切分.md "DE-021-横向扩充与插件切分")）。
//!
//! 判据（「这段错了谁会红」）：闸门判据**只在批首读一次**、整批复用的读数用例红
//! （`breaker` / `elimination` / `task_table` 的批首读场景与 `tool_executor.test.rs`
//! 的闸门组）。读数语句的顺序与拆分前逐字一致——`gate_started` 仍从读判据之前起算。

//! ## 闸门表
//!
//! 本文件同时是**三道具名闸门**的唯一登记处：[`GATES`] 一行 = 一个闸名 + 一个纯判据，
//! 外加该闸可能产出的失败语义。批内执行点只有一个（`batch::run_batch` 的
//! [`first_refusal`]），于是「现在有几道闸、各自什么出口」读这张表即可；新增一道同步闸
//! = 加一行 + 一个判据结构，不再往循环体里抄一遍「拒因 → 结果节点 → 父节点收敛 →
//! 记账 → continue」。
//!
//! **出口不对称是表的一列，不是被抹平的差异**（S09 §6）：
//! - [`RefusalTrace::ZeroEvent`]：拒止**不产事件**（外部执行 `Refuse`，验收 1）；
//! - [`RefusalTrace::Recorded`]：拒因**入格**，经出参交回随收束落 `task.controlled`
//!   （外部执行 `Break`，验收 2）；
//! - [`RefusalTrace::Audited`]：可审计拒绝——拒因进结果、违约承诺照记（淘汰 / PreToolUse）；
//! - [`RefusalTrace::AuditedNoDelegation`]：可审计拒绝但**不记承诺**（任务表：工具没跑，
//!   「声明过什么」无从说起）。
//!
//! PreToolUse 钩子仍是异步的，因此不进 [`GATES`] 的同步判据表；它走同一个
//! [`refuse`] 骨架，并显式声明自己的失败语义（[`Refusal::blocked`]）。

use super::*;

/// 批首一次读齐的全部闸门读数（阶段①的产出；阶段②③只按值消费，不再回读事实源）。
pub(crate) struct GateReadings {
    /// `MODE` 配置读数（`interactive` 下本批可在前序待用户时中止）。
    pub(crate) mode: String,
    /// 熔断 `gate-timeout` 的起算时刻（量「这份输入已经读了多久」，）。
    pub(crate) gate_started: std::time::Instant,
    /// 外部执行闸门判据（S8 第 20 步）。
    pub(crate) breaker: BreakerInputs,
    /// 淘汰闸门判据（S13 §3，）。
    pub(crate) elimination: EliminationInputs,
    /// 任务表写侧闸判据（S03 §3，）。
    pub(crate) task_table: TaskTableInputs,
    /// 工具自声明的行为位。
    pub(crate) decls: ToolDecls,
    /// 工具自声明的风险档：批内执行策略的分流依据，与 `decls` 同一份元数据。
    pub(crate) risks: ToolRisks,
    /// `tool` 端口是否声明同批可并发（DE-015 §3.3 / ）。
    ///
    /// 批首读一次，阶段②只消费这个布尔；`false` 是硬闸，不能被风险档绕过。
    pub(crate) concurrent: bool,
    /// 本批留底 `(id, name, args)`——阶段③「哪些没被执行」的收口依据。
    pub(crate) batch_meta: Vec<(String, String, serde_json::Value)>,
    /// 同参重复调用台账；阶段②逐条登记，故为可变读数。
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
    // 已经读了多久」。
    // 从前它在 `gate()` 前一刻才起算，`elapsed_ms` 恒为 0，判据永不触发。
    let gate_started = std::time::Instant::now();
    let session_snapshot = facts.range(Seq::new(0));
    let breaker = BreakerInputs::of(&session_snapshot, &ctx);
    // 淘汰闸门的判据同批首读一次（见 [`EliminationInputs`]，S13 §3）：部署级覆盖层
    // 由 session 折叠成**一个值**（它持有全部会话的事实源）；loop 只收结论。
    let elimination = EliminationInputs::of(elimination_revoked, &ctx);
    // 任务表写侧闸的判据同批首读一次（见 `TaskTableInputs`，S03 §3 / ）：
    // 只问本会话主体那一行——任务表是会话自己的工作跟踪，写它的资格随主体走。
    let task_table = TaskTableInputs::of(&ctx);
    // 工具自声明的行为位同批首读一次（见 `ToolDecls`，）：淘汰闸 / 承诺
    // 记账 / 任务表三族的管辖对象从能力注册表读，不按名字判。没有 visitor ⇒
    // 三个集合皆空 ⇒ 三道闸都不命中（缺声明与漏声明同形，e2e 兜行为）。
    //
    // 同一份元数据还给出**风险档**（执行策略）：登记表只读一次，两个
    // 消费者按值复用——再遍历一遍收集管线等于把批首读数变成两处时延源。
    let metas = crate::symbio_core::tool_metas(&ctx).await;
    let decls = ToolDecls::of_metas(&metas);
    let risks = ToolRisks::of_metas(&metas);
    // 端口声明与元数据同一处读数：批内能否并发由 `PortDecl` 说，执行层只消费。
    let concurrent =
        crate::symbio_core::ring::port_of(super::super::super::loop_rows::PORTS, "tool")
            .is_some_and(|p| p.concurrent);

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

    // 同参重复调用的**台账**（[] 步 2）：批首收进本批之外已出现过的调用签名，
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
        risks,
        concurrent,
        batch_meta,
        seen_calls,
    }
}

// ── 闸门表与拒止骨架──────────────────────────────────────────────

/// 一次拒止的失败语义——**出口不对称的登记处**（S09 §6）。
///
/// 四个变体不是四份实现，而是四种**事实出口**：它们决定 [`refuse`] 是否把理由
/// 交回入格、是否记违约承诺。把差异收进类型而不是抹平，是为了让「哪类拒止留下
/// 什么痕迹」成为可读的，而不是靠读每个分支的注释。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RefusalTraceKind {
    /// 零事件：拒止不产任何领域事件（外部执行 `Refuse`）。
    ZeroEvent,
    /// 入格：拒因经出参交回，收束时落 `task.controlled`（外部执行 `Break`）。
    Recorded,
    /// 可审计拒绝：结果节点 + 父节点收敛 + 违约承诺记账（淘汰 / PreToolUse）。
    Audited,
    /// 可审计拒绝但不记承诺（任务表：工具没跑，声明无从说起）。
    AuditedNoDelegation,
}

/// 一次拒止的具体失败语义。与 [`RefusalTraceKind`] 同构，`Recorded` 额外携带
/// 入格理由（去重键与事件载荷都用它）。
#[derive(Debug, Clone)]
pub(crate) enum RefusalTrace {
    ZeroEvent,
    Recorded(&'static str),
    Audited,
    AuditedNoDelegation,
}

impl RefusalTrace {
    /// 登记表里对应的列值（测试用它核对「判据返回的语义必在所属行声明内」）。
    pub(crate) fn kind(&self) -> RefusalTraceKind {
        match self {
            Self::ZeroEvent => RefusalTraceKind::ZeroEvent,
            Self::Recorded(_) => RefusalTraceKind::Recorded,
            Self::Audited => RefusalTraceKind::Audited,
            Self::AuditedNoDelegation => RefusalTraceKind::AuditedNoDelegation,
        }
    }

    /// 入格理由（只有 `Break` 有）。调用方按理由去重：同一轮熔断两次会拿到同一个
    /// 事件号，第二条撞幂等键被丢——那条静默失效正是验收 2 要禁的。
    fn break_reason(&self) -> Option<&'static str> {
        match self {
            Self::Recorded(reason) => Some(reason),
            _ => None,
        }
    }

    /// 是否记违约承诺（`agent_run` 的约没履约，拒止同样是事实）。
    fn notes_delegation(&self) -> bool {
        !matches!(self, Self::AuditedNoDelegation)
    }
}

/// 一道闸的拒因：放行是 `None`，拒止是这里的一整份交代。
#[derive(Debug, Clone)]
pub(crate) struct Refusal {
    /// 闸名（日志与测试的稳定标识）。
    pub(crate) gate: &'static str,
    /// 回灌模型的完整拒因（结果正文 = `"{prefix}: {reason}"`）。
    pub(crate) reason: String,
    /// 结果正文前缀：`Refused`（闸门）或 `Blocked`（钩子）。
    pub(crate) prefix: &'static str,
    /// `meta.failure_kind`（约定字段，控制流不靠猜正文）。
    pub(crate) kind: &'static str,
    /// 本闸这一次的失败语义。
    pub(crate) trace: RefusalTrace,
    /// 违约承诺的记账理由；`None` = 用拒因本身。钩子保留自己的原措辞。
    pub(crate) note_why: Option<String>,
}

impl Refusal {
    fn new(gate: &'static str, reason: String, kind: &'static str, trace: RefusalTrace) -> Self {
        Self {
            gate,
            reason,
            prefix: "Refused",
            kind,
            trace,
            note_why: None,
        }
    }

    /// PreToolUse 钩子拦下的拒止：形如闸门拒止，但正文前缀是 `Blocked`，
    /// `failure_kind = "blocked"`，且违约理由保留钩子的原措辞。
    pub(crate) fn blocked(reason: String) -> Self {
        Self {
            note_why: Some(format!("被 PreToolUse 钩子拦下：{reason}")),
            gate: "PreToolUse",
            reason,
            prefix: "Blocked",
            kind: "blocked",
            trace: RefusalTrace::Audited,
        }
    }
}

/// 一次判据调用需要的全部输入：批首读数（只读）+ 本条调用的工具名。
///
/// 判据是**纯函数**——不读事实源、不发事件、不记账；副作用全部收在 [`refuse`]。
pub(crate) struct GateContext<'a> {
    pub(crate) gate_started: std::time::Instant,
    pub(crate) breaker: &'a BreakerInputs,
    pub(crate) elimination: &'a EliminationInputs,
    pub(crate) task_table: &'a TaskTableInputs,
    pub(crate) decls: &'a ToolDecls,
    pub(crate) name: &'a str,
}

/// 闸门表的一行：闸名 / 判据 / 本闸可能产出的失败语义。
pub(crate) struct GateRow {
    /// 闸名（唯一、稳定；改名 = 改事实）。
    pub(crate) name: &'static str,
    /// 判据 fn：`None` = 放行。
    pub(crate) decide: fn(&GateContext<'_>) -> Option<Refusal>,
    /// 本闸**可能**产出的出口语义。具体一次取 [`Refusal::trace`]；本列是登记面，
    /// 测试钉「返回的 trace 必在所属行声明之内」。
    pub(crate) traces: &'static [RefusalTraceKind],
}

/// 三道同步闸的唯一登记表：顺序即判据顺序（先命中者胜）。
///
/// 外部执行闸排第一：它是「这通调用能不能开窗」的总闸，预算 / 授权先于
/// 淘汰与任务表资格（后两者只对自声明受管辖的工具生效）。
pub(crate) const GATES: &[GateRow] = &[
    GateRow {
        name: "外部执行",
        decide: external_execution,
        traces: &[RefusalTraceKind::ZeroEvent, RefusalTraceKind::Recorded],
    },
    GateRow {
        name: "淘汰",
        decide: elimination,
        traces: &[RefusalTraceKind::Audited],
    },
    GateRow {
        name: "任务表",
        decide: task_table,
        traces: &[RefusalTraceKind::AuditedNoDelegation],
    },
];

/// 外部执行闸（S8 第 20 步）：授权 + 时延 / 预算。判据紧贴执行窗口之前、
/// **先于 PreToolUse 钩子**——对一通马上要被拒的调用先跑钩子，等于让拒绝白白
/// 付一次钩子往返。
fn external_execution(ctx: &GateContext<'_>) -> Option<Refusal> {
    let decision = crate::symbio_core::CircuitBreaker.gate(
        ctx.breaker.authorized,
        ctx.breaker.spent_ms,
        ctx.breaker.requested_ms,
        ctx.breaker.budget_ms,
        ctx.gate_started.elapsed().as_millis() as u64,
        // 输入是会话事实源的整段重放 ⇒ 拿深度档的预算，不拿反射档（见 `gate` 文档）。
        crate::symbio_core::LatencyTier::Deep.budget_ms(),
    );
    match decision {
        crate::symbio_core::GateDecision::Allow => None,
        crate::symbio_core::GateDecision::Break { reason } => Some(Refusal::new(
            "外部执行",
            format!("外部执行闸门熔断（{reason}），本轮工具调用未执行"),
            failure_kind::TOOL_UNAVAILABLE,
            RefusalTrace::Recorded(reason),
        )),
        crate::symbio_core::GateDecision::Refuse => Some(Refusal::new(
            "外部执行",
            "未持外部执行能力（produce.artifact），本次调用未执行".to_string(),
            failure_kind::PERMISSION_DENIED,
            RefusalTrace::ZeroEvent,
        )),
    }
}

/// 淘汰闸（S13 §3，）：只拦声明 `provides_assign_work` 的工具。
///
/// 出口是**可审计拒绝**：`verdict.asserted` 已在事实源里（闸门只是读侧），本层
/// 不再产事件；结果上的 `Refused: …` 与随收束入格的违约承诺就是它的两面。
fn elimination(ctx: &GateContext<'_>) -> Option<Refusal> {
    if !(ctx.decls.provides_assign_work(ctx.name) && ctx.elimination.delegation_revoked) {
        return None;
    }
    Some(Refusal::new(
        "淘汰",
        format!(
            "主体 {} 的 assign.work 已被淘汰回路收回（verdict.asserted，声誉低于阈值），本次委派未执行",
            ctx.elimination.principal
        ),
        failure_kind::PERMISSION_DENIED,
        RefusalTrace::Audited,
    ))
}

/// 任务表写侧闸（S03 §3，）：只拦声明 `owns_task_table` 的工具。
///
/// 出口是**可审计拒绝但不记承诺**：工具没跑，「声明过什么」无从说起
/// （`note_tasks` 只在跑完后记成功调用）。
fn task_table(ctx: &GateContext<'_>) -> Option<Refusal> {
    let blocked = ctx.decls.owns_task_table(ctx.name) && !ctx.task_table.may_write;
    if !blocked {
        return None;
    }
    Some(Refusal::new(
        "任务表",
        format!(
            "主体 {} 不持 define.work / assert.verification，不得写任务表（todo_write）",
            ctx.task_table.principal
        ),
        failure_kind::PERMISSION_DENIED,
        RefusalTrace::AuditedNoDelegation,
    ))
}

/// 判据的唯一执行点：按 [`GATES`] 顺序问，先命中者胜。
pub(crate) fn first_refusal(ctx: &GateContext<'_>) -> Option<Refusal> {
    GATES
        .iter()
        .find_map(|row| (row.decide)(ctx).map(|refusal| (row, refusal)))
        .map(|(row, refusal)| {
            // 登记表与判据的漂移检查：一次拒止的失败语义必须在该行声明过。新增出口
            // （例如给某道闸加 `Recorded`）却忘了登记 `traces` 时，这里当场报——
            // 否则表会退化成「一列谁也不读的注释」，审计面重新变成口头约定。
            debug_assert!(
                row.traces.contains(&refusal.trace.kind()),
                "闸「{}」产出了未声明的失败语义 {:?}",
                row.name,
                refusal.trace.kind()
            );
            refusal
        })
}

/// 拒止骨架（**唯一实现**）：拒因 → 结果子节点 → 父节点收敛 → 记账 → 调用方 continue。
///
/// 四条既有出口的全部不变量都在这里兑现一次（不变量 1「每个调用以终态收场」、
/// 不变量 2「有调用必有结果」）；各闸的差异只经 [`RefusalTrace`] 表达，不再各写一遍。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn refuse(
    sink: &ExecEventSink,
    context_messages: &[ChatNode],
    id: &str,
    name: &str,
    args: &Value,
    result_msg_id: &str,
    refusal: Refusal,
    tool_messages: &mut Vec<ChatNode>,
    parent_updates: &mut Vec<ChatNode>,
    delegations: &mut Vec<Delegation>,
    decls: &ToolDecls,
    breaks: &mut Vec<&'static str>,
) {
    plugin_warn!(
        "session",
        "[Tool] {}：{id} / {name}——{reason}",
        refusal.gate,
        reason = refusal.reason
    );

    // `Break` 才入格：`Refuse` 按 S09 §6 验收 1 不得产生任何事件。
    if let Some(reason) = refusal.trace.break_reason() {
        if !breaks.contains(&reason) {
            breaks.push(reason);
        }
    }

    let tool_msg = llm_build_tool_message(
        id,
        &format!("{}: {}", refusal.prefix, refusal.reason),
        Some(false),
        Some(result_msg_id.to_string()),
        Some(name),
    );
    tool_messages.push(tool_msg);

    // 拒掉的调用同样必须收敛父节点：只推结果子节点，父节点就永远停在
    // 「运行中」（那条兜底 `finalize_assistant_turn` 已移除）。
    if let Some(parent_update) = emit_parent_finalized(
        sink,
        context_messages,
        id,
        NodeStatus::Completed,
        json!({ "success": false, "failure_kind": refusal.kind }),
        None,
    )
    .await
    {
        parent_updates.push(parent_update);
    }

    // 没跑 = 没履约（`agent_run` 的承诺记账见 `note_delegation`）；任务表闸
    // 明确不记——工具没跑，声明无从说起。
    if refusal.trace.notes_delegation() {
        let why = refusal.note_why.as_deref().unwrap_or(&refusal.reason);
        note_delegation(delegations, decls, id, name, args, false, why);
    }
}

#[cfg(test)]
#[path = "gates.test.rs"]
mod tests;
