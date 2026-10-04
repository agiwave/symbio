//! v2 事实桥——把 v1 会话轮次**转写**进事件网格（ADR-044：实测与判据同源）。
//!
//! 定位：chat_loop 切到 v2 链路之前的过渡件。v1 管线行为**零变化**，只是在
//! 每轮收束时把「用户发言 / 助手答复 / 实测耗时」转写为 v2 事件，落进
//! per-session 的持久事实源（`<会话目录>/v2-events.wal`）——P99 / 兜底率
//! 等口径从此有真实流量可扫。
//!
//! 为什么这不是「旁路遥测」：ADR-044 反对的是**不进事件网格的计数器**；
//! 本桥是把 v1 流量的既有事实**转写为网格事件**（同一事实落入事实源），
//! 与 v1 侧的对话存储是两份持久化、一份语义。
//!
//! 口径：
//! - 档位恒 `deep`——v1 chat_loop 的每一轮都装配完整模型 + 工具；
//! - `turn` 号 = WAL 内既有 user.message 计数（0 起）；
//! - 重试（resume 重跑同一用户消息）在 v2 网格里是**新的轮次**（attempt
//!   递增）——失败与成功都是真实发生的收束，各自成格，N3 仍由构造成立；
//! - 中止（Aborted）不转写：轮未收束，网格少一格是诚实的缺口，不是假象；
//! - 临时会话（不落盘）不转写：事实源本就不持久，转写无从安放；
//! - 总开关：`SessionConfig::v2_mode`（`off` / `bridge`，默认 `bridge`）——
//!   `off` 档不转写（用户关的是数据源，不是对话；v1 行为照旧）。

use std::path::PathBuf;

use crate::symbio_core::{
    Entity, Event, EventWalStore, PermissionMatrix, Seq, Store, Verb, EVENT_ASSISTANT_FALLBACK,
    EVENT_ASSISTANT_FINAL, EVENT_USER_MESSAGE,
};

use super::chat_session::PersistentChatSession;
use crate::symbio_core::chat_message as cm;

/// 一轮的收束形态（转写的第二只脚；第一只脚是用户发言）。
pub(crate) enum V2Closure {
    /// 模型作答完成。
    Final { text: String, cost_ms: u64 },
    /// 兜底（I3：失败也是一句话——`why` 是用户实际看到的东西）。
    Fallback { why: String, cost_ms: u64 },
}

/// 把一轮事实转写进该会话的 v2 WAL。
///
/// 所有失败都只记日志不冒泡：桥的故障不得拖垮 v1 对话——但**必须被看见**
/// （plugin_warn），不允许静默吞。
///
/// `recalled` = 本轮开头召回的长期记忆视图（S5 步 12），非空且有条目时收束入格一条
/// `memory.recalled`。它从 [`super::chat_loop::state::TurnState`] 一路带到这里才落笔：
/// 溯源锚是本轮 `user.message` 格，只有轮末它才在事实源里。
pub(crate) fn record(
    session: &PersistentChatSession,
    user_id: &str,
    user_text: &str,
    closure: V2Closure,
    recalled: Option<&crate::symbio_core::RecallView>,
) {
    // 总开关（`v2_mode`，ADR-045 过渡期的切换档位）：`off` 档网格零增长
    // （用户关的是数据源，不是对话）。`bridge` 档：v1 轮次全部经此转写；
    // `full` 档：轮次由 v2 运行器原生记账，调用侧以 `TurnState::v2_executed`
    // 拦下，不经此转写——**记忆写方同此档位**（full 档的记忆写随 full 档启用，
    // 见 `v2_exec` 侧的同批注记）。检查在取目录之前：关掉时连 WAL 的打开开销都不该有。
    if matches!(session.v2_mode(), super::config::V2Mode::Off) {
        return;
    }
    let Some(dir) = session.session_dir() else {
        return; // 临时会话：事实源不持久，转写无从安放（见模块文档口径）
    };
    let result = record_to_wal(
        dir.join(super::paths::V2_WAL_FILE),
        user_id,
        user_text,
        closure,
        recalled,
    );
    if let Err(why) = result {
        crate::plugin_warn!(
            "session",
            "[v2-bridge] 转写失败（{}）：{}",
            dir.display(),
            why
        );
    }
}

fn record_to_wal(
    wal: PathBuf,
    user_id: &str,
    user_text: &str,
    closure: V2Closure,
    recalled: Option<&crate::symbio_core::RecallView>,
) -> Result<(), String> {
    let store = EventWalStore::open(&wal).map_err(|e| format!("打开 WAL 失败：{e}"))?;
    let snapshot = store.range(Seq::new(0));

    // turn 号 = 既有 user.message 计数；attempt = 同一 v1 用户消息的转写次数
    //（重试各成一格，失败与成功都是真实发生的收束）。
    let mut turn = 0u64;
    let mut attempt = 0u64;
    for e in &snapshot {
        if e.kind == EVENT_USER_MESSAGE && e.entity == Entity::Turn {
            turn += 1;
            if e.payload.get("turn_ref").and_then(|v| v.as_str()) == Some(user_id) {
                attempt += 1;
            }
        }
    }

    let user = Event::pending(
        format!("v2u-{user_id}-a{attempt}"),
        EVENT_USER_MESSAGE,
        Entity::Turn,
        Verb::Opened,
        turn,
        crate::authz::PRINCIPAL_USER,
    )
    .with_payload(serde_json::json!({ "text": user_text, "tier": "deep", "turn_ref": user_id }));
    let user_seq = store
        .append(user)
        .map(|s| s.value())
        .map_err(|e| format!("用户消息入格失败：{e:?}"))?;

    let (id, kind, payload, cost_ms) = match closure {
        V2Closure::Final { text, cost_ms } => (
            format!("v2f-{user_id}-a{attempt}"),
            EVENT_ASSISTANT_FINAL,
            serde_json::json!({ "text": text, "model": "v1-chat_loop" }),
            cost_ms,
        ),
        V2Closure::Fallback { why, cost_ms } => (
            format!("v2fb-{user_id}-a{attempt}"),
            EVENT_ASSISTANT_FALLBACK,
            serde_json::json!({ "why": why }),
            cost_ms,
        ),
    };

    // 01 §7 写侧闸（[04 §3.1 批⑥](../../../docs/plan/04-工程落地.md)）：收束格是
    // 主智能体的**权威发言**，入格前判它是否持有对应能力——该轮首条 →
    // `reply.first`，后续（同轮追加 / 重试）→ `reply.append`。映射口径在 core 的
    // `can_reply`，此处不复述；表由 `crate::authz` 提供，全机只有那一张。
    // 拒绝 = **不入格**：用户发言已入格、本轮留在未收束态，`check_all` 会把它
    // 报进第五列（不变量）——拒绝可被看见，且 v1 对话行为不变（由 `record` 吞下）。
    let prior_closures = snapshot
        .iter()
        .filter(|e| e.entity == Entity::Turn && e.verb == Verb::Closed && e.turn == turn)
        .count() as u64;
    authorize_close(
        crate::authz::production_matrix(),
        crate::authz::PRINCIPAL_MAIN,
        prior_closures == 0,
    )?;

    store
        .append(
            Event::pending(
                id,
                kind,
                Entity::Turn,
                Verb::Closed,
                turn,
                crate::authz::PRINCIPAL_MAIN,
            )
            .with_produced_by(user_seq)
            .with_cost_ms(cost_ms)
            .with_payload(payload),
        )
        .map_err(|e| format!("收束事件入格失败：{e:?}"))?;

    // ── 记忆三段（S5 步 11–13，04 §3.1 批⑦）：本轮收束时写记忆 ────────────
    // 顺序有讲究：
    //  1. 编码（步 11）先写——本轮新记忆要能进本轮的巩固；
    //  2. 检索事实（步 12）与它平级：溯源锚是刚入格的 `user.message`（`user_seq`）；
    //  3. 巩固（步 13）最后，且用**刷新后的**快照——它要看得见 1 刚写下的那条。
    // 三条都**只记日志不冒泡**：记忆是本轮的附加事实，它失败不该被说成「转写失败」
    // （那会把桥的健康度算错）。`now` 是统一的墙钟时刻——记忆类事件的 `ts` 是
    // `RecallEntry::ts` 契约里的「编码时刻」，跨会话新近度排序全靠它。
    let now = crate::symbio_core::clock_now_ms();
    let remember = |step: &str, why: String| {
        crate::plugin_warn!(
            "session",
            "[memory] {step} 失败（{}）：{why}",
            wal.display()
        );
    };
    if let Err(why) = super::v2_memory::encode(
        &store,
        &snapshot,
        turn,
        user_seq,
        user_text,
        &format!("v2m-{user_id}-a{attempt}"),
        now,
    ) {
        remember("步 11 编码", why);
    }
    if let Some(view) = recalled {
        if let Err(why) = super::v2_memory::record_recalled(&store, view, user_seq, now) {
            remember("步 12 检索入格", why);
        }
    }
    if let Err(why) = super::v2_memory::consolidate(&store, &store.range(Seq::new(0)), turn, now) {
        remember("步 13 巩固", why);
    }
    Ok(())
}

/// 收束入格前的写侧授权闸（[plan/01 §7](../../../docs/plan/01-核心架构.md) 写侧）。
///
/// `first` = 该轮尚无收束格。拒绝的语义是**不入格**，不是「照写但记一笔」——
/// 记一笔会把授权判定降级成一个可以忽略的日志项。
fn authorize_close(matrix: &PermissionMatrix, principal: &str, first: bool) -> Result<(), String> {
    if matrix.can_reply(principal, first) {
        return Ok(());
    }
    Err(format!(
        "收束被授权拒绝（{} 缺 {}）",
        principal,
        if first { "reply.first" } else { "reply.append" }
    ))
}

/// 本轮用户发言的**兜底取法**（消息 id + 文本）：历史里**第一条已提交**的用户 Text 节点。
///
/// 正路是 `chat_loop::state::TurnState::input_utterance`——循环前在 `single_message`
/// 上锚定的那一份（判决与转写共用同一份锚）。本函数只服务**没有锚**的请求
/// （`resume` 重跑等，那时「本轮」无从界定），退而取历史首条。**有锚时不得用它**：
/// `context.messages` 在 `load_history = true` 下装着整段历史，取「第一条」会逐轮
/// 指回首轮那句——转写的 `user.message` 文本 / `attempt` 判据与记忆编码会一起记错。
///
/// 判别式 = `role` + `msg_type` 两条**构造即成立**的字段，再加 `status` 只用来
/// 排除**显式在途**：`None` 与 `Completed` 都算已提交，`Streaming` / `Pending` /
/// `WaitingUserAction` 不算。
///
/// 为什么 `None` 算数：用户消息不是流式产物，而它的写入方有好几个——`chat/send`
/// 的调用方（CLI / 前端）、收件箱入口（`plugin/nodes.rs::parse_inbox_message`）、
/// 心跳——各写各的形状，`chat/send` 那一支就**不填** `status`
/// （`ChatMessage::default()` → `None`）。把「填了且 = `Completed`」当判据，
/// 转写就在不填的那一支上**静默不发生**：不报错、连 `plugin_warn!` 都没有，
/// 事实源默默少一格，P99 与兜底率默默缺一轮——比转写失败更难发现。
///
/// [`last_assistant_text`] 要求 `status == Completed`：助手消息**是**流式的，
/// 未收束的那条不该被当成终稿。两条判据因此不对称，这不是笔误。
pub(crate) fn first_user_utterance(messages: &[cm::ChatMessage]) -> Option<(String, String)> {
    messages
        .iter()
        .find(|m| {
            m.role == Some(cm::MessageRole::User)
                && m.msg_type == Some(cm::MessageType::Text)
                && matches!(m.status, None | Some(cm::MessageStatus::Completed))
        })
        .and_then(|m| {
            let text = m.content.as_ref()?.to_text();
            Some((m.id.clone(), text))
        })
}

/// 本轮助手答复文本：**最后一条**已完成的助手 Text 节点。
pub(crate) fn last_assistant_text(messages: &[cm::ChatMessage]) -> Option<String> {
    messages
        .iter()
        .rev()
        .find(|m| {
            m.role == Some(cm::MessageRole::Assistant)
                && m.msg_type == Some(cm::MessageType::Text)
                && m.status == Some(cm::MessageStatus::Completed)
        })
        .and_then(|m| m.content.as_ref().map(|c| c.to_text()))
}

#[cfg(test)]
#[path = "v2_bridge.test.rs"]
mod tests;
