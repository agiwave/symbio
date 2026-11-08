//! 真实能力包的装配验收（EX-001 口径）：新增一个能力包只改它自己 + 生成物。
//!
//! 四件事同时成立才算「装进去并跑起来」：
//! 1. `PLUGIN.yml` 声明的插件进了默认装配清单（生成物，不手改）；
//! 2. 生产冻结表接纳它的 `PortDecl` 与行（装载期判据会红在缺执行体 / 缺行上）；
//! 3. 环的通用选债器按 `request(port=probe_echo)` 选中这一行；
//! 4. 同一条链路被真的驱动：执行体还了事实、环按同一债键配对、默认会话不欠账。

use std::sync::Arc;

use symbio_host::{base_context, DriveOutcome, Host};
use symbio_kernel::decl::ring::{owed, Lit};
use symbio_kernel::decl::Frozen;
use symbio_kernel::event::{Event, EVENT_REQUEST};
use symbio_kernel::{Entity, Verb};
use symbio_plugin_api::rows::PORT_ACTOR;
use symbio_plugin_api::{
    creator_create_object, sealed_declarations, ExecEventSink, Plugin, PluginError,
    PluginInvokeRequest, PluginInvokeResponse, PluginMeta, PluginPayload, EVENT_SENT,
};

const PROBE: &str = "probe_echo";

#[test]
fn the_probe_pack_is_in_the_default_assembly_and_accepted_by_the_sealed_table() {
    // 必须经本 crate 的门面触达一次，全部插件的链接行才会进最终链接。
    let _ = crate::assembly::PLUGIN_ID_HOME;

    // 1. 装配清单（由 PLUGIN.yml 生成）：新包只写数据就进默认树。
    assert!(
        symbio_deploy::ASSEMBLY_SUB_AGENT_PLUGINS.contains(&symbio_deploy::PLUGIN_ID_PROBE),
        "probe 应在默认装配里，实际：{:?}",
        symbio_deploy::ASSEMBLY_SUB_AGENT_PLUGINS
    );

    let frozen: Arc<Frozen> =
        sealed_declarations().expect("生产声明表应能冻结（含 loop + 全部插件）");

    // 2. 端口声明进了表：`before` 是协议格 `sent`（与 notify 同形）。
    let port = frozen
        .ports()
        .iter()
        .find(|p| p.port == PROBE)
        .expect("生产表里应有 probe_echo 端口的 PortDecl");
    assert_eq!(port.before, EVENT_SENT);

    // 3. 该端口的行绑在通用受理执行体上——行由插件声明，环不认识端口名。
    let row = frozen
        .rows()
        .iter()
        .find(|r| {
            r.by == Some(PORT_ACTOR)
                && r.owes.source == EVENT_REQUEST
                && r.owes
                    .eq
                    .iter()
                    .any(|(field, lit)| *field == "port" && *lit == Lit::Str(PROBE))
        })
        .expect("生产表里应有 by=port、owes=request(port=probe_echo) 的行");

    // 4. 选债器：一条 request 事实就能让这一行欠账（可还、未被挂起）。
    let request = Event::pending(
        "request-probe-n1-1",
        EVENT_REQUEST,
        Entity::System,
        Verb::Asserted,
        "system",
    )
    .with_payload(serde_json::json!({
        "port": PROBE,
        "id": "n1",
        "attempt": 1,
    }));
    let debts = owed(frozen.rows(), &frozen, &[request], 0);
    assert!(
        debts.iter().any(|d| d.decl.name == row.name && !d.blocked),
        "probe_echo 的 request 事实应选中 `{}`，实际欠账：{:?}",
        row.name,
        debts
            .iter()
            .map(|d| (d.decl.name, d.blocked))
            .collect::<Vec<_>>()
    );
}

/// 夹具根：经 `creator_create_object` 取探针与环——与真实装配同一条注册路径，
/// 根自己不认识这两个插件的端口内容。
struct ProbeRoot;

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
        let probe =
            creator_create_object::<dyn Plugin>(symbio_deploy::PLUGIN_ID_PROBE, Arc::clone(&ctx))
                .ok_or_else(|| PluginError::NotFound("探针插件未注册".into()))?;
        probe.traverse(path.clone(), Arc::clone(&ctx)).await?;
        let ring =
            creator_create_object::<dyn Plugin>(symbio_deploy::PLUGIN_ID_LOOP, Arc::clone(&ctx))
                .ok_or_else(|| PluginError::NotFound("环插件未注册".into()))?;
        ring.traverse(path, ctx).await
    }
}

#[tokio::test]
async fn the_probe_port_runs_through_the_ring() {
    let host = Host::new(Arc::new(ProbeRoot));
    let report = host
        .drive(
            &base_context(),
            "probe-acceptance",
            None,
            ExecEventSink::silent(),
        )
        .await
        .expect("装配成立");

    assert_eq!(report.dispatched, 1, "环选中了探针自报的那一笔债");
    assert_eq!(report.outcome, DriveOutcome::NoDebt, "还清后无债");

    let facts = host.facts();
    let response = facts
        .iter()
        .find(|fact| fact.closing() == Some(true))
        .expect("执行体还了事实，环按同一债键配对");
    assert_eq!(response.key.port, PROBE, "环只按插件给的端口名解析");
    assert_eq!(response.key.id, "probe:probe-acceptance");
    assert_eq!(
        response.payload["echo"], "hello",
        "载荷字符串原样透传到执行体再回来"
    );

    // 惰性：换了会话前缀，同一装配不欠账——这就是它进了默认树也不扰动既有会话的原因。
    let other = host
        .drive(&base_context(), "e2e-t47", None, ExecEventSink::silent())
        .await
        .expect("非探针会话仍可驱动");
    assert_eq!(other.dispatched, 0, "非 probe- 前缀的会话不欠探针债");
}
