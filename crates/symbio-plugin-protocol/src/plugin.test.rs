//! `plugin.rs` 的单元测试 —— 它交出什么、路由读到什么、又不接什么。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;
use crate::decl::{PROTOCOL_JOINED, PROTOCOL_PROPOSED};
use serde_json::json;
use std::sync::Mutex;
use symbio_plugin_api::{
    CapabilityFamily, Entity, Event, Fact, FactDraft, FactSource, PluginSimpleRequest, Registry,
    Verb, FACT_SOURCE, SESSION_ID,
};
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

/// 只读事实源：路由只读，测试不需要写口。
struct FakeFacts {
    facts: Mutex<Vec<Fact>>,
}

impl FactSource for FakeFacts {
    fn append(&self, _actor: String, _draft: FactDraft) -> Result<u64, String> {
        Err("测试事实源只读".to_string())
    }

    fn facts(&self) -> Vec<Fact> {
        self.facts.lock().unwrap().clone()
    }

    fn range(&self, from_seq: u64) -> Vec<Fact> {
        self.facts()
            .into_iter()
            .filter(|e| e.seq.map(|s| s.value()).unwrap_or(0) >= from_seq)
            .collect()
    }

    fn head(&self) -> u64 {
        self.facts.lock().unwrap().len() as u64
    }
}

fn proposed(name: &str, title: &str) -> Event {
    Event::pending(
        "e-1",
        PROTOCOL_PROPOSED,
        Entity::Domain,
        Verb::Asserted,
        "session/a",
    )
    .with_payload(json!({
        "name": name,
        "title": title,
        "purpose": "p",
        "messages": ["request", "done"],
    }))
}

fn joined(name: &str, participant: &str) -> Event {
    Event::pending(
        "e-2",
        PROTOCOL_JOINED,
        Entity::Domain,
        Verb::Asserted,
        "session/a",
    )
    .with_payload(json!({ "name": name, "participant": participant }))
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

fn route_ctx(path: &str) -> Arc<dyn PluginInvokeRequest> {
    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
    ctx.set(PATH, path.to_string());
    ctx
}

/// 收集期交出**两个**工具，且两者的声明面各自带行为名。
#[tokio::test]
async fn traverse_contributes_exactly_two_tools() {
    let (ctx, visitor) = ctx();
    Arc::new(ProtocolPlugin::new())
        .traverse(String::new(), ctx)
        .await
        .expect("traverse 必须成功");

    let tools = visitor.list(CapabilityFamily::multi("tool")).await;
    assert_eq!(tools.len(), 2, "本插件有提出与加入两个工具");

    let propose = tools
        .iter()
        .find(|t| t.meta().name == "protocol_propose")
        .expect("应有 protocol_propose");
    assert_eq!(
        propose.meta().behaviors,
        &[crate::behavior::PROTOCOL_PROPOSED_BEHAVIOR],
        "工具必须声明行为名——否则效果端口选不到实现，事实永远不落格"
    );

    let join = tools
        .iter()
        .find(|t| t.meta().name == "protocol_join")
        .expect("应有 protocol_join");
    assert_eq!(
        join.meta().behaviors,
        &[crate::behavior::PROTOCOL_JOINED_BEHAVIOR]
    );
}

/// 路由 `protocol/catalog` 与视图 `protocol.catalog` 读的是同一份折叠。
#[tokio::test]
async fn route_catalog_folds_the_fact_source() {
    let store = Arc::new(FakeFacts {
        facts: Mutex::new(vec![
            proposed("handoff", "任务交接"),
            proposed("review", "评审"),
            joined("review", "main"),
            joined("review", "reviewer"),
        ]),
    });

    let ctx = route_ctx("catalog");
    ctx.set(FACT_SOURCE, store as Arc<dyn FactSource>);

    let payload = Arc::new(ProtocolPlugin::new())
        .route(ctx)
        .await
        .expect("protocol/catalog 应成功");
    let value: serde_json::Value = payload.get().expect("载荷应是 JSON");

    assert_eq!(value["count"], 2, "{value}");
    assert_eq!(value["observations"], 4, "{value}");
    assert_eq!(value["by_status"]["draft"], 1, "{value}");
    assert_eq!(value["by_status"]["established"], 1, "{value}");
}

/// 未知路径指路到工具与读入口。
#[tokio::test]
async fn route_rejects_unknown_path() {
    let r = Arc::new(ProtocolPlugin::new())
        .route(route_ctx("propose"))
        .await;
    assert!(matches!(r, Err(PluginError::NotFound(_))));
}

/// 既没有事实源句柄、也没有会话 id ⇒ 参数错误（不是静默返回空清单）。
#[tokio::test]
async fn route_without_session_identity_is_a_validation_error() {
    let r = Arc::new(ProtocolPlugin::new())
        .route(route_ctx("catalog"))
        .await;
    assert!(matches!(r, Err(PluginError::ValidationError(_))));
}

/// 有会话 id 但测试进程没有登记事实源开面 ⇒ 明确报内部错误。
#[tokio::test]
async fn route_without_a_registered_opener_is_an_internal_error() {
    let ctx = route_ctx("catalog");
    ctx.set(SESSION_ID, "session/a".to_string());
    let r = Arc::new(ProtocolPlugin::new()).route(ctx).await;
    assert!(matches!(r, Err(PluginError::InternalError(_))));
}

#[tokio::test]
async fn traverse_rejects_unknown_path() {
    let (ctx, _visitor) = ctx();
    ctx.set(PATH, "available_options".to_string());
    let r = Arc::new(ProtocolPlugin::new())
        .traverse(String::new(), ctx)
        .await;
    assert!(matches!(r, Err(PluginError::NotFound(_))));
}

/// 没有 VDFS 挂载点：本插件不带配置。
#[tokio::test]
async fn has_no_vdfs_mount() {
    assert!(Arc::new(ProtocolPlugin::new()).get_vfs_provider().is_none());
}

/// 元数据里 id 必须等于目录名（E-001 的现场形态）。
#[test]
fn meta_id_is_the_plugin_directory() {
    let meta = ProtocolPlugin::metadata();
    assert_eq!(meta.id, PLUGIN_ID_PROTOCOL);
    assert_eq!(PLUGIN_ID_PROTOCOL, "protocol");
    assert_eq!(meta.name, PLUGIN_TITLE, "name 是展示标题，不是目录名");
}

/// 路由常量与生成物一致：第二方按公开地址调到的就是 `route` 里那条臂。
#[test]
fn the_route_constant_matches_the_generated_address() {
    assert_eq!(
        symbio_plugin_api::routes::ROUTE_PROTOCOL_CATALOG,
        "protocol/catalog"
    );
    assert_eq!(_ROUTE, symbio_plugin_api::routes::ROUTE_PROTOCOL_CATALOG);
}
