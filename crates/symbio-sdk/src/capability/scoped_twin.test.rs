use std::sync::Arc;

use async_trait::async_trait;
use symbio_kernel::capability::{CapabilityFamily, CapabilityRow, Registry};
use tokio::sync::RwLock;

use super::{capability_scope_twins, SCOPE_TWIN};

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

/// 一条声明 = 一对名字。声明方与读方之间的全部约定就是原始元组。
#[tokio::test]
async fn collects_declared_pairs_without_parsing_names() {
    let registry = FakeRegistry::new();
    registry
        .put(CapabilityRow::new(
            SCOPE_TWIN,
            "twin:agent_x_read_skill",
            ("agent_x_read_skill".to_string(), "read_skill".to_string()),
            false,
        ))
        .await;
    registry
        .put(CapabilityRow::new(
            SCOPE_TWIN,
            "twin:agent_y_agent_run",
            ("agent_y_agent_run".to_string(), "agent_run".to_string()),
            false,
        ))
        .await;

    let pairs = capability_scope_twins(&registry).await;
    assert_eq!(pairs.len(), 2);
    assert!(pairs.contains(&("agent_x_read_skill".to_string(), "read_skill".to_string())));
    assert!(pairs.contains(&("agent_y_agent_run".to_string(), "agent_run".to_string())));
}

/// 反向用例：别的族、同族但载荷形状不对的行都不许进读数——否则一个无关的
/// 表键就能让装配面删掉一个真工具。
#[tokio::test]
async fn unrelated_rows_and_wrong_payloads_are_ignored() {
    let registry = FakeRegistry::new();
    registry
        .put(CapabilityRow::prompt("note", "一段提示词".to_string()))
        .await;
    registry
        .put(CapabilityRow::new(
            SCOPE_TWIN,
            "twin:bad",
            "不是一对名字".to_string(),
            false,
        ))
        .await;
    assert!(capability_scope_twins(&registry).await.is_empty());
}

/// 反向用例：声明方整个缺席（卸载）⇒ 空表，装配面一个工具都不删。
#[tokio::test]
async fn no_declaration_means_no_drops() {
    let registry = FakeRegistry::new();
    assert!(capability_scope_twins(&registry).await.is_empty());
}
