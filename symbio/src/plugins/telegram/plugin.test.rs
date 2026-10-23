//! `symbio/src/plugins/telegram/plugin.rs` 的单元测试 —— 与实现同级分文件。

use super::*;
use crate::symbio_core::{PluginSimpleRequest, PATH};

fn temp_plugin() -> (tempfile::TempDir, PluginDir, Arc<TelegramPlugin>) {
    let tmp = tempfile::TempDir::new().unwrap();
    let dir = PluginDir::at(tmp.path().join("telegram"), "telegram");
    let plugin = Arc::new(TelegramPlugin::new(TelegramConfig::default(), dir.clone()));
    (tmp, dir, plugin)
}

/// 游标与自动绑定的 `chat_id` 都是**跨重启事实**：确认一批更新后必须能在
/// 同目录重新加载出来，否则重启就会重复处理历史更新 / 丢失绑定。
#[tokio::test]
async fn acknowledge_updates_persists_cursor_and_captured_chat_id() {
    let (_tmp, dir, plugin) = temp_plugin();
    let updates = vec![
        json!({
            "update_id": 100_100,
            "message": { "chat": { "id": -42 }, "from": { "id": 7 }, "text": "hi" }
        }),
        json!({ "update_id": 100_105 }),
    ];

    acknowledge_updates(&plugin.config, &plugin.config_file, &updates).await;

    let loaded = dir
        .load::<TelegramConfig>()
        .expect("读取落盘配置不应失败")
        .expect("确认后应已落盘");
    assert_eq!(
        loaded.update_offset, 100_106,
        "游标取整批最大的 update_id + 1"
    );
    assert_eq!(
        loaded.chat_id.as_deref(),
        Some("-42"),
        "首条消息的 chat 应被绑定"
    );

    // 重启：同目录重建实例，游标与绑定都从 PLUGIN.yml 恢复。
    let restarted =
        TelegramPlugin::new(dir.load::<TelegramConfig>().unwrap().unwrap(), dir.clone());
    assert_eq!(restarted.config.read().await.update_offset, 100_106);
    assert_eq!(
        restarted.config.read().await.chat_id.as_deref(),
        Some("-42")
    );

    // 更旧的一批（重放/乱序）不得让游标倒退。
    acknowledge_updates(
        &plugin.config,
        &plugin.config_file,
        &[json!({ "update_id": 100_001 })],
    )
    .await;
    assert_eq!(
        dir.load::<TelegramConfig>().unwrap().unwrap().update_offset,
        100_106,
        "游标是单调的：旧 update_id 不能让它倒退"
    );
}

/// `set_chat_id` 是路由臂，绑定必须落盘而不是只改内存。
#[tokio::test]
async fn set_chat_id_persists_to_plugin_file() {
    let (_tmp, dir, plugin) = temp_plugin();

    let req = PluginSimpleRequest::new(None, None);
    req.set(PATH, "set_chat_id".to_string());
    req.set_payload(TelegramConfig {
        chat_id: Some("-100".into()),
        ..Default::default()
    })
    .unwrap();

    plugin
        .clone()
        .route(Arc::new(req))
        .await
        .expect("set_chat_id 应成功");

    assert_eq!(
        dir.load::<TelegramConfig>()
            .unwrap()
            .expect("绑定应已落盘")
            .chat_id
            .as_deref(),
        Some("-100")
    );
}

/// 停用 ⇒ 长轮询的取消令牌被触发，运行位复位（相应批次）。
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
