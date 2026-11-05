//! 收束贡献：情景记忆编码 —— 把本轮的一问一答编成至多两条 `memory.encoded`
//! （DE-036 §3 / DE-025 D-250 / DE-026 D-263）。
//!
//! ## 两条编码
//!
//! | 标签 | 内容 | 溯源 | 可见域 |
//! |---|---|---|---|
//! | [`MEMORY_TAG`]（经验） | 本轮**用户发言** | 本轮开口格 | `shared`——人的认知可供本部署矩阵内主体借用（SC-010） |
//! | [`JUDGMENT_TAG`]（判断） | 本轮**成功收束的正文**（兜底 / 中止为 `None` ⇒ 不编） | 同一个开口格（这条判断因它而起） | `thread_private`——会话私有的工作记忆（C10） |
//!
//! ## 四条口径
//!
//! - **空不编**：trim 后为空 ⇒ 这一条不存在（空正文进记忆只会稀释检索）；
//! - **截断**：超过 [`ENCODE_MAX_CHARS`] 只存前段——转写已存全文，记忆存提炼，
//!   两处各存全文就是两份事实源；
//! - **同文去重（跨标签）**：史里已有同内容的**活**记忆（`memory.encoded` ∪
//!   `memory.consolidated`，未被 `memory.forgotten` 排除，认知归属人为
//!   [`PRINCIPAL_USER`]）⇒ 不再追加。判定范围是完整史而不是窗口：窗口省掉的是
//!   最便宜的一节，代价是窗外的同文被再记一条；
//! - **不保证产出**：空发言 / 同文重复都交空集，所以它是贡献而不是债——写成环上
//!   的债就得给这一行编一个「天然空转」的 `owes`。
//!
//! 同轮两条各编各的（彼此不可见，与旧实现同形：两条草稿对着同一份 `history` 判）；
//! 编码落格后，宿主的巩固在同轮的**之后**读刷新快照——「编码先于巩固」由贡献阶段
//! 排在 `record_learning` 之前兑现（D-263）。
//!
//! 载荷不带 `vec` 键：向量索引是 Store 的实现细节（DE-001 §9.2，D-152）。

use serde_json::json;
use symbio_plugin_api::events::EVENT_MEMORY_ENCODED;
use symbio_plugin_api::{
    truncate_chars, Contribution, Contributor, Entity, Event, FactDraft, Settling, Verb,
};

use super::decl::{ENCODE_MAX_CHARS, JUDGMENT_TAG, MEMORY_TAG};
use crate::decl::OWNER;

pub(crate) struct EpisodicEncode;

impl Contributor for EpisodicEncode {
    fn owner(&self) -> &'static str {
        OWNER
    }

    fn contribute(&self, at: &Settling<'_>) -> Vec<Contribution> {
        let mut out = Vec::new();
        // 经验（用户发言）：可见域 `shared`（内容侧产地，读侧只消费不重算）。
        if let Some(encoded) = encode(at.user_text, at.history) {
            out.push(draft(
                format!("v2m-{}", at.anchor),
                MEMORY_TAG,
                encoded,
                "shared",
                at.user_seq,
            ));
        }
        // 判断（成功收束的正文）：兜底 / 中止为 `None` ⇒ 不编。
        if let Some(response) = at.response {
            if let Some(encoded) = encode(response, at.history) {
                out.push(draft(
                    format!("v2j-{}", at.anchor),
                    JUDGMENT_TAG,
                    encoded,
                    "thread_private",
                    at.user_seq,
                ));
            }
        }
        out
    }
}

/// 一条正文 → 可编码的内容：trim 为空 / 史里已有同文活记忆 ⇒ `None`。
fn encode(text: &str, history: &[Event]) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let encoded = truncate_chars(trimmed, ENCODE_MAX_CHARS);
    if live_contents(history).contains(&encoded) {
        return None;
    }
    Some(encoded)
}

/// 一条待落格的 `memory.encoded` 草稿（信封 `turn` / `ts` 由宿主落格时盖章）。
fn draft(
    event_id: String,
    tag: &'static str,
    content: String,
    vis_scope: &'static str,
    user_seq: u64,
) -> Contribution {
    Contribution::new(FactDraft {
        key: event_id,
        kind: EVENT_MEMORY_ENCODED.to_string(),
        entity: Entity::Memory,
        verb: Verb::Opened,
        payload: json!({
            "content": content,
            "tag": tag,
            "generation": 0u32,
            "vis_scope": vis_scope,
        }),
        produced_by: Some(user_seq),
    })
}

/// 活记忆的正文集：**折叠只有一份**（[`crate::recall::live::live`]），这里只取正文。
///
/// 去重问的是「在不在」，不是「排多前」⇒ 不带查询、不关心次序。技能标签的同文也算
/// 重复——同一个事实不必按标签记两遍。
///
/// 不在这里再写一份折叠：`D-268`「一副折叠两个读者」要求的就是这个形状，两份实现
/// 必然漂移（同一 crate 内更没有任何接口边界可以拦住它）。
fn live_contents(history: &[Event]) -> Vec<String> {
    crate::recall::live::live(history, i64::MAX)
        .into_iter()
        .map(|e| e.content)
        .collect()
}

// 链接期提交：宿主按 `(prio, owner)` 询问。
symbio_plugin_api::submit_contributor!(EpisodicEncode);

#[cfg(test)]
#[path = "contributor.test.rs"]
mod tests;
