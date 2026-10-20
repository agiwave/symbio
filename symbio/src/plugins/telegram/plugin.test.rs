//! `symbio/src/plugins/telegram/plugin.rs` 的单元测试 —— 与实现同级分文件。

use super::*;

/// 停用 ⇒ 长轮询的取消令牌被触发，运行位复位（[T-185](../../../../docs/009-task/185-插件停用收口批.md "T-185")）。
///
/// 判据落在**取消位真的被触发**上：只把 `listener_running` 置 false 而不 cancel，
/// 后台任务会在下一次循环里继续长轮询——那是「看起来停了」。
#[tokio::test]
async fn stop_cancels_the_listener_token() {
    let dir = PluginDir::at(
        std::env::temp_dir().join("symbio-test-telegram-stop"),
        "telegram",
    );
    let plugin = Arc::new(TelegramPlugin::new(TelegramConfig::default(), dir));

    let token = CancellationToken::new();
    *plugin.listener_token.write().await = Some(token.clone());
    plugin.listener_running.store(true, Ordering::SeqCst);

    plugin
        .clone()
        .stop(PluginStopReason::Disabled)
        .await
        .expect("停用 telegram 插件不应失败");

    assert!(token.is_cancelled(), "停用必须触发取消令牌");
    assert!(!plugin.listener_running.load(Ordering::SeqCst));
    assert!(plugin.listener_token.read().await.is_none());
}
