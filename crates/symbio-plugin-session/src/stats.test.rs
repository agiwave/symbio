//! `session/stats` 读数口的判据：
//!
//! 1. **复算**——出口的读数与直接 `apply` 逐字相等。同一切片、同一个 as-of：
//!    出口只能调已有的投影，不许另写一份统计（否则两套口径会静默漂移）。
//! 2. **反向**——从事实源里删掉一条事件，对应那一列必须跟着变（证明它真的在算，
//!    不是常数）。
//! 3. **纯读**——没有事实源时是「有据的零」，且**不得**为了读而创建文件。
//! 4. **读侧闸**——声明读方身份才判可见域：属主全量、非属主全零。
//!
//! ## 这一份用例**不**验时延 / 兜底 / 跨度的数字
//!
//! 三列的口径已经搬进能力包 `slo`（名册 #13）：`P99 是什么`、`率怎么算`、`跨度怎么折`
//! 都归 owner，判据在 `crates/symbio-plugin-slo/src/decl.test.rs`（11 条数字用例）。
//! 本测试进程**不链接** `slo`（`E-009`：本插件不引用兄弟插件），所以这里能验、也只该验
//! **读方这一半**：
//!
//! - 视图缺席 ⇒ 出口交**文档写的空形状**并出声，而不是编一个数（`unavailable_columns_…`）；
//! - 出口的逐档行是从 owner 的形状**搬**出来的（`the_exit_rows_carry_the_owner_shape`）；
//! - 面板里的数与列里的数同源（`panel_matches_the_columns_it_describes`）。
//!
//! 组合（真实装配下三列的数字对不对）归 e2e `t28-session-stats.mjs`：它读生产二进制的
//! `tiers[0]` 与 `fleet.by_tier`，那才是「session 读了 slo」这件事的证据。若把数字搬回
//! 这里，就只能在本进程里塞一份替身 owner——那份替身会让「没链接 owner 必须出声」这条
//! 判据变成假绿，且卸载 `slo` 之后这些用例照样通过（那不是判据，是噪声）。

use super::{fleet_reading, read, TierRow};
use crate::declarations::{CHAT_VOCAB, COMMITMENT_VOCAB, MESSAGE_VOCAB};
use crate::stats_reading::{panel_of, totals_of};
use crate::store::EventWalStore;
use std::path::{Path, PathBuf};
use symbio_kernel::event::{Entity, Event, EventEnvelope as _, Seq, Verb};
use symbio_kernel::store::Store;
use symbio_kernel::view::Budget;
use symbio_sdk::actors::SKILL_TAG;
use symbio_sdk::invariants::check_all;
use symbio_sdk::projection::transcript::transcript;

use crate::declarations::EVENT_SESSION_CONSOLIDATED;
use crate::projections::checkpoint::checkpoint;
use symbio_plugin_api::events::{
    EVENT_ARTIFACT_ADDED, EVENT_CHAT_ASSISTANT_FALLBACK, EVENT_CHAT_ASSISTANT_FINAL,
    EVENT_USER_MESSAGE,
};
// 任务族的名字走清单生成物（写方在各自能力包，本插件只是读方，DE-024 D-204）。
use symbio_plugin_api::events::EVENT_TASK_OPENED;
use symbio_plugin_api::vocab;

/// 校准观测的名字：**本测试二进制按线上字面量造它**。
///
/// 这条事实归技能能力包（`D-262`），而它的折叠也住在那里（`D-286`）——本插件不链接兄弟
/// 插件（E-009），名字也不在清单生成物里（除 owner 之外没有生产读方，`CS-007`）。写闸要的
/// 格位由 `declarations.test.rs` 的夹具补（只补格位、不补视图），夹具带着真实形状，试的
/// 正是「不认识这一格的插件读不出账」。
const EVENT_MEMORY_CALIBRATED: &str = "memory.calibrated";

/// 一个会话的事实**值**（生产侧由 `SessionStore::snapshot` 交回）。
fn facts_of(wal: &Path) -> Option<Vec<Event>> {
    if !wal.is_file() {
        return None;
    }
    let store = EventWalStore::open_readonly(wal).ok()?;
    Some(store.range(Seq::new(0)))
}

/// 部署级快照（测试侧按生产布局自己深遍；生产侧这段在存储层）。
fn fleet_of(root: &Path) -> Vec<Vec<Event>> {
    super::super::paths::session_wals_deep(root)
        .into_iter()
        .filter_map(|wal| EventWalStore::open_readonly(&wal).ok())
        .map(|store| store.range(Seq::new(0)))
        .collect()
}

/// 读数（**测试专用窄口**）：把路径翻成值后调 [`read`]。
fn read_path(
    session_id: &str,
    wal: &Path,
    viewer: Option<&str>,
    principal: &str,
    root: &Path,
) -> Result<super::SessionStats, symbio_kernel::plugin::PluginError> {
    read(
        session_id,
        facts_of(wal),
        || fleet_of(root),
        viewer,
        principal,
        false,
        symbio_plugin_api::TASK_CAP,
    )
}

/// 单会话读数：`storage_root` 固定成 WAL 所在目录本身。
///
/// 于是每个用例都只有一个会话 ⇒ 聚合列读到的就是「一个 WAL」的形状，可与
/// per-session 各列对照。多会话判据另建目录、不走这个窄口。
///
/// 为什么包一层而不是给 21 处调用都加参数：测试要盯的是**各列口径**，
/// 聚合列的扫描范围由它自己那条判据钉。两件事混在一个入口上，改聚合范围
/// 就要动 21 处。
fn read_at(wal: &Path, principal: &str) -> super::SessionStats {
    let root = wal.parent().expect("WAL 有父目录").to_path_buf();
    read_path("s1", wal, None, principal, &root).expect("读数")
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
        Entity::Scope,
        Verb::Opened,
        "user",
    )
    .with_payload(serde_json::json!({ "text": "你好", "tier": "deep", "turn_ref": "u-0" }));
    let s0 = store.append(u0).unwrap().value();
    store
        .append(
            Event::pending(
                "f0",
                EVENT_CHAT_ASSISTANT_FINAL,
                Entity::Scope,
                Verb::Closed,
                "session/main",
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
        Entity::Scope,
        Verb::Opened,
        "user",
    )
    .with_payload(serde_json::json!({ "text": "再问", "tier": "deep", "turn_ref": "u-1" }));
    let s1 = store.append(u1).unwrap().value();
    store
        .append(
            Event::pending(
                "fb1",
                EVENT_CHAT_ASSISTANT_FALLBACK,
                Entity::Scope,
                Verb::Closed,
                "session/main",
            )
            .with_produced_by(s1)
            .with_cost_ms(3000)
            .with_payload(serde_json::json!({ "reason": "fallback", "detail": "模型超时" })),
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

// ── 实例级聚合列 ───────────────────────────────────────────────────────────
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
                    Entity::Scope,
                    Verb::Opened,
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
                    EVENT_CHAT_ASSISTANT_FINAL,
                    Entity::Scope,
                    Verb::Closed,
                    "session/main",
                )
                .with_produced_by(u)
                .with_cost_ms(100 * (i as u64 + 1)),
            )
            .unwrap();
        wals.push(wal);
    }
    (root, wals)
}

/// **正向**：三个会话 ⇒ `wals = 3`、逐档两列的形状恒在。
///
/// 这条是实例级聚合列的**范围**判据——它红的方式有两种，都是静默的：扫不到（`wals` 少）、
/// 扫错范围（`wals` 多）。两者都不抛错，所以必须有一个断言直接盯数字。
///
/// 逐档两列里**有什么**归 owner（`slo`）：本进程不链接它，于是两列是文档写的空形状、
/// 合计两格是有据的零。这不是「少验了一条」——是「这一格不是本插件的数」：
/// 数字在 `crates/symbio-plugin-slo/src/decl.test.rs`，组合在 e2e `t28`。
#[test]
fn fleet_aggregates_every_session_in_the_instance() {
    let (root, wals) = seed_instance("agg", 3);
    let got = read_path(
        "session-0",
        &wals[0],
        None,
        symbio_sdk::authz::PRINCIPAL_MAIN,
        &root,
    )
    .expect("读数");

    assert_eq!(
        got.fleet["wals"], 3,
        "实例下三个会话的 WAL 全部读进来（少一个就是静默漏扫）"
    );
    assert_eq!(got.fleet["sessions"], 3, "目录里有 WAL 的会话数");
    assert_eq!(
        got.fleet["by_tier"],
        serde_json::json!({ "latency": [], "fallback": [] }),
        "owner 不在 ⇒ 逐档两列是空表（形状恒在：读方不必为每个键分诊它在不在）"
    );
    assert_eq!(
        got.fleet["turns"], 0,
        "合计两格由 owner 的逐档行累加而来 ⇒ owner 不在时是有据的零，不是 null"
    );
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

    let got = read_path(
        "session-0",
        &wals[0],
        None,
        symbio_sdk::authz::PRINCIPAL_MAIN,
        &root,
    )
    .expect("读数");
    assert_eq!(
        got.fleet["wals"], 1,
        "只扫一层：子目录里的同名文件不是事实源（递归会虚高）"
    );
}

/// **反向（韧性）**：一个**根本不是 WAL** 的路径不进清单、不拖垮整列。
///
/// 撕裂尾行被写方修复是正常状态，`open_readonly` 对「读得出一部分」是会成功的
/// （实测写坏内容仍开得出、只是 0 条事件）。枚举的判据是**存在性**（是文件）：
/// 把 WAL 换成**目录**，连清单都进不了——跳过韧性由存储层的深遍负责
/// （本用例只钉出口读到的值形状）。
#[test]
fn fleet_survives_a_session_whose_wal_cannot_be_opened() {
    let (root, wals) = seed_instance("torn", 2);
    // 把第二个会话的 WAL 换成一个**目录**：路径存在但不是文件 ⇒ 不算事实源。
    let _ = std::fs::remove_file(&wals[1]);
    std::fs::create_dir_all(&wals[1]).expect("把 WAL 换成目录");

    let got = read_path(
        "session-0",
        &wals[0],
        None,
        symbio_sdk::authz::PRINCIPAL_MAIN,
        &root,
    )
    .expect("读数");
    assert_eq!(got.fleet["wals"], 1, "目录不是事实源：存在性枚举根本不认它");
    assert_eq!(
        got.fleet["sessions"], 1,
        "sessions = 清单数（是文件的路径），不是「路径存在」数"
    );
}

/// **聚合吃值**：交几份快照就合并几份，不碰路径、不自行枚举。
///
/// 「哪些事实源打得开」由存储层判（`deep_snapshots` 跳过并出声）；本列只对
/// 交到手里的值套投影——两半不要混在一个函数里（[D-084](../../../docs/003-design/001-核心架构.md#d-084-能力模块公用基础协议上下文只递值事实只经引擎abcde-五类需要的唯一取得方式 "DE-001-核心架构")）。
#[test]
fn fleet_aggregates_the_snapshots_it_is_given() {
    let (root, _wals) = seed_instance("values", 2);
    let facts = fleet_of(&root);
    assert_eq!(facts.len(), 2, "存储层深遍交回两个会话的值");

    let ctx = symbio_plugin_api::reports::ReportCtx {
        principal: symbio_sdk::authz::PRINCIPAL_MAIN,
        cap: 0,
        readable: true,
    };
    let got = fleet_reading(&facts, &ctx);
    assert_eq!(got["wals"], 2);
    assert_eq!(got["sessions"], 2, "与 wals 同值：都是交到手里的快照数");
}

/// **读侧闸**：不给读的人 ⇒ 聚合列是空形状，且**不扫**。
///
/// 扫遍实例再报「不给你看」是白花代价；结论与空形状相同，故直接走空。
#[test]
fn fleet_is_empty_when_the_reader_may_not_read() {
    let (root, wals) = seed_instance("gate", 2);
    let got = read_path(
        "session-0",
        &wals[0],
        Some(symbio_sdk::authz::PRINCIPAL_MAIN),
        symbio_sdk::authz::PRINCIPAL_MAIN,
        &root,
    )
    .expect("读数");
    assert_eq!(
        got.fleet,
        serde_json::json!({
            "wals": 0, "sessions": 0, "turns": 0, "fallbacks": 0,
            "by_tier": { "latency": [], "fallback": [] },
            "axes": {
                "conation": { "expressed": 0, "escalated": 0, "adoption_rate": 0.0 },
                "span": { "sessions": 0, "median_ms": null },
                "autonomy": { "closed": 0, "unattended": 0, "unattended_rate": 0.0 },
            },
        }),
        "不给你读 ⇒ 空形状（与满读数同键：三族给有据的零 / 空，不缺列），不扫也不报数"
    );
}

// ── 出口排版（本插件自己的代码，与 owner 在不在无关）────────────────────────
//
// 逐档行的口径与排版（`sample_ms` → `samples`、率原样搬、按档并表）已随报表登记搬回
// owner（`D-387`），由 `crates/symbio-plugin-slo/src/decl.test.rs` 钉。本插件只剩**加法**：
// 合计两格逐档相加。

/// 合计两格 = 逐档相加（计数可加）。
#[test]
fn totals_add_the_owner_rows() {
    // owner 交的形状（`tiers.fleet_render` 的 `fallback` 数组）
    let fallback = serde_json::json!([
        { "tier": "deep", "turns": 4, "fallbacks": 1, "rate": 0.25 },
        { "tier": "fast", "turns": 2, "fallbacks": 2, "rate": 1.0 },
    ]);
    assert_eq!(
        totals_of(&fallback),
        (6, 3),
        "合计两格 = 逐档相加（计数可加）"
    );

    // 缺席（空表 / `null`）⇒ 零，不 panic
    assert_eq!(totals_of(&serde_json::json!([])), (0, 0));
    assert_eq!(totals_of(&serde_json::Value::Null), (0, 0));
}

// ── 外部效标两列（DE-008 §3.1.1：主动性 / 时域跨度）────────────────────────────────
//
// ## 这两列今天归谁
//
// 主动性（`conation`）、自主程度（`autonomy`）与跨度（`span`）的折叠都在各自的能力包里；
// 本插件只按报表 `slot` 取整条读数、把值装进出口（`D-387`）。于是这里能验的是**读方
// 那一半**：缺席时交什么形状、形状的键是不是那一列自己的。数字（分子分母、中位数）
// 归 owner 的单测与 e2e。

/// 报表列**没有 owner** 时整条不出现，而不是由读方猜一个空形状。
///
/// 本进程不链接任何 owner（E-009）⇒ `reports()` 为空 ⇒ `SessionStats::reports` 是空表。
/// 这与迁移前的「读方替 owner 交一份文档写过的空形状」是两种降级语义：空形状会让
/// 「装配里没有这个 owner」长得像「这一列本来就是空的」；整条缺席才可诊断
/// （`D-281` 理由 3）。
///
/// 诊断列同样没有名字可报：不链接 owner 就没有报表登记的 `views`，而清单生成表已随
/// 报表迁移清空（`D-387`）。**有名字却读不出来**的缺席由
/// [`capability_views_column_follows_the_shared_manifest`] 与全装配 e2e 覆盖。
#[test]
fn unregistered_reports_are_absent_from_the_exit() {
    let wal = seed("absent");
    let got = read_at(&wal, symbio_sdk::authz::PRINCIPAL_MAIN);

    assert!(got.has_wal, "事实源在——缺席不是因为没源");
    assert!(
        got.reports.is_empty(),
        "本进程不链接任何 owner ⇒ 报表列一格都不出现：{:?}",
        got.reports.keys().collect::<Vec<_>>()
    );
    let absent = got.capability_views.as_object().expect("能力视图列是对象");
    assert!(
        absent.is_empty(),
        "一个登记名都没有 ⇒ 没有可诊断的缺席：{absent:?}"
    );
}

/// 实例级 `axes` 三族：形状恒在，各自归各自的 owner。
///
/// - `span` / `conation` / `autonomy` 都归能力包（`slo` / `conation`）：本进程不链接它们
///   ⇒ `span` 是有据的空，`conation` / `autonomy` 是 `null`（零是「一个会话都没发生过」，
///   `null` 是「读不出来」）；
/// - 跨会话合并的**数字**归 owner 的单测与 e2e（`e2e/cases/t36-autonomous-initiator.mjs`
///   读真实装配的 `fleet.axes.conation`），不在这里造替身。
#[test]
fn fleet_axes_report_owner_absence_as_null_not_zero() {
    let (root, wals) = seed_instance("axes", 2);
    let got = read_path(
        "session-0",
        &wals[0],
        None,
        symbio_sdk::authz::PRINCIPAL_MAIN,
        &root,
    )
    .expect("读数");
    let axes = got.fleet.get("axes").expect("axes 族");
    // 缺席的两轴交 `null`，不是零或三个零字段：把「读不出来」写成「有据的零」就是让
    // 装配故障看起来像一次正常的安静；若在这里交替身把它算成零，`capability_views` 那条
    // 「本进程没链接 owner ⇒ 必须出声」的用例也会变成假绿。
    assert!(
        axes["conation"].is_null(),
        "视图缺席 ⇒ 主动性轴交 null（不是有据的零；实际: {}）",
        axes["conation"]
    );
    assert!(
        axes["autonomy"].is_null(),
        "视图缺席 ⇒ 自主程度轴交 null（不是零形状；实际: {}）",
        axes["autonomy"]
    );
    assert_eq!(
        axes["span"],
        serde_json::json!({ "sessions": 0, "median_ms": null }),
        "跨度轴缺席 ⇒ 有据的空（`null` 不是 0：没有样本与中位数 0ms 是两件事）"
    );
}

// ── 读数面板（后端下发描述，前端零业务知识）────────────────────────────────
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
    let mut got = read_at(&wal, symbio_sdk::authz::PRINCIPAL_MAIN);
    // 逐档两列的**数**归 owner（本进程不链接 `slo`），而「这些数怎么说」是本插件的排版
    // 代码（`panel_of` 收 `&SessionStats`）。这里往报表列里交一档合成的行，验的正是那段
    // 排版：面板里的值必须逐字来自列里的值——与 owner 在不在无关，也不会因此造出一份
    // 替身口径。行按 owner 的出口形状（`Vec<TierRow>` 序列化后的 JSON 数组）入表。
    let tiers = vec![TierRow {
        tier: "deep".into(),
        turns: 2,
        fallbacks: 1,
        rate: 0.5,
        p50: 1200,
        p95: 1200,
        p99: 1200,
        samples: 1,
    }];
    got.reports.insert(
        "tiers".into(),
        serde_json::to_value(&tiers).expect("档位行"),
    );
    let panel = panel_of(&got);

    // 分区：时延兜底 / 事实源 / 不变量 / 技能 / 承诺，再加**能力视图**段
    // （段数随登记面，不是本插件写死的常数——DE-025 D-221）
    let expected_sections = 5 + usize::from(!expected_capability_view_names().is_empty());
    assert_eq!(
        panel.sections.len(),
        expected_sections,
        "分区段数 = 固定五段 + 清单非空时的一段能力视图：{:?}",
        panel
            .sections
            .iter()
            .filter_map(|s| s.title.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(panel.subtitle.as_deref(), Some("s1"), "副标题带会话 id");

    // ① 时延段：一行对应一个档位，值与列逐字对上
    let perf = &panel.sections[0];
    assert_eq!(perf.rows.len(), tiers.len(), "一行一档，不多不少");
    for (row, col) in perf.rows.iter().zip(&tiers) {
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
    assert_eq!(has_row.tone, symbio_kernel::present::ReadingTone::Normal);

    // ③ 不变量段：seed 的 4 格干净 ⇒ 0 条违规、平常语气
    let inv = &panel.sections[2];
    assert_eq!(inv.rows.len(), 1);
    assert_eq!(inv.rows[0].value, "0 条", "干净的事实源 ⇒ 0 违规");
    assert_eq!(
        inv.rows[0].tone,
        symbio_kernel::present::ReadingTone::Normal
    );
    // 反过来：seed 的兜底率是 1/2 = 50%，远超 5% 阈值 ⇒ 时延段**本该**是 warn。
    // 这一条与下面「有违规 ⇒ warn」合起来覆盖两个方向：该标色的标、不该标的不标。
    assert_eq!(
        perf.rows[0].tone,
        symbio_kernel::present::ReadingTone::Warn,
        "兜底率 50% > 5% ⇒ 那一行需留意（面板不许把问题显示成没事）"
    );
    assert!(panel.has_warn(), "有 warn 行 ⇒ `has_warn()` 为真");

    // ④ 技能段：开关与已编译数都从那两列搬来（同源），不另算
    let skills = &panel.sections[3];
    let skill_row = skills.rows.first().expect("有技能行");
    assert_eq!(
        skill_row.value,
        if got.skill_compile_enabled {
            "开"
        } else {
            "关"
        },
        "开关行 = 列里的开关"
    );
    if got.skill_compile_enabled {
        assert!(
            skill_row
                .hint
                .as_deref()
                .unwrap_or("")
                .contains(&got.compiled_skills.to_string()),
            "已编译数要对上：{:?} vs {}",
            skill_row.hint,
            got.compiled_skills
        );
    }

    // ⑤ 承诺段：条数与违约数都从那列搬来（seed 没委托过 ⇒ 「没有委托」+ 平常语气）
    let cm_row = panel.sections[4].rows.first().expect("有承诺行");
    assert_eq!(
        got.commitments["total"].as_u64().unwrap_or(1),
        0,
        "seed 不产承诺"
    );
    assert_eq!(
        cm_row.value, "本会话没有委托",
        "零条要说人话，不显示「0 条 · 违约 0 条」"
    );
    assert_eq!(cm_row.tone, symbio_kernel::present::ReadingTone::Normal);
}

/// 能力视图列应有的键：报表登记的 `views`（同名去重）。
///
/// 与 `stats_reading::capability_views` 的唯一来源一一对应；共享清单退役后，
/// 这里不再有第二个来源。
fn expected_capability_view_names() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = match symbio_plugin_api::reports::reports() {
        Ok(reports) => reports
            .iter()
            .flat_map(|report| report.views.iter().copied())
            .collect(),
        Err(_) => Vec::new(),
    };
    names.sort_unstable();
    names.dedup();
    names
}

/// 能力视图列（D-387）：**键完全由报表登记面决定，本插件一个名字也不写**。
///
/// 这一条判据盯的是「扩展不必改本插件」：登记面里多一条视图，列里就多一格，
/// 而这个文件与 `stats_reading.rs` 都没有为此改过一行。
///
/// 本测试二进制不链接任何 owner ⇒ 两侧都是空；读到真值的路径在链接了全部插件的
/// e2e（`capability_views[<登记名>]` 读出 `status: ok`），两侧合起来才是「跨插件
/// 可读」的完整证据。
#[test]
fn capability_views_column_follows_the_report_registry() {
    let wal = seed("capability-views");
    let got = read_at(&wal, symbio_sdk::authz::PRINCIPAL_MAIN);
    let col = got.capability_views.as_object().expect("能力视图列是对象");

    let mut names: Vec<&str> = col.keys().map(String::as_str).collect();
    names.sort_unstable();
    let expected = expected_capability_view_names();
    assert_eq!(
        names, expected,
        "列的键 = 报表登记的 views，一个不多一个不少"
    );
}

/// 面板的能力视图段：**一行一条登记视图**，标签就是登记面里的名字。
///
/// 反向判据（登记面为空时段落整段不出现）由 `expected_sections` 那条并排覆盖：
/// 段数不是常数，而是「固定五段 + 有无能力视图」的和。
#[test]
fn the_capability_section_has_one_row_per_registered_view() {
    let wal = seed("capability-panel");
    let got = read_at(&wal, symbio_sdk::authz::PRINCIPAL_MAIN);
    let expected = expected_capability_view_names();
    let Some(section) = got
        .panel
        .sections
        .iter()
        .find(|s| s.title.as_deref() == Some("能力视图"))
    else {
        assert!(
            expected.is_empty(),
            "登记面非空却没有能力视图段： {:?}",
            got.panel
                .sections
                .iter()
                .map(|s| s.title.clone())
                .collect::<Vec<_>>()
        );
        return;
    };
    assert_eq!(
        section.rows.len(),
        expected.len(),
        "一行一条登记视图，不多不少"
    );
    for name in expected {
        let row = section
            .rows
            .iter()
            .find(|r| r.label == name)
            .unwrap_or_else(|| panic!("登记面里的 {} 在面板上没有行", name));
        assert!(
            row.value.starts_with("可读") || row.value.starts_with("读不出来"),
            "值只说「有没有数」，语义属于 owner：{}",
            row.value
        );
        assert!(
            row.hint.is_some(),
            "缺席要带原因、可读要带预览：{}",
            row.label
        );
    }
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

    let got = read_at(&wal, symbio_sdk::authz::PRINCIPAL_MAIN);
    let inv = &got.panel.sections[2];
    let row = &inv.rows[0];
    assert_eq!(
        row.tone,
        symbio_kernel::present::ReadingTone::Warn,
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

    let got = read_path("s1", &wal, None, symbio_sdk::authz::PRINCIPAL_MAIN, &root)
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
        symbio_kernel::present::ReadingTone::Warn,
        "没跑过 ⇒ 需留意"
    );
}

/// 出口读数 == 直接 `apply`（复算：同源，不是「看起来差不多」）。
#[test]
fn stats_recompute_equals_direct_apply() {
    let wal = seed("recompute");
    let got = read_at(&wal, symbio_sdk::authz::PRINCIPAL_MAIN);

    assert!(got.has_wal);
    assert_eq!(got.session_id, "s1");

    // 读方复算：同一份切片、同一个 as-of，各列各自直接 apply。
    let store = EventWalStore::open_readonly(&wal).unwrap();
    let snap = store.range(Seq::new(0));
    let ck = checkpoint()
        .apply(&snap, i64::MAX, Budget::generous())
        .value;

    // 报表列（就绪集 / 时延兜底 / 成本等）的口径与形状都归 owner（`D-387`）：本测试
    // 进程不链接任何 owner ⇒ `reports()` 为空 ⇒ 这些键整条不出现。**数字**归 owner 的
    // 单测（`crates/symbio-plugin-task/src/readyset.test.rs`、
    // `crates/symbio-plugin-slo/src/decl.test.rs`、`crates/symbio-plugin-cost/src/decl.test.rs`），
    // **组合**归 e2e（`t28-session-stats.mjs` / `t31-task-table.mjs`）。在这里补一份
    // 替身 owner 只会让那些判据变成假绿。
    assert!(
        got.reports.is_empty(),
        "owner 不在 ⇒ 报表列一格都不出现：{:?}",
        got.reports.keys().collect::<Vec<_>>()
    );
    assert_eq!(got.checkpoint, serde_json::to_value(&ck).unwrap());

    // 复算还得**算对**：期望值直接写在这里，不从出口自己身上取。
    // 成本 / 时延两列没有「算对」的期望值可写在这里——它们的数归 owner 的单测，
    // 本进程只复算仍由本插件折叠的列。
    assert_eq!(got.checkpoint["event_count"], 4);
}

/// 反向用例：删一格 ⇒ 对应那一列必须变（不是常数）。
///
/// 这里的手术刀落在**本插件仍自己折叠的列**上：断点列数格子、转写列按事件顺序排条目。
/// 时延 / 兜底两列的反向判据归 owner（`crates/symbio-plugin-slo/src/decl.test.rs`
/// 用同一把刀：删掉 `chat.assistant.final` ⇒ 样本 1 → 0），成本列同理归
/// `crates/symbio-plugin-cost/src/decl.test.rs`。
#[test]
fn reverse_case_the_readout_moves_when_the_wal_changes() {
    let wal = seed("reverse");
    let before = read_at(&wal, symbio_sdk::authz::PRINCIPAL_MAIN);
    assert_eq!(before.checkpoint["event_count"], 4, "种子四格");
    assert_eq!(
        before.transcript["entries"].as_array().map_or(0, Vec::len),
        4,
        "转写列把四格都排出来（不是常数）"
    );

    // 删收束格（成功轮 1200ms）⇒ 断点少一条、转写少一行
    drop_line(&wal, "f0");
    let after = read_at(&wal, symbio_sdk::authz::PRINCIPAL_MAIN);
    assert_eq!(after.checkpoint["event_count"], 3, "断点是事实计数");
    assert_eq!(
        after.transcript["entries"].as_array().map_or(0, Vec::len),
        3,
        "删掉那一格 ⇒ 转写跟着少一行（口径同源，不是各列各扫一遍）"
    );

    // 删开轮格 ⇒ 两列再各少一条
    drop_line(&wal, "u0");
    let after = read_at(&wal, symbio_sdk::authz::PRINCIPAL_MAIN);
    assert_eq!(after.checkpoint["event_count"], 2);
    assert_eq!(
        after.transcript["entries"].as_array().map_or(0, Vec::len),
        2
    );
}

/// 不变量列随事实源反向变化：
/// 真实形状的两轮全绿 ⇒ 空；删掉中间一行 ⇒ seq 跳号必须报出。
/// 轮次配对不在这里判——那是义务环的债表。
#[test]
fn invariants_move_when_the_wal_changes() {
    let wal = seed("invariants");
    let clean = read_at(&wal, symbio_sdk::authz::PRINCIPAL_MAIN);

    // 复算：出口的清单与直接 `check_all` 逐字相等（口径只活在 core，出口不复判）。
    // 切片口径 = 完整史（归档段 + 未取代那段）：出口读什么，复算就比什么。
    let store = EventWalStore::open_readonly(&wal).unwrap();
    let snap = store.range(Seq::new(0));
    assert_eq!(
        clean.invariants,
        serde_json::to_value(check_all(vocab(), &snap)).unwrap()
    );
    assert_eq!(
        clean.invariants,
        serde_json::json!([]),
        "两轮都收束、档位已声明且预算内 ⇒ 不变量清单为空（首日不假红）"
    );

    // 删轮 0 的收束格 ⇒ 行删了 ⇒ C1 报 seq 跳号；轮次配对归义务环，不在此列。
    drop_line(&wal, "f0");
    let list = read_at(&wal, symbio_sdk::authz::PRINCIPAL_MAIN).invariants;
    let list = list.as_array().expect("清单是数组");
    assert_eq!(list.len(), 1, "{list:?}");
    assert!(
        list.iter()
            .any(|v| v["why"].as_str().unwrap_or_default().contains("seq")),
        "C1：删掉中间一行 ⇒ 后续 seq 跳号：{list:?}"
    );
}

/// **归档不缩小读数**：压缩把被取代的前缀移进归档段后，读数口拿到的
/// 仍是完整史（归档段 + 未取代那段），而不是那一小段窗口。
///
/// ## 只给窗口会错在两处（都是静默的）
///
/// 1. **C1 假红**：未取代那段的**首号不是 0**（它是全局号空间的一个窗口），
///    `seq_monotonic` 会把「应为 0 却得 4」报成违规 ⇒ 每次压缩后读数口凭空多一条；
/// 2. **四列缩水**：被归档的轮次仍然发生过（钱花过、兜底过），看不见它们会把
///    它们从分母 / 分子 / 样本里抹掉。
///
/// 按布局分叉的第二个读法（只给窗口）因此不是「多一个选择」，而是把布局知识
/// 交给每个调用方去挑——挑错还是静默的（[DE-001 §4.3](../../../docs/003-design/001-核心架构.md "DE-001-核心架构") 一源）。
///
/// ## 反向自检
///
/// 把 `stats::read` 里的 `store.range(Seq::new(0))` 换成「从最新快照起自切窗口」
/// （`super::store::current_window(...)`）⇒ 本用例红（`invariants` 非空、`tiers` 从 2 轮掉到 0 轮）。
#[test]
fn archival_does_not_shrink_the_readout() {
    let wal = seed("archive");
    let before = read_at(&wal, symbio_sdk::authz::PRINCIPAL_MAIN);
    assert_eq!(before.checkpoint["event_count"], 4, "两轮四格全见");

    // 压缩：落一条快照，再把被取代的前缀整段归档（未取代那段只剩快照一条）。
    let store = EventWalStore::open(&wal).unwrap();
    let seq = store
        .append(
            Event::pending(
                "v2sc-3",
                EVENT_SESSION_CONSOLIDATED,
                Entity::Message,
                Verb::Progressed,
                "session/main",
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
    assert_eq!(store.len(), 1, "没被取代的那段只剩快照");
    drop(store);

    // 读数口仍看完整史：不变量不假红，各列与成本一分不缩。
    let after = read_at(&wal, symbio_sdk::authz::PRINCIPAL_MAIN);
    assert_eq!(
        after.invariants,
        serde_json::json!([]),
        "归档是号空间的窗口边界，不是「事件被丢弃」——C1 不该报违规"
    );
    assert_eq!(
        after.checkpoint["event_count"], 5,
        "断点列读的是完整史（归档段 4 格 + 快照 1 格），不是那一小段窗口（1 格）"
    );
    // 转写列**有意**在压缩点另起一段（读的是「当前这段历史」，不是全史），所以这里
    // 不比它——拿它当「没缩水」的证据会读反：压缩后它本来就该只剩快照之后的部分。
    // 成本 / 时延 / 兜底三列走同一个跨插件读口（`read_shared_view_all` ⇒ `range(0)`），
    // 扫描范围因此与断点列同源：断点列这条判据一旦守住，读口就不会只交窗口那段。
    // 三列各自的**数**归 owner 的单测，组合归 e2e `t28-session-stats.mjs`——本进程
    // 不链接它们，在这里比不出「缩水」。
}

/// 没有事实源 = 有据的零，且读方不许创建文件（真·只读）。
#[test]
fn missing_wal_is_an_honest_zero_and_creates_nothing() {
    let dir = std::env::temp_dir().join(format!("symbio-stats-{}-missing", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("临时目录");
    let wal = dir.join(super::super::paths::V2_WAL_FILE);

    let got = read_at(&wal, symbio_sdk::authz::PRINCIPAL_MAIN);
    assert!(!got.has_wal);
    assert!(got.reports.is_empty(), "没有源 ⇒ 报表列整条缺席");
    assert_eq!(got.checkpoint["event_count"], 0);
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

/// 读侧闸：声明**属主**身份 ⇒ 与不声明逐字相同——闸的存在不是为了把属主挡在外面。
#[test]
fn declaring_the_owner_reads_everything() {
    let wal = seed("gate-owner");
    let root = wal.parent().expect("WAL 有父目录").to_path_buf();
    let plain = read_at(&wal, symbio_sdk::authz::PRINCIPAL_MAIN);
    let as_owner = read_path(
        "s1",
        &wal,
        Some(symbio_sdk::authz::PRINCIPAL_USER),
        symbio_sdk::authz::PRINCIPAL_MAIN,
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
///
/// 前提（② 之后要说清的一句）：本事实源**没有共享内容**——`seed` 只种
/// 对话格不种记忆，内容侧可见域因此是 C10 缺省 `thread_private`，闸的答案
/// 与接线前逐字相同。共享内容的读方见下一条。
#[test]
fn a_non_owner_reads_nothing_at_all() {
    let wal = seed("gate-nonowner");
    let root = wal.parent().expect("WAL 有父目录").to_path_buf();
    for viewer in ["session/main", "session/ghost"] {
        let got = read_path(
            "s1",
            &wal,
            Some(viewer),
            symbio_sdk::authz::PRINCIPAL_MAIN,
            &root,
        )
        .expect("读数");
        assert!(got.has_wal, "{viewer}: 事实源存在，只是不给你看");
        assert!(
            got.reports.is_empty(),
            "{viewer}: 报表列整批缺席：{:?}",
            got.reports.keys().collect::<Vec<_>>()
        );
        assert_eq!(
            got.checkpoint["event_count"], 0,
            "{viewer}: 断点列的事件数为 0"
        );
        assert_eq!(
            got.invariants,
            serde_json::json!([]),
            "{viewer}: 没有可见事实 ⇒ 无可报的违规"
        );
        // 声誉列随四列一起没：**全有全无**，不按字段裁——按字段裁会留下
        // 「四列不给你、声誉给你」这种没有任何理由的半开半掩。
        assert!(
            !got.reports.contains_key("reputation"),
            "{viewer}: 没有可见事实 ⇒ 声誉列整条不出"
        );
        // 转写列同四列一起没（空切片 ⇒ `{entries: []}`）：别人的对话不该从你的读数里漏出去。
        assert_eq!(
            got.transcript,
            serde_json::json!({ "entries": [] }),
            "{viewer}: 没有可见事实 ⇒ 转写为空表"
        );
    }
}

/// ② 的判据：读侧闸传的是**内容侧**的可见域——本会话记忆载荷里记了
/// `shared`（人的认知可供借用，SC-010 判据 3）⇒ 矩阵内主体（`agent:main`）
/// 读得到他人共享内容；矩阵外主体（`agent:ghost`）照旧空切片。
///
/// 三条并排证的是同一件事的三个面：闸读的是**产地的事实**（载荷 `vis_scope`），
/// 不是恒传的缺省值、也不是调用方声明——请求方自己声明「我能看到什么」
/// 等于给自己授权，那条捷径 `governance` 模块头点名禁止。
///
/// 反向对照（同一次播种里自带）：把那条唯一共享记忆换成私有的 ⇒ `agent:main`
/// 回到空切片——证明翻面的是**内容的可见域**这一个变量，不是别的。
#[test]
fn a_matrix_member_reads_shared_content_and_an_outsider_still_cannot() {
    let dir = std::env::temp_dir().join(format!("symbio-stats-{}-gate-shared", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("临时目录");
    let wal = dir.join(super::super::paths::V2_WAL_FILE);
    let store = EventWalStore::open(&wal).unwrap();
    let t = 1_700_000_000_000i64;
    // 一轮对话（与 `seed` 同形的最小事实源）+ 一条 **shared** 的经验记忆
    // （`derived_facts::record_learning` 的生产形态）。
    let u0 = Event::pending(
        "u0",
        EVENT_USER_MESSAGE,
        Entity::Scope,
        Verb::Opened,
        "user",
    )
    .with_payload(serde_json::json!({ "text": "你好", "tier": "deep", "turn_ref": "u-0" }));
    let s0 = store.append(u0).unwrap().value();
    store
        .append(
            Event::pending(
                "f0",
                EVENT_CHAT_ASSISTANT_FINAL,
                Entity::Scope,
                Verb::Closed,
                "session/main",
            )
            .with_produced_by(s0)
            .with_cost_ms(1200)
            .with_ts(t)
            .with_payload(serde_json::json!({ "text": "你好！", "tier": "deep" })),
        )
        .unwrap();
    store
        .append(
            Event::pending(
                "v2m-shared",
                symbio_plugin_api::events::EVENT_MEMORY_ENCODED,
                Entity::Memory,
                Verb::Opened,
                "user",
            )
            .with_produced_by(s0)
            .with_ts(t)
            .with_payload(serde_json::json!({
                "content": "我喜欢喝不加糖的咖啡",
                "tag": "经验",
                "generation": 0u32,
                "vis_scope": "shared",
            })),
        )
        .unwrap();

    let root = wal.parent().expect("WAL 有父目录").to_path_buf();
    let session_principal = symbio_sdk::authz::PRINCIPAL_MAIN;

    // ① 矩阵内主体（非属主）：读得到——共享内容让这份事实源不再纯私有。
    let as_member = read_path("s1", &wal, Some("session/main"), session_principal, &root)
        .expect("矩阵内主体读数");
    assert!(as_member.has_wal);
    assert_eq!(
        as_member.checkpoint["event_count"], 3,
        "矩阵内主体读得到他人共享内容（三格全见）"
    );
    assert_eq!(
        as_member.transcript["entries"]
            .as_array()
            .map_or(0, Vec::len),
        2,
        "转写列同见：闸翻的是切片，不是只有断点列"
    );

    // ② 矩阵外主体：照旧空切片（fail-closed 不因共享内容松动）。
    let as_outsider = read_path("s1", &wal, Some("session/ghost"), session_principal, &root)
        .expect("矩阵外主体读数");
    assert!(as_outsider.has_wal, "有源但不给你看");
    assert_eq!(
        as_outsider.checkpoint["event_count"], 0,
        "矩阵外主体断点列为空"
    );
    assert_eq!(
        as_outsider.transcript["entries"]
            .as_array()
            .map_or(0, Vec::len),
        0,
        "转写列同空：闸门落在切片上，各列一起没"
    );

    // ③ 属主本人：与不声明时逐字一致（闸不是为了把属主挡在外面）。
    let plain = read_path("s1", &wal, None, session_principal, &root).expect("本机默认读数");
    let as_owner = read_path(
        "s1",
        &wal,
        Some(symbio_sdk::authz::PRINCIPAL_USER),
        session_principal,
        &root,
    )
    .expect("属主读数");
    assert_eq!(as_owner, plain);

    // 反向对照：唯一那条共享记忆改成私有 ⇒ agent:main 回到空切片。
    // （同一次播种、同一份代码，只翻内容的可见域这一个变量。）
    let mut events = store.range(Seq::new(0));
    let last = events.len() - 1;
    events[last].payload["vis_scope"] = serde_json::json!("thread_private");
    drop(store);
    std::fs::remove_file(&wal).unwrap();
    let store2 = EventWalStore::open(&wal).unwrap();
    for e in events {
        store2.append(e).unwrap();
    }
    let denied =
        read_path("s1", &wal, Some("session/main"), session_principal, &root).expect("读数");
    assert!(
        denied.reports.is_empty() && denied.checkpoint["event_count"] == 0,
        "内容全私有 ⇒ 矩阵内非属主回到空切片（翻面的是可见域，不是别的）"
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// 声誉列**没有 owner** 时整条不出现（`D-387`）：本插件不再替 owner 补 `own` 行。
///
/// 本测试二进制不链接 `crates/symbio-plugin-reputation`（E-009）⇒ `reports()` 为空 ⇒
/// `reputation` 这个键不存在。`own` 行的主体、配对规则与 as-of 都归 owner 的单测
/// （`crates/symbio-plugin-reputation/src/decl.rs::tests`：账记在立约方、配对靠 `id`、
/// 自报不进账、as-of、同切片同账），生产二进制里读得到真数归 e2e
/// （`t30-multi-principal.mjs` / `t42-full-derives-commitment-task.mjs` 钉 `own` 的
/// `offered` / `kept`，`t71-peer-commitment.mjs` 钉对等主体的条目，`t74-elimination-loop.mjs`
/// 钉同一张账喂给淘汰回路）。
///
/// 本插件只钉自己那一半：**不再由读方猜空账**——账算不出来就整条不出。
#[test]
fn reputation_column_is_absent_when_no_owner_registers_it() {
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
    // 一次守约的代际立约（写方 `derived_facts::record_derived` 落的就是这三格）。
    // 格子照写：本插件不认识它们，带着它们才试得出「不认识」是不是真的。
    for e in symbio_plugin_api::commitment_events(
        COMMITMENT_VOCAB,
        "c-rep",
        symbio_sdk::authz::PRINCIPAL_MAIN,
        symbio_sdk::authz::PRINCIPAL_USER,
        "把这段复查看完",
        Ok(()),
        user_seq,
    ) {
        store.append(e).expect("承诺入格");
    }

    let got = read_at(&wal, symbio_sdk::authz::PRINCIPAL_MAIN);
    assert!(got.has_wal, "事实源在，账算不出来不改变这件事");
    assert!(
        !got.reports.contains_key("reputation"),
        "本进程不链接声誉 owner ⇒ 整条不出现（不是读方补一份空账冒充读数）"
    );
    assert!(
        got.invariants.as_array().is_some_and(|l| l.is_empty()),
        "承诺四格不新增违规：{:?}",
        got.invariants
    );

    // 读侧闸关掉时同样整条缺席：列不在，就没有「给不给」的第二种形状可言。
    let denied = read_path(
        "s1",
        &wal,
        Some(symbio_sdk::authz::PRINCIPAL_MAIN),
        symbio_sdk::authz::PRINCIPAL_MAIN,
        &root,
    )
    .expect("读数");
    assert!(!denied.reports.contains_key("reputation"));
}

/// 承诺列（段二，DE-038 §5 的读侧出口）：
/// 一轮里委托过一次 ⇒ 这里就**读得出那一条**（谁向谁、承诺了什么、违约为什么）。
///
/// ## 这条判据补的是哪一半
///
/// `agent_run` 在能力元数据里自声明「我记委托账」，声明面的**名单封闭**由
/// `tool-decls-audit` 守着；但声明说得 ≠ 事实落了。写侧到格子的链条由
/// `turn_exec.test.rs::full_turn_lands_derived_commitment_facts` 钉（走真实通道，
/// 假 provider 回一个 `agent_run`），本用例钉读侧：格子 → 一条可查记录。
/// 两头之间只有 `commitment_events` 这一个成格口，夹具调的就是它——写侧改名，
/// 这两条一起红。
///
/// ## 三个方向
///
/// ① 违约必须带原因出现在读数里（"违约可被观测"是 DE-038 §5 的全部前提）；
/// ② 违约率的分母是**已了结**数：一条在途 + 一条违约 ⇒ 率 100%，不是 50%——
///   把没跑完的轮算进分母等于把「还在做」读成「没做到」；
/// ③ 无读权限 ⇒ 四列那一族的空形状（`{ items: [], total: 0 }`），不是缺列。
#[test]
fn commitments_column_exposes_each_delegation_record() {
    let wal = seed("commitments");
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
    // 一次违约的委托（写方 `record_derived` 落的正是这三格：立约 + 违约 + 告知）
    for e in symbio_plugin_api::commitment_events(
        COMMITMENT_VOCAB,
        "v2c-t1-run-1",
        symbio_sdk::authz::PRINCIPAL_MAIN,
        "agent:reviewer",
        "让 reviewer 复查这段",
        Err("子智能体没起来"),
        user_seq,
    ) {
        store.append(e).expect("承诺入格");
    }
    // 一条只立了约、还没了结的（在途）
    store
        .append(
            symbio_plugin_api::commitment_events(
                COMMITMENT_VOCAB,
                "v2c-t2-run-1",
                symbio_sdk::authz::PRINCIPAL_MAIN,
                "agent:worker",
                "整理周报",
                Ok(()),
                user_seq,
            )[0]
            .clone(),
        )
        .expect("立约入格");

    let got = read_at(&wal, symbio_sdk::authz::PRINCIPAL_MAIN);
    let col = &got.commitments;
    assert_eq!(col["total"], 2, "两条约：一条违约、一条在途");
    assert_eq!(col["broken"], 1);
    assert_eq!(col["kept"], 0);
    assert_eq!(col["settled"], 1, "分母只算已了结的（在途那条不算）");
    assert!(
        col["breach_rate"].as_f64().unwrap_or(0.0) > 0.99,
        "违约率 = 违约 / 已了结 = 1.0：{:?}",
        col["breach_rate"]
    );
    let items = col["items"].as_array().expect("items 是数组");
    assert_eq!(
        items
            .iter()
            .map(|i| (
                i["id"].as_str().unwrap_or(""),
                i["state"].as_str().unwrap_or("")
            ))
            .collect::<Vec<_>>(),
        vec![("v2c-t1-run-1", "broken"), ("v2c-t2-run-1", "open"),],
        "按承诺号排序，同一份事实源永远读出同一个清单"
    );
    assert_eq!(items[0]["promise"], "让 reviewer 复查这段");
    assert_eq!(items[0]["to"], "agent:reviewer");
    assert_eq!(
        items[0]["why"], "子智能体没起来",
        "违约原因就是读数的一部分——用户在这里读「为什么没做到」"
    );
    assert_eq!(items[0]["anchor_seq"].as_u64(), Some(user_seq));
    assert!(
        check_all(vocab(), &store.range(Seq::new(0))).is_empty(),
        "承诺格子不新增违规：{:?}",
        check_all(vocab(), &store.range(Seq::new(0)))
    );

    // 面板那一份（人读的）与列同源：数字直接来自本列，不重算
    let row = &got.panel.sections[4].rows[0];
    assert!(
        row.value.contains("2 条 · 违约 1 条"),
        "面板与列对上：{:?}",
        row.value
    );
    assert_eq!(
        row.tone,
        symbio_kernel::present::ReadingTone::Warn,
        "有违约 ⇒ 需留意"
    );

    // 无读权限 ⇒ 空形状（四列那一族），不是缺列、也不是零值条目
    let denied = read_path(
        "s1",
        &wal,
        Some(symbio_sdk::authz::PRINCIPAL_MAIN),
        symbio_sdk::authz::PRINCIPAL_MAIN,
        &root,
    )
    .expect("读数");
    assert_eq!(
        denied.commitments["items"].as_array().map(|a| a.len()),
        Some(0),
        "无权读 ⇒ 切片为空 ⇒ 一条也没有（与「没委托过」同形，由 has_wal 分辨）"
    );
    assert_eq!(denied.commitments["total"], 0);
}

/// 就绪集列**没有 owner** 时整条不出现（`D-387`）：折叠与容量套用都归任务能力包。
///
/// 本测试二进制不链接 `crates/symbio-plugin-task`（E-009）⇒ 报表 `readyset` 没有登记项
/// ⇒ `readyset` 这个键不存在（读方不再猜 `{ready: [], cap}` 这份空形状）。在这里补一份
/// 替身 owner 只会把两条真判据变成假绿——**谁就绪、什么序、租约与挂起怎么算**归 owner 的单测
/// （`crates/symbio-plugin-task/src/readyset.test.rs`：依赖闭包 / 一次一个 / 活租约出局 /
/// 挂起与恢复 / 返工开新节点），**`cap` 由 owner 套上**归 owner 的报表单测
/// （`crates/symbio-plugin-task/src/decl.test.rs`），**出口读得到真数**归 e2e
/// （`t31-task-table.mjs` 钉成员与 `deniedReady.cap`，`t34-preemption.mjs` 钉不抢占时任务留在就绪集）。
///
/// 载荷仍按**写方落下的真实形状**造（`task × opened`、`depends_on` 数组）：本插件不认识
/// 这些字段，带着它们才试得出「不认识」是不是真的。
#[test]
fn readyset_column_is_absent_when_no_owner_registers_it() {
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
                    symbio_sdk::authz::PRINCIPAL_MAIN,
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

    let got =
        read_path("s-rs", &wal, None, symbio_sdk::authz::PRINCIPAL_MAIN, &root).expect("读数");
    assert!(
        !got.reports.contains_key("readyset"),
        "owner 不在 ⇒ 整条不出现（不是读方猜一份空集合冒充读数）"
    );
    assert!(got.has_wal, "事实源在，账算不出来不改变这件事");

    // 反向：读侧闸关 ⇒ 空切片 ⇒ 连不变量列也空（有源但不给你看）。
    // 列不在，就没有「没有任务」与「不给你看」的第二种形状可言（由 `has_wal` 分辨）。
    let denied = read_path(
        "s-rs",
        &wal,
        Some(symbio_sdk::authz::PRINCIPAL_MAIN),
        symbio_sdk::authz::PRINCIPAL_MAIN,
        &root,
    )
    .expect("读数");
    assert!(!denied.reports.contains_key("readyset"));
    assert_eq!(denied.invariants, serde_json::json!([]));
    assert!(denied.has_wal, "has_wal 独立于可见域：文件在，只是不给你看");
}

/// 校准列**没有 owner** 时整条不出现（`D-387`）：折叠归技能能力包。
///
/// 本测试二进制不链接 `crates/symbio-plugin-skill`（E-009）⇒ `reports()` 为空 ⇒
/// `calibration` 这个键不存在（读方不再猜 `{ by_skill: {} }` 这份空账）。
/// **数字**归 owner 的单测（`crates/symbio-plugin-skill/src/decl.test.rs`：
/// 三次里回退两次 ⇒ 置信度 1/3、缺字段的观测不计数、as-of 之后的观测不进账），
/// **生产二进制里读得到真数**归 e2e（`t35-skill-compile.mjs`
/// 按已知轮次钉 `by_skill` 的 `uses`）。
///
/// 观测仍按**写方落下的真实形状**造（`Memory × Asserted`、`turn = 0`、`produced_by` 指向本轮
/// 用户格）：本插件不认识这些字段，带着它们才试得出「不认识」是不是真的。格位由
/// `declarations.test.rs` 的夹具交（写闸 fail-closed，没声明的名字落不进环），**报表不登记**
/// ⇒ 这一列整条不出。
#[test]
fn calibration_column_is_absent_when_no_owner_registers_it() {
    let wal = seed("calibration");
    {
        let store = EventWalStore::open(&wal).expect("WAL");
        for (id, fallback) in [("calib-1", false), ("calib-2", true)] {
            store
                .append(
                    Event::pending(
                        id,
                        // 校准观测走**自己的格子**：`memory × asserted` 是共享的，
                        // 按格子或按载荷字段认输入都会把别的断言算进技能置信度。
                        EVENT_MEMORY_CALIBRATED,
                        Entity::Memory,
                        Verb::Asserted,
                        "session/main",
                    )
                    .with_produced_by(0)
                    .with_payload(serde_json::json!({
                        "skill_id": "sk-a",
                        "fallback": fallback,
                    })),
                )
                .expect("路由观测入格");
        }
    }

    let got = read_at(&wal, symbio_sdk::authz::PRINCIPAL_MAIN);
    assert!(
        !got.reports.contains_key("calibration"),
        "owner 不在 ⇒ 整条不出现（不是读方猜一份空账冒充读数）"
    );
    assert!(got.has_wal, "事实源在，账算不出来不改变这件事");
}

/// 技能编译两列：`skill_compile_enabled`（部署开关）与 `compiled_skills`（本会话
/// `memory.encoded{tag:"skill"}` 的条数）。
///
/// 判据是**两种零必须分得开**：关着 ⇒ `false`（配置选择）；开着但没编译 ⇒ `true` +
/// `0`（运行结果）。计数不新增投影，只数事实源里已有的格子；读侧闸对它同样生效
/// （无权 ⇒ 空切片 ⇒ 0），开关是部署事实、不随闸变化。
#[test]
fn skill_compile_columns_distinguish_off_from_on_but_nothing_compiled() {
    let wal = seed("skills");
    let root = wal.parent().expect("WAL 有父目录").to_path_buf();
    let read_with = |enabled: bool, viewer: Option<&str>| {
        read(
            "s1",
            facts_of(&wal),
            || fleet_of(&root),
            viewer,
            symbio_sdk::authz::PRINCIPAL_MAIN,
            enabled,
            symbio_plugin_api::TASK_CAP,
        )
        .expect("读数")
    };

    // 关着：事实源里也没有技能格 ⇒ 两列各自如实
    let off = read_with(false, None);
    assert!(!off.skill_compile_enabled);
    assert_eq!(off.compiled_skills, 0);

    // 开着但本会话还没编译过 ⇒ 与上一行并读才分得清（两行计数相同、开关不同）
    let on_empty = read_with(true, None);
    assert!(on_empty.skill_compile_enabled);
    assert_eq!(on_empty.compiled_skills, 0);

    // 落两条技能 + 一条普通记忆：按 tag 认，不是按整格数
    {
        let store = EventWalStore::open(&wal).expect("WAL");
        for (id, tag) in [("sk-1", SKILL_TAG), ("sk-2", SKILL_TAG), ("mem-1", "fact")] {
            store
                .append(
                    Event::pending(
                        id,
                        symbio_plugin_api::events::EVENT_MEMORY_ENCODED,
                        Entity::Memory,
                        Verb::Opened,
                        "user",
                    )
                    .with_produced_by(0)
                    .with_payload(serde_json::json!({ "tag": tag, "trigger": id, "content": id })),
                )
                .expect("记忆入格");
        }
    }

    let on = read_with(true, None);
    assert_eq!(on.compiled_skills, 2, "只有 tag=skill 的两条算技能");

    // 读侧闸：矩阵外主体 ⇒ 空切片 ⇒ 计数 0；开关是部署事实，照旧如实报告。
    let denied = read_with(true, Some(symbio_sdk::authz::PRINCIPAL_MAIN));
    assert_eq!(denied.compiled_skills, 0);
    assert!(denied.skill_compile_enabled);
    assert!(denied.has_wal, "has_wal 独立于可见域");
}

/// 转写列（**对话面读侧**）：
/// 读的是**事实源**（`user.message` / `chat.assistant.final` / `chat.assistant.fallback` /
/// `artifact.added` 四格）而不是会话存储里的消息副本。
///
/// 判据 = 复算同源（口径只活在 core 的 `transcript` 投影）+ 期望值钉死 + 反向
/// （删一格 ⇒ 那一句从转写里消失）+ 读侧闸（非属主 ⇒ `{entries: []}`，四列形态）。
#[test]
fn transcript_column_reads_the_conversation_back_from_the_wal() {
    let wal = seed("transcript");
    let root = wal.parent().expect("WAL 有父目录").to_path_buf();
    let got = read_at(&wal, symbio_sdk::authz::PRINCIPAL_MAIN);

    // 复算：出口的转写列与直接 `apply` 逐字相等（口径只活在 core 的投影，出口不复判）。
    let ro = EventWalStore::open_readonly(&wal).expect("WAL 只读");
    let snap = ro.range(Seq::new(0));
    let direct = serde_json::to_value(
        transcript(CHAT_VOCAB, MESSAGE_VOCAB)
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
    let after = read_at(&wal, symbio_sdk::authz::PRINCIPAL_MAIN);
    let entries = after.transcript["entries"].as_array().expect("条目数组");
    assert_eq!(entries.len(), 3, "删一条 final ⇒ 转写 4 → 3：{entries:?}");
    assert!(
        !entries.iter().any(|e| e["text"] == "答"),
        "被删的那句不得再出现在转写里：{entries:?}"
    );

    // 读侧闸：非属主 ⇒ 空切片 ⇒ `{ entries: [] }`（与就绪集 / 校准同族，不是声誉的空对象）。
    let denied = read_path(
        "s1",
        &wal,
        Some(symbio_sdk::authz::PRINCIPAL_MAIN),
        symbio_sdk::authz::PRINCIPAL_MAIN,
        &root,
    )
    .expect("读数");
    assert_eq!(denied.transcript, serde_json::json!({ "entries": [] }));
    assert!(denied.has_wal, "has_wal 独立于可见域：文件在，只是不给你看");
}

/// 转写列把**工具结果**也读出来：
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
                "session/main",
            )
            .with_produced_by(0)
            .with_payload(serde_json::json!({ "tool": "vdfs_read", "text": "文件内容" })),
        )
        .expect("追加工具格");

    let got = read_at(&wal, symbio_sdk::authz::PRINCIPAL_MAIN);
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

// ── 用量那一段（排版半边）────────────────────────────────────────────
//
// 口径（哪些应答进账、缺字段怎么跳、币种怎么归并）已经随写方搬到
// `crates/symbio-plugin-model/src/ledger.rs`，判据在 `ledger.test.rs`。
// 这里只钉本插件这一半：**按 `slot` 取两格报表，排成那一段**。
//
// 「列真的挂到出口上」不在本进程判：这里不链接 `model`（E-009），`reports()` 恒空，
// 任何「出口有 usage 键」的断言都会退化成恒真。那条判据在全装配的
// `crates/symbio/src/report_columns_acceptance.test.rs`（slot 集合 == 出口键集合）。

/// 造一份只有 `usage` / `spend` 两格的报表表——这正是 [`usage_section`] 的全部入参
/// （`D-452`：入参面不许比比较面宽）。
fn reports_of(
    usage: Option<serde_json::Value>,
    spend: Option<serde_json::Value>,
) -> std::collections::BTreeMap<String, serde_json::Value> {
    let mut map = std::collections::BTreeMap::new();
    if let Some(v) = usage {
        map.insert("usage".to_string(), v);
    }
    if let Some(v) = spend {
        map.insert("spend".to_string(), v);
    }
    map
}

/// 判据：**「没测量」与「测到零」不是一件事**——`calls` 是「测到过几次」，
/// 为 0 时整段不出现（写一行「输入 0」等于替 provider 编一个它没给的数）。
#[test]
fn the_usage_section_counts_only_measured_responses() {
    let reports = reports_of(
        Some(serde_json::json!({
            "input": 1500,
            "output": 200,
            "calls": 2,
            "by_principal": {
                "session/main": { "principal": "session/main", "input": 1000, "output": 200, "measured": 1 },
                "sub/worker": { "principal": "sub/worker", "input": 500, "output": 0, "measured": 1 },
            },
        })),
        None,
    );

    let usage = super::usage_section(&reports).expect("测到过用量就必须有这一段");
    assert_eq!(
        usage.title.as_deref(),
        Some("用量"),
        "没有钱的行就不预告花费"
    );

    let row = |label: &str| {
        usage
            .rows
            .iter()
            .find(|r| r.label == label)
            .unwrap_or_else(|| panic!("缺少 {label} 行：{:?}", usage.rows))
    };
    assert_eq!(row("输入").value, "1.5k");
    assert_eq!(row("输出").value, "200");
    assert_eq!(
        row("合计").value,
        "1.7k",
        "1500 + 200 走 format_tokens 的唯一实现"
    );
    // 多主体才逐条摊开：单主体时上面的合计就是全部。
    assert!(
        row("按主体").value.contains("sub/worker"),
        "两个主体要摊开：{:?}",
        row("按主体")
    );

    // 反向：一次都没测到 ⇒ 整段不出现。
    let none = reports_of(
        Some(serde_json::json!({ "input": 0, "output": 0, "calls": 0, "by_principal": {} })),
        None,
    );
    assert!(
        super::usage_section(&none).is_none(),
        "calls == 0 时不画一条全零的读数"
    );
}

/// 判据：钱是**第三种量**——一个币种一行、币种不合计、零金额仍算一次带定价的往返。
#[test]
fn the_usage_section_reports_one_row_per_currency() {
    let reports = reports_of(
        Some(serde_json::json!({
            "input": 10, "output": 5, "calls": 1,
            "by_principal": { "session/main": { "principal": "session/main", "input": 10, "output": 5, "measured": 1 } },
        })),
        Some(serde_json::json!({
            "by_currency": {
                "CNY": { "currency": "CNY", "amount_nano": 12_345_000, "charges": 2, "by_principal": {} },
                "USD": { "currency": "USD", "amount_nano": 2_000_000, "charges": 2, "by_principal": {} },
            },
        })),
    );

    let section = super::usage_section(&reports).expect("有钱的行就要出这一段");
    assert_eq!(
        section.title.as_deref(),
        Some("用量与花费"),
        "有钱的行时标题要说出「花费」"
    );
    let row = |label: &str| {
        section
            .rows
            .iter()
            .find(|r| r.label == label)
            .unwrap_or_else(|| panic!("缺少 {label} 行：{:?}", section.rows))
    };
    assert_eq!(row("花费 · USD").value, "0.0020");
    assert_eq!(row("花费 · CNY").value, "0.0123");
    let labels: Vec<&str> = section
        .rows
        .iter()
        .filter(|r| r.label.starts_with("花费"))
        .map(|r| r.label.as_str())
        .collect();
    assert_eq!(
        labels,
        vec!["花费 · CNY", "花费 · USD"],
        "一个币种一行，顺序由账本的 BTreeMap 给，不跨币种合计"
    );
}

/// 判据：有用量、没定价 ⇒ 只有「用量」三行：不凭空写一行「0 元」。
#[test]
fn usage_without_pricing_does_not_report_zero_money() {
    let reports = reports_of(
        Some(serde_json::json!({
            "input": 1000, "output": 200, "calls": 1,
            "by_principal": {},
        })),
        Some(serde_json::json!({ "by_currency": {} })),
    );
    let section = super::usage_section(&reports).expect("没定价不影响 token 那三行");
    assert_eq!(section.title.as_deref(), Some("用量"));
    assert!(
        section.rows.iter().all(|r| !r.label.starts_with("花费")),
        "没定价就没有金额可报：{:?}",
        section.rows
    );
}

/// 判据：**没链接 owner 时不编数**——两格都不在 ⇒ 整段不出现（不是「0 token」）。
#[test]
fn an_absent_report_reads_as_no_usage_at_all() {
    let empty = std::collections::BTreeMap::new();
    assert!(
        super::usage_section(&empty).is_none(),
        "报表缺席 ⇒ 一段都不出；读方不替 owner 猜空形状"
    );
}
