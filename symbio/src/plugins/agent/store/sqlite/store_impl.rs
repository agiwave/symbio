//! SqliteStorage 的 `AgentStore` trait 实现
//!
//! 统一 `query` 方法支持：
//! - 结构化过滤（FilterExpr 下推到 SQL WHERE）
//! - 语义搜索（`FilterExpr::Semantic` → sqlite-vec KNN 向量搜索）
//! - FTS5 全文搜索（空语义查询降级）
//! - 计数（`query(match_all, PageRequest::first(0))` 后读 `total`）

use super::{build_fts5_query, compile_filter, SqliteStorage};
use crate::plugins::agent::core::{
    evaluate_filter, unit_with_id, AgentStore, CognitiveUnit, FilterExpr, PageRequest, PageResult,
    StoreError,
};

#[async_trait::async_trait]
impl AgentStore for SqliteStorage {
    async fn get(&self, id: &str) -> Result<Option<CognitiveUnit>, StoreError> {
        let conn = self.get_conn().await?;
        let id = id.to_string();

        let data_str: Option<String> = conn
            .call(move |c| {
                let mut stmt = c.prepare("SELECT data FROM units WHERE id = ?")?;
                let res = stmt.query_row([id], |row| row.get(0)).ok();
                Ok(res)
            })
            .await
            .map_err(|e| StoreError::Backend(e.to_string()))?;

        Ok(data_str.and_then(|s| serde_json::from_str(&s).ok()))
    }

    async fn shutdown(&self) {}

    async fn insert(&self, unit: &CognitiveUnit) -> Result<CognitiveUnit, StoreError> {
        let conn = self.get_conn().await?;
        let id = unit.id();
        if id.is_empty() {
            return Err(StoreError::InvalidInput("Unit has no id".to_string()));
        }
        let id = id.to_string();
        let data_str =
            serde_json::to_string(&unit).map_err(|e| StoreError::Backend(e.to_string()))?;
        let version = unit.version();

        conn.call(move |c| {
            c.execute(
                "INSERT INTO units (id, data, version) VALUES (?, ?, ?)",
                rusqlite::params![id, data_str, version],
            )
            .map_err(|e| tokio_rusqlite::Error::Other(Box::new(e)))
        })
        .await
        .map_err(|e| StoreError::Backend(e.to_string()))?;

        // 如果已有 embedding，立即同步到 sqlite-vec；否则 enqueue 异步生成
        if unit.get_embedding().is_some() {
            let _ = self.upsert_vec_embedding(unit).await;
        } else if let Some(tx) = &self.embed_tx {
            let _ = tx.try_send(unit.id().to_string());
        }

        Ok(unit.clone())
    }

    async fn update(&self, unit: &CognitiveUnit) -> Result<CognitiveUnit, StoreError> {
        let conn = self.get_conn().await?;
        let id = unit.id();
        if id.is_empty() {
            return Err(StoreError::InvalidInput("Unit has no id".to_string()));
        }
        let id = id.to_string();
        let id_for_error = id.clone();
        let data_str =
            serde_json::to_string(&unit).map_err(|e| StoreError::Backend(e.to_string()))?;
        let version = unit.version();

        let affected = conn
            .call(move |c| {
                let mut stmt = c.prepare("UPDATE units SET data = ?, version = ? WHERE id = ?")?;
                let n = stmt.execute(rusqlite::params![data_str, version, id])?;
                Ok(n)
            })
            .await
            .map_err(|e| StoreError::Backend(format!("update({}): {}", id_for_error, e)))?;

        if affected == 0 {
            return Err(StoreError::NotFound(format!(
                "id '{id_for_error}' not found"
            )));
        }

        // 同步向量索引
        if unit.get_embedding().is_some() {
            let _ = self.upsert_vec_embedding(unit).await;
        }

        Ok(unit.clone())
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
        let conn = self.get_conn().await?;
        let id_owned = id.to_string();
        let existed = self.get(id).await?.is_some();
        conn.call(move |c| {
            c.execute("DELETE FROM units WHERE id = ?", [id_owned])
                .map_err(|e| tokio_rusqlite::Error::Other(Box::new(e)))
        })
        .await
        .map_err(|e| StoreError::Backend(e.to_string()))?;
        // 同步删除向量索引（rowid 已随 units 行删除，vec0 会自动清理）
        Ok(existed)
    }

    async fn query(
        &self,
        filter: &FilterExpr,
        page: &PageRequest,
    ) -> Result<PageResult, StoreError> {
        // 提取 Semantic 变体（如有），其余作为结构化约束
        let (semantic_query, structural_filter) = extract_semantic(filter);

        if let Some(query_text) = semantic_query {
            if self.embed_service.is_some() {
                // 语义搜索模式：sqlite-vec KNN 向量搜索
                return self
                    .vec_knn_search(&query_text, structural_filter.as_ref(), page)
                    .await;
            }
            // 降级：FTS5 全文搜索
            return self
                .fts5_query(&query_text, structural_filter.as_ref(), page)
                .await;
        }

        // 纯结构化过滤模式
        self.structured_query(filter, page).await
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

// ── 内部方法 ──

impl SqliteStorage {
    /// 纯结构化过滤查询
    async fn structured_query(
        &self,
        filter: &FilterExpr,
        page: &PageRequest,
    ) -> Result<PageResult, StoreError> {
        let compiled = compile_filter(filter);
        let conn = self.get_conn().await?;

        // total 始终返回匹配总数
        let count_sql = format!("SELECT COUNT(*) FROM units WHERE {}", compiled.where_clause);
        let count_params = compiled.params.clone();
        let total: usize = conn
            .call(move |c| {
                let mut stmt = c.prepare(&count_sql)?;
                let n: i64 = stmt
                    .query_row(rusqlite::params_from_iter(count_params.iter()), |row| {
                        row.get(0)
                    })?;
                Ok(n as usize)
            })
            .await
            .map_err(|e| StoreError::Backend(format!("count query failed: {}", e)))?;

        if total == 0 || page.limit == 0 {
            return Ok(PageResult {
                items: Vec::new(),
                total,
                offset: page.offset,
                limit: page.limit,
                scores: None,
            });
        }

        let page_sql = format!(
            "SELECT id, data FROM units WHERE {} ORDER BY id LIMIT ? OFFSET ?",
            compiled.where_clause
        );
        let mut page_params = compiled.params.clone();
        page_params.push(rusqlite::types::Value::Integer(page.limit as i64));
        page_params.push(rusqlite::types::Value::Integer(page.offset as i64));

        let rows: Vec<(String, String)> = conn
            .call(move |c| {
                let mut stmt = c.prepare(&page_sql)?;
                let iter = stmt
                    .query_map(rusqlite::params_from_iter(page_params.iter()), |row| {
                        Ok((row.get(0)?, row.get(1)?))
                    })?;
                let mut out = Vec::new();
                for res in iter.flatten() {
                    out.push(res);
                }
                Ok(out)
            })
            .await
            .map_err(|e| StoreError::Backend(format!("page query failed: {}", e)))?;

        let items: Vec<CognitiveUnit> = rows
            .into_iter()
            .filter_map(|(_, data_str)| serde_json::from_str(&data_str).ok())
            .collect();

        Ok(PageResult {
            items,
            total,
            offset: page.offset,
            limit: page.limit,
            scores: None,
        })
    }

    /// FTS5 语义搜索（bm25 排序）+ 结构化约束
    async fn fts5_query(
        &self,
        query_text: &str,
        constraint: Option<&FilterExpr>,
        page: &PageRequest,
    ) -> Result<PageResult, StoreError> {
        let conn = self.get_conn().await?;
        let limit = page.limit.max(1);
        let candidate_pool = (limit * 4).max(32);

        let fts_query = if query_text.trim().is_empty() {
            None
        } else {
            Some(build_fts5_query(query_text))
        };

        // FTS5 候选集
        let mut candidates: Vec<(String, f32, String)> = match fts_query {
            Some(fts_q) => conn
                .call(move |c| {
                    let mut stmt = c.prepare(
                        "SELECT u.id, bm25(units_fts) AS score, u.data
                     FROM units_fts fts
                     JOIN units u ON u.rowid = fts.rowid
                     WHERE units_fts MATCH ?1
                     ORDER BY score
                     LIMIT ?2",
                    )?;
                    let rows =
                        stmt.query_map(rusqlite::params![fts_q, candidate_pool as i64], |row| {
                            let id: String = row.get(0)?;
                            let score: f32 = row.get(1)?;
                            let data: String = row.get(2)?;
                            Ok((id, score, data))
                        })?;
                    let mut out = Vec::with_capacity(candidate_pool);
                    for r in rows {
                        out.push(r?);
                    }
                    Ok(out)
                })
                .await
                .map_err(|e| StoreError::Backend(format!("FTS5 search failed: {}", e)))?,
            None => conn
                .call(move |c| {
                    let mut stmt =
                        c.prepare("SELECT id, data FROM units ORDER BY rowid DESC LIMIT ?1")?;
                    let rows = stmt.query_map([candidate_pool as i64], |row| {
                        let id: String = row.get(0)?;
                        let data: String = row.get(1)?;
                        Ok((id, 0.0_f32, data))
                    })?;
                    let mut out = Vec::with_capacity(candidate_pool);
                    for r in rows {
                        out.push(r?);
                    }
                    Ok(out)
                })
                .await
                .map_err(|e| StoreError::Backend(format!("FTS5 fallback query failed: {}", e)))?,
        };

        // 结构化过滤 + 分数归一化
        let mut scored: Vec<(CognitiveUnit, f32)> = Vec::new();
        for (_id, bm25_score, data) in candidates.drain(..) {
            let unit: CognitiveUnit = match serde_json::from_str(&data) {
                Ok(u) => u,
                Err(_) => continue,
            };
            if let Some(f) = constraint {
                if !evaluate_filter(&unit, f) {
                    continue;
                }
            }
            // bm25 分数归一化到 0~1（bm25 越小越相关）
            let normalized = 1.0 / (1.0 + bm25_score.abs());
            scored.push((unit, normalized));
        }

        // 按相关度降序
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let total = scored.len();

        // 分页
        let start = page.offset.min(scored.len());
        let end = (start + page.limit).min(scored.len());
        let page_items = &scored[start..end];

        let items: Vec<CognitiveUnit> = page_items.iter().map(|(u, _)| u.clone()).collect();
        let scores: Vec<f32> = page_items.iter().map(|(_, s)| *s).collect();

        Ok(PageResult {
            items,
            total,
            offset: page.offset,
            limit: page.limit,
            scores: Some(scores),
        })
    }
}

/// 从 FilterExpr 中提取 Semantic 变体，返回 (semantic_query, 其余约束)
pub(crate) fn extract_semantic(filter: &FilterExpr) -> (Option<String>, Option<FilterExpr>) {
    match filter {
        FilterExpr::Semantic { query, .. } => (Some(query.clone()), None),
        FilterExpr::And(exprs) => {
            let mut semantic = None;
            let mut rest: Vec<FilterExpr> = Vec::new();
            for e in exprs {
                if let FilterExpr::Semantic { query, .. } = e {
                    semantic = Some(query.clone());
                } else {
                    rest.push(e.clone());
                }
            }
            let constraint = if rest.is_empty() {
                None
            } else if rest.len() == 1 {
                Some(rest.into_iter().next().unwrap())
            } else {
                Some(FilterExpr::And(rest))
            };
            (semantic, constraint)
        },
        _ => (None, Some(filter.clone())),
    }
}
