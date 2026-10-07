//! 委派者三项真源（[ADR-047](../../../../../docs/decisions/session.md)）——**数据，不是机制**。
//!
//! 目标形态是**主会话不持有工具**，于是它缺三样外部事实：worker 何时启动、自己有哪些
//! 能力、进展如何。三问的答案都必须是**可穷举测试的数据**，否则主会话就得重新持有
//! 工具——那正是「不持有工具」这个目标形态的反面。本模块是其中两样的生产处，
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
//! ## 本批刻意未做的两件事
//!
//! - **不往 `SessionConfig` 加旋钮**：ADR-047 要的是「答案是数据」，出厂值
//!   （前缀 `/work `、关键词空集、阈值 0）由本模块的常量给出。旋钮会连带改配置面与
//!   `CONFIGURATION.md` 字段表，属独立一批——登记在 [04 §3.2](../../../../../docs/plan/04-工程落地.md)。
//! - **Q3（worker 进展）不在本模块**：它的读侧口径（`chat_loop` 如何枚举子会话、
//!   子转写从哪来）尚未钉死，见 04 §3.2 的登记。先落 Q1/Q2，不给未定口径写死实现。

use crate::symbio_core::CapabilityMeta;
use std::collections::BTreeMap;

// ── Q1：worker 启动条件 ────────────────────────────────────────────────

/// Q1 理由码。与 `classify::reasons` 同一条约定：理由码是**英文短词**，
/// 可观测、可穷举、可被测试逐个钉住——没有理由码的判定不可审计（ADR-047 不变量）。
pub(crate) const REASON_PREFIX: &str = "explicit_prefix";
pub(crate) const REASON_KEYWORD: &str = "keyword_hit";
pub(crate) const REASON_LENGTH: &str = "topic_length";

/// 出厂值：显式前缀，**带尾空格**——`/work 干活` 命中，`/worker 是什么` 不命中。
pub(crate) const FORCE_PREFIX: &str = "/work ";
/// 出厂值：关键词（子串、忽略大小写）。**空集 = 本层关闭**。
pub(crate) const KEYWORDS: &[&str] = &[];
/// 出厂值：主题长度阈值（字符数）。**0 = 本层关闭**。
pub(crate) const MIN_CHARS: usize = 0;

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

/// Q1 + Q2 合成**一条**请求视图段落——`prepare_turn_inputs` 的唯一装配入口。
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
/// - 两者皆空 ⇒ 返回 `None`，视图**一个字节都不多**。
///
/// ## 为什么「未命中」也要写出来
///
/// Q1 存在的全部理由是「判定可审计」——只在命中时出声，就无从区分**判了不动**与
/// **压根没判**（这正是本仓反复钉的「没有观测 = 静默失效」）。故未命中同样落一条
/// 事实，只是不带理由码：`worker_start_reason` 的 `None` 语义（未命中不回理由）
/// 由这条文本承载，函数本身不破例。
pub(crate) fn delegate_section(
    utterance: Option<&str>,
    tools: &[CapabilityMeta],
) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();

    if let Some(text) = utterance {
        parts.push(
            match worker_start_reason(text, FORCE_PREFIX, KEYWORDS, MIN_CHARS) {
                Some(reason) => format!("- 委派判定：本轮应启动 worker（reason={reason}）"),
                None => "- 委派判定：未启动 worker（未命中任何委派判据）".to_string(),
            },
        );
    }

    if let Some(digest) = capability_digest(tools) {
        parts.push(digest);
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
