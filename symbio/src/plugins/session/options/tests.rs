//! `options` 模块的单元测试。
//!
//! 与实现分文件（约定同 `store/tests.rs` / `chat_session/tests.rs`）：
//! `options.rs` 只保留生产代码，测试全部放本文件。

use super::*;
// 仅测试断言用（生产路径已改为 `OptionAction::session_state_bind`）
use crate::symbio_core::schemas::options::SESSION_STATE_ENDPOINT;
// 未装配容器时没有 PLUGIN_DIR，配置文件落盘目标指个临时目录
use crate::plugins::session::test_dir;

#[test]
fn basename_handles_both_separators() {
    assert_eq!(basename("D:\\work\\proj"), "proj");
    assert_eq!(basename("/home/u/proj/"), "proj");
    assert_eq!(basename("plain"), "plain");
}

#[test]
fn state_invoke_carries_metadata_patch() {
    let n = OptionNode::session_state("mode:auto", "自动", "mode", json!("auto"));
    assert_eq!(n.id, "mode:auto");
    let action = n.action.expect("action 存在");
    assert_eq!(action.endpoint, SESSION_STATE_ENDPOINT);
    assert_eq!(action.payload["metadata"]["mode"], json!("auto"));
    assert_eq!(n.value.as_deref(), Some("auto"));
}

#[test]
fn heartbeat_form_keeps_basic_settings_visible() {
    use crate::symbio_core::schemas::session::session_config::SessionConfig;

    let plugin = SessionPlugin::new(None, SessionConfig::default(), test_dir());
    let node = plugin.heartbeat_option(None);
    let form = node.form.expect("心跳表单定义存在");
    let fields = &form.sections[0].fields;

    // 基础设置必须恒可见（不得带 visible_when）：否则未启用时表单只剩开关，
    // 用户看不到任何可填参数（历史回归）。
    let keys: Vec<&str> = fields.iter().map(|f| f.key.as_str()).collect();
    assert_eq!(
        keys,
        vec!["enabled", "interval_seconds", "prompt", "include_history"]
    );
    for f in fields {
        assert!(f.visible_when.is_none(), "字段 {} 不应带显隐门控", f.key);
    }

    // 表单初始数据回落默认值：未启用 / 300 秒 / 空提示词 / 携带历史
    let data = node.data.expect("表单初始数据存在");
    assert_eq!(data["enabled"], json!(false));
    assert_eq!(data["interval_seconds"], json!(DEFAULT_HEARTBEAT_INTERVAL));
    assert_eq!(data["prompt"], json!(""));
    assert_eq!(data["include_history"], json!(true));

    // 保存动作：写入 metadata.heartbeat（bind 点路径，统一走 session/update）
    let action = node.action.expect("保存动作存在");
    assert_eq!(action.endpoint, SESSION_STATE_ENDPOINT);
    assert_eq!(action.bind.as_deref(), Some("metadata.heartbeat"));
    // 节点为 form 类型，且禁用态下仍展示「未开启」
    assert!(matches!(node.option_type, OptionType::Form));
    assert_eq!(node.value_label.as_deref(), Some("未开启"));
}

#[test]
fn heartbeat_trigger_is_disabled_without_enabled_and_prompt() {
    use crate::symbio_core::schemas::session::session_config::SessionConfig;

    let plugin = SessionPlugin::new(None, SessionConfig::default(), test_dir());

    // 未启用 / 缺提示词 → 只读并说明原因
    let off = plugin.heartbeat_trigger_option(None);
    assert_eq!(off.option_type, OptionType::Invoke);
    assert!(!off.enabled);
    assert_eq!(off.status, OPTION_STATUS_DISABLED);
    assert_eq!(
        off.action.expect("触发动作存在").endpoint,
        HEARTBEAT_TRIGGER_ENDPOINT
    );

    // 已启用且提示词非空 → 可点击
    let on = plugin.heartbeat_trigger_option(Some(json!({
        "enabled": true,
        "prompt": "推进一项小任务",
    })));
    assert!(on.enabled);
    assert_eq!(on.status, OPTION_STATUS_ACTIVE);
}

#[test]
fn find_node_is_depth_first() {
    let inner = OptionNode::sub(
        "parent",
        "P",
        vec![OptionNode::invoke("child", "C", OptionAction::default())],
    );
    let nodes = vec![inner];
    assert!(find_node(&nodes, "child").is_some());
    assert!(find_node(&nodes, "missing").is_none());
}

#[test]
fn inject_session_scope_is_recursive_and_non_destructive() {
    let child = OptionNode::session_state("mode:auto", "自动", "mode", json!("auto"));
    let parent = OptionNode::sub("mode", "运行模式", vec![child]);
    let mut nodes = vec![parent];

    inject_session_scope(&mut nodes, "s1");

    // 父节点无 action（sub）→ 不注入；子节点的 metadata 载荷保留并补 session_id
    assert!(nodes[0].action.is_none());
    let payload = nodes[0].children[0]
        .action
        .as_ref()
        .unwrap()
        .payload
        .clone();
    assert_eq!(payload["session_id"], json!("s1"));
    assert_eq!(payload["metadata"]["mode"], json!("auto"));

    // 已显式声明 session_id 的载荷不被覆盖
    let mut fixed = vec![OptionNode::invoke(
        "x",
        "X",
        OptionAction {
            endpoint: "e".into(),
            payload: json!({ "session_id": "own", "v": 1 }),
            ..Default::default()
        },
    )];
    inject_session_scope(&mut fixed, "s1");
    assert_eq!(
        fixed[0].action.as_ref().unwrap().payload["session_id"],
        json!("own")
    );
}
