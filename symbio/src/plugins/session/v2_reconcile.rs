//! **双源对账**（轨 S 第一步，[ADR-049](../../../../docs/decisions/session.md#adr-049-wal-是会话唯一事实源sessionstore-降为缓存轨-s-的起点)）。
//!
//! 会话消息今天有**两份东西在记同一件事**：`SessionStore` 的 `messages.json`（整份覆盖）
//! 与 `v2-events.wal`（append-only 的事实源）。它们今天内容收敛，但那是**巧合层面的
//! 正确**——两边写方不同、时机不同、失败路径不同，而**没有任何判据在对账它们**。
//! 八个静默缺口全部源于这一类分叉（ADR-048 补记）。
//!
//! 本模块就是那层对账的**第一步**：读路径同时问两源，不一致只记警告 + 落一条事实，
//! **返回值一字不改**。它不切流（那是 S2–S4 的事），因此零行为变更、随时可删。
//!
//! ## 为什么对账的是 `(role, 正文)` 而不是整个 `ChatMessage`
//!
//! 两源的**同一性**在「谁说了什么」这一层；`seq` / `parent_id` / `status` 是**呈现态**
//! （分发时机、节点树形状、在途标记），ADR-049 明确它们不入格、也不是对账对象。
//! 拿它们比会把「呈现形态不同」误报成「事实不一致」——而后者的正确处置是切流。
//!
//! ## 落的那条事实是什么形状
//!
//! `session.reconciled`（`Entity::System`，载荷 `{ mismatches, wal_missing, … }`）。
//! 落它而不是只打日志：**日志会滚，事实不会**。S5 的判据（「真实用户数据已迁」）
//! 要能回答「迁移期间有没有不一致、有多少」，靠的就是这条。
//!
//! ## 为什么落在 `System` 而不是新开 `Session` 实体
//!
//! `Entity` 是**闭集**（`event/mod.rs` 的枚举注释：加臂要 ADR 登记）。对账结果是
//! **运维诊断**——它说的不是「这个会话发生了什么」，而是「两个读源此刻是否一致」，
//! 属系统自身状态，与 `system.triggered` 同族。为一个诊断事件去开一个实体臂，
//! 是用架构变更换一行方便。

use std::path::Path;

use crate::symbio_core::{
    Budget, Entity, Event, EventWalStore, Seq, Store, Verb, EVENT_SESSION_RECONCILED,
};

/// 对账结果的**摘要**（调用方据此决定要不要出声）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Reconciliation {
    /// `messages.json` 侧的条数。
    pub store_msgs: usize,
    /// WAL 侧投影出的条数（同口径：有正文的格子）。
    pub wal_entries: usize,
    /// 两边**不一致**的条数（按下标逐条比 `(role, 正文)`）。
    pub mismatches: usize,
    /// WAL 不在（还没跑过任何一轮）——这不是错误，
    /// 是「这一份本来就是空的」，故与 `mismatches` 分开报。
    pub wal_missing: bool,
}

impl Reconciliation {
    /// 一致 = 条数相同且逐条相同。
    ///
    /// **WAL 不在时判「一致」**（`wal_missing` 优先）：没有第二源可比，就谈不上不一致。
    /// 把它算成不一致会让还没跑过轮的会话天天出声——诊断吵到没人再听，
    /// 正是"该报的不报、不该报的乱报"的后半句。「在不在」由 `wal_missing` 单独回答。
    pub fn is_consistent(&self) -> bool {
        self.wal_missing || (self.mismatches == 0 && self.store_msgs == self.wal_entries)
    }

    /// 日志用的一句话（人读）。
    pub fn summary(&self) -> String {
        if self.wal_missing {
            return format!("双源对账：WAL 不在（store {} 条）", self.store_msgs);
        }
        if self.is_consistent() {
            format!("双源对账：一致（{} 条）", self.store_msgs)
        } else {
            format!(
                "双源对账：不一致（store {} 条 / wal {} 条 / 逐条差 {} 处）",
                self.store_msgs, self.wal_entries, self.mismatches
            )
        }
    }
}

/// `ChatMessage` 的**对账投影**：`(role, 正文)`。
///
/// 只取这两样，理由见模块头。`role` 缺省按 `assistant` 计（落库镜像的历史形状），
/// 正文取 `content` 的文本形态，其余形态（工具调用等）归「无正文」——它们在对账眼里
/// 是同一条消息的**呈现细节**，不是另一句话。
fn project(msg: &crate::symbio_core::chat_message::ChatMessage) -> (String, String) {
    use crate::symbio_core::chat_message::{MessageContent, MessageRole};
    let role = match msg.role {
        Some(MessageRole::User) => "user",
        Some(MessageRole::Assistant) | None => "assistant",
        Some(MessageRole::Tool) => "tool",
        Some(MessageRole::System) => "system",
    };
    let text = match &msg.content {
        Some(MessageContent::Text(t)) => t.trim().to_string(),
        _ => String::new(),
    };
    (role.to_string(), text)
}

/// 对账**一个会话**的两个源。
///
/// `store_msgs` = `SessionStore` 那一侧的消息列表（调用方已有，不必再读一次盘）。
/// `session_dir` = 会话目录（WAL 就在它下面）。
///
/// 零 IO 副作用之外的读取：WAL 经 `open_readonly` 打开——不创建文件、不截尾。
pub fn reconcile(
    session_dir: &Path,
    store_msgs: &[crate::symbio_core::chat_message::ChatMessage],
) -> Reconciliation {
    let path = session_dir.join(super::paths::V2_WAL_FILE);
    // **在不在看文件，不看 open 的结果**：`EventWalStore::open_readonly` 对不存在
    // 的文件返回 `Ok(空 store)`（`wal.rs::replay` 的「文件不存在 ⇒ 空」）——那是
    // 读方不创建文件的正确语义，但会把「没有第二源」显示成「第二源是空的」。
    // 而这两件事对对账是**不同的**：前者不可比（不该出声音），后者可比且大概率不一致。
    if !path.exists() {
        return Reconciliation {
            store_msgs: store_msgs.len(),
            wal_entries: 0,
            mismatches: 0,
            wal_missing: true,
        };
    }
    let Ok(store) = EventWalStore::open_readonly(&path) else {
        // 存在但读不了（权限 / 撕裂到无法重放）⇒ 同样不可比，不是不一致
        return Reconciliation {
            store_msgs: store_msgs.len(),
            wal_entries: 0,
            mismatches: 0,
            wal_missing: true,
        };
    };
    let snapshot = store.range(Seq::new(0));
    let view = crate::symbio_core::transcript()
        .apply(&snapshot, i64::MAX, Budget::generous())
        .value;

    // 空正文 = 呈现态占位（turn 容器 / 流式半截），不是一句话 ⇒ 不参与对账。
    // e2e 探针实测：messages.json 每轮多一条空 assistant，WAL 侧无对应事件
    // （store 6 条 vs wal 4 条）。ADR-049：呈现态不入格、也不是对账对象。
    let store_side: Vec<(String, String)> = store_msgs
        .iter()
        .map(project)
        .filter(|(_, text)| !text.is_empty())
        .collect();
    let wal_side: Vec<(String, String)> = view
        .entries
        .iter()
        .map(|e| (e.role.clone(), e.text.trim().to_string()))
        .collect();

    // 逐条比：下标对齐（两边都按时间序）。长度不同时多出来的那几段**每一段都算一处
    // 不一致**——悄悄只比前 N 条会让「WAL 少一段」这种真实缺口只报 0。
    let mismatches = store_side
        .iter()
        .zip(wal_side.iter())
        .filter(|(a, b)| a != b)
        .count()
        + store_side.len().abs_diff(wal_side.len());

    Reconciliation {
        store_msgs: store_side.len(),
        wal_entries: wal_side.len(),
        mismatches,
        wal_missing: false,
    }
}

/// 对账结果落到**事实源**里的一条事件（`session.reconciled`）。
///
/// ## 为什么落事件而不只打日志
///
/// 日志会滚、会被级别滤掉、且**没有判据能读它**。S5 的回退语义要求「能证明迁移期间
/// 有没有不一致」——那需要可查询的事实。ADR-044「实测与判据同源」的同族纪律。
///
/// ## 幂等键
///
/// `event_id = r-{turn?}`：一条会话在同一时刻只该有一条待处理的对账记录。重复对账
/// （同一会话被读多次）撞 `Duplicate` ⇒ **静默跳过**（返回 `false`），不报错：
/// 对账是诊断，不是事实，重复记录它本身没有信息量。
pub fn record(session_dir: &Path, session_id: &str, r: &Reconciliation) -> bool {
    // 一致（含 WAL 不在那种"无从比对"的一致）⇒ 没什么可记的
    if r.is_consistent() {
        return false;
    }
    let event = Event::pending(
        format!("r-{session_id}"),
        EVENT_SESSION_RECONCILED,
        Entity::System,
        Verb::Asserted,
        0,
        "agent:main",
    )
    .with_payload(serde_json::json!({
        "session_id": session_id,
        "store_msgs": r.store_msgs,
        "wal_entries": r.wal_entries,
        "mismatches": r.mismatches,
    }));
    // 写侧打开（可能要创建文件）。创建不了（只读部署 / 磁盘满）⇒ 不落，也不因此
    // 让读路径失败：对账的失败形态必须是「少一条诊断」，不能是「会话读不出来」。
    match EventWalStore::open(session_dir.join(super::paths::V2_WAL_FILE)) {
        Ok(store) => store.append(event).is_ok(),
        Err(_) => false,
    }
}

#[cfg(test)]
#[path = "v2_reconcile.test.rs"]
mod tests;
