//! `session/stats` 读数口的判据（[plan/12 §4](../../../../docs/plan/12-价值验收与基线埋点.md)）：
//!
//! 1. **复算**——出口的读数与直接 `apply` 逐字相等。同一切片、同一个 as-of：
//!    出口只能调已有的投影，不许另写一份统计（否则两套口径会静默漂移）。
//! 2. **反向**——从事实源里删掉一条事件，对应那一列必须跟着变（证明它真的在算，
//!    不是常数）。
//! 3. **纯读**——没有事实源时是「有据的零」，且**不得**为了读而创建文件。
//! 4. **读侧闸**——声明读方身份才判可见域：属主全量、非属主全零（批⑥）。

use super::read;
use crate::symbio_core::{
    check_all, checkpoint, cost_ledger, fallback_rate, slo_report, Budget, Entity, Event,
    EventEnvelope as _, EventWalStore, Seq, Store, Verb, EVENT_ASSISTANT_FALLBACK,
    EVENT_ASSISTANT_FINAL, EVENT_TASK_OPENED, EVENT_USER_MESSAGE,
};
use std::path::{Path, PathBuf};

/// 播种一个事实源：两轮（一轮成功 final、一轮兜底 fallback），共 4 格。
///
/// 两轮都声明 `tier = deep`——分母 / 分子 / 样本数因此都落在同一行上，
/// 断言不必先做档位归并。
fn seed(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("symbio-stats-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("临时目录");
    let wal = dir.join(super::super::paths::V2_WAL_FILE);
    let store = EventWalStore::open(&wal).unwrap();

    // 轮 0：开轮 → final（实测 1200ms，成功轮才计入时延样本）
    let u0 = Event::pending(
        "u0",
        EVENT_USER_MESSAGE,
        Entity::Turn,
        Verb::Opened,
        0,
        "user",
    )
    .with_payload(serde_json::json!({ "text": "你好", "tier": "deep", "turn_ref": "u-0" }));
    let s0 = store.append(u0).unwrap().value();
    store
        .append(
            Event::pending(
                "f0",
                EVENT_ASSISTANT_FINAL,
                Entity::Turn,
                Verb::Closed,
                0,
                "agent:main",
            )
            .with_produced_by(s0)
            .with_cost_ms(1200)
            .with_payload(serde_json::json!({ "text": "答" })),
        )
        .unwrap();

    // 轮 1：开轮 → 兜底（失败轮的耗时不算时延样本，只进兜底分子）
    let u1 = Event::pending(
        "u1",
        EVENT_USER_MESSAGE,
        Entity::Turn,
        Verb::Opened,
        1,
        "user",
    )
    .with_payload(serde_json::json!({ "text": "再问", "tier": "deep", "turn_ref": "u-1" }));
    let s1 = store.append(u1).unwrap().value();
    store
        .append(
            Event::pending(
                "fb1",
                EVENT_ASSISTANT_FALLBACK,
                Entity::Turn,
                Verb::Closed,
                1,
                "agent:main",
            )
            .with_produced_by(s1)
            .with_cost_ms(3000)
            .with_payload(serde_json::json!({ "why": "模型超时" })),
        )
        .unwrap();

    wal
}

/// 按 `event_id` 删掉一行（反向用例的手术刀）。
fn drop_line(wal: &Path, event_id: &str) {
    let raw = std::fs::read_to_string(wal).expect("读 WAL");
    let kept: Vec<&str> = raw
        .lines()
        .filter(|line| {
            let keep = serde_json::from_str::<serde_json::Value>(line)
                .map(|v| v.get("event_id").and_then(|x| x.as_str()) != Some(event_id))
                .unwrap_or(true);
            keep
        })
        .collect();
    std::fs::write(wal, format!("{}\n", kept.join("\n"))).expect("回写 WAL");
}

/// 出口读数 == 直接 `apply`（复算：同源，不是「看起来差不多」）。
#[test]
fn stats_recompute_equals_direct_apply() {
    let wal = seed("recompute");
    let got = read("s1", &wal, None, crate::symbio_core::authz::PRINCIPAL_MAIN).expect("读数");

    assert!(got.has_wal);
    assert_eq!(got.session_id, "s1");
    assert_eq!(
        got.wal,
        wal.display().to_string(),
        "证据链要给出可复算的路径"
    );

    // 读方复算：同一份切片、同一个 as-of，四列各自直接 apply。
    let store = EventWalStore::open_readonly(&wal).unwrap();
    let snap = store.range(Seq::new(0));
    let latency = slo_report()
        .apply(&snap, i64::MAX, Budget::generous())
        .value;
    let fallback = fallback_rate()
        .apply(&snap, i64::MAX, Budget::generous())
        .value;
    let cost = cost_ledger()
        .apply(&snap, i64::MAX, Budget::generous())
        .value;
    let ck = checkpoint()
        .apply(&snap, i64::MAX, Budget::generous())
        .value;
    let rs = serde_json::to_value(
        crate::symbio_core::readyset()
            .apply(&snap, i64::MAX, Budget::generous())
            .value,
    )
    .expect("就绪集序列化");
    assert_eq!(got.readyset, rs, "就绪集列同样复算（同切片、同 as-of）");

    assert_eq!(got.tiers.len(), 1, "种子数据只有一档");
    let row = &got.tiers[0];
    assert_eq!(row.tier, "deep");
    assert_eq!(row.turns, fallback.of("deep").turns);
    assert_eq!(row.fallbacks, fallback.of("deep").fallbacks);
    assert_eq!(row.rate, fallback.of("deep").rate());
    assert_eq!(row.p50, latency.of("deep").p50());
    assert_eq!(row.p95, latency.of("deep").p95());
    assert_eq!(row.p99, latency.of("deep").p99());
    assert_eq!(row.samples, latency.of("deep").count());
    assert_eq!(got.cost, serde_json::to_value(&cost).unwrap());
    assert_eq!(got.checkpoint, serde_json::to_value(&ck).unwrap());

    // 复算还得**算对**：期望值直接写在这里，不从出口自己身上取。
    assert_eq!((row.turns, row.fallbacks), (2, 1));
    assert_eq!(row.rate, 0.5);
    assert_eq!((row.samples, row.p95), (1, 1200), "兜底轮不进时延样本");
    assert_eq!(got.checkpoint["event_count"], 4);
    assert_eq!(got.cost["total_ms"], 4200, "两轮的 cost_ms 都入账");
}

/// 反向用例：删一格 ⇒ 对应那一列必须变（不是常数）。
#[test]
fn reverse_case_each_column_moves_when_the_wal_changes() {
    let wal = seed("reverse");
    assert_eq!(
        read("s1", &wal, None, crate::symbio_core::authz::PRINCIPAL_MAIN)
            .unwrap()
            .tiers[0]
            .samples,
        1
    );

    // 删收束格 ⇒ 时延样本列归零、P95 归零
    drop_line(&wal, "f0");
    let after = read("s1", &wal, None, crate::symbio_core::authz::PRINCIPAL_MAIN).unwrap();
    assert_eq!(after.tiers[0].samples, 0, "删掉唯一的 final ⇒ 样本 1 → 0");
    assert_eq!(after.tiers[0].p95, 0, "无样本 ⇒ 分位数为 0（空档位不报警）");
    assert_eq!(after.tiers[0].turns, 2, "开轮格还在 ⇒ 分母不受收束格影响");
    assert_eq!(after.checkpoint["event_count"], 3, "断点是事实计数");

    // 删开轮格 ⇒ 兜底率的**分母**必须变（2 → 1），比率随之变成 1.0
    drop_line(&wal, "u0");
    let after = read("s1", &wal, None, crate::symbio_core::authz::PRINCIPAL_MAIN).unwrap();
    assert_eq!(
        after.tiers[0].turns, 1,
        "删掉一个 user.message ⇒ 分母 2 → 1"
    );
    assert_eq!(after.tiers[0].fallbacks, 1, "兜底格与被删的开轮格无关");
    assert_eq!(after.tiers[0].rate, 1.0, "分母变了 ⇒ 比率跟着变");
    assert_eq!(after.checkpoint["event_count"], 2);
}

/// 不变量列随事实源反向变化（[04 §3.1 批④](../../../../docs/plan/04-工程落地.md)）：
/// 真实形状的两轮全绿 ⇒ 空；删掉收束格 ⇒ C4 必须报出那个缺口。
#[test]
fn invariants_move_when_the_wal_changes() {
    let wal = seed("invariants");
    let clean = read("s1", &wal, None, crate::symbio_core::authz::PRINCIPAL_MAIN).unwrap();

    // 复算：出口的清单与直接 `check_all` 逐字相等（口径只活在 core，出口不复判）。
    let store = EventWalStore::open_readonly(&wal).unwrap();
    let snap = store.range(Seq::new(0));
    assert_eq!(
        clean.invariants,
        serde_json::to_value(check_all(&snap)).unwrap()
    );
    assert_eq!(
        clean.invariants,
        serde_json::json!([]),
        "两轮都收束、档位已声明且预算内 ⇒ 不变量清单为空（首日不假红）"
    );

    // 删轮 0 的收束格 ⇒ 轮 0 被轮 1 越过 ⇒ C4 报未收束；行删了 ⇒ C1 报 seq 跳号。
    drop_line(&wal, "f0");
    let list = read("s1", &wal, None, crate::symbio_core::authz::PRINCIPAL_MAIN)
        .unwrap()
        .invariants;
    let list = list.as_array().expect("清单是数组");
    assert_eq!(list.len(), 2, "{list:?}");
    assert!(
        list.iter()
            .any(|v| v["why"].as_str().unwrap_or_default().contains("未收束")),
        "C4：删掉收束格 ⇒ 该轮被判未收束：{list:?}"
    );
    assert!(
        list.iter().any(|v| v["event_id"] == "u0"),
        "违规锚在开轮那条（缺口本身，不是切片末尾某条无关事件）：{list:?}"
    );
    assert!(
        list.iter()
            .any(|v| v["why"].as_str().unwrap_or_default().contains("seq")),
        "C1：删掉中间一行 ⇒ 后续 seq 跳号：{list:?}"
    );
}

/// 没有事实源 = 有据的零，且读方不许创建文件（真·只读）。
#[test]
fn missing_wal_is_an_honest_zero_and_creates_nothing() {
    let dir = std::env::temp_dir().join(format!("symbio-stats-{}-missing", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("临时目录");
    let wal = dir.join(super::super::paths::V2_WAL_FILE);

    let got = read("s1", &wal, None, crate::symbio_core::authz::PRINCIPAL_MAIN)
        .expect("没有事实源不是错误");
    assert!(!got.has_wal);
    assert!(got.tiers.is_empty());
    assert_eq!(got.checkpoint["event_count"], 0);
    assert_eq!(got.cost["total_ms"], 0);
    assert_eq!(got.invariants, serde_json::json!([]), "空事实源 ⇒ 断言全绿");
    assert!(
        !wal.exists(),
        "读方不得为了读而创建事实源文件——那会把「没跑过一轮」变成「跑过一轮空的」"
    );
}

/// 读侧闸（[04 §3.1 批⑥](../../../../docs/plan/04-工程落地.md)）：声明**属主**
/// 身份 ⇒ 与不声明逐字相同——闸的存在不是为了把属主挡在外面。
#[test]
fn declaring_the_owner_reads_everything() {
    let wal = seed("gate-owner");
    let plain =
        read("s1", &wal, None, crate::symbio_core::authz::PRINCIPAL_MAIN).expect("本机默认读数");
    let as_owner = read(
        "s1",
        &wal,
        Some("user"),
        crate::symbio_core::authz::PRINCIPAL_MAIN,
    )
    .expect("属主读数");
    assert_eq!(
        as_owner, plain,
        "属主本人的读数与本机默认逐字一致（声明不改变她能看什么）"
    );
    assert_eq!(as_owner.checkpoint["event_count"], 4, "4 格全见");
}

/// 读侧闸的反向：**非属主 / 矩阵外主体读数为空**（fail-closed）。
///
/// `has_wal` 仍为真 ⇒ 「有源但不给你看」与 `missing_wal` 那条「没有源」可分辨；
/// 不变量列同为空——没看见事实，就没有属于你的缺口可报。
#[test]
fn a_non_owner_reads_nothing_at_all() {
    let wal = seed("gate-nonowner");
    for viewer in ["agent:main", "agent:ghost"] {
        let got = read(
            "s1",
            &wal,
            Some(viewer),
            crate::symbio_core::authz::PRINCIPAL_MAIN,
        )
        .expect("读数");
        assert!(got.has_wal, "{viewer}: 事实源存在，只是不给你看");
        assert!(
            got.tiers.is_empty(),
            "{viewer}: 四列应为空：{:?}",
            got.tiers
        );
        assert_eq!(
            got.checkpoint["event_count"], 0,
            "{viewer}: 断点列的事件数为 0"
        );
        assert_eq!(got.cost["total_ms"], 0, "{viewer}: 成本列为 0");
        assert_eq!(
            got.invariants,
            serde_json::json!([]),
            "{viewer}: 没有可见事实 ⇒ 无可报的违规"
        );
        // 声誉列随四列一起没：**全有全无**，不按字段裁——按字段裁会留下
        // 「四列不给你、声誉给你」这种没有任何理由的半开半掩。
        assert_eq!(
            got.reputation,
            serde_json::json!({}),
            "{viewer}: 没有可见事实 ⇒ 不替你看不见的主体报数"
        );
    }
}

/// 声誉列（[plan/12 批 2](../../../../docs/plan/12-价值验收与基线埋点.md)，S6 第 15 步的读侧）：
/// `own` 取**本会话主体**，`by_principal` 把这份事实源里的主体原样摊开（`own` 在其中，
/// 不另造口径）；判据只有一处——core 的 `reputation` 投影，本处只取数 + 排版。
#[test]
fn reputation_column_reports_own_and_by_principal() {
    let wal = seed("rep");
    let store = EventWalStore::open(&wal).unwrap();
    let snap = store.range(Seq::new(0));
    let user_seq = snap
        .iter()
        .find(|e| e.kind == EVENT_USER_MESSAGE)
        .expect("用户格")
        .seq()
        .map(|s| s.value())
        .expect("已入格");
    // 一次守约的代际立约（写方 `v2_bridge::record_to_wal` 落的就是这三格）。
    for e in crate::symbio_core::commitment_events(
        "c-rep",
        crate::symbio_core::authz::PRINCIPAL_MAIN,
        crate::symbio_core::authz::PRINCIPAL_USER,
        "把这段复查看完",
        true,
        "",
        user_seq,
    ) {
        store.append(e).expect("承诺入格");
    }

    let got = read("s1", &wal, None, crate::symbio_core::authz::PRINCIPAL_MAIN).expect("读数");
    assert_eq!(
        got.reputation["own"],
        serde_json::json!({
            "principal": crate::symbio_core::authz::PRINCIPAL_MAIN,
            "offered": 1,
            "kept": 1,
            "broken": 0,
            "score": 1,
        }),
        "own = 本会话主体的条目；score = 平凡打分（守约 − 违约）"
    );
    assert_eq!(
        got.reputation["by_principal"][crate::symbio_core::authz::PRINCIPAL_MAIN],
        got.reputation["own"],
        "own 就在 by_principal 里——同一张表的两个视角，不是两套数"
    );
    assert!(
        got.invariants.as_array().is_some_and(|l| l.is_empty()),
        "承诺四格不新增违规：{:?}",
        got.invariants
    );

    // 无读权限 ⇒ **整列为空对象**，不是零值条目（零值 = 替一个你看不见的主体报数）。
    let denied = read(
        "s1",
        &wal,
        Some(crate::symbio_core::authz::PRINCIPAL_MAIN),
        crate::symbio_core::authz::PRINCIPAL_MAIN,
    )
    .expect("读数");
    assert_eq!(denied.reputation, serde_json::json!({}));
}

/// 就绪集列（S7 步 16，[04 §3.1 批⑨]）：候选集入列 + **复算同源** + 读侧闸（反向）。
///
/// `may_read = false ⇒ {ready: []}` 与四列同款（空切片 = 有源但不给你看），
/// 不是声誉那列的空对象——「没有任务」与「不给你看」由 `has_wal` 与列的上下游分辨。
#[test]
fn readyset_column_lists_candidates_and_obeys_the_read_gate() {
    let dir = std::env::temp_dir().join(format!("symbio-stats-rs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("临时目录");
    let wal = dir.join(super::super::paths::V2_WAL_FILE);
    let store = EventWalStore::open(&wal).expect("WAL");
    for (id, deps) in [("t1", vec![]), ("t2", vec!["t1"])] {
        store
            .append(
                Event::pending(
                    id.to_string(),
                    EVENT_TASK_OPENED,
                    Entity::Task,
                    Verb::Opened,
                    0,
                    crate::symbio_core::authz::PRINCIPAL_MAIN,
                )
                .with_produced_by(0)
                .with_payload(serde_json::json!({
                    "task_id": id,
                    "depends_on": deps,
                    "goal": id,
                })),
            )
            .expect("任务开格");
    }

    let got = read(
        "s-rs",
        &wal,
        None,
        crate::symbio_core::authz::PRINCIPAL_MAIN,
    )
    .expect("读数");
    assert_eq!(
        got.readyset["ready"].as_array().map(|a| a.len()),
        Some(1),
        "t1 无依赖（就绪）、t2 等 t1（未终态）：{}",
        got.readyset
    );
    assert_eq!(got.readyset["ready"][0]["task_id"], "t1");

    // 复算：同切片、同一个 as-of——出口只能调投影，不许另写一份统计。
    let ro = EventWalStore::open_readonly(&wal).expect("WAL 只读");
    let snap = ro.range(Seq::new(0));
    let direct = serde_json::to_value(
        crate::symbio_core::readyset()
            .apply(&snap, i64::MAX, Budget::generous())
            .value,
    )
    .expect("序列化");
    assert_eq!(got.readyset, direct, "就绪集列必须复算相等");

    // 反向：读侧闸关 ⇒ 空切片 ⇒ 连不变量列也空（有源但不给你看）。
    let denied = read(
        "s-rs",
        &wal,
        Some(crate::symbio_core::authz::PRINCIPAL_MAIN),
        crate::symbio_core::authz::PRINCIPAL_MAIN,
    )
    .expect("读数");
    assert_eq!(denied.readyset, serde_json::json!({ "ready": [] }));
    assert_eq!(denied.invariants, serde_json::json!([]));
    assert!(denied.has_wal, "has_wal 独立于可见域：文件在，只是不给你看");
}
