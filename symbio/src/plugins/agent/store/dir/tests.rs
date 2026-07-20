use super::*;
use crate::plugins::agent::core::cu_from_json;
use serde_json::json;
use tempfile::tempdir;

#[tokio::test]
async fn test_dir_storage_single_file_yaml() {
    let dir = tempdir().unwrap();
    let file_path = dir.path().join("test_agent.yaml");

    let content = r#"
- id: identity
  is_a: identity
  name: Test Agent
- id: safety
  is_a: rule
  text: Follow safety rules
"#;
    fs::write(&file_path, content).await.unwrap();

    let storage = DirStorage::new(&file_path, StorageFormat::Yaml);

    let all = storage.load_all().await;
    assert_eq!(all.len(), 2);
    assert!(all.contains_key("identity"));
    assert!(all.contains_key("safety"));

    let au = storage.get("identity").await.unwrap();
    assert!(au.is_some());
}

#[tokio::test]
async fn test_dir_storage_directory_mode() {
    let dir = tempdir().unwrap();
    let storage = DirStorage::new(dir.path(), StorageFormat::Yaml);

    let au = json!({
        "id": "identity::test",
        "is_a": "identity",
        "name": "Test Agent",
        "_ext_version": 1
    });

    storage.insert(&cu_from_json(au.clone())).await.unwrap();

    let retrieved = storage.get("identity::test").await.unwrap();
    assert!(retrieved.is_some());

    let all = storage.load_all().await;
    assert_eq!(all.len(), 1);
}
