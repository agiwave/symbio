//! 宿主的机制状态：无债即 `idle`，有未还债即撤销。

use super::*;
// 只做链接：环经 creator 注册被宿主发现，测试二进制必须把它的注册项带进来。
use symbio_plugin_loop as _;
use symbio_kernel::plugin::{
    Plugin, PluginError, PluginInvokeRequest, PluginInvokeResponse, PluginMeta, PluginPayload,
    PluginSimpleRequest,
};

/// 没有子插件、也不注册任何行的根：用来把驱动走到「无债」。
struct EmptyRoot;

#[async_trait::async_trait]
impl Plugin for EmptyRoot {
    fn meta(&self) -> PluginMeta {
        PluginMeta::new("test-root", "test-root")
    }

    async fn route(
        self: Arc<Self>,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        Err(PluginError::NotFound("route".into()))
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        Ok(PluginPayload::Empty)
    }
}

#[tokio::test]
async fn a_scope_with_no_debt_is_idle() {
    let host = Host::new(Arc::new(EmptyRoot));
    let base: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
    let report = host.drive(&base, "s-1", None).await.expect("驱动装配成立");

    assert_eq!(report.scope, scope_of("s-1"));
    assert_eq!(report.outcome, DriveOutcome::NoDebt);
    assert_eq!(report.dispatched, 0);
    assert!(host.idle_at(report.scope).is_some(), "无未还债 ⇒ 落 idle");
}

#[test]
fn scope_identity_never_degrades_to_zero() {
    assert_ne!(scope_of(""), 0, "空会话 id 也有确定身份");
    assert_ne!(scope_of("a"), scope_of("b"));
}
