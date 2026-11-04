//! 验收：假插件只改本 crate，就能被环选中、派发、配对。

use super::*;
// 只做链接：环经 creator 注册被宿主发现，测试二进制必须把它的注册项带进来。
use symbio_plugin_loop as _;
use symbio_host::{base_context, DriveOutcome, Host};

#[tokio::test]
async fn demo_port_runs_through_the_ring() {
    let host = Host::new(DemoPlugin::build(base_context()));
    let report = host
        .drive(&base_context(), "demo-session", None)
        .await
        .expect("装配成立");

    assert_eq!(report.dispatched, 1, "环选中了本插件自报的那一笔债");
    assert_eq!(report.outcome, DriveOutcome::NoDebt, "还清后无债");

    let facts = host.facts(report.scope);
    let response = facts
        .iter()
        .find(|fact| fact.closing() == Some(true))
        .expect("执行体还了事实，环按同一债键配对");
    assert_eq!(response.key.scope, report.scope);
    assert_eq!(response.key.port, "demo", "环只按插件给的端口名解析");
    assert_eq!(response.key.id, format!("demo:{}", report.scope));
    assert_eq!(response.key.attempt, 1, "配对键不含分片号、不改 attempt");
    assert_eq!(
        response.payload["echo"], "hello",
        "载荷字符串原样透传到执行体再回来"
    );
    assert!(host.idle_at(report.scope).is_some(), "无未还债 ⇒ idle");

    let again = host
        .drive(&base_context(), "demo-session", None)
        .await
        .expect("二次驱动成立");
    assert_eq!(again.dispatched, 0, "已配对的债不再欠");
}
