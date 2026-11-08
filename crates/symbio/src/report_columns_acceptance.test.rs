use symbio_plugin_api::reports::{read_all, reports, ReportCtx};

/// 每一条已登记的 `slot` 恰好渲染一次，且出口不多出任何键。
#[test]
fn every_registered_slot_is_rendered_exactly_once() {
    let reports = reports().expect("全装配的报表登记必须合法");
    assert!(!reports.is_empty(), "全装配至少要有一条报表登记");

    let ctx = ReportCtx {
        principal: "acceptance",
        cap: 1,
        readable: true,
    };
    let columns = read_all(&[], &ctx);

    let mut slots: Vec<&str> = reports.iter().map(|r| r.slot).collect();
    slots.sort_unstable();
    let keys: Vec<&str> = columns.keys().map(String::as_str).collect();
    assert_eq!(
        keys, slots,
        "session/stats 顶层报表键必须与已登记 slot 一个不多一个不少（少一个 = 某个 slot 悄悄没渲染）"
    );
}
