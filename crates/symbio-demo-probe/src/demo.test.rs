//! 验收：假插件只改本 crate，就能被环选中、派发、配对。

use std::sync::Arc;

use super::*;
// 只做链接：环经 creator 注册被这个根构造，测试二进制必须把它的注册项带进来。
use symbio_plugin_loop as _;
use symbio_host::{base_context, DriveOutcome, Host};

/// 夹具根：先收假插件的端口/债行，再挂上系统环的 `drive` 行。
///
/// 与真实装配一致——根只负责把子插件逐个 `traverse`，自己不认识它们的内容。
struct ProbeRoot {
    demo: Arc<dyn Plugin>,
}

#[async_trait::async_trait]
impl Plugin for ProbeRoot {
    fn meta(&self) -> PluginMeta {
        PluginMeta::new("probe-root", "probe-root")
    }

    async fn route(
        self: Arc<Self>,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        Err(PluginError::NotFound("route".into()))
    }

    async fn traverse(
        self: Arc<Self>,
        path: String,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        Arc::clone(&self.demo)
            .traverse(path.clone(), Arc::clone(&ctx))
            .await?;
        let ring = symbio_plugin_api::creator_create_object::<dyn Plugin>(
            symbio_deploy::PLUGIN_ID_LOOP,
            Arc::clone(&ctx),
        )
        .ok_or_else(|| PluginError::NotFound("环插件未注册".into()))?;
        ring.traverse(path, ctx).await
    }
}

#[tokio::test]
async fn demo_port_runs_through_the_ring() {
    let host = Host::new(Arc::new(ProbeRoot {
        demo: DemoPlugin::build(base_context()),
    }));
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
