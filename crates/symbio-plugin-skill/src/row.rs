//! 行执行体：`skill.recall` —— 本幕召回的技能逐条过闸（DE-041 §5 / D-241）。
//!
//! ## 一次判定，两个后果
//!
//! 每条技能**只判一次**，后果有两个且必须同源：
//!
//! - **观测**：落一条 `memory.calibrated{skill_id, fallback}`——校准账的数据源。
//!   没有它，回退永远不会发生（零使用按 1.0 算，技能再错也一直被用）。
//! - **命中**：本轮发言逐字等于这条技能的触发串、且开关开着、且这条技能没被判回退
//!   ⇒ [`RowStep::Answer`]，以技能正文收束（零模型往返，D-260）。
//!
//! 分两处各判一遍必然漂移成「落观测的那条」与「拿去作答的那条」不是同一条。
//!
//! ## 判据全部从事实读
//!
//! - 债键 `(recall, stage)` 来自宿主公开的 `turn.recalled` 视图（`recall` = 技能事件的
//!   seq，`stage` = 召回阶段格的 seq）；
//! - 技能正文 / 触发串 / id 从 `history` 里同 seq 的 `memory.encoded` 取；
//! - 置信度从 `memory.calibrated` 折（`calibration_for`，与回退视图同一份实现）；
//! - 开关从最后一条 `session.config` 读（D-252：能力面不拿配置包）；
//! - 本幕是否续写幕从召回阶段格的 `resumed` 读（续写幕不走快路：落格形状会撞幂等键）。
//!
//! ## 找不到技能事件时不观测、只关账
//!
//! 跨会话编译的技能与巩固产物在本会话没有可归并的 `skill_id`（前者的校准账不在
//! 本会话，后者合并后已不带 id）⇒ 原样留在提示词里当背景事实，**不假装判过**。
//! 但债必须关：落一条 `hit = false` 的判定，环才不会在同一笔债上空转。

use serde_json::{json, Value};
use symbio_plugin_api::events::{
    EVENT_MEMORY_CALIBRATED, EVENT_MEMORY_ENCODED, EVENT_SESSION_CONFIG, EVENT_USER_MESSAGE,
};
use symbio_plugin_api::rows::{AnswerStep, RowHandler, RowRequest, RowStep};
use symbio_plugin_api::{calibration_for, truncate_chars, Entity, Event, FactDraft, Verb};

use super::decl::{
    COMPILE_LIMIT, CONFIDENCE_THRESHOLD, OWNER, SKILL_ANSWERED, SKILL_RECALL, SKILL_TAG,
};

pub(crate) struct SkillRecall;

impl RowHandler for SkillRecall {
    fn owner(&self) -> &'static str {
        OWNER
    }

    fn row(&self) -> &'static str {
        SKILL_RECALL
    }

    fn writes(&self) -> &'static [&'static str] {
        // 观测格 + 判定格。两者同一次判定产出，缺一个都不能关账。
        &[EVENT_MEMORY_CALIBRATED, SKILL_ANSWERED]
    }

    fn answers(&self) -> bool {
        // 命中时零往返作答；宿主据此豁免它代落的 `turn.stage{stage:"reflex"}`。
        true
    }

    fn run(&self, req: &RowRequest<'_>) -> RowStep {
        let (Some(recall), Some(stage)) = (
            req.key_of("recall").and_then(|v| v.parse::<u64>().ok()),
            req.key_of("stage").and_then(|v| v.parse::<u64>().ok()),
        ) else {
            // 债键由视图给出，缺席只在装配面出错时出现；关不了账时交空集，
            // 由派发点的步数上界兜底（不在这里造一个假的判定）。
            return RowStep::Facts(Vec::new());
        };

        // 这条 seq 的技能事实：本会话编译的才带 `skill_id` 与 `trigger`。
        let encoded = req.history.iter().find(|e| {
            e.seq.map(|s| s.value()) == Some(recall)
                && e.kind == EVENT_MEMORY_ENCODED
                && e.payload.get("tag").and_then(Value::as_str) == Some(SKILL_TAG)
        });
        // 跨会话遗留的条目缺 `skill_id` / `trigger`（巩固产物根本没有这两个字段）：
        // 判不了 ⇒ 不观测、只关账，原样留在提示词里当背景事实。
        let Some((skill_id, trigger, content)) = encoded.and_then(|e| {
            Some((
                e.payload.get("skill_id").and_then(Value::as_str)?,
                e.payload.get("trigger").and_then(Value::as_str)?,
                e.payload.get("content").and_then(Value::as_str)?,
            ))
        }) else {
            return RowStep::Facts(vec![answered(req, recall, stage, "", false)]);
        };

        // 置信度与回退视图同一份折叠（`calibration_for`）：阈值只有一处。
        let confidence = calibration_for(EVENT_MEMORY_CALIBRATED)
            .read_all(req.history)
            .value
            .of(skill_id)
            .confidence();
        let fallback = confidence < CONFIDENCE_THRESHOLD;

        let mut drafts = vec![FactDraft {
            key: format!("skc-{}-{stage}-{recall}", req.anchor),
            kind: EVENT_MEMORY_CALIBRATED.to_string(),
            entity: Entity::Memory,
            verb: Verb::Asserted,
            payload: json!({ "skill_id": skill_id, "fallback": fallback }),
            produced_by: Some(req.user_seq),
        }];

        let hit = !fallback
            && fast_path_enabled(req.history)
            && !resumed(req, stage)
            && last_user_text(req.facts).is_some_and(|text| truncate(text.trim()) == trigger);
        if hit {
            drafts.push(answered(req, recall, stage, skill_id, true));
            return RowStep::Answer(AnswerStep {
                text: content.to_string(),
                facts: drafts,
            });
        }
        drafts.push(answered(req, recall, stage, skill_id, false));
        RowStep::Facts(drafts)
    }
}

// 链接期提交：本实现随本 crate 进入二进制，宿主按行名选它。
symbio_plugin_api::submit_row!(SkillRecall);

/// 这一条技能的判定格（行的出口）。
fn answered(req: &RowRequest<'_>, recall: u64, stage: u64, skill_id: &str, hit: bool) -> FactDraft {
    FactDraft {
        key: format!("ska-{}-{stage}-{recall}", req.anchor),
        kind: SKILL_ANSWERED.to_string(),
        entity: Entity::Domain,
        verb: Verb::Asserted,
        payload: json!({
            "recall": recall,
            "stage": stage,
            "skill_id": skill_id,
            "hit": hit,
        }),
        produced_by: Some(req.user_seq),
    }
}

/// 本轮是否走续写幕：召回阶段格自己记的那一格（`resumed`）。
fn resumed(req: &RowRequest<'_>, stage: u64) -> bool {
    req.history
        .iter()
        .find(|e| e.seq.map(|s| s.value()) == Some(stage))
        .and_then(|e| e.payload.get("resumed"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// 最后一条 `session.config` 的 `skill_fast_path`（缺省 `false`——与出厂值同一方向）。
fn fast_path_enabled(history: &[Event]) -> bool {
    history
        .iter()
        .rev()
        .find(|e| e.kind == EVENT_SESSION_CONFIG)
        .and_then(|e| e.payload.get("skill_fast_path"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// 本轮用户发言（`req.facts` 是本轮切片，开口格在第一条）。
fn last_user_text(facts: &[Event]) -> Option<&str> {
    facts
        .iter()
        .rev()
        .find(|e| e.kind == EVENT_USER_MESSAGE)
        .and_then(|e| e.payload.get("text"))
        .and_then(Value::as_str)
}

/// 与写方同一对函数、同一个上限（见 `decl::COMPILE_LIMIT` 的说明）。
fn truncate(text: &str) -> String {
    truncate_chars(text, COMPILE_LIMIT)
}

#[cfg(test)]
#[path = "row.test.rs"]
mod tests;
