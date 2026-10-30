//! `tool` 模块的单元测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）。
//!
//! 这里锁的是**静态面**：给模型的声明（`meta`）与回执形状（`view`），以及
//! 「拿不到宿主 / 拿不到 VDFS 视图」时诚实报错而不是猜一个默认值。带会话的完整链路
//! 由 `crates/symbio-plugin-session` 的存储用例与端到端验证覆盖——本文件不 mock 地址面。

use super::*;
use symbio_plugin_api::PluginSimpleRequest;

/// 工具名与 `action` 枚举：模型侧唯一的入口声明，改名等于工具消失。
#[test]
fn meta_declares_the_three_actions() {
    let meta = HeartbeatTool::new(Weak::new()).meta();
    assert_eq!(meta.name, "heartbeat");
    let actions: Vec<&str> = meta.input_schema["properties"]["action"]["enum"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    assert_eq!(actions, vec!["set", "get", "cancel"]);
}

/// 回执就是 `HeartbeatConfig` 的四格：工具的每次回执回这一份，调度侧下一次读的
/// 也是这一份——两处不同义时，「设置了却没生效」没有任何信号。
#[test]
fn view_matches_config_slots() {
    let hb = HeartbeatConfig {
        enabled: true,
        interval_seconds: 60,
        prompt: "巡检".into(),
        include_history: false,
    };
    let v = hb.view();
    assert_eq!(v["enabled"], json!(true));
    assert_eq!(v["interval_seconds"], json!(60));
    assert_eq!(v["prompt"], json!("巡检"));
    assert_eq!(v["include_history"], json!(false));
}

/// 宿主插件已被释放（能力行里的 `Weak` 失效）⇒ 诚实报错。
///
/// 工具行随能力表活到下一轮收集，而插件实例可能先走。这一跳不报错的替代方案是
/// 「返回一份默认配置」——那会让模型以为会话没配心跳，而实际上只是问不到。
#[tokio::test]
async fn execute_without_a_live_host_reports_honestly() {
    let tool = HeartbeatTool::new(Weak::new());
    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
    let env = ExecEnv::from_request(&*ctx);

    let err = tool
        .execute(json!({ "action": "get" }), &env, ctx)
        .await
        .expect_err("没有宿主必须失败");
    assert!(
        matches!(err, PluginError::InternalError(_)),
        "失效的弱引用该报内部错误，而不是校验错误: {err:?}"
    );
}

/// 容器不可达（本插件没有父）⇒ 取不到 VDFS 视图，同样诚实报错。
///
/// 这条锁的是「不回落成一份进程级全局 provider」——那等于绕过容器去看别人的树。
#[tokio::test]
async fn execute_without_a_provider_reports_honestly() {
    let plugin = Arc::new(HeartbeatPlugin::new(None));
    let tool = HeartbeatTool::new(Arc::downgrade(&plugin));
    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
    ctx.set(SESSION_ID, "s-1".to_string());
    let env = ExecEnv::from_request(&*ctx);

    let err = tool
        .execute(json!({ "action": "get" }), &env, ctx)
        .await
        .expect_err("没有 VDFS 视图必须失败");
    assert!(
        matches!(err, PluginError::InternalError(_)),
        "取不到视图是内部条件不满足: {err:?}"
    );
}
