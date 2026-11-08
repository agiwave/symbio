//! 装配侧的两条判据：主部署入口指向谁，以及安全阀出口落格的**形状**。
//!
//! 落格端到端必须在这一侧：`land_stall` 的 owner 从声明表取，而冻结声明表要求
//! 全部插件都在链接图里（`symbio` 是唯一满足的测试二进制）；宿主单测只判
//! `stall_draft` 的形状。
//!
//! 其余驱动语义（`settled` 的等待、在飞不清闲、按根绑定）住在
//! `crates/symbio-host/src/drive.test.rs`——它们判的是宿主运行时，不是装配。

use super::*;
use symbio_kernel::event::{Entity, Verb, OWNER_PRINCIPAL};
use symbio_kernel::plugin::{Plugin, PluginInvokeRequest, PluginMeta, PluginPayload};
use symbio_kernel::store::MemoryStore;
use symbio_plugin_api::drive::{DriveKey, EVENT_DRIVER_STALLED};
use symbio_plugin_api::fact_source::{Fact, FactSource};
use symbio_plugin_api::{FactDraft, PluginInvokeResponse};

/// 驱动 trait 的返回别名需要具体化载荷类型（与内核签名一致）。
type Response = PluginInvokeResponse<PluginPayload>;

/// 只用于身份判据的假根：不路由、不遍历。
struct FakeRoot;

#[async_trait::async_trait]
impl Plugin for FakeRoot {
    fn meta(&self) -> PluginMeta {
        PluginMeta::new("fake-root", "fake-root")
    }

    async fn route(self: Arc<Self>, _ctx: Arc<dyn PluginInvokeRequest>) -> Response {
        Ok(PluginPayload::new(&serde_json::json!({})))
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> Response {
        Ok(PluginPayload::new(&serde_json::json!([])))
    }
}

/// 主部署入口解析到的必须就是 `bind` 会交出的那一份。
///
/// 这条是 t29 的直接判据：装配根（`publish_deployment_root`）与插件
/// （`drive_entry` → `bind`）取到的若是两个实例，`settled` 永远立即返回。
#[test]
fn main_entry_resolves_to_the_bound_instance() {
    let root: Arc<dyn Plugin> = Arc::new(FakeRoot);
    symbio_plugin_api::publish_deployment_root(Arc::clone(&root));
    let bound = symbio_plugin_api::declared_drive_binding()
        .expect("驱动绑定必须已登记")
        .bind(Arc::clone(&root));
    let resolved = main_drive().expect("主部署根已登记");
    assert!(
        Arc::ptr_eq(&resolved, &bound),
        "主部署入口解析到的实例必须与 bind 交出的是同一份（t29：等的那份不能是另一个）"
    );
    assert!(
        symbio_plugin_api::declared_drive_entry().is_some(),
        "主部署驱动入口必须可被 declared_drive_entry 取到"
    );
}

/// 空闲查询是**查询口**，不是装配层的状态副本：空闲会话立即返回。
///
/// 等待体（`wait_idle`）与三个状态位住在 `symbio-host` 的 `KickState`；这里只验
/// 装配侧能经登记入口找到那份实例——找不到入口时静默返回，找到时由宿主的等待体
/// 决定何时放行。
#[tokio::test]
async fn settled_port_resolves_the_registered_entry() {
    let root: Arc<dyn Plugin> = Arc::new(FakeRoot);
    symbio_plugin_api::publish_deployment_root(Arc::clone(&root));
    let _bound = symbio_plugin_api::declared_drive_binding()
        .expect("驱动绑定必须已登记")
        .bind(Arc::clone(&root));

    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        super::settled("port-idle"),
    )
    .await
    .expect("空闲会话的查询口必须立即返回（等待体读的是宿主的 KickState）");
}

fn key() -> DriveKey {
    DriveKey {
        port: "p".into(),
        id: "x".into(),
        attempt: 1,
    }
}

// ==================== 安全阀出口落格（`D-393`） ====================
//
// stderr 只对 CLI 有意义：Tauri / gateway 没有控制台，两条安全阀出口在界面上与
// 「正常结束」同形。出口必须同时是**一条机制事实**，非 CLI 宿主经 VDFS 订阅面读到。

/// 停摆落一条机制事实：种类 / 载荷 / 溯源锚 / 写方主体都对得上。
///
/// 写方主体由**声明表**给（`driver.stalled` 声明为 `@principal`），解析成会话主体；
/// 不再有第二条「直接拿 `principal_of` 当 owner」的写口。
#[test]
fn safety_valve_lands_one_mechanism_fact() {
    let source = MemoryStore::<Fact>::default();
    // 先垫一条事实，让溯源锚（I2）有指处。
    FactSource::append(
        &source,
        "session/main".into(),
        FactDraft {
            key: "seed".into(),
            kind: "user.message".into(),
            entity: Entity::Scope,
            verb: Verb::Asserted,
            payload: serde_json::json!({}),
            produced_by: None,
        },
    )
    .expect("垫一条事实");

    symbio_host::drive::land_stall(
        &source,
        "sess-1",
        "agent-1",
        &symbio_host::DriveOutcome::Stalled(vec![key()]),
    );

    let facts = FactSource::facts(&source);
    assert_eq!(facts.len(), 2, "停摆只落一条：{facts:?}");
    let stall = &facts[1];
    assert_eq!(stall.kind, EVENT_DRIVER_STALLED, "出口必须用机制事实名落格");
    assert_eq!(stall.payload["outcome"], "stalled");
    assert_eq!(stall.payload["session_id"], "sess-1");
    assert_eq!(stall.payload["outstanding"][0]["port"], "p");
    assert_eq!(stall.produced_by, Some(0), "溯源锚指落格前的最后一条");
    assert!(
        stall.actor.contains("agent-1"),
        "写方是会话主体，不是匿名：{}",
        stall.actor
    );
}

/// 归属只由声明表决定：`driver.stalled` 声明成会话主体哨兵，落格时解析成会话主体。
///
/// 若哪天把它改成 `.owned("person/local")`，这条会在「声明仍是哨兵」的断言处先红
/// ——改归属必须同时改这条用例，那是显式的架构动作，不是一次顺手。
#[test]
fn stall_owner_comes_from_the_declaration_table() {
    let frozen = symbio_plugin_api::sealed_declarations().expect("装配侧声明表必须冻结成立");
    assert_eq!(
        frozen.event_writer(EVENT_DRIVER_STALLED),
        Some(OWNER_PRINCIPAL),
        "`driver.stalled` 必须声明成会话主体哨兵"
    );

    let source = MemoryStore::<Fact>::default();
    symbio_host::drive::land_stall(
        &source,
        "sess-9",
        "agent-9",
        &symbio_host::DriveOutcome::Stalled(vec![key()]),
    );
    let facts = FactSource::facts(&source);
    assert_eq!(facts.len(), 1);
    assert_eq!(
        facts[0].actor,
        symbio_sdk::authz::principal_of(Some("agent-9")),
        "哨兵解析成会话主体（与 SessionFactSource::append 同一个函数）"
    );
}
