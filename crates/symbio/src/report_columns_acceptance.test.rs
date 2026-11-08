use symbio_plugin_api::reports::{read_all, read_view_with_ver, reports, ReportCtx};

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

/// 每一条已登记视图都必须能**按名字**求值（登记表有它、声明表也冻结了）。
///
/// `capability_views` 只遍历登记面（`D-387`），它不给「名字解析不了」留后路：缺席原因
/// 只有 `not_sealed` / `unknown` 两态，没有任何一态叫「登记了但清单里没有」。若某条
/// 报表的 `views` 在冻结声明表里查不到，诊断列那一格就是 `unknown`——报表照样出数，
/// 诊断列却瘸一格，而两边都不报错。这条判据挡的就是那个半截状态。
#[test]
fn every_registered_view_is_readable_by_name() {
    let reports = reports().expect("全装配的报表登记必须合法");
    for report in reports {
        for name in report.views {
            let got = read_view_with_ver(name, &[]);
            assert!(
                got.is_ok(),
                "报表 `{}` 的视图 `{name}` 按名字读不出来：{:?}",
                report.slot,
                got.err()
            );
        }
    }
}
