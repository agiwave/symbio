//! 收束贡献：淘汰裁决 —— 声誉分低于阈值 ⇒ 落一条 `verdict.asserted`（DE-043 §2/§3）。
//!
//! ## 它补的是哪一格
//!
//! 声誉台账（[`crate::decl::REPUTATION_LEDGER`]）折的是**数**，判决住在读它的这一侧。
//! 本贡献者只做三件事：按名字读账、按部署表与既有裁决筛主体、把裁决交成草稿。落格
//! 归属不在这里——`verdict.asserted` 的 `EventDecl` 声明作者是仲裁者
//! （`session/arbiter`），信封 `actor` 由声明表给定，执行体无权指定（D-339 / D-252）。
//!
//! ## 三条筛选口径（与搬出前逐字相同）
//!
//! - **持 `assign.work` 的部署主体才在淘汰面**：绑定登记是运行期身份事实，不构成
//!   「部署授予过 `assign.work`」；未部署的 `agent:<id>` 不在表里 ⇒ 不淘汰。
//! - **净分为负才淘汰**：阈值是平凡值 0（`score = 守约 − 违约`），只认派生、不认自报；
//! - **幂等**：史里 last-wins 折叠出「这个主体的 `assign.work` 已被收回」⇒ 不再落第二条。
//!   恢复事件（`change = "restore"`）同样被折叠，将来有申诉写方时不需要改这里。
//!
//! ## 写侧闸（fail-closed）
//!
//! 落裁决需要**仲裁者**持 `assert.verification`：缺能力 ⇒ 一条都不写。没有资格的判定
//! 不该留下事实。读账失败 / 形状读不懂同样交空集——淘汰是收回权威的动作，必须建立在
//! 一份读得到的账上；缺席时的默认值是「不动 grants」。

use std::collections::BTreeMap;

use serde_json::{json, Value};
use symbio_plugin_api::{
    principal_of, production_matrix, Contribution, Contributor, Entity, Event, FactDraft, Settling,
    Verb, CAP_ASSERT_VERIFICATION,
};

use crate::decl::{ledger, EVENT_VERDICT_ASSERTED, OWNER};

/// 被收回的能力（DE-043 §2 淘汰行的字面机制）。
///
/// 名字住 `plugin-api` 的词汇面（D-364），门面转出 `symbio-sdk` 的同一常量；
/// 本包仍不依赖兄弟插件，也不需要直接依赖 sdk（E-009）。
use symbio_plugin_api::vocab::CAP_ASSIGN_WORK;

/// 淘汰阈值：平凡值 0（`score = 守约 − 违约`，净分为负即淘汰）。
const THRESHOLD: i64 = 0;

pub(crate) struct EliminationVerdicts;

impl Contributor for EliminationVerdicts {
    fn owner(&self) -> &'static str {
        OWNER
    }

    fn contribute(&self, at: &Settling<'_>) -> Vec<Contribution> {
        // 写侧闸：裁决的作者行必须持 `assert.verification`，缺能力一条都不写。
        let arbiter = principal_of(Some("arbiter"));
        if !production_matrix().can_write_name(&arbiter, CAP_ASSERT_VERIFICATION) {
            return Vec::new();
        }
        // 读自己的视图：本包就是 owner，直接调折叠函数，不经过 `shared_views` 生成物
        // （D-387：报表登记才是公开动作）。as-of 与读口一致：落盘即已发生。
        let ledger = ledger(at.history, i64::MAX);
        let Some(rows) = ledger.get("by_principal").and_then(Value::as_object) else {
            return Vec::new();
        };
        // 既有裁决 → 覆盖层：只问「这个主体的这项能力是否已被收回」。
        let revoked = revoked_capabilities(at.history);
        let matrix = production_matrix();
        let mut out = Vec::new();
        for (principal, entry) in rows {
            let Some(score) = entry.get("score").and_then(Value::as_i64) else {
                // 那一行读不出 `score` = 版本对不上：跳过它，不动这个主体。
                continue;
            };
            if !matrix.deployed_can_write_name(principal, CAP_ASSIGN_WORK) {
                continue;
            }
            if score >= THRESHOLD {
                continue;
            }
            if revoked
                .get(principal)
                .is_some_and(|caps| caps.iter().any(|cap| cap == CAP_ASSIGN_WORK))
            {
                continue;
            }
            out.push(Contribution::new(verdict_draft(
                principal,
                score,
                at.user_seq,
            )));
        }
        out
    }
}

/// 裁决格（`verdict × asserted`）→ 各主体当前被收回的能力集（last-wins）。
///
/// 认**格子**而不是名字：`verdict × asserted` 上只有裁决者说过的话。`change` 认不出
/// （含缺失）⇒ 跳过该条，不猜「多半是撤销吧」——把笔误读成一次撤销等于静默改判。
fn revoked_capabilities(events: &[Event]) -> BTreeMap<String, Vec<String>> {
    let mut state: BTreeMap<(String, String), bool> = BTreeMap::new();
    for e in events {
        if !(e.entity == Entity::Verdict && e.verb == Verb::Asserted) {
            continue;
        }
        let Some(subject) = e.payload.get("subject").and_then(Value::as_str) else {
            continue;
        };
        let capability = e
            .payload
            .get("capability")
            .and_then(Value::as_str)
            .unwrap_or(CAP_ASSIGN_WORK);
        let revoked = match e.payload.get("change").and_then(Value::as_str) {
            Some("revoke") => true,
            Some("restore") => false,
            _ => continue,
        };
        state.insert((subject.to_string(), capability.to_string()), revoked);
    }
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for ((subject, capability), revoked) in state {
        if revoked {
            out.entry(subject).or_default().push(capability);
        }
    }
    out
}

/// 一条待落格的裁决草稿（信封 `turn` / `ts` / `actor` 由宿主按声明表盖）。
fn verdict_draft(subject: &str, score: i64, source_seq: u64) -> FactDraft {
    FactDraft {
        key: format!("v-elim-{subject}-{source_seq}"),
        kind: EVENT_VERDICT_ASSERTED.to_string(),
        entity: Entity::Verdict,
        verb: Verb::Asserted,
        payload: json!({
            "subject": subject,
            "capability": CAP_ASSIGN_WORK,
            "change": "revoke",
            "score": score,
            "threshold": THRESHOLD,
            "why": format!("声誉分 {score} 低于淘汰阈值 {THRESHOLD}（守约 − 违约，只认派生）"),
        }),
        produced_by: Some(source_seq),
    }
}

// 链接期提交：宿主按 `(prio, owner)` 询问。
symbio_plugin_api::submit_contributor!(EliminationVerdicts);
