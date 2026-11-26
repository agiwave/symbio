//! `plugin.rs` 的单元测试 —— 收集期发布句柄 + 内置档只读（ / 裁决 B）。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;
use symbio_plugin_api::{vdfs_context, PluginSimpleRequest, SymbioKey, TOOL_CALL_ID};

fn ctx() -> Arc<dyn PluginInvokeRequest> {
    Arc::new(PluginSimpleRequest::new(None, None))
}

fn management_ctx() -> VdfsContext {
    let host = ctx();
    host.set(VDFS_CALLER, VdfsCaller::Manager);
    vdfs_context(&host)
}

fn model_ctx() -> VdfsContext {
    let host = ctx();
    host.set(VDFS_CALLER, VdfsCaller::Model);
    host.set(TOOL_CALL_ID, "model-call-1".to_string());
    vdfs_context(&host)
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
        reload_lock: Arc::new(tokio::sync::Mutex::new(())),
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
    assert_eq!(
        reg.archives()
            .get("strict")
            .unwrap()
            .risk_threshold
            .as_deref(),
        Some("low")
    );
    assert_eq!(
        reg.archives()
            .get("default")
            .unwrap()
            .risk_threshold
            .as_deref(),
        Some("medium")
    );
    assert_eq!(
        reg.archives()
            .get("unattended")
            .unwrap()
            .risk_threshold
            .as_deref(),
        Some("medium")
    );
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
async fn write_accepts_structured_json_fields() {
    // json widget 提交结构化值；字段留空时前端省略该键，serde default 生效。
    let plugin = test_plugin();
    let payload = serde_json::json!({
        "name": "custom",
        "approval_mode": "interactive",
        "risk_threshold": "medium",
        // allowed_facts / blocked_tools 留空（前端省略）⇒ 空 HashMap
    })
    .to_string();
    let resp = plugin
        .dispatch(
            &management_ctx(),
            "",
            VdfsRequest::Write {
                content: VdfsContent::text(payload).with_create(),
            },
        )
        .await
        .expect("结构化字段必须能保存（留空 = 不限制）");
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

#[tokio::test]
async fn write_roundtrips_structured_json_fields() {
    // json widget 提交的对象能原样落盘、读回（格式化由前端承担，后端只存结构）。
    let plugin = test_plugin();
    let payload = serde_json::json!({
        "name": "structured",
        "approval_mode": "interactive",
        "risk_threshold": "medium",
        "allowed_facts": { "shell_command": ["git status", "ls"] },
        "blocked_tools": { "dangerous_tool": { "blocked": true } },
    })
    .to_string();
    let resp = plugin
        .dispatch(
            &management_ctx(),
            "",
            VdfsRequest::Write {
                content: VdfsContent::text(payload).with_create(),
            },
        )
        .await
        .expect("结构化字段必须能保存");
    let VdfsResponse::Write(w) = resp else {
        panic!("期望 Write 响应，实得：{resp:?}")
    };
    let id = w.name.expect("匿名写必须回传生成的 id");
    let read = plugin
        .dispatch(&management_ctx(), &id, VdfsRequest::Read)
        .await
        .expect("读回档案");
    let VdfsResponse::Read(content) = read else {
        panic!("期望 Read 响应，实得：{read:?}")
    };
    let value: serde_json::Value = serde_json::from_str(content.text.as_deref().unwrap_or(""))
        .expect("read 内容应是合法 JSON");
    assert_eq!(
        value.get("allowed_facts").unwrap(),
        &serde_json::json!({
            "shell_command": ["git status", "ls"]
        })
    );
    assert_eq!(
        value.get("blocked_tools").unwrap(),
        &serde_json::json!({
            "dangerous_tool": { "blocked": true }
        })
    );
    // 不再下发 `_json` 字符串字段。
    assert!(value.get("allowed_facts_json").is_none());
    assert!(value.get("blocked_tools_json").is_none());
}

#[tokio::test]
async fn write_roundtrips_rule_table() {
    // 规则表格提交的对象数组能原样落盘、读回（表格 widget 的存储态就是行对象数组）。
    let plugin = test_plugin();
    let payload = serde_json::json!({
        "name": "rules-profile",
        "approval_mode": "interactive",
        "risk_threshold": "medium",
        "rules": [
            { "kind": "shell_command", "match": "contains", "value": "rm -rf", "action": "deny", "risk": "high" },
            { "kind": "vdfs_path", "match": "glob", "value": "/home/me/**", "action": "allow", "risk": "low" }
        ]
    })
    .to_string();
    let resp = plugin
        .dispatch(
            &management_ctx(),
            "",
            VdfsRequest::Write {
                content: VdfsContent::text(payload).with_create(),
            },
        )
        .await
        .expect("规则表格必须能保存");
    let VdfsResponse::Write(w) = resp else {
        panic!("期望 Write 响应，实得：{resp:?}")
    };
    let id = w.name.expect("匿名写必须回传生成的 id");
    let read = plugin
        .dispatch(&management_ctx(), &id, VdfsRequest::Read)
        .await
        .expect("读回档案");
    let VdfsResponse::Read(content) = read else {
        panic!("期望 Read 响应，实得：{read:?}")
    };
    let value: serde_json::Value = serde_json::from_str(content.text.as_deref().unwrap_or(""))
        .expect("read 内容应是合法 JSON");
    let rules = value
        .get("rules")
        .and_then(|v| v.as_array())
        .expect("rules 数组");
    assert_eq!(rules.len(), 2);
    assert_eq!(rules[0]["kind"], "shell_command");
    assert_eq!(rules[0]["match"], "contains");
    assert_eq!(rules[0]["action"], "deny");
    assert_eq!(rules[0]["risk"], "high");
    assert_eq!(rules[1]["kind"], "vdfs_path");
    assert_eq!(rules[1]["match"], "glob");
}

fn valid_payload(name: &str) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "approval_mode": "interactive",
        "risk_threshold": "medium",
        "rules": []
    })
}

#[tokio::test]
async fn reload_is_transactional_and_retains_last_known_good_registry() {
    let plugin = test_plugin();
    plugin.reload().await;
    assert_eq!(plugin.registry.read().unwrap().version(), 1);

    plugin
        .store()
        .write_json("custom", &valid_payload("known-good"))
        .await
        .unwrap();
    plugin.reload().await;
    assert_eq!(plugin.registry.read().unwrap().version(), 2);
    assert!(plugin
        .registry
        .read()
        .unwrap()
        .archives()
        .contains_key("custom"));

    plugin
        .store()
        .write_json(
            "custom",
            &serde_json::json!({
                "name": "broken",
                "approval_mode": "interactive",
                "risk_threshold": "critical"
            }),
        )
        .await
        .unwrap();
    plugin.reload().await;

    let registry = plugin.registry.read().unwrap();
    assert_eq!(registry.version(), 2, "失败重载不得递增版本");
    assert_eq!(
        registry.archives().get("custom").unwrap().name,
        "known-good",
        "失败重载必须保留上一份档案"
    );
    assert!(registry.archives().contains_key("strict"));
}

#[tokio::test]
async fn failed_reload_rolls_back_management_write_and_reports_failure() {
    let plugin = test_plugin();
    plugin.reload().await;
    plugin
        .dispatch(
            &management_ctx(),
            "custom",
            VdfsRequest::Write {
                content: VdfsContent::text(valid_payload("before").to_string()).with_create(),
            },
        )
        .await
        .unwrap();
    let before = plugin.store().read_text("custom").await.unwrap();
    let version = plugin.registry.read().unwrap().version();
    let broken = plugin.store().entry_dir("broken").join(MANIFEST);
    tokio::fs::create_dir_all(broken.parent().unwrap())
        .await
        .unwrap();
    tokio::fs::write(&broken, "{not-json").await.unwrap();

    let error = plugin
        .dispatch(
            &management_ctx(),
            "custom",
            VdfsRequest::Write {
                content: VdfsContent::text(valid_payload("after").to_string()).with_create(),
            },
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("已回滚"), "{error}");
    assert_eq!(plugin.store().read_text("custom").await.unwrap(), before);
    let registry = plugin.registry.read().unwrap();
    assert_eq!(registry.version(), version);
    assert_eq!(registry.archives().get("custom").unwrap().name, "before");
    assert!(registry.reload_error.is_some());
}

#[tokio::test]
async fn failed_reload_rolls_back_management_delete_and_copy() {
    let plugin = test_plugin();
    plugin.reload().await;
    plugin
        .dispatch(
            &management_ctx(),
            "victim",
            VdfsRequest::Write {
                content: VdfsContent::text(valid_payload("victim").to_string()).with_create(),
            },
        )
        .await
        .unwrap();
    let broken = plugin.store().entry_dir("broken").join(MANIFEST);
    tokio::fs::create_dir_all(broken.parent().unwrap())
        .await
        .unwrap();
    tokio::fs::write(&broken, "{not-json").await.unwrap();
    let before = plugin.store().entries().await.unwrap().len();

    let delete = plugin
        .dispatch(
            &management_ctx(),
            "victim",
            VdfsRequest::Delete { recursive: true },
        )
        .await
        .unwrap_err();
    assert!(delete.to_string().contains("已回滚"), "{delete}");
    assert!(plugin.store().exists("victim"));

    let copy = plugin
        .dispatch(
            &management_ctx(),
            "default",
            VdfsRequest::Action {
                action: crate::detail::ACTION_COPY.into(),
                payload: None,
            },
        )
        .await
        .unwrap_err();
    assert!(copy.to_string().contains("已回滚"), "{copy}");
    assert_eq!(plugin.store().entries().await.unwrap().len(), before);
}

#[tokio::test]
async fn initial_reload_failure_never_publishes_an_allow_fallback() {
    let plugin = test_plugin();
    plugin
        .store()
        .write_json("broken", &serde_json::json!({"name": "broken"}))
        .await
        .unwrap();
    tokio::fs::write(
        plugin.store().entry_dir("broken").join(MANIFEST),
        "{not-json",
    )
    .await
    .unwrap();

    plugin.reload().await;
    let registry = plugin.registry.read().unwrap();
    assert_eq!(registry.version(), 0);
    assert!(registry.archives().is_empty());
    let engine = Engine::new(plugin.registry.clone(), None);
    let decision = engine.decide(&[], "tool", &serde_json::json!({}));
    assert!(!decision.permitted);
    assert_eq!(decision.profile_version, 0);
    assert!(!engine.permits(&[]));
    assert!(decision
        .reason
        .as_deref()
        .is_some_and(|r| r.contains("no successfully loaded snapshot")));
}

#[tokio::test]
async fn unreadable_archive_also_retains_last_known_good_registry() {
    let plugin = test_plugin();
    plugin.reload().await;
    tokio::fs::create_dir_all(plugin.store().entry_dir("unreadable"))
        .await
        .unwrap();
    plugin.reload().await;
    let registry = plugin.registry.read().unwrap();
    assert_eq!(registry.version(), 1);
    assert!(registry.archives().contains_key("default"));
    assert!(!registry.archives().contains_key("unreadable"));
}

#[tokio::test]
async fn model_context_cannot_write_delete_or_action_sandbox_profiles() {
    let plugin = test_plugin();
    let denied = plugin
        .dispatch(
            &model_ctx(),
            "model-profile",
            VdfsRequest::Write {
                content: VdfsContent::text(valid_payload("model-profile").to_string())
                    .with_create(),
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(denied, VdfsError::Forbidden(_)));
    assert!(!plugin.store().exists("model-profile"));

    plugin
        .dispatch(
            &management_ctx(),
            "victim",
            VdfsRequest::Write {
                content: VdfsContent::text(valid_payload("victim").to_string()).with_create(),
            },
        )
        .await
        .unwrap();
    let denied = plugin
        .dispatch(
            &model_ctx(),
            "victim",
            VdfsRequest::Delete { recursive: true },
        )
        .await
        .unwrap_err();
    assert!(matches!(denied, VdfsError::Forbidden(_)));
    assert!(plugin.store().exists("victim"));

    let denied = plugin
        .dispatch(
            &model_ctx(),
            "victim",
            VdfsRequest::Action {
                action: crate::detail::ACTION_COPY.into(),
                payload: None,
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(denied, VdfsError::Forbidden(_)));
}

#[tokio::test]
async fn unknown_or_metadata_forged_caller_cannot_manage_profiles() {
    let plugin = test_plugin();
    let unknown = vdfs_context(&ctx());
    assert!(matches!(
        ensure_management_context(&unknown),
        Err(VdfsError::Forbidden(_))
    ));

    let mut metadata = serde_json::Map::new();
    metadata.insert(VDFS_CALLER.name().to_string(), serde_json::json!("manager"));
    let forged: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::from_parts(
        serde_json::json!({}),
        Some(&metadata),
    ));
    let denied = plugin
        .dispatch(
            &vdfs_context(&forged),
            "forged",
            VdfsRequest::Write {
                content: VdfsContent::text(valid_payload("forged").to_string()).with_create(),
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(denied, VdfsError::Forbidden(_)));
    assert!(!plugin.store().exists("forged"));
}

#[tokio::test]
async fn management_context_can_write_and_delete_profiles() {
    let plugin = test_plugin();
    plugin
        .dispatch(
            &management_ctx(),
            "custom",
            VdfsRequest::Write {
                content: VdfsContent::text(valid_payload("custom").to_string()).with_create(),
            },
        )
        .await
        .expect("管理上下文应允许写入");
    assert!(plugin.store().exists("custom"));
    plugin
        .dispatch(
            &management_ctx(),
            "custom",
            VdfsRequest::Delete { recursive: true },
        )
        .await
        .expect("管理上下文应允许删除");
    assert!(!plugin.store().exists("custom"));

    let action = plugin
        .dispatch(
            &management_ctx(),
            "custom",
            VdfsRequest::Action {
                action: crate::detail::ACTION_COPY.into(),
                payload: None,
            },
        )
        .await
        .unwrap_err();
    assert!(!matches!(action, VdfsError::Forbidden(_)));
}

#[tokio::test]
async fn canonical_builtin_ids_are_protected_for_write_and_delete() {
    let plugin = test_plugin();
    for path in ["/strict.sandbox", "/ strict /", "default.sandbox"] {
        let write = plugin
            .dispatch(
                &management_ctx(),
                path,
                VdfsRequest::Write {
                    content: VdfsContent::text(valid_payload("overwrite").to_string())
                        .with_create(),
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(write, VdfsError::Forbidden(_)), "path={path}");
        let delete = plugin
            .dispatch(
                &management_ctx(),
                path,
                VdfsRequest::Delete { recursive: true },
            )
            .await
            .unwrap_err();
        assert!(matches!(delete, VdfsError::Forbidden(_)), "path={path}");
    }
}

#[tokio::test]
async fn empty_write_is_a_validation_error_and_creates_nothing() {
    let plugin = test_plugin();
    let error = plugin
        .dispatch(
            &management_ctx(),
            "",
            VdfsRequest::Write {
                content: VdfsContent::text("  \n").with_create(),
            },
        )
        .await
        .unwrap_err();
    let VdfsError::Invalid(validation) = error else {
        panic!("空写入必须是校验错误：{error:?}")
    };
    assert!(validation
        .fields
        .iter()
        .any(|field| field.field == "content"));
    assert!(plugin.store().entries().await.unwrap().is_empty());
}

#[tokio::test]
async fn invalid_configs_return_field_paths_and_are_not_written() {
    let plugin = test_plugin();
    let cases = [
        (
            serde_json::json!({
                "name": "bad",
                "approval_mode": "interactive",
                "risk_threshold": "medium",
                "rules": [{"kind": "vdfs_path", "match": "prefix", "value": "  ", "action": "allow", "risk": "low"}]
            }),
            "rules.0.value",
        ),
        (
            serde_json::json!({
                "name": "bad",
                "approval_mode": "interactive",
                "risk_threshold": "medium",
                "rules": [
                    {"kind": "shell_command", "match": "exact", "value": "git", "action": "allow", "risk": "low"},
                    {"kind": "shell_command", "match": "exact", "value": "git", "action": "deny", "risk": "high"}
                ]
            }),
            "rules.1.value",
        ),
        (
            serde_json::json!({
                "name": "bad",
                "approval_mode": "manual",
                "risk_threshold": "medium"
            }),
            "approval_mode",
        ),
        (
            serde_json::json!({
                "name": "bad",
                "approval_mode": "interactive",
                "risk_threshold": "critical"
            }),
            "risk_threshold",
        ),
        (
            serde_json::json!({
                "name": "bad",
                "approval_mode": "interactive",
                "risk_threshold": "medium",
                "rules": [{"kind": "shell_command", "match": "regex", "value": "git", "action": "allow", "risk": "low"}]
            }),
            "rules.0.match",
        ),
    ];
    for (payload, expected_field) in cases {
        let id = format!("bad-{}", symbio_providers::vdfs_auto_id(PLUGIN_ID_SANDBOX));
        let error = plugin
            .dispatch(
                &management_ctx(),
                &id,
                VdfsRequest::Write {
                    content: VdfsContent::text(payload.to_string()).with_create(),
                },
            )
            .await
            .unwrap_err();
        let VdfsError::Invalid(validation) = error else {
            panic!("非法配置必须是校验错误：{error:?}")
        };
        assert!(
            validation
                .fields
                .iter()
                .any(|field| field.field == expected_field),
            "expected {expected_field}, got {validation:?}"
        );
        assert!(!plugin.store().exists(&id));
    }
}
