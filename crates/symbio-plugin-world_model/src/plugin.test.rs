//! `plugin.rs` 的单元测试 —— 它交出什么、又不接什么。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;
use std::sync::Arc;
use symbio_plugin_api::{CapabilityFamily, PluginInvokeRequestExt, PluginSimpleRequest, Registry};
use tokio::sync::RwLock;

/// 最小登记表：只装行、按族过滤。本插件不注册模型 / 挂载行，故不需要审批闸与单槽移交。
struct FakeRegistry {
    rows: RwLock<Vec<Arc<CapabilityRow>>>,
}

impl FakeRegistry {
    fn new() -> Self {
        Self {
            rows: RwLock::new(Vec::new()),
        }
    }
}

#[async_trait]
impl Registry for FakeRegistry {
    async fn put(&self, row: CapabilityRow) {
        self.rows.write().await.push(Arc::new(row));
    }

    async fn get(&self, name: &str) -> Option<Arc<CapabilityRow>> {
        self.rows
            .read()
            .await
            .iter()
            .rev()
            .find(|r| r.name() == name)
            .cloned()
    }

    async fn list(&self, family: CapabilityFamily) -> Vec<Arc<CapabilityRow>> {
        self.rows
            .read()
            .await
            .iter()
            .filter(|r| r.family() == family)
            .cloned()
            .collect()
    }
}

fn ctx() -> (Arc<dyn PluginInvokeRequest>, Arc<FakeRegistry>) {
    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
    ctx.set(PATH, TRAVERSE_AVAILABLE_TOOLS.to_string());
    let visitor = Arc::new(FakeRegistry::new());
    ctx.set(
        CAPABILITY_VISITOR,
        Arc::clone(&visitor) as Arc<dyn Registry>,
    );
    (ctx, visitor)
}

/// 收集期交出**一个**工具，且它的声明面自带行为名（宿主据此选行为实现）。
#[tokio::test]
async fn traverse_contributes_exactly_one_tool() {
    let (ctx, visitor) = ctx();
    Arc::new(WorldModelPlugin::new())
        .traverse(String::new(), ctx)
        .await
        .expect("traverse 必须成功");

    let tools = visitor.list(CapabilityFamily::multi("tool")).await;
    assert_eq!(tools.len(), 1, "本插件只有一个工具");
    let meta = tools[0].meta();
    assert_eq!(meta.name, "world_observe");
    assert_eq!(
        meta.behaviors,
        &[crate::behavior::WORLD_OBSERVED_BEHAVIOR],
        "工具必须声明行为名——否则效果端口选不到实现，事实永远不落格"
    );
}

#[tokio::test]
async fn route_is_always_not_found() {
    let c: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
    c.set(PATH, "world_model/model".to_string());
    let r = Arc::new(WorldModelPlugin::new()).route(c).await;
    assert!(
        matches!(r, Err(PluginError::NotFound(_))),
        "观察走工具、读出口走视图，没有自有协议"
    );
}

#[tokio::test]
async fn traverse_rejects_unknown_path() {
    let (ctx, _visitor) = ctx();
    ctx.set(PATH, "available_options".to_string());
    let r = Arc::new(WorldModelPlugin::new())
        .traverse(String::new(), ctx)
        .await;
    assert!(matches!(r, Err(PluginError::NotFound(_))));
}

/// 没有 VDFS 挂载点：本插件不带配置，也就不该在 VDFS 上多占一个目录。
#[tokio::test]
async fn has_no_vdfs_mount() {
    assert!(Arc::new(WorldModelPlugin::new())
        .get_vfs_provider()
        .is_none());
}

/// 元数据里 id 必须等于目录名（E-001 的现场形态）；`name` 是挂载点标题，不是 id。
#[test]
fn meta_id_is_the_plugin_directory() {
    let meta = WorldModelPlugin::metadata();
    assert_eq!(meta.id, PLUGIN_ID_WORLD_MODEL);
    assert_eq!(PLUGIN_ID_WORLD_MODEL, "world_model");
    assert_eq!(meta.name, PLUGIN_TITLE, "name 是展示标题，不是目录名");
}
