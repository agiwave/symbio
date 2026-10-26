//! 控制面主体：淘汰回路（[roadmap/S13](../../../../docs/003-design/043-多智能体社会.md "DE-043-多智能体社会")）。
//!
//! 抢占判定归 `plugins/session/transcript/inbox`、外部执行闸门归
//! `plugins/loop/tools/dispatch/gates`（同插件内两处消费，D-190）；留在这里的
//! [`EliminationDecider`] 是唯一真正跨 owner 的策略——写侧裁决在 loop 轮末、
//! 读侧翻译（[`EliminationDecider::verdict_grants_changes`]）在 session 工具层，
//! 配对映射因此必须留核（D-187 第 1 条）。

use crate::symbio_core::authz::PRINCIPAL_ARBITER;
use crate::symbio_core::governance::GrantsChange;
use crate::symbio_core::projection::reputation::ReputationEntry;
use crate::symbio_core::{pattern_for_principal, ActorSpec, Event, Scope};

/// 淘汰判定者（S13 §3，[roadmap/S13 §2/§3](../../../../docs/003-design/043-多智能体社会.md "DE-043-多智能体社会")）：
/// 声誉投影低于阈值 ⇒ 收回 `assign.work` 的**裁决侧**。
///
/// 与已归 owner 的抢占判定同形而同用：判定是**纯函数**（读视图出结论），落格是
/// **事件**（经 Store，无直连——I1）。三件事各住一边：
///
/// | 事 | 住哪 |
/// |---|---|
/// | 判据（阈值、低于即淘汰） | [`Self::should_eliminate`]（纯函数） |
/// | 那一格事件（`verdict × asserted`） | [`Self::elimination_event`] / [`Self::restoration_event`] |
/// | 什么时候判、对谁判 | 调用方（`plugins/loop/derived_facts` 的轮末步） |
///
/// ## 为什么 actor 是仲裁者而不是被淘汰主体
///
/// 与抢占判定的先例同一条理由：事件的 `actor` 是**判定者**——
/// `session/arbiter`（授权表里唯一持 `assert.verification` 的一行，C15 构造期保证
/// 它不持 `define.work` / `produce.artifact`，自成对安全）。被淘汰的主体是载荷里的
/// `subject`（数据），不是作者。把两者混同会让「谁做的裁决」在事实源里说不出来。
///
/// ## 判据的输入是投影不是自报
///
/// `score` 来自 `reputation` 投影（从 `commitment.*` 事件派生），主体**自报**的
/// 声誉事件进不了投影（断言 2：只认派生）——于是「自己报个好分数免于淘汰」
/// 在构造上不成立，不是靠纪律拦住。
///
/// ## 结构性的诚实缺口：自动恢复不可能
///
/// 淘汰后唯一能涨声誉的路径是**成功委派**（`agent_run` 守约 ⇒ `commitment.released`），
/// 而委派恰是被收回的 `assign.work` 那个动作——闸门会把它拒掉。于是恢复不可能
/// **自动**发生：申诉 / 人工复位必须经同一条事件通道写 `restore`（机制半边在
/// [`Self::restoration_event`]，由撤销回路单测证）。生产侧的申诉触发器是
///
/// 之后的事，此处不造一个「假装能自动恢复」的回路。
pub struct EliminationDecider;

impl EliminationDecider {
    /// 淘汰阈值（平凡值）：声誉分**低于**它即淘汰。
    ///
    /// 平凡打分 = 守约 − 违约（`plain_score`），阈值 0 ⇒ **任何未配平的违约**
    /// （净分为负）就触发。它是**参数不是架构**（S13 §4 的平凡值表同口径）：
    /// 换阈值不改本判定的形状；`should_eliminate` 是唯一读它的地方。
    ///
    /// **为什么不分档（项 13 的复核结论）**：分档要的是**分数上的梯度**，
    /// 而 `plain_score` 的取值只有计数差（一次立约了结 = ±1），没有任何一维能撑起
    /// 「警告档 / 观察档 / 收回档」的**后果差异**——今天唯一的后果就是收回
    /// `assign.work` 这一格（[D-108](../../../../docs/003-design/002-架构总览.md#d-108-淘汰是单向门恢复的生产触发器缺席记为边界自动恢复在本判据下不可能 "DE-002-架构总览") 记的
    /// 单向门正是它的对面）。多分一档 = 多一个没有消费者的后果档位，而收回不可自动
    /// 恢复，档位只会把「何时被永久锁死」变成一处更难看清的参数。真要有梯度，先要有
    /// 第二种后果（分档的对象是后果，不是分数），那属 S13 社会面的裁决，不在此顺手加。
    pub const THRESHOLD: i64 = 0;

    /// S13 仲裁者**行**（`ActorSpec` 生产构造点，与抢占判定者同批同形）。
    ///
    /// `{ principal: session/arbiter, pattern: decider, capabilities: [assert.verification],
    /// budget_ms: 80, scope: root }`——五个字段全部**取实**：principal / capabilities
    /// 同源于授权表那一行（落地的 `PRINCIPAL_ARBITER` 窄行），pattern 按
    /// [`pattern_for_principal`] 派生（仲裁者无 `reply.*` 也无 `produce.artifact`
    /// ⇒ `Decider`，三新角色同一答案——**模式闭集不变宽**，exit ① 的另一半），
    /// budget = 反射档（淘汰判定是规则折叠，毫秒级），scope = root。
    pub fn spec(&self) -> ActorSpec {
        let principal = PRINCIPAL_ARBITER.to_string();
        ActorSpec {
            pattern: pattern_for_principal(&principal),
            capabilities: crate::symbio_core::authz::production_matrix().grant_names_of(&principal),
            budget_ms: crate::symbio_core::adapters::LatencyTier::Reflex.budget_ms(),
            scope: Scope::Root,
            principal,
        }
    }

    /// 纯判定：该主体的声誉条目是否低于淘汰阈值。
    ///
    /// 只问分数——「对谁判」（持 `assign.work` 的主体）、「判过没有」（无既有
    /// verdict）都由调用方定：判定者不持有授权表以外的任何判据（与
    /// `plugins/loop/tools/dispatch/gates::CircuitBreaker::gate` 的 `authorized`
    /// 出参同一条纪律）。
    pub fn should_eliminate(&self, entry: &ReputationEntry) -> bool {
        entry.score < Self::THRESHOLD
    }

    /// 淘汰事件（`verdict × asserted`，`change = "revoke"`）——可审计可撤销的那一格。
    ///
    /// 载荷六字段全为**数据**：`subject`（被收回的主体）、`capability`
    /// （收回的能力，恒为 `assign.work`——S13 §2 淘汰行的字面机制）、`change`、
    /// `score` / `threshold`（判据的当时值，审计时要能复算）、`why`（人话理由）。
    /// `source_seq` = 溯源锚（I2：断言类事件必须非空）。
    pub fn elimination_event(&self, subject: &str, score: i64, source_seq: u64) -> Event {
        Event::pending(
            format!("v-elim-{subject}-{source_seq}"),
            crate::symbio_core::event::EVENT_VERDICT_ASSERTED,
            crate::symbio_core::event::Entity::Verdict,
            crate::symbio_core::event::Verb::Asserted,
            0,
            PRINCIPAL_ARBITER,
        )
        .with_produced_by(source_seq)
        .with_payload(serde_json::json!({
            "subject": subject,
            "capability": crate::symbio_core::authz::CAP_ASSIGN_WORK,
            "change": "revoke",
            "score": score,
            "threshold": Self::THRESHOLD,
            "why": format!("声誉分 {score} 低于淘汰阈值 {}（守约 − 违约，只认派生）", Self::THRESHOLD),
        }))
    }

    /// 恢复事件（`verdict × asserted`，`change = "restore"`）——淘汰的逆操作。
    ///
    /// 与 [`Self::elimination_event`] **同格同键形状**：恢复不是删历史（Log 永不删），
    /// 是再写一条 `restore`，last-wins 折叠出当前状态（`GrantsOverlay`）。申诉成立 /
    /// 人工复位走的都是这一格；`why` 由调用方给（申诉受理、人工复核……）。
    ///
    /// **为什么它今天没有生产调用方**（诚实缺口，不是漏接）：淘汰后唯一能涨声誉的
    /// 路径是成功委派，而委派恰是被收回的 `assign.work` 那个动作——闸门会拒掉它，
    /// 于是恢复不可能**自动**发生，必须有一个外部触发（申诉 / 人工复位）。这条缺席
    /// 已在 [D-108](../../../../docs/003-design/002-架构总览.md#d-108-淘汰是单向门恢复的生产触发器缺席记为边界自动恢复在本判据下不可能 "DE-002-架构总览") 记为**边界**并写明重开
    /// 条件；触发器落地时摘掉这个属性即可，载荷口径不随之改动。
    /// 把它当成「已实现的恢复能力」是本条要拦的读法：形状是真的，写方是缺席的。
    #[cfg(test)]
    pub fn restoration_event(&self, subject: &str, why: &str, source_seq: u64) -> Event {
        Event::pending(
            format!("v-rest-{subject}-{source_seq}"),
            crate::symbio_core::event::EVENT_VERDICT_ASSERTED,
            crate::symbio_core::event::Entity::Verdict,
            crate::symbio_core::event::Verb::Asserted,
            0,
            PRINCIPAL_ARBITER,
        )
        .with_produced_by(source_seq)
        .with_payload(serde_json::json!({
            "subject": subject,
            "capability": crate::symbio_core::authz::CAP_ASSIGN_WORK,
            "change": "restore",
            "why": why,
        }))
    }

    /// `verdict.asserted` 事件流水 → [`GrantsChange`] 序列：**载荷 ↔ 变更的映射**。
    ///
    /// 淘汰回路的读侧半边：闸门（`plugins/session` 的工具执行层）拿不到裁决者本体，
    /// 它有的是事件切片——本函数就是「切片 → 覆盖层」之间唯一的那步翻译。
    /// 与 [`commitment_events`] 同形态（写侧的成格函数 / 读侧的翻译函数各住一边）。
    ///
    /// 认不出的 `change` 值（含缺失）⇒ **跳过该条**：不猜「多半是撤销吧」——
    /// 把笔误读成一次撤销等于把仲裁裁决静默改判，fail-closed 的方向是**不动**。
    pub fn verdict_grants_changes(events: &[Event]) -> Vec<GrantsChange> {
        let mut out = Vec::new();
        for e in events {
            if e.kind != crate::symbio_core::event::EVENT_VERDICT_ASSERTED {
                continue;
            }
            let Some(subject) = e.payload.get("subject").and_then(|v| v.as_str()) else {
                continue;
            };
            let capability = e
                .payload
                .get("capability")
                .and_then(|v| v.as_str())
                .unwrap_or(crate::symbio_core::authz::CAP_ASSIGN_WORK);
            match e.payload.get("change").and_then(|v| v.as_str()) {
                Some("revoke") => out.push(GrantsChange::Revoke {
                    principal: subject.to_string(),
                    capability: capability.to_string(),
                }),
                Some("restore") => out.push(GrantsChange::Restore {
                    principal: subject.to_string(),
                    capability: capability.to_string(),
                }),
                _ => {}
            }
        }
        out
    }
}
