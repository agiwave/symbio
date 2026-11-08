//! 装配验收的断言（通用判据，无任何插件专名）。
//!
//! 两条用例回答两个不同问题：
//! 1. **装配清单里的包能被装出来**：工厂已注册、构造成功、出厂 id 等于目录名
//!    （E-001）。这条不依赖某个具体包，新增包自动被遍历到。
//! 2. **已链接包交来的端口都进了冻结表并有受理行**：`submit_declarations!` 的每一条
//!    `PortDecl` 都在 `sealed_declarations()` 里，且存在 `by = PORT_ACTOR`、
//!    `owes = request(port=<该端口>)` 的行。这条同样不看包名，也不要求包在默认装配里
//!    ——只由测试装配的包（`plugin_assembly: none`）照样被覆盖。
//!
//! 工具面不变由 e2e 字节棘轮持有（文件头已说明），这里不重复。

use std::sync::Arc;

use symbio_plugin_api::inventory;
use symbio_plugin_api::ring::Lit;
use symbio_plugin_api::rows::PORT_ACTOR;
use symbio_plugin_api::{
    creator_create_object, creator_ids, sealed_declarations, DeclarationsDecl, Plugin, PluginDir,
    PluginInvokeRequest, PluginInvokeRequestExt, PluginSimpleRequest, EVENT_REQUEST, PLUGIN_DIR,
};

#[tokio::test]
async fn every_assembled_package_can_be_constructed() {
    // 必须经本 crate 的门面触达一次，全部插件的链接行才会进最终链接；否则本二进制里
    // `inventory` 为空，判据退化成空跑。
    let _ = crate::assembly::PLUGIN_ID_HOME;

    let linked: Vec<&'static str> = creator_ids::<dyn Plugin>();
    for id in symbio_deploy::ASSEMBLY_SUB_AGENT_PLUGINS {
        assert!(
            linked.contains(id),
            "装配表里的 `{id}` 没有注册工厂（`submit_object_creator!` 漏了？）"
        );
    }

    // 遍历**全部已链接包**（含只由测试装配的包），逐个构造。加包不加文件：
    // 新 crate 只要链接进来就自动进入这份清单。
    let tmp = tempfile::tempdir().expect("临时插件目录");
    // 遍历里会构造 Home，而 Home 构造时 publish 部署根：与驱动用例串行，
    // 否则 t29 的「先 publish、后解析」会被这里的构造插队（见锁的注释）。
    let _serial = crate::DEPLOYMENT_ROOT_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    for id in linked {
        let dir = tmp.path().join(id);
        std::fs::create_dir_all(&dir).expect("建临时插件目录");
        let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
        ctx.set(PLUGIN_DIR, PluginDir::at(dir, id));
        let plugin = creator_create_object::<dyn Plugin>(id, ctx)
            .unwrap_or_else(|| panic!("工厂 `{id}` 已注册但构造失败"));
        assert_eq!(
            plugin.meta().id,
            id,
            "PluginMeta 首参必须等于目录名（E-001）"
        );
    }
}

#[test]
fn every_submitted_port_is_in_the_sealed_table_with_an_executor_row() {
    let frozen = sealed_declarations().expect("生产声明表应能冻结（含 loop 与全部已链接插件）");
    assert!(
        frozen.actors().iter().any(|a| a.name == PORT_ACTOR),
        "端口行绑定后 `{PORT_ACTOR}` 应进执行体声明表"
    );

    let mut ports = 0;
    for decl in inventory::iter::<DeclarationsDecl>() {
        let submitted = (decl.make)();
        for port in submitted.ports {
            ports += 1;
            assert!(
                frozen.ports().iter().any(|p| p.port == port.port),
                "`{}` 申报的端口 `{}` 没有被冻结表接纳",
                decl.owner,
                port.port
            );
            assert!(
                frozen.rows().iter().any(|r| {
                    r.by == Some(PORT_ACTOR)
                        && r.owes.source == EVENT_REQUEST
                        && r.owes
                            .eq
                            .iter()
                            .any(|(field, lit)| *field == "port" && *lit == Lit::Str(port.port))
                }),
                "`{}` 申报的端口 `{}` 没有 `by = {PORT_ACTOR}` 的受理行",
                decl.owner,
                port.port
            );
        }
    }
    assert!(ports > 0, "至少要有一个包申报端口，否则这条判据是空跑");
}
