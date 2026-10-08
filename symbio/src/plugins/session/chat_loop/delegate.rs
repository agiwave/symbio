//! 委派者三项真源（[ADR-047](../../../../../docs/decisions/session.md)）——**数据，不是机制**。
//!
//! 目标形态是**主会话不持有工具**，于是它缺三样外部事实：worker 何时启动、自己有哪些
//! 能力、进展如何。三问的答案都必须是**可穷举测试的数据**，否则主会话就得重新持有
//! 工具——那正是「不持有工具」这个目标形态的反面。本模块是**三样**的生产处，
//! owner 是 [`prepare_turn_inputs`](super::inputs::prepare_turn_inputs)：
//! **能力收集之后**才动笔，产出的段落并进 `build_request_view` 既有的请求视图次序。
//!
//! ## 为什么不做成插件 / 注册表 / 跨插件调用
//!
//! ADR-047 的后果条把边界写死了：三者都是数据——**不新增存储、不新增注册表、
//! 不新增跨插件调用**。所以 Q1 的判定不在 `classify` 插件里（那要路由过去，等于给
//! 主会话添一次机制），Q3 的进展不记进程内账（重启后与磁盘对不上 = 第二份真相）。
//!
//! ## 能力目录与工具声明为什么不是第二份真相
//!
//! 主会话今天仍随请求收到 `tools`（含 schema）；本段给的是**按分类分组的名字目录**——
//! 只有名字、没有 schema、没有参数。两者不同物：一个是「我有哪些能力」的一页索引，
//! 一个是「怎么调」的契约。索引复制的是**名字**，而名字的真源是
//! `CapabilityVisitor::list_capability()`（同一轮同一次收集）⇒ 同源，不构成第二份。
//!
//! ## 三条判据可配：读 `SessionConfig`，出厂值真源在配置面
//!
//! Q1 的三条判据（前缀 / 关键词 / 阈值）取 `SessionConfig` 的 `worker_force_prefix`
//! / `worker_keywords` / `worker_min_chars`；出厂值的**单一真源在配置面**
//! （`mod chat_loop` / `mod delegate` 皆私有，判定侧能 import 配置面、反过来不行），
//! 本模块**不持第二份字面量**，出厂参数由判定侧的测试显式 import 同一批常量。
//! 字段表同步 [CONFIGURATION](../../../../../docs/reference/CONFIGURATION.md)。
//!
//! ## Q3 现读磁盘，不记进程内账
//!
//! - **Q3（worker 进展）也在本模块**，且**同样不记进程内账**——每次现读磁盘，
//!   重启后与磁盘天然对得上。读侧走 [`PersistentChatSession::list_sub_sessions`]
//!   （枚举 `<父>/sessions/`，**按路径天然只含本父之子**）+
//!   [`PersistentChatSession::load_sub_session`]（取 `messages.json`），
//!   投影是纯函数 [`progress_of`]。三处口径见
//!   [04 §3.2](../../../../../docs/plan/04-工程落地.md)。

use crate::symbio_core::chat_message as cm;
use crate::symbio_core::{CapabilityMeta, PluginError};
use std::collections::BTreeMap;

use super::super::chat_session::PersistentChatSession;
use super::super::config::SessionConfig;
use super::super::types::{Session, SessionSummary};

// ── Q1：worker 启动条件 ────────────────────────────────────────────────

/// Q1 理由码。与 `schemas::dialog` 的 `REASON_*` 同一条约定：理由码是**英文短词**，
/// 可观测、可穷举、可被测试逐个钉住——没有理由码的判定不可审计（ADR-047 不变量）。
///
/// 出厂值（前缀 / 关键词 / 阈值）的真源在**配置面** `session::config`，本模块
/// **不持有第二份字面量**；判定侧与它的测试各自显式 import 那三个常量。
pub(crate) const REASON_PREFIX: &str = "explicit_prefix";
pub(crate) const REASON_KEYWORD: &str = "keyword_hit";
pub(crate) const REASON_LENGTH: &str = "topic_length";

/// Q1 · worker 启动条件：三层判据**按序短路**，命中回理由码、未命中回 `None`。
///
/// 形态照搬 `classify::rules::classify_by_rule` 的那条纪律：纯函数、无 I/O、
/// 无状态、无 async——全部行为可被测试穷举。
///
/// ## 「合取」是「三层合成一个函数」，判定语义是**先命中先赢**
///
/// 判据表按序给：① 显式前缀 → ② 关键词 → ③ 主题长度。这不是逻辑 AND——关键词
/// 出厂是空集，取 AND 判定恒为否，功能等于没接；原文（`feat` 分支
/// `delegate/decide.rs` 的判据表）也是「按序，先命中先赢」。
///
/// ## 两处平凡值：空集 / 阈值 0 都是「这一层关闭」，不是「永远命中」
///
/// 若 `min_chars == 0` 也判长度，任何非空消息都命中 ⇒ **每条寒暄都产生一个后台
/// 会话**，那正是 ADR-047 被否决的方案第一条。同理 `keywords` 取空集才让出厂行为
/// 收敛到「只有 `/work ` 才动手」。空前缀同理：`starts_with("")` 恒真，故显式跳过。
///
/// ## 未命中回 `None` 是判定的一部分
///
/// `None` = 「本轮不动手」，与「判了但没理由」不是一件事：本函数**只在命中时**才有
/// 返回值，所以任何 `Some(_)` 都自带可审计的理由。空输入（`None` / 空串 / 纯空白）
/// 不动后台会话，故归 `None`。
pub(crate) fn worker_start_reason(
    text: &str,
    force_prefix: &str,
    keywords: &[&str],
    min_chars: usize,
) -> Option<&'static str> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }

    // ① 显式前缀：用户直接指挥，零歧义，故排最前（短路顺序是判据的一部分）。
    if !force_prefix.is_empty() && trimmed.starts_with(force_prefix) {
        return Some(REASON_PREFIX);
    }

    // ② 关键词：子串、忽略大小写；空串关键词跳过（空白不该命中任何消息）。
    let lower = trimmed.to_lowercase();
    if keywords.iter().any(|k| {
        let k = k.trim();
        !k.is_empty() && lower.contains(&k.to_lowercase())
    }) {
        return Some(REASON_KEYWORD);
    }

    // ③ 主题长度：阈值 0 = 关闭（见类型文档）。
    if min_chars > 0 && trimmed.chars().count() >= min_chars {
        return Some(REASON_LENGTH);
    }

    None
}

// ── Q2：能力可见性 ────────────────────────────────────────────────────

/// Q2 · 能力目录：把能力清单折成**按分类分组的名字目录**（digest）。
///
/// ## 必须在收集之后才动笔
///
/// 收集是**广播**：容器 `composite::traverse` 逐子插件转发，取的实例表快照
/// （`PluginRegistry::snapshot`）直接遍历 `HashMap` 且**不排序**。在遍历尚未结束的
/// 任一子插件里取能力集合，必然漏掉还没被访问到的兄弟；顺序又每次不同，**漏了哪
/// 几项次次不同**——既静默（不报错、不缺项告警）又难复现。故 owner 是收口后的
/// `prepare_turn_inputs`，本函数只做折叠。
///
/// ## 顺序必须稳定
///
/// 同一轮里请求包要能逐字重现：分组用 `BTreeMap`（分类名按字典序），**组内也排序**
/// ——`traverse` 的遍历顺序不定，若组内按遇上的先后落笔，同一份能力清单在两轮之间
/// 就可能产出不同文本，而「同一轮内事实不漂移」是本仓反复钉住的口径。
///
/// ## 空 ⇒ `None`
///
/// 与就绪段「候选集空 ⇒ 整段不出现」同一条判据：给模型一段空目录只会诱导它瞎猜。
pub(crate) fn capability_digest(tools: &[CapabilityMeta]) -> Option<String> {
    if tools.is_empty() {
        return None;
    }

    // BTreeMap ⇒ 分类段稳定；组内 Vec 排序 ⇒ 同一清单逐字同形。
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for t in tools {
        let category = t
            .category
            .map(|c| format!("{c:?}"))
            .unwrap_or_else(|| "Other".to_string());
        groups
            .entry(category)
            .or_default()
            .push(short_name(&t.name));
    }
    for names in groups.values_mut() {
        names.sort();
        names.dedup();
    }

    let mut out = String::from("【能力目录】按分类分组（只列名字，不列参数）\n");
    for (category, names) in &groups {
        out.push_str("- ");
        out.push_str(category);
        out.push_str(": ");
        out.push_str(&names.join(", "));
        out.push('\n');
    }
    // 末尾换行不留：段落文本由消费方拼接，多一个空行会让两段之间出现双空行。
    while out.ends_with('\n') {
        out.pop();
    }
    Some(out)
}

/// 工具短名：`mcp/github/create_issue` → `create_issue`。
///
/// 与 `prepare_turn_inputs` 解析 `context_retention` 用的是同一条 `rsplit('/')` 口径
/// ——两处若各写一套，目录里的名字会与保留策略表里的名字对不上。
fn short_name(name: &str) -> String {
    name.rsplit('/').next().unwrap_or(name).to_string()
}

// ── Q3：worker 进展 ────────────────────────────────────────────────

/// 单步文本上限（字符）。进展是**一眼看完的进度**，不是转写重放：把 worker 的
/// 整段回复塞进每轮请求，等于让模型背着别人的全部历史跑。
const STEP_MAX_CHARS: usize = 60;
/// 步骤条数上限（**取最近的**）。进展关心"到哪了"，不是"都干过什么"。
const STEPS_MAX: usize = 8;

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

/// Q3 投影：由子会话清单 + 完整转写算进展。**纯函数**（形参只有两个 `&`，可穷举测试）。
pub(crate) fn progress_of(summary: &SessionSummary, session: &Session) -> WorkerProgress {
    // rounds：只数 user 消息。`role` 为 `None` 是「还没接线」，不冒充一轮——
    // 与 Q1 的 `None` 同一条纪律：没有观测就不要编出一个数来。
    let rounds = session
        .messages
        .iter()
        .filter(|m| matches!(m.role, Some(cm::MessageRole::User)))
        .count();

    // steps：非 user 节点的非空正文，**保留最近 STEPS_MAX 条**（drain 掉最早的）。
    let mut steps: Vec<String> = session
        .messages
        .iter()
        .filter(|m| !matches!(m.role, Some(cm::MessageRole::User)))
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
fn step_text(m: &cm::ChatMessage) -> Option<String> {
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

/// 终态集合（`MessageStatus` 的后四个变体）。
fn is_terminal(status: Option<&cm::MessageStatus>) -> bool {
    matches!(
        status,
        Some(cm::MessageStatus::Completed)
            | Some(cm::MessageStatus::Aborted)
            | Some(cm::MessageStatus::Failed)
            | Some(cm::MessageStatus::Removed)
    )
}

/// Q3 · 当前会话的 worker 进展：枚举子会话 → 逐个取转写 → 归属复核 → 投影。
///
/// **无子会话 ⇒ 零额外 IO**：`list_sub_sessions` 对不存在的 `sessions/` 直接返回空
/// （`nested.exists()` 短路），[`PersistentChatSession::load_sub_session`] 一次都不调。
/// 主会话没开 worker 时，本函数只多一次目录存在性判定。
pub(crate) async fn worker_progress(
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
        out.push(progress_of(&summary, &child));
    }
    Ok(out)
}

/// Q3 一条进展的线格式——**字段名逐字可见**（`rounds=` / `in_flight=` / `steps=`），
/// 与 B2 的状态行同名，两处漂移时一眼能对出来。
fn worker_line(p: &WorkerProgress) -> String {
    let title = if p.title.is_empty() {
        p.id.as_str()
    } else {
        p.title.as_str()
    };
    let state = if p.in_flight { "在途" } else { "已结束" };
    let steps = if p.steps.is_empty() {
        String::new()
    } else {
        format!("、steps=[{}]", p.steps.join("；"))
    };
    format!(
        "- worker `{title}`：rounds={}、in_flight={}（{state}）{steps}",
        p.rounds, p.in_flight
    )
}

/// Q1 + Q2 + Q3 合成**一条**请求视图段落——`prepare_turn_inputs` 的唯一装配入口。
///
/// 为什么合成一条而不是三条：`build_request_view` 每条段落都是一个参数、一个位置，
/// 三条 ⇒ 三个新位置要与记忆 0 / 就绪 1 的既有次序逐一排位，而三条真源的**出现条件**
/// 又互不相同（Q1 每轮恒有结论、Q2 视有无能力）。合成一条后只有「整段是否出现」
/// 一个开关，位置只有一个（index 2），次序判据只有一条。
///
/// ## 出现条件
///
/// - `utterance = None`（resume / 心跳等无用户新发言的请求）⇒ Q1 不下结论；
/// - `tools` 空 ⇒ Q2 整段不出现（给模型一段空目录只会诱导它瞎猜）；
/// - `workers` 空 ⇒ Q3 一条不写（没开 worker 时不占一个字节）；
/// - 三者皆空 ⇒ 返回 `None`，视图**一个字节都不多**。
///
/// ## 为什么「未命中」也要写出来
///
/// Q1 存在的全部理由是「判定可审计」——只在命中时出声，就无从区分**判了不动**与
/// **压根没判**（这正是本仓反复钉的「没有观测 = 静默失效」）。故未命中同样落一条
/// 事实，只是不带理由码：`worker_start_reason` 的 `None` 语义（未命中不回理由）
/// 由这条文本承载，函数本身不破例。
///
/// ## 三条判据按 `cfg` 取值
///
/// `cfg` 的 `worker_force_prefix` / `worker_keywords` / `worker_min_chars` 是**运行时的
/// 当前值**，出厂值只是它们的默认（真源在配置面）。改配置即改判定，两者不会各自
/// 漂移——判定侧不持第二份默认，生产路径与测试所测是**同一个函数体**。
pub(crate) fn delegate_section_with(
    utterance: Option<&str>,
    tools: &[CapabilityMeta],
    workers: &[WorkerProgress],
    cfg: &SessionConfig,
) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();

    if let Some(text) = utterance {
        // 关键词存**逗号串**（面板无数组控件），切开即用；空串由 `worker_start_reason`
        // 的「空关键词跳过」处理 ⇒ 空串 = 空集 = 本层关闭，与出厂值同一语义。
        let keywords: Vec<&str> = cfg.worker_keywords.split(',').collect();
        parts.push(
            match worker_start_reason(
                text,
                &cfg.worker_force_prefix,
                &keywords,
                cfg.worker_min_chars,
            ) {
                Some(reason) => format!("- 委派判定：本轮应启动 worker（reason={reason}）"),
                None => "- 委派判定：未启动 worker（未命中任何委派判据）".to_string(),
            },
        );
    }

    if let Some(digest) = capability_digest(tools) {
        parts.push(digest);
    }

    // Q3：每个 worker 一条，`rounds=` / `in_flight=` / `steps=` 三个字段名逐字可见。
    for p in workers {
        parts.push(worker_line(p));
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n"))
    }
}

#[cfg(test)]
#[path = "delegate.test.rs"]
mod tests;
