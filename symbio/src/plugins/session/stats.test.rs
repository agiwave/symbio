//! `session/stats` 读数口的判据（[plan/12 §4](../../../../docs/goal/12-价值验收与基线埋点.md "docs/goal/12-价值验收与基线埋点")）：
//!
//! 1. **复算**——出口的读数与直接 `apply` 逐字相等。同一切片、同一个 as-of：
//!    出口只能调已有的投影，不许另写一份统计（否则两套口径会静默漂移）。
//! 2. **反向**——从事实源里删掉一条事件，对应那一列必须跟着变（证明它真的在算，
//!    不是常数）。
//! 3. **纯读**——没有事实源时是「有据的零」，且**不得**为了读而创建文件。
//! 4. **读侧闸**——声明读方身份才判可见域：属主全量、非属主全零（批⑥）。

use super::read;
use crate::symbio_core::{
    calibration, check_all, checkpoint, cost_ledger, fallback_rate, slo_report, transcript, Budget,
    Entity, Event, EventEnvelope as _, EventWalStore, Seq, Store, Verb, EVENT_ARTIFACT_ADDED,
    EVENT_ASSISTANT_FALLBACK, EVENT_ASSISTANT_FINAL, EVENT_MEMORY_RECALLED,
    EVENT_SESSION_CONSOLIDATED, EVENT_TASK_OPENED, EVENT_USER_MESSAGE,
};
use std::path::{Path, PathBuf};

/// 读数（**测试专用窄口**）：`storage_root` 固定成 WAL 所在目录本身。
///
/// 于是每个用例都只有一个会话 ⇒ 聚合列读到的就是「一个 WAL」的形状，可与
/// per-session 各列对照。P2 的多会话判据另建目录、不走这个窄口。
///
/// 为什么包一层而不是给 21 处调用都加第五个参数：测试要盯的是**各列口径**，
/// 聚合列的扫描范围由它自己那条判据钉。两件事混在一个入口上，改聚合范围
/// 就要动 21 处。
fn read_at(wal: &Path, principal: &str) -> super::SessionStats {
    let root = wal.parent().expect("WAL 有父目录").to_path_buf();
    read("s1", wal, None, principal, &root).expect("读数")
}

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

// ── 实例级聚合列（P2）──────────────────────────────────────────────────────
//
// ## 为什么单独一组用例
//
// 其余各列的口径是「一个会话、一份切片、一套投影」；聚合列的口径是「**实例下所有
// 会话**、多份切片、合并后再出数」。它多出来的自由度正是它独有的失效形态：
// 少扫一个会话、把 archive 扫进来、一个坏 WAL 拖垮整列——三条都**不报错**，
// 只是数字悄悄不对（样本少几个、目录多几个）。

/// 造一个有 N 个会话的实例根，每个会话一格 final（同档位 `deep`）。
///
/// 返回 `(实例根, 各会话的 WAL 路径)`。**会话目录 = 存储根的直接子目录**
/// （`paths::session_dir` 的形状），故意按真布局造——判据要盯的正是
/// 「扫描范围对不对」，用假布局就照不出范围错。
fn seed_instance(tag: &str, n: usize) -> (PathBuf, Vec<PathBuf>) {
    let root = std::env::temp_dir().join(format!("symbio-fleet-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("实例根");
    let mut wals = Vec::new();
    for i in 0..n {
        let dir = root.join(format!("session-{i}"));
        std::fs::create_dir_all(&dir).expect("会话目录");
        let wal = dir.join(super::super::paths::V2_WAL_FILE);
        let store = EventWalStore::open(&wal).expect("WAL");
        // 每个会话一轮：final 实测各不相同 ⇒ 合并后样本数与分位数都可判
        let u = store
            .append(
                Event::pending(
                    format!("u{i}"),
                    EVENT_USER_MESSAGE,
                    Entity::Turn,
                    Verb::Opened,
                    0,
                    "user",
                )
                .with_payload(serde_json::json!({ "text": "问", "tier": "deep" })),
            )
            .unwrap()
            .value();
        store
            .append(
                Event::pending(
                    format!("f{i}"),
                    EVENT_ASSISTANT_FINAL,
                    Entity::Turn,
                    Verb::Closed,
                    0,
                    "agent:main",
                )
                .with_produced_by(u)
                .with_cost_ms(100 * (i as u64 + 1)),
            )
            .unwrap();
        wals.push(wal);
    }
    (root, wals)
}

/// **正向**：三个会话 ⇒ `wals = 3`、样本 3 个、时延取全量口径。
///
/// 这条是 P2 的出口判据本身——它红的方式有两种，都是静默的：扫不到（`wals` 少）、
/// 扫错范围（`wals` 多）。两者都不抛错，所以必须有一个断言直接盯数字。
#[test]
fn fleet_aggregates_every_session_in_the_instance() {
    let (root, wals) = seed_instance("agg", 3);
    let got = read(
        "session-0",
        &wals[0],
        None,
        crate::symbio_core::authz::PRINCIPAL_MAIN,
        &root,
    )
    .expect("读数");

    assert_eq!(
        got.fleet["wals"], 3,
        "实例下三个会话的 WAL 全部读进来（少一个就是静默漏扫）"
    );
    assert_eq!(got.fleet["sessions"], 3, "目录里有 WAL 的会话数");
    assert_eq!(
        got.fleet["by_tier"]["latency"][0]["samples"], 3,
        "三个会话各一格 final ⇒ 合并后 3 个样本（并集，不是覆盖）"
    );
    // 样本是 100/200/300 ⇒ P50 = 200（全量口径）。若只扫到本会话，P50 会是 100。
    assert_eq!(
        got.fleet["by_tier"]["latency"][0]["p50"], 200,
        "分位数按合并后的全量样本算，不是任一会话自己的"
    );
    // 兜底分母同样是全量口径（三格开轮，零兜底）
    assert_eq!(got.fleet["turns"], 3);
    assert_eq!(got.fleet["fallbacks"], 0);
}

/// **反向（范围）**：只走一层——`tool_archives` 里的同名文件不算事实源。
///
/// 递归扫会把 archive 读进来，`wals` 于是虚高、样本混入归档。这条把假 WAL
/// 塞进 **两层深**的子目录，`wals` 必须不动。
#[test]
fn fleet_scan_does_not_descend_into_session_subdirectories() {
    let (root, wals) = seed_instance("shallow", 1);
    // 会话子目录里再放一个同名文件（形如 `<root>/session-0/tool_archives/v2-events.wal`）
    let nested = wals[0].parent().unwrap().join("tool_archives");
    std::fs::create_dir_all(&nested).expect("子目录");
    std::fs::copy(&wals[0], nested.join(super::super::paths::V2_WAL_FILE)).expect("放假 WAL");

    let got = read(
        "session-0",
        &wals[0],
        None,
        crate::symbio_core::authz::PRINCIPAL_MAIN,
        &root,
    )
    .expect("读数");
    assert_eq!(
        got.fleet["wals"], 1,
        "只扫一层：子目录里的同名文件不是事实源（递归会虚高）"
    );
}

/// **反向（韧性）**：一个**根本不是 WAL** 的会话不拖垮整列。
///
/// 撕裂尾行被写方修复是正常状态，`open_readonly` 对「读得出一部分」是会成功的
/// （实测写坏内容仍开得出、只是 0 条事件）。真正打不开的是**路径不是文件**
/// 一类（例如目录被替换、权限不通）——那才是这条判据要钉的：其余会话的读数照出。
#[test]
fn fleet_survives_a_session_whose_wal_cannot_be_opened() {
    let (root, wals) = seed_instance("torn", 2);
    // 把第二个会话的 WAL 换成一个**目录**：路径存在但不是文件 ⇒ 开不了。
    let _ = std::fs::remove_file(&wals[1]);
    std::fs::create_dir_all(&wals[1]).expect("把 WAL 换成目录");

    let got = read(
        "session-0",
        &wals[0],
        None,
        crate::symbio_core::authz::PRINCIPAL_MAIN,
        &root,
    )
    .expect("读数");
    assert_eq!(got.fleet["wals"], 1, "打不开的那个不计入");
    assert_eq!(
        got.fleet["sessions"], 2,
        "但目录层面仍承认有两个会话（差就是被跳过的）"
    );
    assert_eq!(got.fleet["by_tier"]["latency"][0]["samples"], 1);
}

/// **读侧闸**：不给读的人 ⇒ 聚合列是空形状，且**不扫**。
///
/// 扫遍实例再报「不给你看」是白花代价；结论与空形状相同，故直接走空。
#[test]
fn fleet_is_empty_when_the_reader_may_not_read() {
    let (root, wals) = seed_instance("gate", 2);
    let got = read(
        "session-0",
        &wals[0],
        Some(crate::symbio_core::authz::PRINCIPAL_MAIN),
        crate::symbio_core::authz::PRINCIPAL_MAIN,
        &root,
    )
    .expect("读数");
    assert_eq!(
        got.fleet,
        serde_json::json!({ "wals": 0, "sessions": 0 }),
        "不给你读 ⇒ 空形状（与四列同族），不扫也不报数"
    );
}

// ── 读数面板（P4：后端下发描述，前端零业务知识）────────────────────────────
//
// ## 这个机制唯一的失效形态
//
// 面板里的每个数都是从**同一份 SessionStats 的其它字段**搬过来的（`panel_of` 的签名
// 收 `&SessionStats`）。若哪天有人在 `panel_of` 里「顺手重算一遍」，两份口径就能
// 各自漂移，而**两个数单独看都对**——只有并排比才看得出来。所以这里的判据不验
// 「面板长什么样」，验「面板里的数 = 列里的数」。

/// 面板里的每个数都必须能在列里对上（同源，不是「看起来差不多」）。
#[test]
fn panel_matches_the_columns_it_describes() {
    let wal = seed("panel");
    let got = read_at(&wal, crate::symbio_core::authz::PRINCIPAL_MAIN);
    let panel = &got.panel;

    // 分区固定三段：时延兜底 / 事实源 / 不变量
    assert_eq!(panel.sections.len(), 3, "分区固定：{:?}", panel.sections);
    assert_eq!(panel.subtitle.as_deref(), Some("s1"), "副标题带会话 id");

    // ① 时延段：一行对应一个档位，值与列逐字对上
    let perf = &panel.sections[0];
    assert_eq!(perf.rows.len(), got.tiers.len(), "一行一档，不多不少");
    for (row, col) in perf.rows.iter().zip(&got.tiers) {
        assert!(
            row.value.contains(&format!("{} 轮", col.turns)),
            "轮数要对上：{} vs {}",
            row.value,
            col.turns
        );
        assert!(
            row.value.contains(&format!("{:.0}%", col.rate * 100.0)),
            "兜底率要对上（0.* → 取整百分比）"
        );
        assert!(
            row.hint
                .as_deref()
                .unwrap_or("")
                .contains(&format!("P99 {}ms", col.p99)),
            "P99 要对上：{:?}",
            row.hint
        );
    }

    // ② 事实源段：条数 = 断点列的 event_count
    let cont = &panel.sections[1];
    let facts_row = cont
        .rows
        .iter()
        .find(|r| r.label == "已记录事实")
        .expect("有条数行");
    assert!(
        facts_row.value.contains(
            &got.checkpoint["event_count"]
                .as_u64()
                .unwrap_or(0)
                .to_string()
        ),
        "事实条数 = 断点列：{} vs {}",
        facts_row.value,
        got.checkpoint["event_count"]
    );
    let has_row = cont
        .rows
        .iter()
        .find(|r| r.label == "事实源在不在")
        .expect("有在不在行");
    assert_eq!(has_row.value, "在", "seed 造过事实源");
    assert_eq!(has_row.tone, crate::symbio_core::ReadingTone::Normal);

    // ③ 不变量段：seed 的 4 格干净 ⇒ 0 条违规、平常语气
    let inv = &panel.sections[2];
    assert_eq!(inv.rows.len(), 1);
    assert_eq!(inv.rows[0].value, "0 条", "干净的事实源 ⇒ 0 违规");
    assert_eq!(inv.rows[0].tone, crate::symbio_core::ReadingTone::Normal);
    // 反过来：seed 的兜底率是 1/2 = 50%，远超 5% 阈值 ⇒ 时延段**本该**是 warn。
    // 这一条与下面「有违规 ⇒ warn」合起来覆盖两个方向：该标色的标、不该标的不标。
    assert_eq!(
        perf.rows[0].tone,
        crate::symbio_core::ReadingTone::Warn,
        "兜底率 50% > 5% ⇒ 那一行需留意（面板不许把问题显示成没事）"
    );
    assert!(panel.has_warn(), "有 warn 行 ⇒ `has_warn()` 为真");
}

/// 反向：**有违规 ⇒ 那一行变「需留意」**，否则面板把问题显示成没事。
///
/// 手术刀 = 复制一格开轮事件（N-C2 的「每轮至多一条开轮」立刻违规）。
#[test]
fn a_warn_row_appears_when_the_invariants_are_broken() {
    let wal = seed("panel-warn");
    {
        let raw = std::fs::read_to_string(&wal).expect("读 WAL");
        let first = raw
            .lines()
            .find(|l| l.contains("\"user.message\""))
            .expect("有开轮行")
            .to_string();
        // 同一 event_id 会被幂等拦下，故改 id 再追加（复制成「两条开轮」）
        let dup = first.replace("\"u0\"", "\"u0-copy\"");
        std::fs::write(&wal, format!("{raw}{dup}\n")).expect("追加重复开轮");
    }

    let got = read_at(&wal, crate::symbio_core::authz::PRINCIPAL_MAIN);
    let inv = &got.panel.sections[2];
    let row = &inv.rows[0];
    assert_eq!(
        row.tone,
        crate::symbio_core::ReadingTone::Warn,
        "违规 ⇒ 需留意"
    );
    assert!(row.value != "0 条", "且不再显示 0 条：{}", row.value);
    assert!(got.panel.has_warn(), "`has_warn()` 必须为真");
}

/// 没有事实源 ⇒ 「在不在」那一行变成需留意（用户一眼看见「这个会话还没跑过」）。
#[test]
fn a_missing_wal_is_a_warn_row_not_a_silent_zero() {
    let dir = std::env::temp_dir().join(format!("symbio-stats-nowal-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("临时目录");
    let wal = dir.join(super::super::paths::V2_WAL_FILE);
    let root = dir.clone();

    let got = read(
        "s1",
        &wal,
        None,
        crate::symbio_core::authz::PRINCIPAL_MAIN,
        &root,
    )
    .expect("没有事实源不是错");
    assert!(!got.has_wal, "没有源");
    let row = got.panel.sections[1]
        .rows
        .iter()
        .find(|r| r.label == "事实源在不在")
        .expect("有在不在行");
    assert_eq!(row.value, "不在");
    assert_eq!(
        row.tone,
        crate::symbio_core::ReadingTone::Warn,
        "没跑过 ⇒ 需留意"
    );
}

/// 出口读数 == 直接 `apply`（复算：同源，不是「看起来差不多」）。
#[test]
fn stats_recompute_equals_direct_apply() {
    let wal = seed("recompute");
    let got = read_at(&wal, crate::symbio_core::authz::PRINCIPAL_MAIN);

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
        read_at(&wal, crate::symbio_core::authz::PRINCIPAL_MAIN).tiers[0].samples,
        1
    );

    // 删收束格 ⇒ 时延样本列归零、P95 归零
    drop_line(&wal, "f0");
    let after = read_at(&wal, crate::symbio_core::authz::PRINCIPAL_MAIN);
    assert_eq!(after.tiers[0].samples, 0, "删掉唯一的 final ⇒ 样本 1 → 0");
    assert_eq!(after.tiers[0].p95, 0, "无样本 ⇒ 分位数为 0（空档位不报警）");
    assert_eq!(after.tiers[0].turns, 2, "开轮格还在 ⇒ 分母不受收束格影响");
    assert_eq!(after.checkpoint["event_count"], 3, "断点是事实计数");

    // 删开轮格 ⇒ 兜底率的**分母**必须变（2 → 1），比率随之变成 1.0
    drop_line(&wal, "u0");
    let after = read_at(&wal, crate::symbio_core::authz::PRINCIPAL_MAIN);
    assert_eq!(
        after.tiers[0].turns, 1,
        "删掉一个 user.message ⇒ 分母 2 → 1"
    );
    assert_eq!(after.tiers[0].fallbacks, 1, "兜底格与被删的开轮格无关");
    assert_eq!(after.tiers[0].rate, 1.0, "分母变了 ⇒ 比率跟着变");
    assert_eq!(after.checkpoint["event_count"], 2);
}

/// 不变量列随事实源反向变化（[04 §3.1 批④](../../../../docs/plan/04-工程落地.md "docs/plan/04-工程落地")）：
/// 真实形状的两轮全绿 ⇒ 空；删掉收束格 ⇒ C4 必须报出那个缺口。
#[test]
fn invariants_move_when_the_wal_changes() {
    let wal = seed("invariants");
    let clean = read_at(&wal, crate::symbio_core::authz::PRINCIPAL_MAIN);

    // 复算：出口的清单与直接 `check_all` 逐字相等（口径只活在 core，出口不复判）。
    // 切片口径 = `history()`（归档段 + live，S2.7-b2）：出口读什么，复算就比什么。
    let store = EventWalStore::open_readonly(&wal).unwrap();
    let snap = store.history();
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
    let list = read_at(&wal, crate::symbio_core::authz::PRINCIPAL_MAIN).invariants;
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

/// **归档不缩小读数**（S2.7-b2）：压缩把被取代的前缀移出 live 段后，读数口必须读
/// `history()`（归档段 + live）而不是 live 窗口。
///
/// ## 只读 live 会错在两处（都是静默的）
///
/// 1. **C1 假红**：live 窗口的首号不是 0（它是全局号空间的一个窗口），
///    `seq_monotonic` 会把「应为 0 却得 4」报成违规 ⇒ 每次压缩后读数口凭空多一条；
/// 2. **四列缩水**：被归档的轮次仍然发生过（钱花过、兜底过），只读 live 会把
///    它们从分母 / 分子 / 样本里抹掉。
///
/// ## 反向自检
///
/// 把 `stats::read` 里的 `store.history()` 换回 `store.range(Seq::new(0))` ⇒ 本用例红
/// （`invariants` 非空、`tiers` 从 2 轮掉到 0 轮）。
#[test]
fn archival_does_not_shrink_the_readout() {
    let wal = seed("archive");
    let before = read_at(&wal, crate::symbio_core::authz::PRINCIPAL_MAIN);
    assert_eq!(before.tiers.len(), 1, "两轮同档 ⇒ 一行");
    assert_eq!(before.tiers[0].turns, 2, "分母 = 两轮");
    assert_eq!(before.tiers[0].fallbacks, 1, "分子 = 一轮兜底");

    // 压缩：落一条快照，再把被取代的前缀整段归档（live 段只剩快照一条）。
    let store = EventWalStore::open(&wal).unwrap();
    let seq = store
        .append(
            Event::pending(
                "v2sc-3",
                EVENT_SESSION_CONSOLIDATED,
                Entity::Message,
                Verb::Progressed,
                0,
                "agent:main",
            )
            .with_payload(serde_json::json!({
                "messages": [{ "id": "snap-1" }],
                "turns": 2,
            })),
        )
        .unwrap();
    assert_eq!(
        store.archive_before(seq).unwrap(),
        Some(4),
        "前缀 4 条全进归档段"
    );
    assert_eq!(store.len(), 1, "live 段只剩快照");
    drop(store);

    // 读数口仍看完整史：不变量不假红，四列与成本一分不缩。
    let after = read_at(&wal, crate::symbio_core::authz::PRINCIPAL_MAIN);
    assert_eq!(
        after.invariants,
        serde_json::json!([]),
        "归档是号空间的窗口边界，不是「事件被丢弃」——C1 不该报违规"
    );
    assert_eq!(
        after.tiers, before.tiers,
        "分母 / 分子 / 样本不因归档而缩水"
    );
    assert_eq!(after.cost, before.cost, "成本台账看的是完整史");
}

/// 没有事实源 = 有据的零，且读方不许创建文件（真·只读）。
#[test]
fn missing_wal_is_an_honest_zero_and_creates_nothing() {
    let dir = std::env::temp_dir().join(format!("symbio-stats-{}-missing", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("临时目录");
    let wal = dir.join(super::super::paths::V2_WAL_FILE);

    let got = read_at(&wal, crate::symbio_core::authz::PRINCIPAL_MAIN);
    assert!(!got.has_wal);
    assert!(got.tiers.is_empty());
    assert_eq!(got.checkpoint["event_count"], 0);
    assert_eq!(got.cost["total_ms"], 0);
    assert_eq!(got.invariants, serde_json::json!([]), "空事实源 ⇒ 断言全绿");
    assert_eq!(
        got.transcript,
        serde_json::json!({ "entries": [] }),
        "没有事实源 ⇒ 转写为空表（有据的空，不是缺列）"
    );
    assert!(
        !wal.exists(),
        "读方不得为了读而创建事实源文件——那会把「没跑过一轮」变成「跑过一轮空的」"
    );
}

/// 读侧闸（[04 §3.1 批⑥](../../../../docs/plan/04-工程落地.md "docs/plan/04-工程落地")）：声明**属主**
/// 身份 ⇒ 与不声明逐字相同——闸的存在不是为了把属主挡在外面。
#[test]
fn declaring_the_owner_reads_everything() {
    let wal = seed("gate-owner");
    let root = wal.parent().expect("WAL 有父目录").to_path_buf();
    let plain = read_at(&wal, crate::symbio_core::authz::PRINCIPAL_MAIN);
    let as_owner = read(
        "s1",
        &wal,
        Some("user"),
        crate::symbio_core::authz::PRINCIPAL_MAIN,
        &root,
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
    let root = wal.parent().expect("WAL 有父目录").to_path_buf();
    for viewer in ["agent:main", "agent:ghost"] {
        let got = read(
            "s1",
            &wal,
            Some(viewer),
            crate::symbio_core::authz::PRINCIPAL_MAIN,
            &root,
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
        // 转写列同四列一起没（空切片 ⇒ `{entries: []}`）：别人的对话不该从你的读数里漏出去。
        assert_eq!(
            got.transcript,
            serde_json::json!({ "entries": [] }),
            "{viewer}: 没有可见事实 ⇒ 转写为空表"
        );
    }
}

/// 声誉列（[plan/12 批 2](../../../../docs/goal/12-价值验收与基线埋点.md "docs/goal/12-价值验收与基线埋点")，S6 第 15 步的读侧）：
/// `own` 取**本会话主体**，`by_principal` 把这份事实源里的主体原样摊开（`own` 在其中，
/// 不另造口径）；判据只有一处——core 的 `reputation` 投影，本处只取数 + 排版。
#[test]
fn reputation_column_reports_own_and_by_principal() {
    let wal = seed("rep");
    let root = wal.parent().expect("WAL 有父目录").to_path_buf();
    let store = EventWalStore::open(&wal).unwrap();
    let snap = store.range(Seq::new(0));
    let user_seq = snap
        .iter()
        .find(|e| e.kind == EVENT_USER_MESSAGE)
        .expect("用户格")
        .seq()
        .map(|s| s.value())
        .expect("已入格");
    // 一次守约的代际立约（写方 `v2_facts::record_derived` 落的就是这三格）。
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

    let got = read_at(&wal, crate::symbio_core::authz::PRINCIPAL_MAIN);
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
        &root,
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
    let root = dir.clone();
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
        &root,
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
        &root,
    )
    .expect("读数");
    assert_eq!(denied.readyset, serde_json::json!({ "ready": [] }));
    assert_eq!(denied.invariants, serde_json::json!([]));
    assert!(denied.has_wal, "has_wal 独立于可见域：文件在，只是不给你看");
}

/// 校准列（S9 步 22 的读侧，[plan/12 批 1](../../../../docs/goal/12-价值验收与基线埋点.md "docs/goal/12-价值验收与基线埋点")）：
/// 读数 == 直接 `apply`（复算）+ 随事实源**反向**变化（证明它在算，不是常数）。
///
/// 观测面 = 路由收束时落的 `memory.recalled{skill_id, fallback}`（`v2_skills::route`
/// 的产物）——本处按**写方落下的真实形状**造（`Memory × Asserted`、`turn = 0`、
/// `produced_by` 指向本轮用户格），只把载荷换成校准认的那两把钥匙。
#[test]
fn calibration_column_recomputes_and_moves_with_the_wal() {
    let wal = seed("calibration");
    let root = wal.parent().expect("WAL 有父目录").to_path_buf();
    {
        let store = EventWalStore::open(&wal).expect("WAL");
        for (id, fallback) in [("recalled-1", false), ("recalled-2", true)] {
            store
                .append(
                    Event::pending(
                        id,
                        EVENT_MEMORY_RECALLED,
                        Entity::Memory,
                        Verb::Asserted,
                        0,
                        "agent:main",
                    )
                    .with_produced_by(0)
                    .with_payload(serde_json::json!({
                        "found": 1,
                        "skill_id": "sk-a",
                        "fallback": fallback,
                    })),
                )
                .expect("路由观测入格");
        }
    }

    let got = read_at(&wal, crate::symbio_core::authz::PRINCIPAL_MAIN);

    // 复算：出口的校准列与直接 `apply` 逐字相等（口径只活在 core，出口不复判）。
    let ro = EventWalStore::open_readonly(&wal).expect("WAL 只读");
    let snap = ro.range(Seq::new(0));
    let direct = serde_json::to_value(
        calibration()
            .apply(&snap, i64::MAX, Budget::generous())
            .value,
    )
    .expect("序列化");
    assert_eq!(got.calibration, direct, "校准列必须复算相等");
    // 口径：sk-a 用了 2 次、其中回退 1 次（`confidence = 1 − 回退率` 由 core 算）。
    assert_eq!(got.calibration["by_skill"]["sk-a"]["uses"], 2);
    assert_eq!(got.calibration["by_skill"]["sk-a"]["fallbacks"], 1);

    // 反向：删掉回退那条 ⇒ 回退数 1 → 0（读数跟着事实源动）。
    drop_line(&wal, "recalled-2");
    let after = read_at(&wal, crate::symbio_core::authz::PRINCIPAL_MAIN);
    assert_eq!(after.calibration["by_skill"]["sk-a"]["uses"], 1);
    assert_eq!(after.calibration["by_skill"]["sk-a"]["fallbacks"], 0);

    // 读侧闸：非属主 ⇒ 空切片 ⇒ `{ by_skill: {} }`（与就绪集列同族，不是声誉的空对象）。
    let denied = read(
        "s1",
        &wal,
        Some(crate::symbio_core::authz::PRINCIPAL_MAIN),
        crate::symbio_core::authz::PRINCIPAL_MAIN,
        &root,
    )
    .expect("读数");
    assert_eq!(denied.calibration, serde_json::json!({ "by_skill": {} }));
    assert!(denied.has_wal, "has_wal 独立于可见域");
}

/// 转写列（**对话面读侧**，[plan/12 批 2](../../../../docs/goal/12-价值验收与基线埋点.md "docs/goal/12-价值验收与基线埋点")）：
/// 读的是**事实源**（`user.message` / `chat.assistant.final` / `chat.assistant.fallback` /
/// `artifact.added` 四格）而不是会话存储里的消息副本。
///
/// 判据 = 复算同源（口径只活在 core 的 `transcript` 投影）+ 期望值钉死 + 反向
/// （删一格 ⇒ 那一句从转写里消失）+ 读侧闸（非属主 ⇒ `{entries: []}`，四列形态）。
#[test]
fn transcript_column_reads_the_conversation_back_from_the_wal() {
    let wal = seed("transcript");
    let root = wal.parent().expect("WAL 有父目录").to_path_buf();
    let got = read_at(&wal, crate::symbio_core::authz::PRINCIPAL_MAIN);

    // 复算：出口的转写列与直接 `apply` 逐字相等（口径只活在 core 的投影，出口不复判）。
    let ro = EventWalStore::open_readonly(&wal).expect("WAL 只读");
    let snap = ro.range(Seq::new(0));
    let direct = serde_json::to_value(
        transcript()
            .apply(&snap, i64::MAX, Budget::generous())
            .value,
    )
    .expect("序列化");
    assert_eq!(got.transcript, direct, "转写列必须复算相等");

    // 期望值直接写在这里（不从出口自己身上取）：种子的四格 → user/assistant 交替两条。
    // 兜底话术（`chat.assistant.fallback` 的 `why`）是用户**实际看到**的回复，也算 assistant 行。
    assert_eq!(
        got.transcript["entries"],
        serde_json::json!([
            { "role": "user", "text": "你好" },
            { "role": "assistant", "text": "答" },
            { "role": "user", "text": "再问" },
            { "role": "assistant", "text": "模型超时" },
        ]),
        "转写按事件顺序读出两条问答"
    );

    // 反向：删掉成功轮的收束格 ⇒ 对应那一句必须从转写里消失（证明它真的在读文件，不是常数）。
    drop_line(&wal, "f0");
    let after = read_at(&wal, crate::symbio_core::authz::PRINCIPAL_MAIN);
    let entries = after.transcript["entries"].as_array().expect("条目数组");
    assert_eq!(entries.len(), 3, "删一条 final ⇒ 转写 4 → 3：{entries:?}");
    assert!(
        !entries.iter().any(|e| e["text"] == "答"),
        "被删的那句不得再出现在转写里：{entries:?}"
    );

    // 读侧闸：非属主 ⇒ 空切片 ⇒ `{ entries: [] }`（与就绪集 / 校准同族，不是声誉的空对象）。
    let denied = read(
        "s1",
        &wal,
        Some(crate::symbio_core::authz::PRINCIPAL_MAIN),
        crate::symbio_core::authz::PRINCIPAL_MAIN,
        &root,
    )
    .expect("读数");
    assert_eq!(denied.transcript, serde_json::json!({ "entries": [] }));
    assert!(denied.has_wal, "has_wal 独立于可见域：文件在，只是不给你看");
}

/// 转写列把**工具结果**也读出来（[plan/10 批 3](../../../../docs/plan/10-工具轮v2化实施方案.md "docs/plan/10-工具轮v2化实施方案")）：
/// `artifact.added` 进投影 ⇒ 列里多一条 `role = "tool"`（带 `tool` = 工具名）。
///
/// 这是「口径只活在 core 投影」的直接后果——出口不加任何判断，投影多一格，列就多一行。
/// 判据独立于上一条（上一条的种子不含工具轮）：这条**补一格工具产物**再看列，因此它
/// 证的是「列真的会浮出工具行」，不是「列恒等于种子那四行」。
#[test]
fn transcript_column_surfaces_tool_rows() {
    let wal = seed("transcript-tool");
    // 往事实源补一格工具产物（种子本身不含工具轮）。`produced_by` 指向轮 0 用户格（seq 0）。
    let store = EventWalStore::open(&wal).expect("WAL");
    store
        .append(
            Event::pending(
                "a0",
                EVENT_ARTIFACT_ADDED,
                Entity::Artifact,
                Verb::Asserted,
                0,
                "agent:main",
            )
            .with_produced_by(0)
            .with_payload(serde_json::json!({ "tool": "vdfs_read", "text": "文件内容" })),
        )
        .expect("追加工具格");

    let got = read_at(&wal, crate::symbio_core::authz::PRINCIPAL_MAIN);
    let entries = got.transcript["entries"].as_array().expect("条目数组");
    let tool_row = entries
        .iter()
        .find(|e| e["role"] == "tool")
        .unwrap_or_else(|| panic!("转写列应含工具行：{entries:?}"));
    assert_eq!(tool_row["tool"], "vdfs_read", "工具行带工具名");
    assert_eq!(tool_row["text"], "文件内容", "工具行带结果正文");
    // 非工具行**不带** `tool`（`skip_serializing_if`：新增角色不改旧角色的线格式）。
    assert!(
        entries
            .iter()
            .filter(|e| e["role"] != "tool")
            .all(|e| e.get("tool").is_none()),
        "非工具行不该带 tool：{entries:?}"
    );
}
