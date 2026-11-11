//! 探针端口的判据：新增一个世界端口只改能力插件 crate，不改架构层。
//!
//! 两条用例分别是「注册面」与「执行面」：前者证明提供者照常写进装配期容器、按名取得到，
//! 后者证明债与执行体都在本 crate 内闭合。两者合起来就是
//! **改动架构文件数 = 0** 的可执行形态（D-394 / PTS-007）。

use std::sync::Arc;

use symbio_plugin_api::port::{
    register_ports, PortBindings, PortExecContext, PortFactEmit, PortInput, PortProvider,
    PORT_BINDINGS,
};
use symbio_plugin_api::{
    ExecAbortSignal, ExecEventSink, PluginInvokeRequest, PluginInvokeRequestExt as _,
    PluginSimpleRequest,
};

use super::*;

/// 注册面：一个能力插件申报新端口后，绑定照常进装配期容器并可按名取回——
/// 容器与判重口径都由 `plugin-api` 一份持有，本 crate 只给端口名。
#[tokio::test]
async fn a_new_world_port_registers_without_architecture_changes() {
    let provider: Arc<dyn PortProvider> = Arc::new(ProbePort);
    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
    let bindings = Arc::new(PortBindings::default());
    ctx.set(PORT_BINDINGS, bindings.clone());
    register_ports(&ctx, provider.as_ref()).await;

    assert_eq!(
        bindings.bound(),
        vec![WEB_PROBE_PORT.to_string()],
        "一个端口一条绑定：端口名是数据"
    );
    assert!(
        bindings.get(WEB_PROBE_PORT).expect("无冲突").is_some(),
        "探针端口登记后应可按名取回"
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
                port: WEB_PROBE_PORT.to_string(),
                payload: spec.payload.clone(),
            },
        )
        .await;
    assert!(outcome.success());
    assert_eq!(outcome.facts, vec![spec.payload]);
}
