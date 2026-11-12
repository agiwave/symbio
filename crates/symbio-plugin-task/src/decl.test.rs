//! 装配表在**本 crate 的测试二进制**里也要能冻结。
//!
//! ## 为什么本文件还要交一份「协议行」夹具
//!
//! 本插件的测试二进制只链接自己：协议五格的归宿行住在
//! `crates/symbio-plugin-loop`，不在这里 ⇒ `seal` 报 `OrphanFact`、整张表失败
//! （装载期检查），于是 [`symbio_plugin_api::vocab`] 退回共享清单——而
//! `task.rework_created` 的 `CountsRework` 角色只在本插件自己的声明里。
//!
//! 夹具补的正是完整进程里由 loop 提供的那一半：五条协议自读行，只在 `cfg(test)` 生效；
//! 生产装配（全插件链接）不会走到这份提交——`cfg(test)` 不进 lib 的正常构建。

use super::{EVENT_TASK_REWORK_CREATED, OWNER};
use symbio_plugin_api::reports::ReportCtx;
use symbio_plugin_api::ring::{Decl, Pattern};
use symbio_plugin_api::{Entity, PluginDeclarations};

/// 协议五格的自读行：完整进程里由 loop 交表，本二进制不链接它 ⇒ 夹具补上。
fn protocol_rows() -> &'static [Decl] {
    let mut out = Vec::new();
    for name in [
        symbio_plugin_api::EVENT_REQUEST,
        symbio_plugin_api::EVENT_SENT,
        symbio_plugin_api::EVENT_RESPONSE,
        symbio_plugin_api::EVENT_FAILED,
        symbio_testkit::EVENT_RETRY,
    ] {
        let pattern: &'static [Pattern] = Box::leak(Box::new([Pattern {
            source: name,
            key: &[],
            eq: &[],
        }]));
        out.push(Decl {
            name,
            reads: &[],
            expand: None,
            owes: pattern[0],
            until: pattern,
            unless: &[],
        });
    }
    Box::leak(out.into_boxed_slice())
}

fn harness_declarations() -> PluginDeclarations<'static> {
    PluginDeclarations {
        events: &[],
        facts: &[],
        views: &[],
        ports: &[],
        actors: Vec::new(),
        rows: protocol_rows(),
    }
}

symbio_plugin_api::submit_declarations!(
    "crates/symbio-plugin-task-test-harness",
    harness_declarations
);

/// 本 crate 交的表在装配后能按名字解析，且返工角色留在 `task.rework_created` 上——
/// 后者是 `rework_bounded` 唯一认它的依据（D-203：读侧不抄名字）。
#[test]
fn the_task_event_resolves_through_the_process_assembly() {
    let frozen = symbio_testkit::sealed_declarations().expect("测试进程的装配表应能冻结");
    assert_eq!(
        frozen.event_cell(EVENT_TASK_REWORK_CREATED),
        Some((Entity::Task, symbio_plugin_api::Verb::Asserted))
    );
    assert_eq!(frozen.event_owner(EVENT_TASK_REWORK_CREATED), Some(OWNER));
    assert!(frozen.holds_role(
        EVENT_TASK_REWORK_CREATED,
        symbio_plugin_api::FactRole::CountsRework
    ));
}

/// 就绪集报表（`D-387`）：`slot` 已登记、没有跨会话合并的轴。
#[test]
fn the_readyset_report_is_registered_without_a_fleet_axis() {
    let reports = symbio_plugin_api::reports::reports().expect("登记表应合法");
    let report = reports
        .iter()
        .find(|r| r.slot == "readyset")
        .expect("就绪集列应有登记项");
    assert_eq!(report.owner, "task");
    assert_eq!(report.views, &[super::READYSET_VIEW][..], "视图只有一个");
    assert!(
        report.fleet.is_none(),
        "就绪集是当前候选集：两个会话的候选集并起来不是任何一个读数的口径"
    );
}

/// 容量由读者经 `ReportCtx::cap` 交，owner 套上并报出**生效**值（`cap = 0` 按 1）。
///
/// 这是「容量是读者的预算，不是折叠的一格」在报表这一侧的落点：视图交无界候选集，
/// 报表出口负责截断，并把截了几笔写进同一份读数。
#[test]
fn the_readyset_report_takes_the_readers_capacity() {
    let ready = serde_json::json!({ "ready": [
        { "task_id": "a", "seq": 1, "depends_on": [] },
        { "task_id": "b", "seq": 2, "depends_on": ["a"] },
    ]});
    let ctx = |cap| ReportCtx {
        principal: "session/main",
        cap,
        readable: true,
    };

    let all = super::readyset_report(std::slice::from_ref(&ready), &ctx(9));
    assert_eq!(all["cap"], 9);
    assert_eq!(
        all["ready"].as_array().map(Vec::len),
        Some(2),
        "足够大就交全部"
    );

    let one = super::readyset_report(std::slice::from_ref(&ready), &ctx(1));
    assert_eq!(
        one["ready"].as_array().map(Vec::len),
        Some(1),
        "截断只改长度"
    );
    assert_eq!(one["ready"][0]["task_id"], "a", "先开的先派");

    let zero = super::readyset_report(&[ready], &ctx(0));
    assert_eq!(
        zero["cap"], 1,
        "`cap = 0` 按 1：容量为零的调度器不是更保守，是永不动"
    );
    assert_eq!(zero["ready"].as_array().map(Vec::len), Some(1));

    assert_eq!(
        super::readyset_report(&[serde_json::Value::Null], &ctx(2)),
        serde_json::json!({ "ready": [], "cap": 2 }),
        "视图缺席 ⇒ 空表，形状不变"
    );
}
