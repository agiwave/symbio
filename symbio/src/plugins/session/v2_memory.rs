//! 记忆三段（S5 步 11–13，[04 §3.1 批⑦](../../../../docs/plan/04-工程落地.md)）：
//! **写方 / 召回注入 / 巩固**——全部落在 per-session 的 v2 事实源上。
//!
//! 与 `super::memory`（本会话 `MEMORY.md` 置顶约定，文件形态）的分工：那层记
//! 「钉住的约定」，本层是
//! [roadmap/S06](../../../../docs/plan/roadmap/S06-长期记忆与语义检索.md) 的**事件
//! 记忆**——不另开文件，记忆就是 `v2-events.wal` 里的普通事件（ADR-044 同族纪律：
//! 同一事实一份持久化，没有旁路存储）。
//!
//! ## 三段各自的判定方（一个规则一个判定方）
//!
//! | 段 | core 的判定方 | 本模块做什么 |
//! |---|---|---|
//! | 步 11 编码 | [`RecallView::contains_content`](crate::symbio_core::RecallView) 同文去重 | 本轮用户发言 → `memory.encoded` |
//! | 步 12 召回 | `recall` 投影（as-of / 排除式遗忘 / 预算） | 读视图 → 注入请求视图 + `memory.recalled` |
//! | 步 13 巩固 | `consolidate::accept`（代数上界 + 保真度下界） | 合并最旧两条，**过门才入格** |
//!
//! 算法（怎么合并、保真度怎么算、注入段怎么排版）住本模块：core 只提供**读视图**
//! 与**接受边界**（[plan/01 §9.3](../../../../docs/plan/01-核心架构.md)：架构只提供
//! 位置，不提供算法）。
//!
//! ## 四条口径（为什么是这样）
//!
//! - **记什么**：轮末把本轮用户发言编码成一条**情景记忆**（actor = `user`、溯源指向
//!   本轮 `user.message` 格、标签「经验」——七类认知内容之一，roadmap/00）。同文去重
//!   的判定方在 core，本模块不重写一遍比对。空发言不记；超长截断（`ENCODE_MAX_CHARS`）
//!   ——转写（`messages.json`）已存全文，记忆存**提炼**，两处各存全文就等于两份
//!   事实源。
//! - **`vec` 恒 `[]`**：向量索引是 Store 的实现细节（01 §9.2），召回当前走标签 + 时间
//!   口径、不消费 `vec`；内容恒在 `content`，索引落成时按 `content` 回填。本批不给
//!   每轮对话挂一次 ONNX 推理——那会把推理成本加到每一轮上，而收益是零（没有索引
//!   在读它）。
//! - **存哪 / 怎么读**：写方只写**本会话自己的** `v2-events.wal`（共享文件需要跨进程
//!   锁，而 `WalStore` 没有——plan/11 已确认，两进程同写一份会互相截尾）；召回反过来
//!   **跨会话扫**（[`recall_view`]），读侧无锁、按文件数与字节双预算封顶。
//! - **何时注入**：请求视图**置顶**一条 `role=user`、`meta.kind = recall_context` 的
//!   记忆消息（与 nudge 同机制：请求级、不落库）。不进系统提示词——那里有「唯一真源 =
//!   注册段」的纪律（`plugins/session/README.md`）；也不置尾——置尾会把「最后一条
//!   user 消息」从用户的问题换成记忆，mock 场景匹配与轮次窗口都会读错。

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::authz::PRINCIPAL_USER;
use crate::symbio_core::{
    accept, recall, recalled_event, Budget, ConsolidateParams, Entity, Event, EventWalStore,
    RecallView, Rejection, Seq, Store, Verb, EVENT_MEMORY_CONSOLIDATED, EVENT_MEMORY_ENCODED,
    EVENT_MEMORY_FORGOTTEN, EVENT_MEMORY_RECALLED,
};

// ==================== 常量（口径的数值面，单点定义） ====================

/// 认知内容标签：自动编码的情景内容一律标「**经验**」（七类：知识 / 经验 / 技能 /
/// 判断 / 策略 / 直觉 / 情绪，[roadmap/00](../../../../docs/plan/roadmap/00-路线图总览.md)）。
///
/// 分类需要模型判断，不在本步（分类的落点是 S10）——但标签不能留空：巩固按标签
/// 分组，空标签等于没有分组。
pub(crate) const MEMORY_TAG: &str = "经验";

/// 单条记忆正文的字符上限（截断点）。
pub(crate) const ENCODE_MAX_CHARS: usize = 400;

/// 合并产物正文的字符上限。保真度按「源句保留率」算，装不下被丢的句子即失真
/// ⇒ 两条都很长的记忆合并不过 `accept` 的 0.7 下界，**源记忆原样保留**
/// （拒收路径在生产上可达，不是只活在测试里的死分支）。
pub(crate) const MERGE_MAX_CHARS: usize = 400;

/// 触发一次巩固所需的**活**记忆条数（同标签）。取 4：每合并一次净减 1 条，
/// 条数收敛到 3–4 后停在 `accept` 的代数上界（`max_gen = 3`）上。
pub(crate) const CONSOLIDATE_MIN_ENTRIES: usize = 4;

/// 注入段的字节上限——按**整条**追加，装不下就停（不切半行）。
pub(crate) const RECALL_MAX_BYTES: usize = 4096;

/// 跨会话扫描预算：最多看几个会话的事实源 / 单文件字节上限。
/// 超限 ⇒ 不扫并 `plugin_warn` 记一笔——看不见的文件不假装看得见。
pub(crate) const RECALL_MAX_FILES: usize = 16;
pub(crate) const RECALL_MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;
pub(crate) const RECALL_MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024;

/// `recall` 投影的扫描配额（`Budget::ms` = 事件数，1 事件 = 1ms）：超配额 ⇒
/// `degraded` 的部分结果（I3 在检索面上的形状），不装作扫完了。
pub(crate) const RECALL_SCAN_QUOTA: u64 = 500;

/// 注入段抬头——e2e 与单测都以它为锚。
pub(crate) const RECALL_SECTION_HEAD: &str = "【长期记忆】";

// ==================== 步 12 · 读方（跨会话召回） ====================

/// 步 12 的读方：**跨会话**召回 → [`RecallView`]（一条记忆都没有 ⇒ `None`）。
///
/// 扫 `<会话存储根>/*​/v2-events.wal`（含本会话自己），按 mtime 新近优先、文件数与
/// 字节双预算封顶，再整体交给 `recall` 投影。合并后按 `ts` 降序——**记忆类事件带
/// 真实编码时刻**（`RecallEntry::ts` 的契约），跨会话才比得了新近；Turn 类事件仍
/// `ts = 0`，但它们不进这个视图（投影只认 `entity == memory`）。
///
/// as-of 取 `i64::MAX`：与 `session/stats` 同一条口径——WAL 是落盘事实源，文件里
/// 每一条都已发生（见 `stats.rs` 模块文档）。
pub(crate) fn recall_view(session_dir: &Path) -> Option<RecallView> {
    let root = session_dir.parent()?;

    // ── 找文件：新近优先，超预算的当场弃权并留痕 ──────────────────────────
    let mut files: Vec<(SystemTime, PathBuf, u64)> = Vec::new();
    if let Ok(dir) = std::fs::read_dir(root) {
        for entry in dir.flatten() {
            let path = entry.path().join(super::paths::V2_WAL_FILE);
            let Ok(md) = std::fs::metadata(&path) else {
                continue;
            };
            if !md.is_file() || md.len() == 0 {
                continue;
            }
            if md.len() > RECALL_MAX_FILE_BYTES {
                crate::plugin_warn!(
                    "session",
                    "[memory] 召回跳过超大事实源（{} B > 上限 {} B）：{}",
                    md.len(),
                    RECALL_MAX_FILE_BYTES,
                    path.display()
                );
                continue;
            }
            let mtime = md.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            files.push((mtime, path, md.len()));
        }
    }
    if files.is_empty() {
        return None;
    }
    files.sort_by(|a, b| b.0.cmp(&a.0));

    // ── 读文件：只读打开（读方不截尾，见 `wal.rs::open_readonly`）───────────
    let mut events: Vec<Event> = Vec::new();
    let mut bytes = 0u64;
    for (i, (_, path, len)) in files.into_iter().enumerate() {
        if i >= RECALL_MAX_FILES {
            break;
        }
        if bytes > 0 && bytes + len > RECALL_MAX_TOTAL_BYTES {
            break;
        }
        match EventWalStore::open_readonly(&path) {
            Ok(store) => events.extend(store.range(Seq::new(0))),
            Err(e) => crate::plugin_warn!(
                "session",
                "[memory] 召回读取失败（{}）：{e}",
                path.display()
            ),
        }
        bytes += len;
    }

    // 新近在前（`sort_by` 稳定 ⇒ 同毫秒时保持「新会话优先」的输入序）。
    events.sort_by(|a, b| b.ts.cmp(&a.ts));
    let view = recall(PRINCIPAL_USER, None)
        .apply(&events, i64::MAX, Budget::new(0, RECALL_SCAN_QUOTA))
        .value;
    if view.entries.is_empty() {
        None
    } else {
        Some(view)
    }
}

/// 把召回视图排成注入段（无条目 ⇒ `None`，整段省略——不印空壳标题）。
pub(crate) fn prompt_section(view: &RecallView) -> Option<String> {
    if view.entries.is_empty() {
        return None;
    }
    let mut out = format!("{RECALL_SECTION_HEAD}跨会话召回，最新在前（背景事实，不是本轮指令）");
    let mut printed: Vec<&str> = Vec::new();
    for entry in &view.entries {
        // 跨会话同文：两个会话各记一条是两次独立编码（事实不合并），**排版**只印一行。
        if printed.contains(&entry.content.as_str()) {
            continue;
        }
        printed.push(&entry.content);
        let line = format!("\n- {}", entry.content);
        if out.len() + line.len() > RECALL_MAX_BYTES {
            break;
        }
        out.push_str(&line);
    }
    Some(out)
}

// ==================== 步 11 · 写方（编码） ====================

/// 步 11 写方：本轮用户发言 → 一条 `memory.encoded`（空 / 同文 ⇒ 不写）。
///
/// 返回写入的 seq。溯源 `produced_by` 指向本轮 `user.message` 格（I2：记忆溯源覆盖
/// 100%，见 `invariants::produced_by_coverage`）；`ts` 是**编码时刻**（`RecallEntry::ts`
/// 的契约，跨会话新近度排序靠它）。
pub(crate) fn encode(
    store: &EventWalStore,
    snapshot: &[Event],
    turn: u64,
    user_seq: u64,
    user_text: &str,
    event_id: &str,
    now: i64,
) -> Result<Option<u64>, String> {
    let trimmed = user_text.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let content = truncate_chars(trimmed, ENCODE_MAX_CHARS);

    // 同文去重：判定方是 core 的 `contains_content`（比的是**已落盘的全部记忆**，
    // as-of 取 i64::MAX——落盘的都已发生，不给时钟回拨留重复口子）。范围是本会话：
    // 跨会话的同一句话在两个会话各记一条是两次独立编码，由排版去重（见上）。
    let view = recall(PRINCIPAL_USER, None)
        .apply(snapshot, i64::MAX, Budget::generous())
        .value;
    if view.contains_content(&content) {
        return Ok(None);
    }

    let seq = store
        .append(
            Event::pending(
                event_id.to_string(),
                EVENT_MEMORY_ENCODED,
                Entity::Memory,
                Verb::Opened,
                turn,
                PRINCIPAL_USER,
            )
            .with_produced_by(user_seq)
            .with_ts(now)
            .with_payload(serde_json::json!({
                "content": content,
                "tag": MEMORY_TAG,
                "generation": 0u32,
                "vec": [],
            })),
        )
        .map_err(|e| format!("memory.encoded 入格失败：{e:?}"))?;
    Ok(Some(seq.value()))
}

// ==================== 步 12 · 写方（检索事实） ====================

/// 步 12 的写方：本轮检索 → 一条 `memory.recalled`（Translator 出事实，不造信息）。
///
/// `trigger_seq` 是**本轮 `user.message` 格**——溯源锚要等到那格入盘才存在，所以这条
/// 事实与本轮其它事实一起在收束时入格（不是取视图那一刻）。同一锚已落 ⇒ 幂等跳过。
pub(crate) fn record_recalled(
    store: &EventWalStore,
    view: &RecallView,
    trigger_seq: u64,
    actor: &str,
    now: i64,
) -> Result<(), String> {
    let already = store
        .range(Seq::new(0))
        .iter()
        .any(|e| e.kind == EVENT_MEMORY_RECALLED && e.produced_by == Some(trigger_seq));
    if already {
        return Ok(());
    }
    store
        .append(recalled_event(view, trigger_seq, actor).with_ts(now))
        .map_err(|e| format!("memory.recalled 入格失败：{e:?}"))?;
    Ok(())
}

// ==================== 步 13 · 巩固 ====================

/// 步 13 写方：同标签活记忆 ≥ [`CONSOLIDATE_MIN_ENTRIES`] ⇒ 合并最旧两条，
/// **过 `accept` 才入格**。返回 `memory.consolidated` 的 seq（条数不够 / 被拒收 ⇒
/// `None`——拒收不是错误，源记忆原样留在视图里，下一轮再试）。
///
/// 写入顺序是设计过的：**先写合并产物、后写两条 `memory.forgotten`**——后者失败
/// ⇒ 视图里多一条重复（无害），前者失败 ⇒ 源记忆还在。反过来会丢记忆。
pub(crate) fn consolidate(
    store: &EventWalStore,
    snapshot: &[Event],
    turn: u64,
    now: i64,
) -> Result<Option<u64>, String> {
    let view = recall(PRINCIPAL_USER, Some(MEMORY_TAG.to_string()))
        .apply(snapshot, i64::MAX, Budget::generous())
        .value;
    if view.entries.len() < CONSOLIDATE_MIN_ENTRIES {
        return Ok(None);
    }

    // 召回视图按新近度**降序** ⇒ 倒序取最旧两条（巩固从最旧开始推进代数）。
    let oldest: Vec<&_> = view.entries.iter().rev().take(2).collect();
    if oldest.len() < 2 {
        return Ok(None);
    }
    let source_seqs = [oldest[0].seq, oldest[1].seq];

    // 代数从源事件的载荷读（`recall` 视图不带代数——它是读视图，不是写账本）。
    let generation_of = |seq: u64| -> u32 {
        snapshot
            .iter()
            .find(|e| e.seq.map(|s| s.value()) == Some(seq))
            .and_then(|e| e.payload.get("generation"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32
    };
    let generation = generation_of(source_seqs[0]).max(generation_of(source_seqs[1])) + 1;

    let (content, fidelity) = merge_with_fidelity(
        &[oldest[0].content.as_str(), oldest[1].content.as_str()],
        MERGE_MAX_CHARS,
    );

    // 判定方只有一个：core 的 `accept`。拒收 ⇒ **不入格**（不是降标入库，也不是
    // 写条日志照写）。两个分支分开说，是因为它们是两件事：代数上界是**预期的终点**，
    // 保真度不足是**这条策略装不下它**。
    match accept(&ConsolidateParams::default(), generation, fidelity) {
        Ok(()) => {}
        Err(Rejection::RejectedMaxGen { generation, max }) => {
            crate::plugin_debug!(
                "session",
                "[memory] 巩固停在代数上界（gen {generation} > max {max}）：源记忆原样保留"
            );
            return Ok(None);
        }
        Err(Rejection::RejectedLowFidelity { fidelity, min }) => {
            crate::plugin_debug!(
                "session",
                "[memory] 巩固保真度不足（{fidelity:.3} < {min}）：源记忆原样保留"
            );
            return Ok(None);
        }
    }

    let consolidated_seq = store
        .append(
            Event::pending(
                format!("v2mc-{turn}-{}-{}", source_seqs[0], source_seqs[1]),
                EVENT_MEMORY_CONSOLIDATED,
                Entity::Memory,
                Verb::Progressed,
                turn,
                PRINCIPAL_USER,
            )
            .with_produced_by(source_seqs[0])
            .with_ts(now)
            .with_payload(serde_json::json!({
                "content": content,
                "tag": MEMORY_TAG,
                "generation": generation,
                "fidelity": fidelity,
                "sources": source_seqs,
                "vec": [],
            })),
        )
        .map_err(|e| format!("memory.consolidated 入格失败：{e:?}"))?;

    // 源记忆 → 排除式遗忘：Log 不删，只是 `recall` 投影不再包含（可撤销、可审计）。
    for target in source_seqs {
        store
            .append(
                Event::pending(
                    format!("v2mf-{target}"),
                    EVENT_MEMORY_FORGOTTEN,
                    Entity::Memory,
                    Verb::Closed,
                    turn,
                    PRINCIPAL_USER,
                )
                .with_produced_by(target)
                .with_ts(now)
                .with_payload(serde_json::json!({
                    "why": "consolidated",
                    "into": consolidated_seq.value(),
                })),
            )
            .map_err(|e| format!("memory.forgotten 入格失败：{e:?}"))?;
    }
    Ok(Some(consolidated_seq.value()))
}

// ==================== 小工具 ====================

/// 按**字符**截断（不切 UTF-8 字节）。
///
/// `pub(crate)` 只为共享给 [`super::v2_skills`]：技能的 trigger / response 走同一个
/// 上限（技能内容同样要进提示词，两处各写一份上限迟早分叉）。
pub(crate) fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    text.chars().take(max_chars).collect()
}

/// 句子切分：中英文句末标点与换行为界（标点留在句内），空白段丢弃。
///
/// 刻意**不**把裸 `.` 当界：`1.5` 会被切成 `1.` / `5`，合并后印成 `1.\n5`——
/// 不但失真，还把数字改坏了。英文的句号只在其后是空白或结尾时才算界。
fn split_sentences(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut buf = String::new();
    for (i, ch) in chars.iter().enumerate() {
        buf.push(*ch);
        let next = chars.get(i + 1).copied();
        let boundary = matches!(ch, '。' | '！' | '？' | '!' | '?' | '\n')
            || (*ch == '.' && next.is_none_or(char::is_whitespace));
        if boundary {
            let unit = buf.trim().to_string();
            if !unit.is_empty() {
                out.push(unit);
            }
            buf.clear();
        }
    }
    let tail = buf.trim().to_string();
    if !tail.is_empty() {
        out.push(tail);
    }
    out
}

/// 合并两条来源：去重后的句子序列装进 `max_chars`，并算保真度。
///
/// 保真度 = 各来源中**确实保留在合并结果里**的句字数 / 各来源总字数。
/// 同一件事两份都写了 ⇒ 两份的句子都算保留（去重省的是位置，不是信息）；
/// 装不下被丢掉的才算失真——那正是 `accept` 的 0.7 下界要拦的东西。
fn merge_with_fidelity(sources: &[&str], max_chars: usize) -> (String, f64) {
    let mut units: Vec<String> = Vec::new();
    for s in sources {
        for unit in split_sentences(s) {
            if !units.contains(&unit) {
                units.push(unit);
            }
        }
    }

    let mut kept: Vec<String> = Vec::new();
    let mut used = 0usize;
    for unit in &units {
        let add = unit.chars().count() + if kept.is_empty() { 0 } else { 1 }; // '\n' 分隔
        if used + add > max_chars {
            break;
        }
        used += add;
        kept.push(unit.clone());
    }

    let (mut retained, mut total) = (0usize, 0usize);
    for s in sources {
        for unit in split_sentences(s) {
            total += unit.chars().count();
            if kept.contains(&unit) {
                retained += unit.chars().count();
            }
        }
    }
    let fidelity = if total == 0 {
        1.0
    } else {
        retained as f64 / total as f64
    };
    (kept.join("\n"), fidelity)
}

#[cfg(test)]
#[path = "v2_memory.test.rs"]
mod tests;
