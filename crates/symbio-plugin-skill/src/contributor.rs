//! 收束贡献：技能编译 —— 本轮**成功**轨迹固化成一条 `memory.encoded{tag:"skill"}`
//! （DE-041 §2 / DE-025 D-250）。
//!
//! ## 为什么是贡献而不是债
//!
//! 编译**不保证产出**：没有正文、同一条触发串已编译过、开关关着，都不该写任何东西。
//! 写成环上的债就得给这一行编一个「天然空转」的 `owes`，环会在同一笔债上反复选中它
//! 直到步数上界。所以它是收束这一刻的一条贡献：宿主按 `(prio, owner)` 询问，交空集
//! 就是合法结局。
//!
//! ## 三条判据
//!
//! - **只编译成功收束**：`response` 是收束格 `chat.assistant.final` 的正文；兜底 / 中止
//!   为 `None`——固化失败套路比不记更坏；
//! - **同 trigger 至多一条**：否则技能集随对话长度无界增长（与记忆编码的同文去重同一
//!   条纪律）；这条判据同时兼任幂等；
//! - **开关默认关**：读 `history` 里最后一条 `session.config`（D-252：能力面不拿配置包），
//!   缺省 `false`——不编译，只检索。
//!
//! ## 归属与可见域
//!
//! 技能事件的 `actor` 是**认知归属人**（`PRINCIPAL_USER`），不是跑轮主体：它属于
//! 「人的认知」，召回视图按 `actor == viewer` 过滤时应当看见它。可见域记为
//! `shared`——技能是固化的可复用经验，跨会话命中是它的设计意图。
//!
//! 溯源锚是本轮开口格（`user_seq`）：技能**必带溯源**（I2 的「技能溯源 100%」）。

use serde_json::json;
use symbio_plugin_api::events::{EVENT_MEMORY_ENCODED, EVENT_SESSION_CONFIG};
use symbio_plugin_api::{
    truncate_chars, Contribution, Contributor, Entity, Event, FactDraft, Settling, Verb,
    PRINCIPAL_USER,
};

use super::decl::{COMPILE_LIMIT, OWNER, SKILL_TAG};

pub(crate) struct SkillCompile;

impl Contributor for SkillCompile {
    fn owner(&self) -> &'static str {
        OWNER
    }

    fn contribute(&self, at: &Settling<'_>) -> Vec<Contribution> {
        // 两面都编译：定稿答话面同样是一次成功收束（没有工具交换不代表没有套路）。
        let Some(response) = at.response else {
            return Vec::new();
        };
        if !compile_enabled(at.history) {
            return Vec::new();
        }
        let trigger = truncate(at.user_text.trim());
        let content = truncate(response.trim());
        if trigger.is_empty() || content.is_empty() {
            return Vec::new();
        }
        // 同 trigger 至多编译一次：技能已在史里 ⇒ 这一轮不再追加（幂等）。
        if at.history.iter().any(|e| {
            e.kind == EVENT_MEMORY_ENCODED
                && e.payload.get("tag").and_then(|v| v.as_str()) == Some(SKILL_TAG)
                && e.payload.get("trigger").and_then(|v| v.as_str()) == Some(trigger.as_str())
        }) {
            return Vec::new();
        }
        let skill_id = skill_id_of(&trigger);
        vec![Contribution::new(FactDraft {
            key: format!("sk-{}-{skill_id}", at.anchor),
            kind: EVENT_MEMORY_ENCODED.to_string(),
            entity: Entity::Memory,
            verb: Verb::Opened,
            payload: json!({
                "content": content,
                "tag": SKILL_TAG,
                "skill_id": skill_id,
                "trigger": trigger,
                // 可见域：缺席 = 缺省 `thread_private`，召回预过滤会把技能挡在跨会话之外。
                // 这里写的是 `VisScope::Shared` 的线格式；本 crate 不依赖 core。
                "vis_scope": "shared",
            }),
            produced_by: Some(at.user_seq),
        })
        .with_actor(PRINCIPAL_USER)]
    }
}

// 链接期提交：宿主按 `(prio, owner)` 询问。
symbio_plugin_api::submit_contributor!(SkillCompile);

/// 最后一条 `session.config` 的 `skill_compile_enabled`（缺省 `false`）。
fn compile_enabled(history: &[Event]) -> bool {
    history
        .iter()
        .rev()
        .find(|e| e.kind == EVENT_SESSION_CONFIG)
        .and_then(|e| e.payload.get("skill_compile_enabled"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// 技能 id = trigger 的 FNV-1a 64。
///
/// 必须**跨进程稳定**：校准账按 `skill_id` 归并，用进程内的 `DefaultHasher` 每次
/// 重启都换 key，`uses` / `fallbacks` 就永远归不到一起——技能永远看起来「零使用」，
/// 阈值永远不触发。
fn skill_id_of(trigger: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in trigger.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// 与读方同一对函数、同一个上限（见 `decl::COMPILE_LIMIT` 的说明）。
fn truncate(text: &str) -> String {
    truncate_chars(text, COMPILE_LIMIT)
}

#[cfg(test)]
#[path = "contributor.test.rs"]
mod tests;
