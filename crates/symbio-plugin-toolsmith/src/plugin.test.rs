//! `plugin.rs` 的单元测试 —— 它交出什么、又不接什么。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;
use std::sync::Arc;
use symbio_plugin_api::{CapabilityFamily, PluginInvokeRequestExt, PluginSimpleRequest, Registry};
use tokio::sync::RwLock;

/// 最小登记表：只装行、按族过滤。本插件不注册模型 / 挂载行。
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

/// 收集期交出**一个**工具，且它的声明面自带行为名。
#[tokio::test]
async fn traverse_contributes_exactly_one_tool() {
    let (ctx, visitor) = ctx();
    Arc::new(ToolsmithPlugin::new())
        .traverse(String::new(), ctx)
        .await
        .expect("traverse 必须成功");

    let tools = visitor.list(CapabilityFamily::multi("tool")).await;
    assert_eq!(tools.len(), 1, "本插件只有一个工具");
    let meta = tools[0].meta();
    assert_eq!(meta.name, "toolsmith_define");
    assert_eq!(
        meta.behaviors,
        &[crate::behavior::TOOLSMITH_DEFINED_BEHAVIOR],
        "工具必须声明行为名——否则效果端口选不到实现，事实永远不落格"
    );
}

/// 工具 schema 里没有 `owner`：限制靠形状，不靠调用方守规矩。
#[tokio::test]
async fn the_tool_schema_has_no_owner_field() {
    let (ctx, visitor) = ctx();
    Arc::new(ToolsmithPlugin::new())
        .traverse(String::new(), ctx)
        .await
        .expect("traverse 必须成功");
    let tools = visitor.list(CapabilityFamily::multi("tool")).await;
    let schema = &tools[0].meta().input_schema;
    assert!(
        schema["properties"].get("owner").is_none(),
        "入参里不能有 owner/target 字段：{schema}"
    );
}

#[tokio::test]
async fn route_is_always_not_found() {
    let c: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
    c.set(PATH, "toolsmith/define".to_string());
    let r = Arc::new(ToolsmithPlugin::new()).route(c).await;
    assert!(
        matches!(r, Err(PluginError::NotFound(_))),
        "登记走工具、读出口走视图，没有自有协议"
    );
}

#[tokio::test]
async fn traverse_rejects_unknown_path() {
    let (ctx, _visitor) = ctx();
    ctx.set(PATH, "available_options".to_string());
    let r = Arc::new(ToolsmithPlugin::new())
        .traverse(String::new(), ctx)
        .await;
    assert!(matches!(r, Err(PluginError::NotFound(_))));
}

/// 没有 VDFS 挂载点：本插件不带配置。
#[tokio::test]
async fn has_no_vdfs_mount() {
    assert!(Arc::new(ToolsmithPlugin::new())
        .get_vfs_provider()
        .is_none());
}

/// 元数据里 id 必须等于目录名（E-001 的现场形态）。
#[test]
fn meta_id_is_the_plugin_directory() {
    let meta = ToolsmithPlugin::metadata();
    assert_eq!(meta.id, PLUGIN_ID_TOOLSMITH);
    assert_eq!(PLUGIN_ID_TOOLSMITH, "toolsmith");
    assert_eq!(meta.name, PLUGIN_TITLE, "name 是展示标题，不是目录名");
}
