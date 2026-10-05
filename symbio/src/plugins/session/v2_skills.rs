//! 技能编译与路由（S9 第 22 步，[roadmap/S11](../../../../docs/plan/roadmap/S11-技能编译与自我改进.md)）。
//!
//! **技能是事实，不是特殊类型**（S11 §2 / J1）：编译产物就是一条
//! `memory.encoded{tag:"skill"}`——与普通记忆同格子、同召回、同巩固，读侧
//! （`RecallView`）根本看不出它特殊。本模块只做两件事：
//!
//! | 段 | core 的判定方 | 本模块做什么 |
//! |---|---|---|
//! | 编译 | `SkillCompiler::compile`（溯源指向源轨迹，I2） | 本轮成功轨迹 → 技能事件 |
//! | 路由 | `SkillRouter::route(confidence, threshold)` + `calibration` 投影 | 置信度低于阈值 ⇒ **该技能不进本轮提示词** |
//!
//! ## 三条口径
//!
//! - **回退的作用点是提示词**：`ReasonerFallback` 不是「换个模型跑」，而是把这条
//!   技能从本轮召回视图里**摘掉**——低置信的套路不该被当成可用事实摆在模型面前。
//!   快路那半边（真按 `budget_ms = 80` 装配、跳过模型）是执行档位与埋点的事，
//!   不在本批（S11 §6.2 的 P99 对比是**测量**，不是本模块的行为）。
//! - **观测不加新格子**（S11 §3）：每次判定落一条 `memory.recalled`，载荷只带数据
//!   `{ skill_id, fallback }`（`calibration` 模块文档定的观测面）。**没有这条观测，
//!   回退永远不会发生**——零使用时 `confidence()` 按 1.0 算，技能再错也一直被用
//!   （S11 §5 静默失效第 2 行）。
//! - **不做触发匹配**：哪些轨迹值得编译、怎么判「同类」是算法问题（S11 §7：
//!   架构只提供 `skill_compile` 这个位置），本批只把闸立在置信度上。

use std::collections::BTreeMap;
use std::path::Path;

use crate::symbio_core::authz::PRINCIPAL_USER;
use crate::symbio_core::{
    calibration, Budget, Event, EventWalStore, RecallView, Seq, SkillCompiler, SkillRoute,
    SkillRouter, Store, EVENT_MEMORY_ENCODED,
};

use super::paths::V2_WAL_FILE;

/// 技能的认知内容标签（`memory.encoded` 载荷里的 `tag`）。
///
/// 与 core 里 `SkillCompiler::compile` 写死的那一个字面量是同一份数据（ADR-043：
/// 名字是数据、单点定义）——改这里之前先改 core，否则两边对不上、技能会被
/// 当成普通记忆一路召回却不进路由。
pub(crate) const SKILL_TAG: &str = "skill";

/// 置信度阈值：低于它 ⇒ 不得走技能路径（S11 §5 反自动化回退）。
///
/// `confidence = 1 − 回退率`，0.8 读作「最近 5 次里至多 1 次回退」。取值是裁量
/// 不是架构：零使用的新技能按 1.0 算（不给新技能判死刑），**一次回退**就掉到
/// 0.0 ⇒ 下一次即被摘掉，下线足够快；再松就会把「一直用错的技能」继续放进来。
pub(crate) const SKILL_CONFIDENCE_THRESHOLD: f64 = 0.8;

/// 技能 id = trigger 的 FNV-1a 64。
///
/// 必须**跨进程稳定**：校准账按 `skill_id` 归并，用进程内的 `DefaultHasher` 每次
/// 重启都换 key，`uses` / `fallbacks` 就永远归不到一起——技能永远看起来"零使用"，
/// 阈值永远不触发。同款理由见 `heartbeat` 为何不用 `DefaultHasher` 跨轮比对。
fn skill_id_of(trigger: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in trigger.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// 编译（写方）：本轮**成功**轨迹 → 一条 `memory.encoded{tag:"skill"}`。
///
/// - 只编译成功收束（调用方以 `V2Closure::Final` 过滤）：兜底说明这条路自己没走通，
///   固化它等于把失败写成套路；
/// - **同 trigger 至多编译一次**——否则每收束一次就多一条技能，技能集随对话长度
///   无界增长（与 `v2_memory::encode` 的同文去重同一条纪律）；这条判据同时兼任
///   幂等：重跑同一 attempt 时技能已在 `snapshot` 里，直接 `Ok(None)` 不再 append；
/// - 溯源由 `SkillCompiler::compile` 自带（`produced_by = source_seq`，I2 断言的
///   「技能溯源 100%」）。
///
/// 返回入格的 seq；没编译（空文 / 重复 / 关着）⇒ `Ok(None)`。
pub(crate) fn compile(
    store: &EventWalStore,
    snapshot: &[Event],
    turn: u64,
    user_seq: u64,
    user_text: &str,
    response: &str,
    now: i64,
) -> Result<Option<u64>, String> {
    let trigger =
        super::v2_memory::truncate_chars(user_text.trim(), super::v2_memory::ENCODE_MAX_CHARS);
    let content =
        super::v2_memory::truncate_chars(response.trim(), super::v2_memory::ENCODE_MAX_CHARS);
    if trigger.is_empty() || content.is_empty() {
        return Ok(None);
    }
    if snapshot.iter().any(|e| {
        e.kind == EVENT_MEMORY_ENCODED
            && e.payload.get("tag").and_then(|v| v.as_str()) == Some(SKILL_TAG)
            && e.payload.get("trigger").and_then(|v| v.as_str()) == Some(trigger.as_str())
    }) {
        return Ok(None);
    }

    let skill_id = skill_id_of(&trigger);
    let mut ev = SkillCompiler.compile(&skill_id, &trigger, &content, user_seq);
    // 信封里的 `actor` / `turn` 由 core 造的是占位（`"agent:main"` / `0`）；**`memory × *`
    // 格子的 actor 是属主、不是作者**——同域三条写方（`v2_memory::encode` /
    // `consolidated` / `forgotten`）全落 `PRINCIPAL_USER`（巩固与遗忘更不是"用户做的"），
    // 属主 = `SESSION_OWNER`。而召回投影按 `actor == viewer` 过滤、生产读方
    // （`v2_memory::recall_view`）的 viewer 就是属主：占位不归位 ⇒ 技能被挡在召回
    // 视图外 ⇒ `route` 恒快判返回、观测永不落格、校准账零使用——S9 步 22 的生产
    // 链路正断在这里（[plan/11 批 2 ③] 的 e2e 判据钉的就是这条）。
    // 轮号 / 时刻同理落格前归位：核心契约只管「这条事件说什么」，
    // 何时发生、记在谁头上由写方给（`recalled_event` 同款 `.with_ts`）。
    ev.actor = PRINCIPAL_USER.to_string();
    ev.turn = turn;
    ev.ts = now;
    let seq = store
        .append(ev)
        .map_err(|e| format!("memory.encoded{{tag:\"skill\"}} 入格失败：{e:?}"))?;
    Ok(Some(seq.value()))
}

/// 路由（读方）：本轮召回视图里的技能按校准置信度闸判，低置信的**摘出视图**。
///
/// 返回逐条观测 `(skill_id, fallback)`——调用方在收束时落格（溯源锚是本轮
/// `user.message`，那时它才在事实源里），本轮下一轮的置信度才判得动。
///
/// 三条边界：
/// - **快判**：本轮没召回任何技能 ⇒ 一行 I/O 都不发生（编译关掉时恒走这里，
///   读侧因此不是每轮的固定开销）；
/// - **只认本会话编译的技能**：跨会话技能在本会话没有它的溯源轨迹与校准账，
///   判不了 ⇒ 原样留在视图里当背景事实，不假装判过；
/// - **巩固产物判不了**（合并后的条目有新 seq、不再带 `skill_id`）⇒ 同样原样保留。
pub(crate) fn route(session_dir: &Path, view: &mut RecallView) -> Vec<(String, bool)> {
    if !view.entries.iter().any(|e| e.tag == SKILL_TAG) {
        return Vec::new();
    }
    let path = session_dir.join(V2_WAL_FILE);
    // 只读打开：读方不创建文件、不截尾（`wal.rs::open_readonly`）。
    let Ok(store) = EventWalStore::open_readonly(&path) else {
        return Vec::new();
    };
    let events = store.range(Seq::new(0));

    // seq → skill_id：技能事件的 seq 是 `RecallEntry` 能带出来的唯一东西
    // （视图只有 content / tag / ts），而 `skill_id` 只躺在事件载荷里。
    let mut id_by_seq: BTreeMap<u64, String> = BTreeMap::new();
    for e in &events {
        if e.kind != EVENT_MEMORY_ENCODED
            || e.payload.get("tag").and_then(|v| v.as_str()) != Some(SKILL_TAG)
        {
            continue;
        }
        let (Some(seq), Some(id)) = (
            e.seq.map(|s| s.value()),
            e.payload.get("skill_id").and_then(|v| v.as_str()),
        ) else {
            continue;
        };
        id_by_seq.insert(seq, id.to_string());
    }

    let cal = calibration()
        .apply(&events, i64::MAX, Budget::generous())
        .value;
    let router = SkillRouter;
    let mut obs = Vec::new();
    let mut drop_seqs = Vec::new();
    for e in &view.entries {
        let Some(skill_id) = id_by_seq.get(&e.seq) else {
            continue;
        };
        let confidence = cal.of(skill_id).confidence();
        match router.route(confidence, SKILL_CONFIDENCE_THRESHOLD) {
            SkillRoute::SkillFastPath { .. } => obs.push((skill_id.clone(), false)),
            SkillRoute::ReasonerFallback { .. } => {
                obs.push((skill_id.clone(), true));
                drop_seqs.push(e.seq);
            }
        }
    }
    if !drop_seqs.is_empty() {
        view.entries.retain(|e| !drop_seqs.contains(&e.seq));
    }
    obs
}

#[cfg(test)]
#[path = "v2_skills.test.rs"]
mod tests;
