//! ProfileLoader 单元测试
//!
//! 对应源文件: `loader.rs`

use super::*;
use std::path::Path;

#[test]
fn test_extract_agent_id_dir() {
    let id = extract_agent_id(Path::new("/tmp/agents/my_agent"));
    assert_eq!(id, Some("my_agent".to_string()));
}

#[test]
fn test_extract_agent_id_file() {
    let id = extract_agent_id(Path::new("/tmp/agents/my_agent.json"));
    assert_eq!(id, Some("my_agent".to_string()));
}

#[tokio::test]
async fn test_detect_config_sqlite() {
    let dir = tempfile::TempDir::new().unwrap();
    let file_path = dir.path().join("agent.db");
    tokio::fs::write(&file_path, "").await.unwrap();

    let loader = ProfileLoader::new();
    let config = loader.detect_config(&file_path);
    assert!(matches!(
        config.storage_backend,
        crate::plugins::agent::core::StorageBackendType::Sqlite
    ));
}

#[tokio::test]
async fn test_detect_config_file() {
    let dir = tempfile::TempDir::new().unwrap();
    let file_path = dir.path().join("agent.yaml");
    tokio::fs::write(&file_path, "").await.unwrap();

    let loader = ProfileLoader::new();
    let config = loader.detect_config(&file_path);
    assert!(matches!(
        config.storage_backend,
        crate::plugins::agent::core::StorageBackendType::Dir
    ));
}

#[tokio::test]
async fn test_load_from_path_nonexistent() {
    let loader = ProfileLoader::new();
    let profile = loader.load_from_path(Path::new("/nonexistent/path")).await;
    assert!(profile.is_none());
}
