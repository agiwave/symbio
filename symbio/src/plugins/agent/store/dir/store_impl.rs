use crate::plugins::agent::core::StorageFormat;
use crate::plugins::agent::core::{
    evaluate_filter, unit_with_id, AgentStore, CognitiveUnit, FilterExpr, PageRequest, PageResult,
    StoreError,
};
use serde_json::Value;
use tokio::fs;

use super::{deserialize_cu, DirStorage};

#[async_trait::async_trait]
impl AgentStore for DirStorage {
    async fn get(&self, id: &str) -> Result<Option<CognitiveUnit>, StoreError> {
        if self.is_single_file {
            let units = self.read_single_file().await;
            Ok(units.get(id).cloned())
        } else {
            let path = self.get_au_path(id);
            if !path.exists() {
                return Ok(None);
            }
            let content = fs::read_to_string(path)
                .await
                .map_err(|e| StoreError::Backend(e.to_string()))?;
            let parse_value: fn(&str) -> Option<Value> = match self.format {
                StorageFormat::Yaml => |s| serde_yaml_ng::from_str(s).ok(),
                StorageFormat::Json => |s| serde_json::from_str(s).ok(),
            };
            let au = parse_value(&content).and_then(|v| {
                if matches!(v, Value::Array(_)) {
                    None
                } else {
                    serde_json::from_value::<CognitiveUnit>(v)
                        .ok()
                        .or_else(|| deserialize_cu(&content, self.format))
                }
            });
            Ok(au)
        }
    }

    async fn insert(&self, unit: &CognitiveUnit) -> Result<CognitiveUnit, StoreError> {
        let id = unit.id();
        if id.is_empty() {
            return Err(StoreError::InvalidInput("认知单元没有 id".to_string()));
        }
        let mut new_unit = unit.clone();
        // 异步 embed
        if !self.enqueue_or_sync_embed(&mut new_unit).await {
            // queue 满或无 embed service，跳过
        }
        if self.is_single_file {
            let mut units = self.read_single_file().await;
            if units.contains_key(id) {
                return Err(StoreError::AlreadyExists(format!("id {id} 已存在")));
            }
            units.insert(id.to_string(), new_unit.clone());
            self.write_single_file(&units)
                .await
                .map_err(StoreError::Backend)?;
        } else {
            let path = self.get_au_path(id);
            if path.exists() {
                return Err(StoreError::AlreadyExists(format!("id {id} 已存在")));
            }
            self.save_internal(&new_unit)
                .await
                .map_err(StoreError::Backend)?;
        }
        self.invalidate_cache().await;
        Ok(new_unit)
    }

    async fn update(&self, unit: &CognitiveUnit) -> Result<CognitiveUnit, StoreError> {
        let id = unit.id();
        if id.is_empty() {
            return Err(StoreError::InvalidInput("认知单元没有 id".to_string()));
        }
        let mut updated_unit = unit.clone();
        if !self.enqueue_or_sync_embed(&mut updated_unit).await {
            // skip
        }
        if self.is_single_file {
            let mut units = self.read_single_file().await;
            if !units.contains_key(id) {
                return Err(StoreError::NotFound(format!("id {id} 不存在")));
            }
            units.insert(id.to_string(), updated_unit.clone());
            self.write_single_file(&units)
                .await
                .map_err(StoreError::Backend)?;
        } else {
            let path = self.get_au_path(id);
            if !path.exists() {
                return Err(StoreError::NotFound(format!("id {id} 不存在")));
            }
            self.save_internal(&updated_unit)
                .await
                .map_err(StoreError::Backend)?;
        }
        self.invalidate_cache().await;
        Ok(updated_unit)
    }

    async fn upsert(&self, unit: &CognitiveUnit) -> Result<CognitiveUnit, StoreError> {
        let target = unit_with_id(unit);
        if self.get(target.id()).await?.is_some() {
            self.update(&target).await
        } else {
            self.insert(&target).await
        }
    }

    async fn delete(&self, id: &str) -> Result<bool, StoreError> {
        self.unindex_embedding(id).await;
        let result = if self.is_single_file {
            let mut units = self.read_single_file().await;
            if units.remove(id).is_some() {
                self.write_single_file(&units)
                    .await
                    .map_err(StoreError::Backend)?;
                true
            } else {
                false
            }
        } else {
            let path = self.get_au_path(id);
            if path.exists() {
                fs::remove_file(path)
                    .await
                    .map_err(|e| StoreError::Backend(e.to_string()))?;
                true
            } else {
                false
            }
        };
        if result {
            self.invalidate_cache().await;
        }
        Ok(result)
    }

    async fn shutdown(&self) {}

    async fn query(
        &self,
        filter: &FilterExpr,
        page: &PageRequest,
    ) -> Result<PageResult, StoreError> {
        // 检测 Semantic filter → 走向量语义搜索
        let (semantic_query, structural_filter) =
            crate::plugins::agent::store::sqlite::extract_semantic(filter);
        if let Some(query_text) = semantic_query {
            return self
                .semantic_search(
                    &query_text,
                    structural_filter
                        .as_ref()
                        .unwrap_or(&FilterExpr::match_all()),
                    page,
                )
                .await;
        }
        let all = self.load_all().await;
        let mut matched: Vec<CognitiveUnit> = all
            .into_values()
            .filter(|u| evaluate_filter(u, filter))
            .collect();
        let total = matched.len();
        let start = page.offset.min(total);
        let end = (page.offset + page.limit).min(total);
        matched = matched[start..end].to_vec();
        Ok(PageResult {
            items: matched,
            total,
            offset: page.offset,
            limit: page.limit,
            scores: None,
        })
    }

    async fn insert_batch(&self, units: &[CognitiveUnit]) -> Result<usize, StoreError> {
        let mut count = 0;
        for unit in units {
            self.insert(unit).await?;
            count += 1;
        }
        Ok(count)
    }
}
