//! `heartbeat` 插件本体的单元测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）。
//!
//! 这里锁的是**本片对外声明的那几格**：能力表里恰好两行（Tool + Option）、
//! 停机时那条常驻循环真的退出。两半各管一件事，合起来是 ADR-077 的守卫前半——
//! 「停用 `heartbeat` ⇒ 这两行一起消失」的**跨插件**那一半在
//! `plugins/composite` 的守卫用例里（本文件不能 import 兄弟模块，E-009）。

use super::*;
use crate::providers::DefaultToolVisitor;
use crate::symbio_core::{
    capability_option_fields, CapabilityRowKind, Plugin, PluginSimpleRequest, PluginStopReason,
    Registry,
};

fn plugin() -> Arc<HeartbeatPlugin> {
    Arc::new(HeartbeatPlugin::new(None))
}

/// E-001 的自证：`PluginMeta` 首参必须等于插件目录名（容器按目录名分发）
#[test]
fn meta_id_matches_plugin_dir() {
    assert_eq!(HeartbeatPlugin::metadata().id, PLUGIN_ID_HEARTBEAT);
    assert_eq!(PLUGIN_ID_HEARTBEAT, "heartbeat");
}

/// 工具广播 ⇒ 恰好贡献一行 `heartbeat`（模型侧设置入口）。
///
/// 这是「心跳工具还在 LLM 清单里」在生产路径上的**唯一注册点**：漏注册的表现为
/// 模型调不到 `heartbeat`，而不会有任何编译错误。
#[tokio::test]
async fn traverse_contributes_the_tool_row() {
    let p = plugin();
    let ctx = Arc::new(PluginSimpleRequest::new(None, None)).fork();
    ctx.set(PATH, TRAVERSE_AVAILABLE_TOOLS.to_string());
    let visitor: Arc<dyn Registry> = Arc::new(DefaultToolVisitor::new());
    ctx.set(CAPABILITY_VISITOR, visitor.clone());

    p.traverse(String::new(), ctx).await.expect("traverse 成功");

    let rows = visitor.list(CapabilityRowKind::Tool).await;
    assert_eq!(rows.len(), 1, "本插件在工具族恰好贡献一行");
    assert_eq!(rows[0].name(), "heartbeat");
    assert_eq!(rows[0].meta().name, "heartbeat");
}

/// 选项广播 ⇒ 恰好贡献 `heartbeat` 一行，且号段是登记的 `60`。
#[tokio::test]
async fn traverse_contributes_the_option_row() {
    let p = plugin();
    let ctx = Arc::new(PluginSimpleRequest::new(None, None)).fork();
    ctx.set(PATH, TRAVERSE_AVAILABLE_OPTIONS.to_string());
    let visitor: Arc<dyn Registry> = Arc::new(DefaultToolVisitor::new());
    ctx.set(OPTION_VISITOR, visitor.clone());

    p.traverse(String::new(), ctx).await.expect("traverse 成功");

    let keys: Vec<String> = capability_option_fields(&*visitor)
        .await
        .into_iter()
        .map(|f| f.key)
        .collect();
    assert_eq!(keys, vec!["heartbeat".to_string()]);
}

/// **没有第三行**：本插件不挂 VDFS 点、不贡献模型 / 提示词 / 配置文档。
///
/// 判据是「它对世界只说三句话，全走别人的地址面」（模块文档）。这条用例锁的是
/// 别在 `traverse` 里顺手加一行 `mount` —— 那会让心跳拥有第二份会话视图。
#[tokio::test]
async fn traverse_contributes_nothing_else() {
    let p = plugin();
    let ctx = Arc::new(PluginSimpleRequest::new(None, None)).fork();
    ctx.set(PATH, TRAVERSE_AVAILABLE_TOOLS.to_string());
    let visitor: Arc<dyn Registry> = Arc::new(DefaultToolVisitor::new());
    ctx.set(CAPABILITY_VISITOR, visitor.clone());

    p.traverse(String::new(), ctx).await.expect("traverse 成功");

    assert_eq!(visitor.list(CapabilityRowKind::Tool).await.len(), 1);
    for kind in [
        CapabilityRowKind::Mount,
        CapabilityRowKind::Model,
        CapabilityRowKind::Prompt,
        CapabilityRowKind::Option,
        CapabilityRowKind::Configurable,
    ] {
        assert!(
            visitor.list(kind).await.is_empty(),
            "工具广播里不该出现 {kind:?} 行"
        );
    }
}

/// `stop` 之后扫描循环**真的退出了**（ADR-077 后果 #4 的可执行形态）。
///
/// 循环体在 `start_scan` 里 spawn；没有 runtime 时它一条也不起，所以本用例必须在
/// `#[tokio::test]` 下跑。断言的是「在册句柄已 finished」，不是「停机位置了」——
/// 后者只证明有人按过按钮。
#[tokio::test]
async fn stop_terminates_the_scan_loop() {
    let p = plugin();
    p.start_scan();

    Plugin::stop(p.clone(), PluginStopReason::Disabled)
        .await
        .expect("停机不该失败");

    assert!(p.scan_handle_finished(), "stop 返回时扫描循环仍在跑");
}

/// 没起过循环（构造方不在 runtime 里）时，`stop` 是**无事可做**而不是失败。
#[tokio::test]
async fn stop_without_a_scan_is_ok() {
    let p = plugin();
    Plugin::stop(p.clone(), PluginStopReason::Shutdown)
        .await
        .expect("没启动过的插件停机不该报错");
    assert!(!p.scan_handle_finished(), "句册本来就是空的");
}
