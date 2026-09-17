use super::*;

fn isolated_home() -> HomePlugin {
    HomePlugin {
        instances: Arc::new(RwLock::new(HashMap::new())),
        config: Arc::new(RwLock::new(HomeConfig::default())),
        context: Arc::new(SimpleRequest::new(None, None)),
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
