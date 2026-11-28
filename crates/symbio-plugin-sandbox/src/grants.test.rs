use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use symbio_plugin_api::collectors::ApprovalGrants;
use symbio_plugin_api::CapabilityRiskLevel;

use crate::grants::GrantStore;

static SEQ: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        Self(std::env::temp_dir().join(format!(
            "sandbox-grants-{label}-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        )))
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn grants_persist_raise_only_and_revoke_per_profile() {
    let root = TempDir::new("roundtrip");
    let sandbox = root.0.join("sandbox");
    let session = root.0.join("session");
    let store = GrantStore::load(&sandbox, &session);
    store
        .grant("default", "shell", CapabilityRiskLevel::High)
        .unwrap();
    store
        .grant("default", "shell", CapabilityRiskLevel::Low)
        .unwrap();
    store
        .grant("strict", "vdfs_read", CapabilityRiskLevel::Low)
        .unwrap();

    let reloaded = GrantStore::load(&sandbox, &session);
    assert_eq!(
        reloaded.list("default").ceiling("shell"),
        Some(CapabilityRiskLevel::High)
    );
    assert!(reloaded.list("default").ceiling("vdfs_read").is_none());
    assert_eq!(
        reloaded.list("strict").ceiling("vdfs_read"),
        Some(CapabilityRiskLevel::Low)
    );
    assert!(reloaded.revoke("default", "shell").unwrap().is_empty());
    assert!(GrantStore::load(&sandbox, &session)
        .list("default")
        .is_empty());
}

#[test]
fn legacy_grants_migrate_to_default_with_backup_and_clean_manifest() {
    let root = TempDir::new("migrate");
    let sandbox = root.0.join("sandbox");
    let session = root.0.join("session");
    std::fs::create_dir_all(&session).unwrap();
    let manifest = session.join("PLUGIN.yml");
    let original = "name: session\napproval_grants:\n  shell: high\n";
    std::fs::write(&manifest, original).unwrap();

    let store = GrantStore::load(&sandbox, &session);
    store.ensure_ready().unwrap();
    assert_eq!(
        store.list("default").ceiling("shell"),
        Some(CapabilityRiskLevel::High)
    );
    assert!(store.list("strict").is_empty());
    assert_eq!(
        std::fs::read_to_string(session.join("PLUGIN.yml.approval-grants.bak")).unwrap(),
        original
    );
    let cleaned = std::fs::read_to_string(&manifest).unwrap();
    assert!(!cleaned.contains("approval_grants"));
    assert!(cleaned.contains("name: session"));
    assert!(sandbox.join("approval-grants.json").is_file());
}

#[test]
fn empty_legacy_grants_are_removed_without_backup() {
    let root = TempDir::new("empty");
    let session = root.0.join("session");
    std::fs::create_dir_all(&session).unwrap();
    let manifest = session.join("PLUGIN.yml");
    std::fs::write(&manifest, "approval_grants: {}\n").unwrap();
    let store = GrantStore::load(&root.0.join("sandbox"), &session);
    store.ensure_ready().unwrap();
    assert!(store.list("default").is_empty());
    assert!(!std::fs::read_to_string(&manifest)
        .unwrap()
        .contains("approval_grants"));
    assert!(!session.join("PLUGIN.yml.approval-grants.bak").exists());
}

#[test]
fn corrupt_or_unsupported_grant_storage_fails_closed() {
    for content in ["{broken", r#"{"version":2,"profiles":{}}"#] {
        let root = TempDir::new("corrupt");
        let sandbox = root.0.join("sandbox");
        std::fs::create_dir_all(&sandbox).unwrap();
        std::fs::write(sandbox.join("approval-grants.json"), content).unwrap();
        let store = GrantStore::load(&sandbox, &root.0.join("session"));
        assert!(store.ensure_ready().is_err());
        assert!(store.list("default").is_empty());
        assert!(store
            .grant("default", "shell", CapabilityRiskLevel::High)
            .is_err());
    }
}

#[test]
fn existing_legacy_backup_is_never_overwritten() {
    let root = TempDir::new("backup");
    let session = root.0.join("session");
    std::fs::create_dir_all(&session).unwrap();
    std::fs::write(
        session.join("PLUGIN.yml"),
        "approval_grants:\n  shell: high\n",
    )
    .unwrap();
    std::fs::write(session.join("PLUGIN.yml.approval-grants.bak"), "keep").unwrap();
    let store = GrantStore::load(&root.0.join("sandbox"), &session);
    assert!(store.ensure_ready().is_err());
    assert_eq!(
        std::fs::read_to_string(session.join("PLUGIN.yml.approval-grants.bak")).unwrap(),
        "keep"
    );
    assert!(!root.0.join("sandbox/approval-grants.json").exists());
}

#[test]
fn wire_file_contains_only_profile_scoped_plain_maps() {
    let root = TempDir::new("wire");
    let sandbox = root.0.join("sandbox");
    let store = GrantStore::load(&sandbox, &root.0.join("session"));
    store
        .grant("default", "shell", CapabilityRiskLevel::Medium)
        .unwrap();
    let value: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(sandbox.join("approval-grants.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        value,
        serde_json::json!({
            "version": 1,
            "profiles": {"default": {"shell": "medium"}}
        })
    );
    assert_eq!(
        ApprovalGrants::parse(r#"{"shell":"medium"}"#).ceiling("shell"),
        Some(CapabilityRiskLevel::Medium)
    );
}
