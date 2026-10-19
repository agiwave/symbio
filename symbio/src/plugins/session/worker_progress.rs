//! **Q3（worker 进展）的唯一归属**：类型、两种读法与逐子会话编排都住在本模块。
//!
//! 此前投影类型与旧读法住在 `chat_loop::delegate`，而本模块的 WAL 读法又要回头拿
//! `delegate::WorkerProgress`——一条 `worker_progress → chat_loop` 的反向依赖，
//! 切谁都是环。[ADR-081](../../../../docs/010-adr/081.md "ADR-081") 裁决一把方向拧正：
//! **进展是「读存储算出来的数」，归本域**；`chat_loop::delegate` 只留线格式
//! （`worker_line`）与三段合成，驱动器（`chat_loop::inputs`）只调 [`for_session`]。
//!
//! ## 三个数与事件格的对应（**逐条可指认**，不是"差不多"）
//!
//! | 字段 | 旧读法（`messages.json`） | 新读法（WAL + `transcript` / `turnstate` 投影） |
//! |---|---|---|
//! | `rounds` | `role = user` 的消息条数 | `user.message` 事件条数（同一投影口径） |
//! | `steps` | 非 user 节点的非空正文 | `transcript` 投影里 role≠user 且正文非空的条目 |
//! | `in_flight` | 末条消息未见终态 | `turnstate::settled()` 为假（末轮已开未收束） |
//!
//! **`in_flight` 为什么能对上**：旧读法的"终态"是消息的 `status`，新读法是
//! 「末轮有没有收束事件」。两者语义同源——`chat.assistant.final` / `fallback`
//! 正是落库那条 Completed 消息的**事实来源**（同一个动作既落格又落库）。
//!
//! ## 旧读法的去向（T-053）
//!
//! [`progress_of`]（读落库消息的那一份）**没有退役**：它仍是「没有 WAL 的会话」的
//! 回落——v1 遗留会话的事实从未入格，它们的进展只在落库消息里。退役的是**两读法
//! 并排跑的对账段**：切源之后落库消息即 WAL 重建的那一份，两个源不再独立，对账没有
//! 对象。

use std::path::Path;

use crate::symbio_core::chat_node as cm;
use crate::symbio_core::PluginError;

// core 的域模块私有（C-002：外部只写 symbio_core::<符号>）⇒ 视图类型经根导出拿
use crate::symbio_core::{transcript, turnstate, Budget, EventWalStore, Seq, Store};

use super::chat_session::PersistentChatSession;
use super::types::{Session, SessionSummary};

// ── 类型与投影上限 ────────────────────────────────────────────────────

/// 单步文本上限（字符）。进展是**一眼看完的进度**，不是转写重放：把 worker 的
/// 整段回复塞进每轮请求，等于让模型背着别人的全部历史跑。
const STEP_MAX_CHARS: usize = 60;
/// 步骤条数上限（**取最近的**）。进展关心"到哪了"，不是"都干过什么"。
pub(crate) const STEPS_MAX: usize = 8;

/// Q3 · 单个 worker 的进展投影。
///
/// 三个字段与 B2（前端把「过程段」收起成的那行状态）**逐字同名**：`in_flight` /
/// `rounds` / `steps`。同名约束的是**字段名与语义**，不是算法——后端只见磁盘，
/// 前端还看得见父工具状态（B2 判据 1 的 OR 式），两端各自用可见信息判定同一语义。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct WorkerProgress {
    /// 子会话 id（`metadata.parent_session_id` 归属本会话）。
    pub(crate) id: String,
    /// 标题；存量文件缺投影字段时为空，装配时以 [`Self::id`] 兜底。
    pub(crate) title: String,
    /// 该 worker 是否仍在运行（末条消息**未见终态**即为真）。
    pub(crate) in_flight: bool,
    /// Turn 数 = 其 user 消息数 = **前端过程段 Turn 条数**——B2 判据 2「数值同源」
    /// 的落点。**不能拿 `message_count` 顶**：那是总消息数，与 Turn 数不同量。
    pub(crate) rounds: usize,
    /// 非空内容节点产出的步骤；**无内容节点不产出步骤**（B2 判据 3）。
    pub(crate) steps: Vec<String>,
}

/// Q3 投影（旧读法）：由子会话清单 + 完整转写算进展。**纯函数**（形参只有两个 `&`，可穷举测试）。
pub(crate) fn progress_of(summary: &SessionSummary, session: &Session) -> WorkerProgress {
    // rounds：只数 user 消息。`role` 为 `None` 是「还没接线」，不冒充一轮——
    // 与委派判定的 `None` 同一条纪律：没有观测就不要编出一个数来。
    let rounds = session
        .messages
        .iter()
        .filter(|m| matches!(m.node_role(), Some(cm::NodeRole::User)))
        .count();

    // steps：非 user 节点的非空正文，**保留最近 STEPS_MAX 条**（drain 掉最早的）。
    let mut steps: Vec<String> = session
        .messages
        .iter()
        .filter(|m| !matches!(m.node_role(), Some(cm::NodeRole::User)))
        .filter_map(step_text)
        .collect();
    if steps.len() > STEPS_MAX {
        steps.drain(..steps.len() - STEPS_MAX);
    }

    // in_flight：末条消息未见终态 ⇒ 仍在运行。**`status` 缺失也算在途**——
    // 没观测到结束就不能宣称结束了（谎报"已完成"会让模型抢跑，比多报在途糟）。
    let in_flight = session
        .messages
        .last()
        .map(|m| !is_terminal(m.status.as_ref()))
        .unwrap_or(false);

    WorkerProgress {
        id: summary.id.clone(),
        title: summary.title.clone(),
        in_flight,
        rounds,
        steps,
    }
}

/// 一条步骤 = 非空内容节点的正文（截断到 [`STEP_MAX_CHARS`]）。
///
/// 判据 3「无内容节点不产出步骤」在此自然成立：组合节点（`Turn` / `ToolCall`）
/// 本就不带 content，`as_ref()?` 直接返回 `None`；空字符串也被 `is_empty` 挡下。
fn step_text(m: &cm::ChatNode) -> Option<String> {
    let content = m.content.as_ref()?;
    if content.is_empty() {
        return None;
    }
    let text = content.to_text();
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    Some(if text.chars().count() > STEP_MAX_CHARS {
        let cut: String = text.chars().take(STEP_MAX_CHARS).collect();
        format!("{cut}…")
    } else {
        text.to_string()
    })
}

/// 终态集合（`NodeStatus` 的后四个变体）。
pub(crate) fn is_terminal(status: Option<&cm::NodeStatus>) -> bool {
    matches!(
        status,
        Some(cm::NodeStatus::Completed)
            | Some(cm::NodeStatus::Aborted)
            | Some(cm::NodeStatus::Failed)
            | Some(cm::NodeStatus::Removed)
    )
}

// ── 逐子会话编排（存储侧入口）──────────────────────────────────────────

/// Q3 · 当前会话的 worker 进展：枚举子会话 → 逐个取转写 → 归属复核 → 投影。
///
/// **无子会话 ⇒ 零额外 IO**：`list_sub_sessions` 对不存在的 `sessions/` 直接返回空
/// （`nested.exists()` 短路），[`PersistentChatSession::load_sub_session`] 一次都不调。
/// 主会话没开 worker 时，本函数只多一次目录存在性判定。
///
/// [ADR-081](../../../../docs/010-adr/081.md "ADR-081") 裁决一第 4 条「不自拼存储侧
/// 多步编排」的落点：这条链（枚举 + 加载 + 复核 + 两种读法）整体是本域的**一个入口**，
/// 驱动器（`chat_loop::inputs`）只在收集段调用一次，不持有其中任何中间步骤。
pub(crate) async fn for_session(
    session: &PersistentChatSession,
) -> Result<Vec<WorkerProgress>, PluginError> {
    let mut out = Vec::new();
    for summary in session.list_sub_sessions().await? {
        let child = session.load_sub_session(&summary.id).await?;
        // 归属复核：枚举路径已保证是本父之子，这里按 metadata 再判一次——
        // 「串台比没进展更糟」，宁可少一个 worker，也不能把别人的进展注进来。
        if child.parent_session_id() != Some(session.session_id()) {
            continue;
        }
        // 子会话目录 = 父会话目录下的 <safe_id(子 id)>（`paths::session_dir` 的
        // 同一套寻址，与 stats / turn_exec 同一个取值点）。父会话本身没有目录
        // （临时会话）⇒ 回落旧读法：那种会话没有 WAL，进展只在落库消息里。
        let Some(parent_dir) = session.session_dir() else {
            out.push(progress_of(&summary, &child));
            continue;
        };
        let dir = super::paths::session_dir(&parent_dir, &summary.id);
        // 新读法（WAL）；**没有 WAL ⇒ 回落旧读法**（v1 遗留会话的事实从未入格）。
        //
        // T-053 退掉了这里的**双源并读对账**：切源之后落库消息就是 WAL 重建的
        // 那一份（`store::load_messages` 已 WAL 优先），再拿它跟 WAL 投影比是
        // 「真源跟真源比」——口径差异会变成稳定误报。对账对象已随单源消失。
        let Some(wal_read) = progress_from_wal(&dir, &summary.id, &summary.title) else {
            out.push(progress_of(&summary, &child));
            continue;
        };
        out.push(wal_read);
    }
    Ok(out)
}

// ── WAL 读法（轨 S 第二步，ADR-049：读侧逐点切到事实源）────────────────

/// 从 WAL 算出 Q3 进展（**纯函数**：只吃一个会话目录，可穷举测试）。
///
/// `None` = 这个会话还没有 WAL（没跑过任何一轮 v2）——调用方回落到旧读法，
/// 与 `task_events::prompt_section` 同一条纪律（无源 ⇒ 不产出，不造空壳）。
pub fn progress_from_wal(
    session_dir: &Path,
    fallback_id: &str,
    fallback_title: &str,
) -> Option<WorkerProgress> {
    let path = session_dir.join(super::paths::V2_WAL_FILE);
    if !path.exists() {
        return None;
    }
    let store = EventWalStore::open_readonly(&path).ok()?;
    let snapshot = store.range(Seq::new(0));
    // 不标注类型：视图类型随返回值流动（core 的 projection 域模块私有，C-002）
    let entries = transcript()
        .apply(&snapshot, i64::MAX, Budget::generous())
        .value;

    // rounds：`user.message` 条数。`transcript` 投影把开轮折成的补充（
    // `turn.supplemented`）也渲染成 role=user——它**是用户说的话**，与
    // `messages.json` 里那条 supplement 消息同源，故两种读法在此仍同数。
    let rounds = entries.entries.iter().filter(|e| e.role == "user").count();

    // steps：非 user 条目里正文非空的那些，**保留最近 STEPS_MAX 条**
    // （与旧读法同一条截断纪律，否则两读法的 steps 条数会对不上）。
    let mut steps: Vec<String> = entries
        .entries
        .iter()
        .filter(|e| e.role != "user")
        .filter(|e| !e.text.trim().is_empty())
        .map(|e| e.text.trim().to_string())
        .collect();
    if steps.len() > STEPS_MAX {
        steps.drain(..steps.len() - STEPS_MAX);
    }

    // in_flight：末轮已开未收束 = `turnstate::settled()` 为假。
    // 还没有开轮 ⇒ 不在途（与旧读法"空消息列表 ⇒ false"同值）。
    //
    // 读**现行窗口**（最近一份快照之后）而不是完整史：`turnstate` 是朴素扫描，
    // 不知道「被快照取代」这回事——把归档里的陈轮算进来，一个早已退出上下文的
    // 未收束轮会把 `in_flight` 从 false 翻成 true（`Store::range` 归一后的必动项，
    // 见 `store::current_window`）。上面 `transcript()` 那一头自己从快照起折，不受影响。
    let state = turnstate()
        .apply(
            super::store::current_window(&snapshot),
            i64::MAX,
            Budget::generous(),
        )
        .value;
    let in_flight = state.opened && !state.settled();

    Some(WorkerProgress {
        id: fallback_id.to_string(),
        title: fallback_title.to_string(),
        in_flight,
        rounds,
        steps,
    })
}

#[cfg(test)]
#[path = "worker_progress.test.rs"]
mod tests;
