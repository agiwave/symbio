//! `plugin.rs` 的单元测试 —— 收集期发布句柄 + 内置档只读（D-531 / 裁决 B）。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;
use symbio_plugin_api::PluginSimpleRequest;

fn ctx() -> Arc<dyn PluginInvokeRequest> {
    Arc::new(PluginSimpleRequest::new(None, None))
}

fn test_plugin() -> Arc<SandboxPlugin> {
    test_plugin_in(std::env::temp_dir().join(format!(
        "sandbox-test-{}-{}",
        std::process::id(),
        TEST_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )))
}

fn test_plugin_in(dir: std::path::PathBuf) -> Arc<SandboxPlugin> {
    Arc::new(SandboxPlugin {
        dir: PluginDir::at(dir, PLUGIN_ID_SANDBOX),
        registry: Arc::new(RwLock::new(SandboxRegistry::default())),
    })
}

static TEST_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[tokio::test]
async fn traverse_publishes_a_handle_into_the_shared_slot() {
    // 用同一条 ctx lineage：先建槽，traverse 写入，再从同一 ctx 读回。
    let c = ctx();
    symbio_plugin_api::sandbox_init_slot(&c);
    let plugin = test_plugin();
    let _ = plugin.clone().traverse(String::new(), c.clone()).await;
    assert!(
        symbio_plugin_api::sandbox_of(&*c).is_some(),
        "traverse 后消费方应能读到句柄"
    );
}

#[tokio::test]
async fn reload_seeds_builtin_profiles() {
    let plugin = test_plugin();
    plugin.reload().await;
    let reg = plugin.registry.read().unwrap();
    assert!(reg.archives().contains_key("default"));
    assert!(reg.archives().contains_key("strict"));
    assert!(reg.archives().contains_key("unattended"));
    assert!(reg.archives().get("strict").unwrap().builtin);
}

#[test]
fn builtin_ids_are_not_writable() {
    for id in ["default", "strict", "unattended"] {
        assert!(builtin_profiles().iter().any(|(k, _)| k == id));
    }
}

#[test]
fn archive_node_carries_detail_definition() {
    // form 渲染器的 definition 完全来自 node.schema；漏挂会让右栏空白。
    let cfg = SandboxProfileConfig {
        name: "严格".into(),
        builtin: true,
        ..Default::default()
    };
    let node = SandboxPlugin::archive_node("strict", &cfg);
    assert_eq!(node.ext.as_deref(), Some(VDFS_EXT_FORM));
    let schema = node.schema.expect("档案节点必须自携带详情定义");
    let sections = schema.get("sections").and_then(|v| v.as_array());
    assert!(
        sections.is_some_and(|s| !s.is_empty()),
        "详情定义必须带非空 sections，否则 DetailForm 渲染白屏：{schema}"
    );
    assert_eq!(
        schema,
        detail_definition(),
        "节点 schema 与新建类型必须同源"
    );
}

#[tokio::test]
async fn write_accepts_empty_json_facts_fields() {
    // 回归：两个 JSON textarea 的初始值是空串（「留空=不限制」），
    // 若 Write 直接 serde_json::from_str("") 会让新建沙箱保存失败。
    let plugin = test_plugin();
    let payload = serde_json::json!({
        "name": "custom",
        "approval_mode": "interactive",
        "risk_threshold": "medium",
        "allowed_facts_json": "",
        "blocked_tools_json": "",
    })
    .to_string();
    let resp = plugin
        .dispatch(
            &VdfsContext::empty(),
            "",
            VdfsRequest::Write {
                content: VdfsContent::text(payload).with_create(),
            },
        )
        .await
        .expect("留空的 JSON 字段必须能保存（= 不限制）");
    let VdfsResponse::Write(w) = resp else {
        panic!("期望 Write 响应，实得：{resp:?}")
    };
    // 新建 id 由 provider 生成（与 mcp / skill 一致），不是 name 字段。
    let id = w.name.expect("匿名写必须回传生成的 id");
    plugin.reload().await;
    let reg = plugin.registry.read().unwrap();
    let cfg = reg.archives().get(&id).expect("新档案必须落在注册表");
    assert!(cfg.allowed_facts.is_empty(), "留空 = 不限制（空白名单）");
    assert!(cfg.blocked_tools.is_empty(), "留空 = 不拦截（空黑名单）");
}
