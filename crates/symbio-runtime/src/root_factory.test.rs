use super::*;
use serde_json::json;
use symbio_platform::kernel::plugin::{
    Plugin, PluginChannel, PluginInvokeRequest, PluginInvokeRequestExt, PluginInvokeResponse,
    PluginMeta, PluginPayload,
};
use symbio_platform::sdk::keys::PATH;

// 测试二进制不链接 vdfs 插件，路由测试需要一条测试专用根声明。
symbio_plugin_api::inventory::submit! {
    symbio_plugin_api::AddrRootDecl(".test-vdfs-root")
}

struct TestRoot;

#[async_trait::async_trait]
impl Plugin for TestRoot {
    fn meta(&self) -> PluginMeta {
        PluginMeta::new("test-root", "test-root")
    }

    async fn route(
        self: Arc<Self>,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        match ctx.get(PATH).as_deref() {
            Some(symbio_plugin_api::routes::ROUTE_VDFS_ROOT) => {
                Ok(PluginPayload::new(&json!({ "path": ".test-vdfs-root" })))
            }
            Some(symbio_plugin_api::routes::ROUTE_EVENT_BUS_SUBSCRIBE) => {
                let (peer, _channel) = PluginChannel::pair(1);
                Ok(PluginPayload::Session(peer))
            }
            _ => Ok(PluginPayload::Empty),
        }
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        Ok(PluginPayload::Empty)
    }
}

pub(crate) fn test_root_factory() -> Arc<dyn RootFactory> {
    Arc::new(RootFactoryFn(|_| async {
        Arc::new(TestRoot) as Arc<dyn Plugin>
    }))
}

#[tokio::test]
async fn each_factory_call_constructs_an_independent_root() {
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = Arc::clone(&calls);
    let factory: Arc<dyn RootFactory> = Arc::new(RootFactoryFn(move |_| {
        counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        async { Arc::new(TestRoot) as Arc<dyn Plugin> }
    }));

    let first = factory
        .create_root(Arc::new(PluginSimpleRequest::new(None, None)))
        .await;
    let second = factory
        .create_root(Arc::new(PluginSimpleRequest::new(None, None)))
        .await;

    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert!(!Arc::ptr_eq(&first, &second));
}
