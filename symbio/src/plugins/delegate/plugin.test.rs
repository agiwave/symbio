//! delegate 插件的单元测试：路由面（判定 / 进展 / 停用 / 未知路径）与元数据。
//!
//! 用私有 runtime 跑 async（与 `retrieval` / `actor` 的测试同款）：测试里没有 tokio
//! 运行时，`#[tokio::test]` 会与 `std::sync::Mutex` 之类的同步守卫冲突。
use super::*;
use crate::symbio_core::{PluginInvokeRequestExt, PluginPayload, PluginSimpleRequest};
use std::fs;

fn temp_root(tag: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("symbio-test-delegate-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// `PluginDir` 指向 `<root>/delegate`（与生产 `plugin_dir_from_ctx` 给的路径同形），
/// 于是 `new()` 的 `parent()` 恰好回到 `<root>`。
fn test_dir(root: &Path) -> PluginDir {
    PluginDir::at(root.join(PLUGIN_ID_DELEGATE), PLUGIN_ID_DELEGATE)
}

fn plugin(root: &Path, enabled: bool) -> Arc<DelegatePlugin> {
    let cfg = DelegateConfig {
        enabled,
        ..Default::default()
    };
    Arc::new(DelegatePlugin::new(test_dir(root), cfg))
}

fn ctx(path: &str) -> Arc<dyn PluginInvokeRequest> {
    let req = PluginSimpleRequest::new(None, None);
    req.set(PATH, path.to_string());
    Arc::new(req)
}

fn data_of(p: PluginPayload) -> serde_json::Value {
    match p {
        PluginPayload::Data(d) => d.serialize().unwrap(),
        other => panic!("expected data, got {other:?}"),
    }
}

fn run<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("测试 runtime")
        .block_on(f)
}

#[test]
fn metadata_id_matches_plugin_id() {
    assert_eq!(DelegatePlugin::metadata().id, PLUGIN_ID_DELEGATE);
}

/// 停用 ⇒ 两条路由都不可达（J2 平凡值：系统照常，少的只是委派事实）
#[test]
fn disabled_makes_routes_unreachable() {
    let root = temp_root("disabled");
    let p = plugin(&root, false);
    for path in ["decide", "progress"] {
        let err = run(async {
            let p = p.clone();
            p.route(ctx(path)).await.unwrap_err()
        });
        assert!(
            matches!(err, PluginError::NotFound(_)),
            "{path} 应 NotFound"
        );
    }
}

#[test]
fn unknown_route_is_not_found() {
    let root = temp_root("unknown");
    let p = plugin(&root, true);
    let err = run(async move { p.clone().route(ctx("nope")).await.unwrap_err() });
    assert!(matches!(err, PluginError::NotFound(_)));
}

/// `progress`：没有 worker ⇒ 空表 + 空 `rendered`（**不是错误**）
#[test]
fn progress_without_workers_is_empty_not_an_error() {
    let root = temp_root("progress-empty");
    let p = plugin(&root, true);
    let v = run(async move { data_of(p.clone().route(ctx("progress")).await.unwrap()) });
    assert_eq!(v["count"], 0);
    assert_eq!(v["rendered"], "");
    assert!(v["workers"].as_array().unwrap().is_empty());
}

/// `progress`：真实嵌套布局下回快照与渲染段（与 `progress.rs` 的纯函数同源）
#[test]
fn progress_reads_real_nested_layout() {
    let root = temp_root("progress-real");
    let dir = root
        .join("session")
        .join("main")
        .join("sessions")
        .join("w-1");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("session.json"),
        serde_json::json!({
            "id": "w-1",
            "title": "整理文档",
            "metadata": { "parent_session_id": "main" }
        })
        .to_string(),
    )
    .unwrap();
    fs::write(
        dir.join("messages.json"),
        serde_json::json!({
            "messages": [
                { "id": "u1", "role": "user", "type": "text", "content": "整理文档", "seq": 1, "status": "completed" }
            ]
        })
        .to_string(),
    )
    .unwrap();

    let p = plugin(&root, true);
    let v = run(async move { data_of(p.clone().route(ctx("progress")).await.unwrap()) });
    assert_eq!(v["count"], 1);
    assert_eq!(v["workers"][0]["session_id"], "w-1");
    assert_eq!(v["workers"][0]["state"], "idle");
    assert!(
        v["rendered"].as_str().unwrap().contains("w-1（整理文档）"),
        "渲染段应含会话与标题：{}",
        v["rendered"]
    );
}

/// 配置表单：两个字段（开关 + 目录上限），键名与结构体字段同名
#[test]
fn config_definition_has_two_fields() {
    let d: DetailDefinition = config_definition(&DelegateConfig::default());
    assert_eq!(d.sections.len(), 1);
    let keys: Vec<&str> = d.sections[0]
        .fields
        .iter()
        .map(|f| f.key.as_str())
        .collect();
    assert_eq!(keys, vec!["enabled", "digest_max"]);
}
