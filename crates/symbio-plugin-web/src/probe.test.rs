//! 探针端口的判据：新增一个世界端口只改能力插件 crate，不改架构层。
//!
//! 两条用例分别是「注册面」与「执行面」：前者证明端口行照常入表且读数就绪，
//! 后者证明债与执行体都在本 crate 内闭合。两者合起来就是
//! **改动架构文件数 = 0** 的可执行形态（D-394 / PTS-007）。

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;

use symbio_plugin_api::port::{
    capability_port_of, port_rows, PortExecContext, PortFactEmit, PortInput, PortProvider,
    PortReading,
};
use symbio_plugin_api::{
    CapabilityFamily, CapabilityRow, ExecAbortSignal, ExecEventSink, Registry,
};

use super::*;

/// 最小登记表（与 `plugin-api` / `tool` 的同款夹具：只装行、按族过滤）。
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

    async fn list(&self, fam: CapabilityFamily) -> Vec<Arc<CapabilityRow>> {
        self.rows
            .read()
            .await
            .iter()
            .filter(|r| r.family() == fam)
            .cloned()
            .collect()
    }
}

/// 注册面：一个能力插件申报新端口后，行照常进 `port` 族并读数就绪——
/// 族名、行名前缀、判重口径都由 `plugin-api` 一份持有，本 crate 只给端口名。
#[tokio::test]
async fn a_new_world_port_registers_without_architecture_changes() {
    let provider: Arc<dyn PortProvider> = Arc::new(ProbePort);
    let rows = port_rows(&provider);
    assert_eq!(rows.len(), 1, "一个端口一行：端口名是数据");
    assert_eq!(rows[0].name(), "<port:web_probe>:web_probe");
    assert_eq!(rows[0].family().name(), "port", "全部端口共用唯一 multi 族");

    let registry: Arc<dyn Registry> = Arc::new(FakeRegistry::new());
    registry
        .put(rows.into_iter().next().expect("刚断言过恰一行"))
        .await;
    assert!(
        matches!(
            capability_port_of(&*registry, WEB_PROBE_PORT).await,
            PortReading::Ready(_)
        ),
        "探针端口入表后应读作就绪"
    );
}

/// 执行面：债与执行体都在本 crate 内闭合，机制侧不需要认识该端口。
#[tokio::test]
async fn the_probe_debt_and_executor_close_inside_the_capability_crate() {
    let spec = debt();
    assert_eq!(spec.name, "web-probe-step");
    assert_eq!(spec.port, WEB_PROBE_PORT);

    let executor = ProbePort.executor().expect("探针提供者必须有执行面");
    let ctx = PortExecContext {
        abort: ExecAbortSignal::new(),
        idempotency_key: spec.id.clone(),
        sink: ExecEventSink::silent(),
        emit: PortFactEmit::silent(),
        ctx: None,
        subject: spec.id.clone(),
        group: String::new(),
        session_id: "s-probe".to_string(),
        origin: None,
        scratch_dir: None,
        reader: None,
        source: "probe".to_string(),
    };
    let outcome = executor
        .execute(
            ctx,
            PortInput {
                port: WEB_PROBE_PORT,
                payload: spec.payload.clone(),
            },
        )
        .await;
    assert!(outcome.success());
    assert_eq!(outcome.facts, vec![spec.payload]);
}
