use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;
use symbio_kernel::capability::{CapabilityFamily, CapabilityRow, Registry};
use tokio::sync::RwLock;

use super::{capability_config_snapshot, ConfigSnapshotValue, CONFIG_SNAPSHOT};

/// 最小登记表：只装行、按族过滤（sdk 不依赖任何兄弟 crate）。
struct FakeRegistry {
    rows: RwLock<Vec<Arc<CapabilityRow>>>,
}

impl FakeRegistry {
    fn new() -> Self {
        Self {
            rows: RwLock::new(Vec::new()),
        }
    }
}

#[async_trait]
impl Registry for FakeRegistry {
    async fn put(&self, row: CapabilityRow) {
        self.rows.write().await.push(Arc::new(row));
    }

    async fn get(&self, name: &str) -> Option<Arc<CapabilityRow>> {
        self.rows
            .read()
            .await
            .iter()
            .rev()
            .find(|r| r.name() == name)
            .cloned()
    }

    async fn list(&self, fam: CapabilityFamily) -> Vec<Arc<CapabilityRow>> {
        self.rows
            .read()
            .await
            .iter()
            .filter(|r| r.family() == fam)
            .cloned()
            .collect()
    }
}

#[tokio::test]
async fn collects_declared_keys_without_knowing_them() {
    let registry = FakeRegistry::new();
    registry
        .put(CapabilityRow::new(
            CONFIG_SNAPSHOT,
            "alpha_enabled",
            ConfigSnapshotValue(json!(true)),
            true,
        ))
        .await;
    registry
        .put(CapabilityRow::new(
            CONFIG_SNAPSHOT,
            "beta_cap",
            ConfigSnapshotValue(json!(4)),
            true,
        ))
        .await;

    let snapshot = capability_config_snapshot(&registry).await;
    assert_eq!(snapshot.get("alpha_enabled"), Some(&json!(true)));
    assert_eq!(snapshot.get("beta_cap"), Some(&json!(4)));
    assert_eq!(snapshot.len(), 2);
}

#[tokio::test]
async fn same_key_last_declaration_wins() {
    let registry = FakeRegistry::new();
    for value in [json!(false), json!(true)] {
        registry
            .put(CapabilityRow::new(
                CONFIG_SNAPSHOT,
                "alpha_enabled",
                ConfigSnapshotValue(value),
                true,
            ))
            .await;
    }
    let snapshot = capability_config_snapshot(&registry).await;
    assert_eq!(snapshot.get("alpha_enabled"), Some(&json!(true)));
}

#[tokio::test]
async fn unrelated_rows_are_ignored() {
    let registry = FakeRegistry::new();
    registry
        .put(CapabilityRow::new(
            CONFIG_SNAPSHOT,
            "alpha_enabled",
            ConfigSnapshotValue(json!(true)),
            true,
        ))
        .await;
    registry
        .put(CapabilityRow::prompt("note", "一段提示词".to_string()))
        .await;
    let snapshot = capability_config_snapshot(&registry).await;
    assert_eq!(snapshot.len(), 1);
}
