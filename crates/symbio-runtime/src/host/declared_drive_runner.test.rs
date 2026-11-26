//! 组合根登记优先于 `drive` 族的回归测试。
//!
//! 本测试用 `inventory` 登记一个只在当前线程启用的假 runner：未启用时返回 `None`，
//! 因此不会改变同进程其它宿主用例的装配路径。

use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use serde_json::json;
use symbio_platform::kernel::plugin::{
    Plugin, PluginError, PluginInvokeRequest, PluginInvokeRequestExt, PluginInvokeResponse,
    PluginMeta, PluginPayload,
};
use symbio_plugin_api::drive::{
    DriveOutcome, DrivePayload, DriveReport, DriveSupport, DRIVE_FAMILY,
};
use symbio_plugin_api::{ExecEventSink, PluginSimpleRequest, SymbioKey, CAPABILITY_VISITOR};

use super::*;
use crate::host::Host;

fn drive_row(
    name: &str,
    runner: Arc<dyn DriveRunner>,
) -> symbio_platform::kernel::capability::CapabilityRow {
    symbio_platform::kernel::capability::CapabilityRow::new(
        DRIVE_FAMILY,
        format!("<drive>{name}"),
        DrivePayload::new(runner),
        false,
    )
}

thread_local! {
    static USE_DECLARED: Cell<bool> = const { Cell::new(false) };
}

static DECLARED_CALLS: AtomicUsize = AtomicUsize::new(0);
static FAMILY_CALLS: AtomicUsize = AtomicUsize::new(0);

struct DeclaredRunner;

#[async_trait]
impl DriveRunner for DeclaredRunner {
    async fn drive(&self, _support: Arc<dyn DriveSupport>) -> Result<DriveReport, String> {
        DECLARED_CALLS.fetch_add(1, Ordering::SeqCst);
        Ok(DriveReport {
            dispatched: 7,
            outcome: DriveOutcome::NoDebt,
        })
    }
}

struct FamilyRunner;

#[async_trait]
impl DriveRunner for FamilyRunner {
    async fn drive(&self, _support: Arc<dyn DriveSupport>) -> Result<DriveReport, String> {
        FAMILY_CALLS.fetch_add(1, Ordering::SeqCst);
        Ok(DriveReport {
            dispatched: 9,
            outcome: DriveOutcome::NoDebt,
        })
    }
}

fn declared_runner() -> Option<Arc<dyn DriveRunner>> {
    if USE_DECLARED.with(|flag| flag.get()) {
        Some(Arc::new(DeclaredRunner))
    } else {
        None
    }
}

inventory::submit!(DriveRunnerDecl {
    make: declared_runner
});

struct DeclaredRunnerGuard;

impl DeclaredRunnerGuard {
    fn enable() -> Self {
        USE_DECLARED.with(|flag| flag.set(true));
        Self
    }
}

impl Drop for DeclaredRunnerGuard {
    fn drop(&mut self) {
        USE_DECLARED.with(|flag| flag.set(false));
    }
}

struct RootWithFamilyRow;

#[async_trait]
impl Plugin for RootWithFamilyRow {
    fn meta(&self) -> PluginMeta {
        PluginMeta::new("declared-runner-root", "declared-runner-root")
    }

    async fn route(
        self: Arc<Self>,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        Err(PluginError::NotFound("route".to_string()))
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        if let Some(visitor) = ctx.get(CAPABILITY_VISITOR) {
            visitor
                .put(drive_row("family", Arc::new(FamilyRunner)))
                .await;
        }
        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

fn bound_base() -> Arc<dyn PluginInvokeRequest> {
    let meta = serde_json::Map::from_iter([(
        symbio_plugin_api::AGENT_ID.name().to_string(),
        json!("main"),
    )]);
    Arc::new(PluginSimpleRequest::from_parts(json!({}), Some(&meta)))
}

#[tokio::test]
async fn declared_runner_takes_precedence_over_drive_family() {
    DECLARED_CALLS.store(0, Ordering::SeqCst);
    FAMILY_CALLS.store(0, Ordering::SeqCst);
    let _guard = DeclaredRunnerGuard::enable();

    let host = Host::new(Arc::new(RootWithFamilyRow));
    let report = host
        .drive(
            &bound_base(),
            "s-declared-runner",
            None,
            ExecEventSink::silent(),
        )
        .await
        .expect("declared runner should drive the session");

    assert_eq!(report.dispatched, 7);
    assert_eq!(DECLARED_CALLS.load(Ordering::SeqCst), 1);
    assert_eq!(FAMILY_CALLS.load(Ordering::SeqCst), 0);
}
