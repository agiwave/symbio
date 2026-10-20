//! `symbio/src/plugins/hook/plugin.rs` 的单元测试 —— 与实现同级分文件。

use super::super::registry::{HookConfigEntry, HookType};
use super::*;

/// 停用 ⇒ 注册态清空（[T-185](../../../../docs/009-task/185-插件停用收口批.md "T-185")）。
///
/// 判据是**名单**而不是「调用过 stop」：注册只活在内存里，插件被停用后若名单仍在，
/// 下一次 `hook/list` 会给出一个已经不该存在的答案。
#[tokio::test]
async fn stop_clears_the_registered_hooks() {
    let plugin = Arc::new(HooksPlugin::new(None));
    plugin
        .registry
        .write()
        .await
        .register(&HookRegistration {
            event: "session.start".into(),
            hooks: vec![HookConfigEntry {
                name: "probe".into(),
                hook_type: HookType::Command,
                command: Some("echo hi".into()),
                url: None,
                timeout_ms: None,
                matcher: None,
            }],
        })
        .await;
    assert_eq!(plugin.registry.read().await.list_hooks().await.len(), 1);

    plugin
        .clone()
        .stop(PluginStopReason::Disabled)
        .await
        .expect("停用 hook 插件不应失败");

    assert!(
        plugin.registry.read().await.list_hooks().await.is_empty(),
        "停用后注册表应为空"
    );
}
