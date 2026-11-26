//! Symbio 初始化模块

use std::sync::Arc;
use symbio_platform::deploy::PLUGIN_ID_HOME;
use symbio_platform::kernel::plugin::Plugin;
use symbio_platform::sdk::creator::creator_create_object;

// 编排 runner 由组合根登记；宿主优先于 `drive` 族回退使用。
inventory::submit! {
    symbio_runtime::host::DriveRunnerDecl {
        make: || Some(symbio_providers::orchestrator::runner()),
    }
}

pub fn initialize() {
    symbio_platform::sdk::logger::logger_init();
}

pub fn root_factory() -> Arc<dyn symbio_runtime::RootFactory> {
    Arc::new(symbio_runtime::RootFactoryFn(|_| create_root_plugin()))
}

pub async fn create_root_plugin() -> Arc<dyn Plugin> {
    let context = Arc::new(symbio_platform::kernel::plugin::PluginSimpleRequest::new(
        None, None,
    ));

    creator_create_object::<dyn Plugin>(PLUGIN_ID_HOME, context).expect(
        "home plugin creator not found. Make sure 'home' is registered via submit_object_creator!",
    )
}

// ── panic 面登记（PN-001…003）─────────────────────────────────────────
// 本文件每一处 `unwrap` / `expect` / `panic!` / `unreachable!` 的理由。登记放在
// 文件内而不是集中一张表：理由与它解释的那段代码会一起被 review、一起被删。
// 判据见 `scripts/audits/panic-audit.mjs`。**加一处 panic 必须同时加一行登记，理由非空。**
// panic-allow crates/symbio/src/init.rs::create_root_plugin: 装配期：拿不到就是启动失败。带着半套插件继续跑，比崩掉更难诊断。
