use super::*;
use serde_json::json;

fn test_config() -> WorkerConfig {
    WorkerConfig {
        mode: WorkerMode::Container,
        image: Some(format!("registry.example/symbio@sha256:{}", "a".repeat(64))),
        ..WorkerConfig::default()
    }
}

#[test]
fn image_must_be_digest_pinned() {
    assert!(validate_image("symbio:latest").is_err());
    assert!(validate_image("symbio@sha256:abc").is_err());
    assert!(validate_image(&format!("symbio@sha256:{}", "A".repeat(64))).is_ok());
    let mut config = test_config();
    config.image = None;
    assert!(config.validate().is_err());
}

#[test]
fn image_metadata_rejects_os_digest_and_build_mismatches() {
    let image = format!("registry.example/symbio@sha256:{}", "a".repeat(64));
    let build = BuildId::parse("b".repeat(64)).expect("build");
    let inspect = json!([{
        "RepoDigests": [image.clone()],
        "Config": {"Labels": {"symbio.build-id": build.as_str()}}
    }]);
    assert!(verify_image_metadata("linux", "linux", &inspect, &image, &build).is_ok());
    assert!(verify_image_metadata("windows", "linux", &inspect, &image, &build).is_err());
    assert!(verify_image_metadata("linux", "linux", &json!([]), &image, &build).is_err());
    assert!(verify_image_metadata(
        "linux",
        "linux",
        &inspect,
        &format!("registry.example/other@sha256:{}", "a".repeat(64)),
        &build
    )
    .is_err());
    let wrong_build = BuildId::parse("c".repeat(64)).expect("build");
    assert!(verify_image_metadata("linux", "linux", &inspect, &image, &wrong_build).is_err());
}

#[test]
fn workdir_mount_rejects_home_session_and_credential_roots() {
    let root = tempfile::tempdir().expect("root");
    let home = root.path().join("home");
    let session = home.join("session/parent/child");
    let work = root.path().join("work");
    fs::create_dir_all(&session).expect("session");
    fs::create_dir_all(&work).expect("work");
    fs::create_dir_all(home.join(".aws")).expect("credentials");
    let home = fs::canonicalize(home).expect("home");
    let session = fs::canonicalize(session).expect("session");
    for (index, selected) in [
        home.clone(),
        root.path().to_path_buf(),
        session.clone(),
        home.join(".aws"),
        work.clone(),
    ]
    .into_iter()
    .enumerate()
    {
        fs::write(
            session.join("session.json"),
            serde_json::to_vec(&json!({"metadata": {"workdir": selected.to_string_lossy()}}))
                .expect("json"),
        )
        .expect("session");
        let result = selected_workdir(&session, &home);
        if index == 4 {
            assert_eq!(result.expect("valid workdir"), Some(selected));
        } else {
            assert!(result.is_err(), "{selected:?}");
        }
    }
}

#[test]
fn workdir_rejects_nested_credentials_and_provider_trees() {
    let root = tempfile::tempdir().expect("root");
    let home = root.path().join("home");
    let session = home.join("session/parent/child");
    let work = root.path().join("work");
    fs::create_dir_all(&session).expect("session");
    fs::create_dir_all(home.join("model/p")).expect("model");
    fs::create_dir_all(home.join("mcp/http")).expect("mcp");
    fs::create_dir_all(&work).expect("work");
    let home = fs::canonicalize(home).expect("home");
    let session = fs::canonicalize(session).expect("session");
    let work = fs::canonicalize(work).expect("work");
    for selected in [home.join("model/p"), home.join("mcp/http")] {
        fs::write(
            session.join("session.json"),
            serde_json::to_vec(&json!({"metadata": {"workdir": selected.to_string_lossy()}}))
                .expect("json"),
        )
        .expect("session");
        assert!(selected_workdir(&session, &home).is_err(), "{selected:?}");
    }
    fs::create_dir_all(work.join("nested/.SSH")).expect("credentials");
    fs::write(
        session.join("session.json"),
        serde_json::to_vec(&json!({"metadata": {"workdir": work.to_string_lossy()}}))
            .expect("json"),
    )
    .expect("session");
    assert!(selected_workdir(&session, &home).is_err());

    let work = root.path().join("work-files");
    fs::create_dir_all(work.join("nested")).expect("work files");
    let work = fs::canonicalize(work).expect("work files");
    for secret in [".env", "nested/server.pem"] {
        fs::write(work.join(secret), "host-secret").expect("credential file");
        fs::write(
            session.join("session.json"),
            serde_json::to_vec(&json!({"metadata": {"workdir": work.to_string_lossy()}}))
                .expect("json"),
        )
        .expect("session");
        assert!(selected_workdir(&session, &home).is_err(), "{secret}");
        fs::remove_file(work.join(secret)).expect("remove credential file");
    }
}

#[cfg(unix)]
#[test]
fn container_identity_rejects_root_or_unwritable_mounts() {
    use std::os::unix::fs::PermissionsExt;

    if std::os::unix::process::euid() == 0 {
        return;
    }
    let root = tempfile::tempdir().expect("root");
    let session = root.path().join("session");
    let work = root.path().join("work");
    fs::create_dir_all(&session).expect("session");
    fs::create_dir_all(&work).expect("work");
    let identity = container_user(&session, Some(&work)).expect("identity");
    assert!(identity.starts_with(&format!("{}:", std::os::unix::process::euid())));
    fs::set_permissions(&session, fs::Permissions::from_mode(0o500)).expect("session mode");
    assert!(container_user(&session, Some(&work)).is_err());
    fs::set_permissions(&session, fs::Permissions::from_mode(0o700)).expect("session mode");
    fs::set_permissions(&work, fs::Permissions::from_mode(0o500)).expect("work mode");
    assert!(container_user(&session, Some(&work)).is_err());
}

#[test]
fn invalid_security_limits_fail_closed() {
    for value in ["0", "-1", "nan", "257"] {
        let mut config = test_config();
        config.cpu = value.to_string();
        assert!(config.validate().is_err(), "{value}");
    }
    let mut config = test_config();
    config.pids = 0;
    assert!(config.validate().is_err());
    let mut config = test_config();
    config.runtime = "podman".to_string();
    assert!(config.validate().is_err());
}

#[test]
fn projection_removes_credentials_and_rewrites_model_mcp() {
    let home = tempfile::tempdir().expect("home");
    let projection = tempfile::tempdir().expect("projection");
    fs::create_dir_all(home.path().join("model/p")).expect("model");
    fs::create_dir_all(home.path().join("mcp/http")).expect("mcp");
    fs::create_dir_all(home.path().join("mcp/local")).expect("mcp");
    fs::create_dir_all(home.path().join("session/private")).expect("session");
    fs::create_dir_all(home.path().join(".ssh")).expect("ssh");
    fs::create_dir_all(home.path().join("skill/s")).expect("skill");
    fs::create_dir_all(home.path().join("web")).expect("web");
    fs::create_dir_all(home.path().join("unrelated")).expect("unrelated");
    fs::create_dir_all(home.path().join("subagent/reviewer/model/p")).expect("agent model");
    fs::create_dir_all(home.path().join("subagent/reviewer/mcp/http")).expect("agent mcp");
    fs::create_dir_all(home.path().join("subagent/reviewer/skills/s")).expect("agent skills");
    fs::create_dir_all(home.path().join("subagent/reviewer/.ssh")).expect("agent ssh");
    for plugin in ["session", "model", "mcp"] {
        fs::write(
            home.path().join(plugin).join("PLUGIN.yml"),
            format!("plugin_provider: {plugin}\n"),
        )
        .expect("core manifest");
    }
    fs::write(
        home.path().join("model/p/provider.json"),
        serde_json::to_vec(&json!({
            "id": "p", "api_base": "https://model.example/v1", "api_key": "host-secret",
            "api_protocol": "anthropic_messages", "enabled": true
        }))
        .expect("provider json"),
    )
    .expect("provider");
    fs::write(home.path().join("mcp/http/server.json"), serde_json::to_vec(&json!({
        "type": "http", "url": "https://mcp.example/rpc", "headers": {"Authorization": "host-secret"}, "enabled": true
    })).expect("server json")).expect("server");
    fs::write(home.path().join("mcp/local/server.json"), serde_json::to_vec(&json!({
        "type": "stdio", "command": "secret-tool", "env": {"TOKEN": "host-secret"}, "enabled": true
    })).expect("server json")).expect("server");
    fs::write(home.path().join("session/private/session.json"), "{}").expect("session");
    fs::write(home.path().join(".ssh/id_rsa"), "host-secret").expect("ssh");
    fs::write(home.path().join("skill/s/SKILL.md"), "safe").expect("skill");
    fs::write(
        home.path().join("skill/s/PLUGIN.yml"),
        "plugin_provider: skill\ntoken: host-secret\napiKey: host-secret\nclientSecret: host-secret\nprivateKey: host-secret\nrefresh_token: host-secret\n",
    )
    .expect("skill manifest");
    fs::write(
        home.path().join("web/PLUGIN.yml"),
        "plugin_provider: web\nenabled: true\ntavily_api_key: host-secret\n",
    )
    .expect("web manifest");
    fs::write(home.path().join("unrelated/credentials.txt"), "host-secret").expect("unrelated");
    fs::write(
        home.path().join("subagent/reviewer/model/p/provider.json"),
        serde_json::to_vec(&json!({
            "id": "p", "api_base": "https://agent-model.example/v1", "api_key": "host-secret"
        }))
        .expect("agent provider json"),
    )
    .expect("agent provider");
    fs::write(
        home.path().join("subagent/reviewer/mcp/http/server.json"),
        serde_json::to_vec(&json!({
            "type": "http", "url": "https://agent-mcp.example/rpc",
            "headers": {"Authorization": "host-secret"}
        }))
        .expect("agent server json"),
    )
    .expect("agent server");
    fs::write(
        home.path().join("subagent/reviewer/skills/s/SKILL.md"),
        "agent-safe",
    )
    .expect("agent skill");
    fs::write(
        home.path().join("subagent/reviewer/.ssh/id_rsa"),
        "host-secret",
    )
    .expect("agent ssh");

    let targets = project_runtime(home.path(), projection.path()).expect("projection");
    assert!(!projection.path().join("session/private").exists());
    assert!(!projection.path().join(".ssh").exists());
    assert!(!projection.path().join("unrelated").exists());
    assert!(!projection.path().join("subagent/reviewer/.ssh").exists());
    for plugin in ["session", "model", "mcp"] {
        assert!(
            projection.path().join(plugin).join("PLUGIN.yml").is_file(),
            "{plugin}"
        );
    }
    assert_eq!(
        fs::read_to_string(
            projection
                .path()
                .join("subagent/reviewer/skills/s/SKILL.md")
        )
        .expect("agent skill"),
        "agent-safe"
    );
    let web: Value = serde_yaml_ng::from_str(
        &fs::read_to_string(projection.path().join("web/PLUGIN.yml")).expect("web"),
    )
    .expect("yaml");
    assert_eq!(web["enabled"], json!(false));
    assert!(web.get("tavily_api_key").is_none());
    let skill: Value = serde_yaml_ng::from_str(
        &fs::read_to_string(projection.path().join("skill/s/PLUGIN.yml")).expect("skill config"),
    )
    .expect("yaml");
    for key in [
        "token",
        "apiKey",
        "clientSecret",
        "privateKey",
        "refresh_token",
    ] {
        assert!(skill.get(key).is_none(), "{key}");
    }
    assert_eq!(
        fs::read_to_string(projection.path().join("skill/s/SKILL.md")).expect("skill"),
        "safe"
    );
    let provider: Value = serde_json::from_slice(
        &fs::read(projection.path().join("model/p/provider.json")).expect("provider"),
    )
    .expect("json");
    assert_eq!(provider["api_key"], Value::Null);
    assert_eq!(
        provider["api_base"],
        json!(format!("http://127.0.0.1:{BROKER_PORT}/model/p"))
    );
    let mcp: Value = serde_json::from_slice(
        &fs::read(projection.path().join("mcp/http/server.json")).expect("server"),
    )
    .expect("json");
    assert!(mcp.get("headers").is_none());
    let local: Value = serde_json::from_slice(
        &fs::read(projection.path().join("mcp/local/server.json")).expect("server"),
    )
    .expect("json");
    assert_eq!(local["enabled"], json!(false));
    assert!(local.get("env").is_none());
    assert!(local.get("command").is_none());
    let agent_provider: Value = serde_json::from_slice(
        &fs::read(
            projection
                .path()
                .join("subagent/reviewer/model/p/provider.json"),
        )
        .expect("agent provider"),
    )
    .expect("agent json");
    assert_eq!(agent_provider["api_key"], Value::Null);
    assert_eq!(
        agent_provider["api_base"],
        json!(format!(
            "http://127.0.0.1:{BROKER_PORT}/subagent/reviewer/model/p"
        ))
    );
    let agent_mcp: Value = serde_json::from_slice(
        &fs::read(
            projection
                .path()
                .join("subagent/reviewer/mcp/http/server.json"),
        )
        .expect("agent server"),
    )
    .expect("agent json");
    assert!(agent_mcp.get("headers").is_none());
    assert_eq!(targets.len(), 4);
    assert!(targets.contains_key("subagent:reviewer/model:p"));
    assert!(targets.contains_key("subagent:reviewer/mcp:http"));
    let all = walk_files(projection.path());
    assert!(!all.iter().any(|p| p.contains("host-secret")));
}

#[cfg(unix)]
#[test]
fn projection_never_follows_source_symlinks() {
    let root = tempfile::tempdir().expect("root");
    let home = root.path().join("home");
    let outside = root.path().join("outside");
    let projection = root.path().join("projection");
    fs::create_dir_all(home.join("model/p")).expect("model");
    fs::create_dir_all(home.join("mcp/http")).expect("mcp");
    fs::create_dir_all(home.join("skill/s")).expect("skill");
    fs::create_dir_all(&outside).expect("outside");
    fs::write(outside.join("PLUGIN.yml"), "token: host-secret\n").expect("manifest");
    fs::write(
        outside.join("provider.json"),
        serde_json::to_vec(&json!({
            "api_base": "https://model.example/v1", "api_key": "host-secret"
        }))
        .expect("provider"),
    )
    .expect("provider");
    fs::write(outside.join("server.json"), "{}").expect("server");
    fs::write(outside.join("secret.txt"), "host-secret").expect("secret");
    for (link, file) in [
        ("PLUGIN.yml", "PLUGIN.yml"),
        ("model/p/provider.json", "provider.json"),
        ("mcp/http/server.json", "server.json"),
        ("skill/s/secret.txt", "secret.txt"),
    ] {
        std::os::unix::fs::symlink(outside.join(file), home.join(link)).expect("symlink");
    }

    let targets = project_runtime(&home, &projection).expect("projection");
    assert!(targets.is_empty());
    assert!(!projection.join("PLUGIN.yml").exists());
    assert!(!projection.join("model/p/provider.json").exists());
    assert!(!projection.join("mcp/http/server.json").exists());
    assert!(!projection.join("skill/s/secret.txt").exists());
    let all = walk_files(&projection);
    assert!(!all.iter().any(|content| content.contains("host-secret")));
}

fn walk_files(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for entry in fs::read_dir(root).expect("read") {
        let entry = entry.expect("entry");
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk_files(&path));
        } else {
            out.push(fs::read_to_string(path).unwrap_or_default());
        }
    }
    out
}

#[test]
fn broker_target_rejects_escape_and_unregistered_suffix() {
    let target = BrokerTarget {
        id: "p".to_string(),
        kind: BrokerKind::Model,
        scope: vec![],
        base: Url::parse("https://model.example/v1").expect("url"),
        headers: vec![("authorization".to_string(), "secret".to_string())],
        timeout_secs: 10,
    };
    assert!(target.accepts_path("/model/p/chat/completions"));
    assert!(!target.accepts_path("/model/other/chat/completions"));
    assert!(target.url_for("/model/p/../admin").is_err());
    assert!(target.url_for("/model/p/%2E%2E/admin").is_err());
    assert_eq!(
        target
            .url_for("/model/p/chat/completions")
            .expect("url")
            .host_str(),
        Some("model.example")
    );
    let url = target
        .url_for("/model/p/chat/completions?stream=true&x=1")
        .expect("query");
    assert_eq!(url.query(), Some("stream=true&x=1"));
}
