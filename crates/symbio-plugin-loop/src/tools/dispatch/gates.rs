//! 拒止骨架与外部执行的**中性输入**。
//!
//! **没有批首读数**：每条债在自己的派发点上取读数（`batch::prepare_call`），环不预读
//! 整批的 `mode` / 声明位 / 主体 / 会话事实切片 / 同参台账。
//!
//! ## 开窗判据已搬进执行体
//!
//! 外部执行那道宿主闸不再在这里：授权矩阵与成本台账只产出**中性输入**
//! （`GateInputs`），时延 / 预算的判定与「熔断中不发」的事实由 `symbio-plugin-tool`
//! 的执行体自己落（DE-039 §6，DE-047 D-311）。环按端口名派发，不预判、不拒发。
//!
//! 淘汰与任务表写侧这两道**按能力判**的闸也已随 owner 迁出宿主：它们各自是
//! `ToolBehavior::before`（agent 的 `DelegationGate`、loop 的 `TaskTable`），由
//! `symbio_plugin_api::effects::apply_before` 按工具**自己声明的行为名**选实现。
//! 宿主只把这一笔债的结论交出去（主体 / 控制面撤销值 / 会话事实切片），
//! 并在 [`Refusal::from_before`] 里把实现的判定映射回拒止骨架。
//!
//! 本文件只剩**拒止骨架**：声明闸与 PreToolUse 钩子的出口都经 [`refuse`] 收口，
//! 拒因与是否记违约理由由实现自报。`breaks` 出参仍由环的收束段持有，
//! 但本文件已无写入方——熔断事实随执行体落，下一刀清掉这条空管线。

use super::*;
use symbio_plugin_api::effects::BeforeRefusal;

/// 本条债的**主体**：声明闸按它问（`AGENT_ID` 的会话族地址，未绑定即 `session/main`）。
///
/// 与 `request_principal` 同源：判断与落笔因此指向同一个主体，不会出现「按 A 判、
/// 以 B 记」的两套主体。
pub(crate) fn batch_principal(ctx: &Arc<dyn PluginInvokeRequest>) -> String {
    symbio_sdk::authz::principal_of(ctx.get(symbio_sdk::keys::AGENT_ID).as_deref())
}

// ── 拒止骨架──────────────────────────────────────────────────────

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
    /// 是否记违约承诺（任务表闸明确不记：工具没跑，声明无从说起）。
    pub(crate) notes_delegation: bool,
    /// 违约承诺的记账理由；`None` = 用拒因本身。钩子保留自己的原措辞。
    pub(crate) note_why: Option<String>,
}

impl Refusal {
    /// PreToolUse 钩子拦下的拒止：形如闸门拒止，但正文前缀是 `Blocked`，
    /// `failure_kind = "blocked"`，且违约理由保留钩子的原措辞。
    pub(crate) fn blocked(reason: String) -> Self {
        Self {
            note_why: Some(format!("被 PreToolUse 钩子拦下：{reason}")),
            gate: "PreToolUse",
            reason,
            prefix: "Blocked",
            kind: "blocked",
            notes_delegation: true,
        }
    }
}

/// 声明闸的判定 → 拒止骨架。实现的判定只说「拒 / 为什么 / 记不记承诺」，
/// 骨架要的 `prefix` / 钩子措辞都由宿主按自己的口径补。
///
/// 出口语义由实现自报（`notes_delegation`）：这一位不是宿主对某道闸的特判，而是
/// 实现对自己这次拒绝**知道多少**的交代——先说「约没见过」的工具，拒了也不该记
/// 一笔没履约的承诺（DE-025 D-219）。
impl Refusal {
    pub(crate) fn from_before(refusal: BeforeRefusal) -> Self {
        Self {
            gate: refusal.gate,
            reason: refusal.reason,
            prefix: "Refused",
            kind: refusal.kind,
            notes_delegation: refusal.notes_delegation,
            note_why: None,
        }
    }
}

/// 拒止骨架（**唯一实现**）：拒因 → 结果子节点 → 父节点收敛 → 记账 → 调用方 continue。
///
/// 不变量 1「每个调用以终态收场」与不变量 2「有调用必有结果」在这里兑现一次；
/// 各闸的差异只经 [`Refusal::notes_delegation`] 表达。
///
/// 开窗判据搬进执行体后，本文件不再产生入格理由；`_breaks` 是环收束段的既有出参，
/// 留到下一刀随空管线一起摘（本切点不动 `batch.rs` 的逐条执行段）。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn refuse(
    sink: &ExecEventSink,
    context_messages: &[ChatNode],
    id: &str,
    name: &str,
    args: &Value,
    refusal: Refusal,
    tool_messages: &mut Vec<ChatNode>,
    parent_updates: &mut Vec<ChatNode>,
    effects: &mut Vec<EffectInput>,
    decls: &ToolDecls,
    _breaks: &mut Vec<&'static str>,
) {
    plugin_warn!(
        "session",
        "[Tool] {}：{id} / {name}——{reason}",
        refusal.gate,
        reason = refusal.reason
    );

    let tool_msg = llm_build_tool_message(
        id,
        &format!("{}: {}", refusal.prefix, refusal.reason),
        Some(false),
        Some(symbio_kernel::event::result_node_id(id)),
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

    // 没跑 = 没履约（效果输入由 [`record_effect`] 交出，实现住能力包）；任务表闸
    // 明确不记——工具没跑，声明无从说起。
    if refusal.notes_delegation {
        let why = refusal.note_why.as_deref().unwrap_or(&refusal.reason);
        record_effect(effects, decls, id, name, args, false, "", why);
    }
}

#[cfg(test)]
#[path = "gates.test.rs"]
mod tests;
