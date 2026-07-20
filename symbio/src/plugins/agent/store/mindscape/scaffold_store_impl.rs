use crate::plugins::agent::core::{
    AgentStore, CognitiveUnit, FilterExpr, PageRequest, PageResult, StoreError,
};

use super::scaffold::MindscapeScaffold;

#[async_trait::async_trait]
impl AgentStore for MindscapeScaffold {
    // ── 基础 CRUD：委托给内部 store ──

    async fn get(&self, id: &str) -> Result<Option<CognitiveUnit>, StoreError> {
        self.store.get(id).await
    }

    async fn insert(&self, unit: &CognitiveUnit) -> Result<CognitiveUnit, StoreError> {
        self.store.insert(unit).await
    }

    async fn update(&self, unit: &CognitiveUnit) -> Result<CognitiveUnit, StoreError> {
        self.store.update(unit).await
    }

    async fn upsert(&self, unit: &CognitiveUnit) -> Result<CognitiveUnit, StoreError> {
        self.store.upsert(unit).await
    }

    async fn delete(&self, id: &str) -> Result<bool, StoreError> {
        self.store.delete(id).await
    }

    async fn query(
        &self,
        filter: &FilterExpr,
        page: &PageRequest,
    ) -> Result<PageResult, StoreError> {
        self.store.query(filter, page).await
    }

    fn cancel_background_tasks(&self) {
        self.store.cancel_background_tasks();
    }

    async fn insert_batch(&self, units: &[CognitiveUnit]) -> Result<usize, StoreError> {
        self.store.insert_batch(units).await
    }

    // ── 认知反馈 ──

    async fn record_access(&self, unit_ids: &[&str]) {
        self.feedback.on_units_retrieved(unit_ids).await;
    }

    async fn shutdown(&self) {
        self.store.cancel_background_tasks();
        let n = self.feedback.shutdown().await;
        crate::plugin_info!(
            "agent",
            "[MindscapeScaffold] shutdown: flushed {} belief updates, cancelled background tasks",
            n
        );
    }
}
