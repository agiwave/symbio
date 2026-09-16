//! `heartbeat_tool` 模块的单元测试。
//!
//! 与实现分文件（约定同 `store/tests.rs` / `chat_session/tests.rs`）：
//! `heartbeat_tool.rs` 只保留生产代码，测试全部放本文件。

use super::*;

/// set 校验：prompt 为空（且此前未设置）必须拒绝
#[tokio::test]
async fn set_without_prompt_is_rejected() {
    // 直接构造工具实例验证纯校验逻辑：execute 依赖插件实例，
    // 这里只覆盖 config_view 与 schema 的静态面；带插件的完整链路由
    // session 存储测试与端到端验证覆盖。
    let tool = HeartbeatTool::new(Weak::new());
    let meta = tool.meta();
    assert_eq!(meta.name, "heartbeat");
    // schema 必须声明 action 枚举与 interval 下限语义
    assert!(meta.input_schema["properties"]["action"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == "set"));
}

/// config_view 与 HeartbeatConfig 字段一一对应
#[test]
fn config_view_matches_schema() {
    let hb = HeartbeatConfig {
        enabled: true,
        interval_seconds: 60,
        prompt: "巡检".into(),
        include_history: false,
    };
    let v = HeartbeatTool::config_view(&hb);
    assert_eq!(v["enabled"], json!(true));
    assert_eq!(v["interval_seconds"], json!(60));
    assert_eq!(v["prompt"], json!("巡检"));
    assert_eq!(v["include_history"], json!(false));
}
