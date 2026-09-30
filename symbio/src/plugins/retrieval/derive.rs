//! 从磁盘派生**可召回事实** —— 本插件的机制，全是纯函数。
//!
//! ## 与 `fact_log` 的关系：同一份约定的两个独立消费者
//!
//! 两者都从磁盘派生事实，都只依赖**公开布局**（`<root>/session/<id>/messages.json`
//! 与同目录的 `MEMORY.md`），都**不 import 对方**。区别在**覆盖的格子**：
//!
//! | 事实源 | 覆盖 |
//! |---|---|
//! | `fact_log` | `turn.*` / `artifact.*`（对话基线） |
//! | **`retrieval`** | 上面的格子 **+ `memory.*`**（S06 的长期记忆） |
//!
//! ## 为什么 `memory.*` 从 `MEMORY.md` 派生
//!
//! S06 第 3 节把"记忆"定位为**一条普通事实**（`memory.encoded{payload}`），
//! 索引是 `Store` 的实现细节。本仓现行的记忆落位正是**会话目录下的 `MEMORY.md`**
//! （见 `session::memory` 与 `providers::memory`）——它是公开约定的一部分。
//! 因此：
//!
//! - `MEMORY.md` **非空** ⇒ 一条 `memory.encoded`（"这个会话钉住了一些结论"）；
//! - 每一条**非空行** ⇒ 一条 `memory.recalled` 候选（断言类，`caused_by` 指向
//!   `memory.encoded`）——这是"能检索到"的最小形态：**有记忆，且能被列举**。
//!
//! **不去读 `MEMORY.md` 的语义**（那是召回质量，S06 自己承认做不到）：
//! 本插件只证明"装得下检索"，不证明"检索得到对的东西"。
//!
//! ## 平凡值（J2）
//!
//! `MEMORY.md` 不存在 / 为空 ⇒ **不产生任何 `memory.*` 事实**，
//! 于是 `memory.recall` 投影自然走平凡值分支（只看当前窗口）。这不是错误路径。
//!
//! ## 确定性（A4）
//!
//! 无时钟、无随机、无全局状态。同一份磁盘 → 同一串事实（`seq` 由
//! `(会话序号, 本地序)` 编码，会话按 id 字典序排）。可双跑比对。

use crate::symbio_core::schemas::session::chat_message::ChatMessage;
use crate::symbio_core::{Fact, FactKind, FactPrincipal};
use serde_json::json;
use std::path::Path;

/// 单会话在全局 seq 空间里的步长（与 `fact_log` 同一约定：`2^40`）。
pub const SESSION_STRIDE: u64 = 1 << 40;

/// `memory.*` 事实在会话内占用的**尾部序号区**（避免与消息 `local_seq` 相撞）。
///
/// 消息的 `local_seq` 由存储分配、从 1 起；记忆事实不是消息，没有存储分配的
/// `local_seq`。因此给它们一个高位偏移（`1 << 38`），使
///
/// ```text
/// seq = session_ordinal * SESSION_STRIDE + MEMORY_BASE + k     (k = 0, 1, 2, …)
/// ```
///
/// 仍严格递增、且落在本会话区间内（远小于 `SESSION_STRIDE`），
/// 于是"seq 严格递增"在合并后依然成立。
pub const MEMORY_BASE: u64 = 1 << 38;

/// 会话记忆文件名 —— **字面量副本**（`session::memory::SESSION_MEMORY_FILE`）。
///
/// 不 `use` session：本插件只依赖"记忆住在这个文件名下"这条公开约定，
/// 与 `fact_log` 就地重声明 `messages.json` 的包装结构同款（插件独立原则）。
pub const MEMORY_FILE: &str = "MEMORY.md";

/// 一个会话的原始输入。
pub struct SessionFactsInput<'a> {
    pub session_id: &'a str,
    /// 全局会话位次（决定 seq 高位；由 [`sort_inputs`] 分配）
    pub session_ordinal: u64,
    pub messages: &'a [ChatMessage],
    /// `MEMORY.md` 的**非空行**（已按行序；空行已剔除）
    pub memory_lines: &'a [String],
}

/// 把若干会话派生为**可召回事实**序列（`turn.*` / `artifact.*` / `memory.*`）。
///
/// 输出按 `seq` 升序，`seq` 严格递增。
pub fn derive_facts(inputs: &[SessionFactsInput<'_>], max_facts: usize) -> Vec<Fact> {
    let mut out: Vec<Fact> = Vec::new();
    for input in inputs {
        let base = input.session_ordinal.saturating_mul(SESSION_STRIDE);
        let mut last_user_seq: Option<u64> = None;
        let mut last_assistant_seq: Option<u64> = None;

        // ① 对话事实（与 `fact_log` 同规则，同样是**公开**的派生约定）。
        for msg in input.messages {
            let local = match msg.seq {
                Some(s) if s > 0 => s as u64,
                _ => continue,
            };
            let seq = base.saturating_add(local);
            let Some(kind) = kind_of(msg) else { continue };
            let caused_by = match kind {
                FactKind::TurnAssistantFinal | FactKind::TurnAssistantFallback => last_user_seq,
                FactKind::ArtifactAdded => last_assistant_seq,
                _ => None,
            };
            out.push(Fact {
                seq,
                kind,
                principal: FactPrincipal::new(input.session_id),
                caused_by,
                at_ms: msg.timestamp.unwrap_or(0),
                payload: payload_of(msg),
            });
            match kind {
                FactKind::TurnUserMessage => last_user_seq = Some(seq),
                FactKind::TurnAssistantFinal | FactKind::TurnAssistantFallback => {
                    last_assistant_seq = Some(seq)
                }
                _ => {}
            }
        }

        // ② 记忆事实（S06 的 `memory.*` 格子）——只在 `MEMORY.md` 非空时产出。
        if !input.memory_lines.is_empty() {
            let encoded_seq = base.saturating_add(MEMORY_BASE);
            out.push(Fact {
                seq: encoded_seq,
                kind: FactKind::MemoryEncoded,
                principal: FactPrincipal::new(input.session_id),
                caused_by: None, // 记忆的编码是被钉住的结论，不指向某一条对话
                at_ms: 0,
                payload: json!({
                    "lines": input.memory_lines.len(),
                    "source": MEMORY_FILE,
                }),
            });
            // 每一行 ⇒ 一条 `memory.recalled`（断言类，**必须**带溯源 = I2）。
            for (i, line) in input.memory_lines.iter().enumerate() {
                out.push(Fact {
                    seq: encoded_seq.saturating_add(1 + i as u64),
                    kind: FactKind::MemoryRecalled,
                    principal: FactPrincipal::new(input.session_id),
                    caused_by: Some(encoded_seq), // 召回自"该会话的记忆编码"
                    at_ms: 0,
                    payload: json!({
                        "line": i,
                        // 只存长度读数，不存正文——事实是索引不是副本（同 B1 的取舍）
                        "len": line.chars().count(),
                    }),
                });
            }
        }
    }

    out.sort_by_key(|f| (f.seq, f.kind.wire()));
    if max_facts > 0 && out.len() > max_facts {
        out.truncate(max_facts);
    }
    out
}

/// 按 `session_id` 字典序排序输入，并就地分配 `session_ordinal`（从 1 起）。
pub fn sort_inputs<'a>(inputs: &mut [SessionFactsInput<'a>]) {
    inputs.sort_by(|a, b| a.session_id.cmp(b.session_id));
    for (i, input) in inputs.iter_mut().enumerate() {
        input.session_ordinal = i as u64 + 1;
    }
}

/// 消息 → 事实类型（与 `fact_log` 同一张表；`None` = 不产生事实）。
fn kind_of(msg: &ChatMessage) -> Option<FactKind> {
    let ty = msg.msg_type.as_ref();
    let role = msg.role.as_ref();
    match (role, ty) {
        (Some(crate::symbio_core::schemas::session::chat_message::MessageRole::User), _) => {
            Some(FactKind::TurnUserMessage)
        }
        (
            Some(crate::symbio_core::schemas::session::chat_message::MessageRole::Assistant),
            Some(crate::symbio_core::schemas::session::chat_message::MessageType::ToolCall),
        ) => Some(FactKind::ArtifactAdded),
        (Some(crate::symbio_core::schemas::session::chat_message::MessageRole::Tool), _) => {
            Some(FactKind::ArtifactAdded)
        }
        (Some(crate::symbio_core::schemas::session::chat_message::MessageRole::Assistant), _) => {
            match msg.status.as_ref() {
                Some(crate::symbio_core::schemas::session::chat_message::MessageStatus::Failed) => {
                    Some(FactKind::TurnAssistantFallback)
                }
                _ => Some(FactKind::TurnAssistantFinal),
            }
        }
        _ => None,
    }
}

/// 消息 → 载荷（只取索引字段，不存正文）。
fn payload_of(msg: &ChatMessage) -> serde_json::Value {
    json!({
        "message_id": &msg.id,
        "role": msg.role.as_ref().map(|r| format!("{r:?}")),
        "type": msg.msg_type.as_ref().map(|t| format!("{t:?}")),
        "name": msg.name.as_deref(),
        "status": msg.status.as_ref().map(|s| format!("{s:?}")),
        "timestamp": msg.timestamp,
    })
}

/// 读一个会话目录的消息列表（公开布局：`<session_root>/<id>/messages.json`）。
///
/// 包装结构**就地重声明**（一个只读 `messages` 键），不 import `session`。
pub fn read_messages(session_root: &Path, session_id: &str) -> Option<Vec<ChatMessage>> {
    let path = session_root.join(session_id).join("messages.json");
    let text = std::fs::read_to_string(path).ok()?;
    let file: MessagesFile = serde_json::from_str(&text).ok()?;
    Some(file.messages)
}

/// 读一个会话的 `MEMORY.md` **非空行**（不存在 → 空）。
///
/// 纯函数式：按行扫描，剔除纯空白行，**不改写内容**（不改写就没有副作用面）。
pub fn read_memory_lines(session_root: &Path, session_id: &str) -> Vec<String> {
    let path = session_root.join(session_id).join(MEMORY_FILE);
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

/// 枚举会话根下的全部会话 id（一层目录，含 `messages.json` 者）。
pub fn session_ids(session_root: &Path) -> Vec<String> {
    let mut ids = Vec::new();
    let Ok(entries) = std::fs::read_dir(session_root) else {
        return ids;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if path.join("messages.json").is_file() {
                ids.push(name.to_string());
            }
        }
    }
    ids
}

/// `messages.json` 的落盘形状 —— 只声明本插件要读的那一个键（同 `fact_log`）。
#[derive(serde::Deserialize)]
struct MessagesFile {
    #[serde(default)]
    messages: Vec<ChatMessage>,
}

#[cfg(test)]
#[path = "derive.test.rs"]
mod tests;
