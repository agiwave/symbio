use super::*;

fn isolated_home() -> HomePlugin {
    HomePlugin {
        instances: Arc::new(RwLock::new(HashMap::new())),
        config: Arc::new(RwLock::new(HomeConfig::default())),
        context: Arc::new(PluginSimpleRequest::new(None, None)),
        self_weak: Arc::new(RwLock::new(None)),
    }
}

#[tokio::test]
async fn workspace_save_failure_keeps_previous_memory_and_disk() {
    let temp = tempfile::tempdir().unwrap();
    let dir = PluginDir::at(temp.path(), PLUGIN_HOME);
    let home = isolated_home();
    home.set_workspace_in("previous", &dir).await.unwrap();
    let before = home.config.read().await.work.clone();
    let disk_before = std::fs::read(dir.config_path()).unwrap();

    // A directory at the temporary-file path deterministically prevents writes
    // on Windows and Unix, without relying on permissions or global homedir.
    let blocked = dir.config_path().with_extension("yml.tmp");
    std::fs::create_dir(&blocked).unwrap();
    let error = home.set_workspace_in("unsaved", &dir).await.unwrap_err();
    assert!(error.to_string().contains("持久化自身配置失败"));
    assert_eq!(home.config.read().await.work, before);
    assert_eq!(std::fs::read(dir.config_path()).unwrap(), disk_before);

    std::fs::remove_dir(blocked).unwrap();
    let result = home.set_workspace_in("saved", &dir).await.unwrap();
    assert_eq!(result["status"], "success");
    let saved = dir.load::<HomeConfig>().unwrap().unwrap();
    assert_eq!(saved.work, home.config.read().await.work);
    assert_eq!(saved.work["workdir"], "saved");
    assert_eq!(
        saved.work["recent_workspaces"],
        serde_json::json!(["saved", "previous"])
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_workspace_updates_and_flush_keep_disk_and_memory_consistent() {
    let temp = tempfile::tempdir().unwrap();
    let dir = PluginDir::at(temp.path(), PLUGIN_HOME);
    let home = Arc::new(isolated_home());
    let barrier = Arc::new(tokio::sync::Barrier::new(17));
    let mut tasks = Vec::new();
    for i in 0..16 {
        let home = home.clone();
        let dir = dir.clone();
        let barrier = barrier.clone();
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            if i < 8 {
                home.set_workspace_in(&format!("workspace-{i}"), &dir)
                    .await
                    .unwrap();
            } else {
                home.flush_in(&dir).await.unwrap();
            }
        }));
    }
    barrier.wait().await;
    for task in tasks {
        task.await.unwrap();
    }
    let saved = dir.load::<HomeConfig>().unwrap().unwrap();
    assert_eq!(saved.work, home.config.read().await.work);
    let recent = saved.work["recent_workspaces"].as_array().unwrap();
    assert_eq!(recent.len(), 8);
    assert_eq!(recent.first().unwrap(), &saved.work["workdir"]);
    for i in 0..8 {
        assert!(recent.contains(&serde_json::json!(format!("workspace-{i}"))));
    }
}

// ---- 路由原语 ----

/// `parse_path` 把路径切成「子插件名 + 剩余路径」：前导斜杠可省，无分隔符时剩余为空串。
/// 返回 `None` 即"没有名字可用"——调用方据此走 worker 兜底而不是误转发。
#[test]
fn parse_path_splits_name_and_rest() {
    assert_eq!(
        HomePlugin::parse_path("/work/agent"),
        Some(("work", "agent"))
    );
    assert_eq!(
        HomePlugin::parse_path("work/agent"),
        Some(("work", "agent"))
    );
    assert_eq!(HomePlugin::parse_path("/work"), Some(("work", "")));
    assert_eq!(HomePlugin::parse_path("work"), Some(("work", "")));
    assert_eq!(HomePlugin::parse_path("/work/"), Some(("work", "")));
    assert_eq!(HomePlugin::parse_path("/"), None);
    assert_eq!(HomePlugin::parse_path(""), None);
}

/// 首次启动给一个"可写的空壳"；重复调用幂等，且**已有值不得被覆盖**。
#[test]
fn ensure_defaults_is_idempotent_and_preserves_existing() {
    let mut cfg = HomeConfig::default();
    cfg.ensure_defaults();
    assert_eq!(cfg.work["workdir"], Value::String(String::new()));
    assert_eq!(cfg.work["recent_workspaces"], Value::Array(Vec::new()));

    cfg.work
        .insert("workdir".into(), Value::String("/tmp/ws".into()));
    cfg.ensure_defaults();
    assert_eq!(
        cfg.work["workdir"],
        Value::String("/tmp/ws".into()),
        "已有配置不得被默认值覆盖"
    );
}

// ---- route 分发 ----

fn ctx_at(path: &str) -> Arc<dyn PluginInvokeRequest> {
    let ctx = PluginSimpleRequest::new(None, None);
    ctx.set(PATH, path.to_string());
    Arc::new(ctx)
}

/// `work/get_workspace` 读 Home 自己的配置缓存（"最可靠的数据源"），不去问子插件。
/// 未配置时给空串而非报错——前端据此显示"未选择工作区"。
#[tokio::test]
async fn work_get_workspace_reads_home_config_cache() {
    let home = Arc::new(isolated_home());

    let empty = home
        .clone()
        .route(ctx_at("work/get_workspace"))
        .await
        .expect("未配置工作区也应正常返回")
        .serialize()
        .expect("响应应可序列化");
    assert_eq!(empty["workdir"], "");
    assert_eq!(empty["expanded_path"], "");

    {
        let mut cfg = home.config.write().await;
        cfg.work
            .insert("workdir".into(), Value::String("/tmp/my-ws".into()));
        cfg.work.insert(
            "recent_workspaces".into(),
            serde_json::json!(["/tmp/my-ws"]),
        );
    }

    let filled = home
        .clone()
        .route(ctx_at("work/get_workspace"))
        .await
        .expect("应回读配置缓存")
        .serialize()
        .expect("响应应可序列化");
    assert_eq!(filled["workdir"], "/tmp/my-ws");
    assert_eq!(filled["expanded_path"], "/tmp/my-ws");
    assert_eq!(
        filled["recent_workspaces"],
        serde_json::json!(["/tmp/my-ws"])
    );
}

/// 不认识的路由、且没有 worker 在跑 ⇒ 显式 NotFound，而不是静默成功。
#[tokio::test]
async fn unknown_route_without_worker_is_not_found() {
    let home = Arc::new(isolated_home());
    let res = home.clone().route(ctx_at("no/such/thing")).await;
    // 不用 `expect_err`：`PluginPayload` 未派生 `Debug`，判据直接对错误变体下断言
    assert!(
        matches!(res, Err(PluginError::NotFound(_))),
        "未知路径且无 worker 时必须显式 NotFound"
    );
}
