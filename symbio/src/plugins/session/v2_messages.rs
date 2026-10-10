//! 消息变更事实的写方（S2.7-b1，[ADR-053](../../../../docs/decisions/session.md)）。
//!
//! 「编辑一条消息」「截断一段消息」在轨 S 之前只重写 `messages.json`——事实源里
//! 一个字都没有。后果是**正确性**的：主读路径一旦切到「从 WAL 重建」，被截断的
//! 消息会**复活**、编辑过的内容会**回退**。本模块是这两件事**入格**的写方：
//! `commands::patch_message` / `commands::truncate_messages` 在重写缓存之后各调一次。
//!
//! ## 双写，不是改写（b1 的全部内容）
//!
//! b1 阶段缓存仍是读侧真源（主读切换在 b3），所以本模块**只追加一条事实**、
//! 不动任何既有行为：新落的事实此刻**无人消费**。换来的是零风险回退——去掉
//! 那两处调用就回到从前（ADR-053 迁移表的 b1 行：回退丢零）。
//!
//! ## 失败口径：`AppendError` 只记日志，不冒泡
//!
//! 事实写入失败（拿不到写者令牌 / 撞幂等键）**不得**让一次编辑失败：缓存已经
//! 写成功了，用户看到的就是「改好了」。这与 `v2_facts::record_learning` 同一条
//! 口径（附加事实失败不该把成功的动作说成失败）。
//!
//! 两点边界要写清楚，否则「不冒泡」会被读成「什么都不会炸」：
//!
//! - **存储层的 IO 灾难仍会 panic**——那是 [`Store::append`] 自己的既有契约
//!   （「不可静默」），不是本模块的口径；本模块只吞 [`AppendError`]。
//! - **b3 之后这里会变成真源**，那时失败就该升级为错误。这条不是待办，
//!   是 b3 批内的工序（ADR-053 迁移表的 b3 行）。
//!
//! ## 写者令牌：会话正跑着时编辑会落不了格
//!
//! [`EventWalStore::open`] 用文件锁取写者令牌（I1 单写者的跨进程边界）。一轮
//! 正在跑时它持有令牌，此刻来的编辑拿不到 ⇒ `NotTheWriter` ⇒ 少一条事实。
//! b1 阶段这**无害**（缓存兜着），但它是 b3 必须一起解决的一件事——所以记日志
//! 而不是静默跳过。

use std::path::Path;

use serde_json::Value;

use crate::symbio_core::chat_message::ChatMessage;
use crate::symbio_core::{
    Entity, Event, EventWalStore, Seq, Store, Verb, EVENT_MESSAGE_EDITED, EVENT_MESSAGE_REMOVED,
};

use super::paths::V2_WAL_FILE;

/// 载荷里可能记着**本条事件所承载的节点身份**的键。
///
/// 与 `rebuild` / `transcript` 取节点身份用的是同一批键（见
/// [`crate::symbio_core::event_payload_node_id`]）——这里是反向查：**哪个事件承载了
/// 节点 X**。键列在这里而不是内联进 `find`：加一格带新身份键的事件时，
/// 「哪些键是节点身份」必须只有一处答案。
const NODE_ID_KEYS: &[&str] = &["id", "text_id", "reasoning_id", "tool_call_id", "result_id"];

/// 编辑一条消息 ⇒ 落一条 `message.edited`。
///
/// `msg` 是**补丁应用后**的整条消息（不是补丁本身）——重建与转写都按 id 整条替换，
/// 拿到整条就不必再猜「这次改的是哪几个字段」（理由见 `EVENT_MESSAGE_EDITED`）。
pub(crate) fn edited(session_dir: &Path, msg: &ChatMessage) {
    let payload = match serde_json::to_value(msg) {
        Ok(v) => v,
        Err(e) => {
            crate::plugin_warn!("session", "[message] 编辑事实未入格：序列化失败：{e}");
            return;
        }
    };
    append(
        session_dir,
        EVENT_MESSAGE_EDITED,
        Verb::Asserted,
        "m-edit",
        &msg.id,
        serde_json::json!({ "message": payload }),
    );
}

/// 一段消息没了 ⇒ 落一条 `message.removed`（载荷 `{ ids: [...] }`）。
///
/// `ids` 是**被删节点 id 的完整列表**（与 `truncate_messages` 回执里那份同源）。
/// 空列表不落格：「什么都没删」不该在事实源上留下痕迹——与
/// `truncate_messages` 不发任何变更帧同一条判据。
pub(crate) fn removed(session_dir: &Path, ids: &[String]) {
    if ids.is_empty() {
        return;
    }
    append(
        session_dir,
        EVENT_MESSAGE_REMOVED,
        Verb::Closed,
        "m-del",
        // 溯源锚取**第一条被删的**：截断的语义是「从 X 起」——X 就是那个起点。
        ids.first().map(String::as_str).unwrap_or_default(),
        serde_json::json!({ "ids": ids }),
    );
}

/// 把一条变更事实追加进会话的事实源。
///
/// `target` = 本次变更作用在哪个节点上（用于解溯源锚）；`id_prefix` 是幂等键前缀。
fn append(
    session_dir: &Path,
    kind: &str,
    verb: Verb,
    id_prefix: &str,
    target: &str,
    payload: Value,
) {
    let wal = session_dir.join(V2_WAL_FILE);
    let store = match EventWalStore::open(&wal) {
        Ok(s) => s,
        Err(e) => {
            crate::plugin_warn!(
                "session",
                "[message] 变更事实未入格（{}）：事实源打不开：{e}",
                wal.display()
            );
            return;
        }
    };
    let snapshot = store.range(Seq::new(0));
    let Some(anchor) = provenance(&snapshot, target) else {
        // 切片为空（或切片内所有事件都没有 seq）⇒ 连「这次变更站在哪一份状态上」
        // 都无从谈起。会话还没有任何事实（v1 时代建的会话 / 从未走过 v2 轮）。
        // **不硬造一个号**：溯源是断言类事件的硬要求（I2），造不出就宁可不落——
        // 缓存此刻仍兜着，b3 之前读侧不受影响。
        crate::plugin_warn!(
            "session",
            "[message] 变更事实未入格（{}）：切片内没有可用的溯源锚（{kind} {target}）",
            wal.display()
        );
        return;
    };
    // 幂等键：本会话内 `message` 实体的第 n 条。**不是「一条消息一个号」**——
    // 同一条消息可以被编辑多次，每次都是一条新事实；两种变更共用同一个计数，
    // 于是 id 天然不撞。
    let n = snapshot
        .iter()
        .filter(|e| e.entity == Entity::Message)
        .count();
    let event = Event::pending(
        format!("{id_prefix}-{n}"),
        kind,
        Entity::Message,
        verb,
        // `turn = 0`：变更不属于任何一轮（截断跨轮）。与 `memory.recalled` /
        // `commitment.*` / `session.reconciled` 同一条既有约定。
        0,
        // 变更由**用户对列表的操作**发起（VDFS 边界由前端 / CLI 代表用户）——
        // 与 `user.message` 的 actor 同源（`plugin::attributed` 用的是同一个常量）。
        crate::symbio_core::authz::PRINCIPAL_USER,
    )
    .with_produced_by(anchor)
    .with_payload(payload);
    if let Err(e) = store.append(event) {
        crate::plugin_warn!(
            "session",
            "[message] 变更事实未入格（{}）：{e:?}（缓存已写成功，本次编辑仍然生效）",
            wal.display()
        );
    }
}

/// 溯源锚：**被改的那条事实**（承载该节点的事件 seq）。
///
/// 找不到（节点不在事实源里：v1 时代的历史消息、或按事件 id 合成的节点）⇒
/// 退化为**切片末尾的 seq**——它仍然回答一个确切的问题：「这次变更站在哪一份
/// 状态上」。退化的后果是「这次编辑源自哪条发言」变粗，而不是「没有溯源」。
///
/// 承载事件**没有 seq**（2026-09-30 那次修复之前落下的旧 WAL 行：`seq: null`）
/// 走同一条退化——它同样回答不了「源自哪条」，但不该因此把整条事实丢掉。只有
/// **切片整个为空**才返回 `None`（那时连「站在哪份状态上」都无从谈起）。
fn provenance(snapshot: &[Event], node: &str) -> Option<u64> {
    let carrier = snapshot.iter().find(|e| {
        NODE_ID_KEYS
            .iter()
            .any(|k| e.payload.get(*k).and_then(Value::as_str) == Some(node))
    });
    carrier
        .and_then(|e| e.seq)
        .or_else(|| snapshot.last().and_then(|e| e.seq))
        .map(Seq::value)
}

#[cfg(test)]
#[path = "v2_messages.test.rs"]
mod tests;
