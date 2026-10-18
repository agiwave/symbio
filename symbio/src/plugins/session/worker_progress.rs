//! **Q3 进展的 WAL 读法**（轨 S 第二步，[ADR-049]：读侧逐点切到事实源）。
//!
//! `chat_loop::delegate::worker_progress` 此前只读 `SessionStore`：枚举子会话 →
//! `load_sub_session` → 由**落库消息**算 `rounds` / `steps` / `in_flight`。轨 S 要
//! 把这一点切到 WAL，本模块就是新读法。
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
//! [`super::chat_loop::delegate::progress_of`]（读落库消息的那一份）**没有退役**：
//! 它仍是「没有 WAL 的会话」的回落——v1 遗留会话的事实从未入格，它们的进展只在
//! 落库消息里。退役的是**两读法并排跑的对账段**：切源之后落库消息即 WAL 重建的
//! 那一份，两个源不再独立，对账没有对象。

use std::path::Path;

// core 的域模块私有（C-002：外部只写 symbio_core::<符号>）⇒ 视图类型经根导出拿
use crate::symbio_core::{transcript, turnstate, Budget, EventWalStore, Seq, Store};

/// 从 WAL 算出 Q3 进展（**纯函数**：只吃一个会话目录，可穷举测试）。
///
/// `None` = 这个会话还没有 WAL（没跑过任何一轮 v2）——调用方回落到旧读法，
/// 与 `task_events::prompt_section` 同一条纪律（无源 ⇒ 不产出，不造空壳）。
pub fn progress_from_wal(
    session_dir: &Path,
    fallback_id: &str,
    fallback_title: &str,
) -> Option<super::chat_loop::delegate::WorkerProgress> {
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
    if steps.len() > super::chat_loop::delegate::STEPS_MAX {
        steps.drain(..steps.len() - super::chat_loop::delegate::STEPS_MAX);
    }

    // in_flight：末轮已开未收束 = `turnstate::settled()` 为假。
    // 还没有开轮 ⇒ 不在途（与旧读法"空消息列表 ⇒ false"同值）。
    let state = turnstate()
        .apply(&snapshot, i64::MAX, Budget::generous())
        .value;
    let in_flight = state.opened && !state.settled();

    Some(super::chat_loop::delegate::WorkerProgress {
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
