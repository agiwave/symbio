//! SQLite 存储后端
//!
//! 模块拆分：
//! - `mod.rs` — 结构体定义、SQL 编译器、构造、工厂
//! - `store_impl.rs` — `AgentStore` trait 实现

mod store_impl;

pub(crate) use store_impl::extract_semantic;

use crate::plugins::agent::core::types::cu_fields;
use crate::plugins::agent::core::CognitiveUnit;
use crate::plugins::agent::core::{
    evaluate_filter, AgentStore, EmbeddingService, FilterExpr, PageRequest, PageResult, StoreError,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_rusqlite::Connection;

// ── SQL 过滤编译器 ──

/// SQL 过滤编译结果：WHERE 子句 + 参数列表
pub(crate) struct CompiledFilter {
    pub(crate) where_clause: String,
    pub(crate) params: Vec<rusqlite::types::Value>,
}

impl CompiledFilter {}

pub(crate) fn compile_filter(expr: &FilterExpr) -> CompiledFilter {
    let mut params: Vec<rusqlite::types::Value> = Vec::new();
    let where_clause = match compile_filter_inner(expr, &mut params) {
        Some(clause) => clause,
        None => "1=1".to_string(),
    };
    CompiledFilter {
        where_clause,
        params,
    }
}

fn compile_filter_inner(
    expr: &FilterExpr,
    params: &mut Vec<rusqlite::types::Value>,
) -> Option<String> {
    match expr {
        FilterExpr::Eq { key, value } => {
            if key == cu_fields::ID {
                if let Some(s) = value.as_str() {
                    params.push(rusqlite::types::Value::Text(s.to_string()));
                    return Some("id = ?".to_string());
                }
            }
            params.push(json_to_sql_value(value));
            Some(format!(
                "json_extract(data, '$.{}') = ?",
                json_path_escape(key)
            ))
        }
        FilterExpr::Ne { key, value } => {
            if key == cu_fields::ID {
                if let Some(s) = value.as_str() {
                    params.push(rusqlite::types::Value::Text(s.to_string()));
                    return Some("id != ?".to_string());
                }
            }
            params.push(json_to_sql_value(value));
            Some(format!(
                "json_extract(data, '$.{}') != ?",
                json_path_escape(key)
            ))
        }
        FilterExpr::Gt { key, value } => {
            params.push(rusqlite::types::Value::Real(*value));
            Some(format!(
                "CAST(json_extract(data, '$.{}') AS REAL) > ?",
                json_path_escape(key)
            ))
        }
        FilterExpr::Gte { key, value } => {
            params.push(rusqlite::types::Value::Real(*value));
            Some(format!(
                "CAST(json_extract(data, '$.{}') AS REAL) >= ?",
                json_path_escape(key)
            ))
        }
        FilterExpr::Lt { key, value } => {
            params.push(rusqlite::types::Value::Real(*value));
            Some(format!(
                "CAST(json_extract(data, '$.{}') AS REAL) < ?",
                json_path_escape(key)
            ))
        }
        FilterExpr::Lte { key, value } => {
            params.push(rusqlite::types::Value::Real(*value));
            Some(format!(
                "CAST(json_extract(data, '$.{}') AS REAL) <= ?",
                json_path_escape(key)
            ))
        }
        FilterExpr::In { key, values } => {
            if values.is_empty() {
                return Some("0=1".to_string());
            }
            let placeholders = values.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            for v in values {
                params.push(json_to_sql_value(v));
            }
            Some(format!(
                "json_extract(data, '$.{}') IN ({})",
                json_path_escape(key),
                placeholders
            ))
        }
        FilterExpr::Contains { key, substring } => {
            let pattern = format!("%{}%", escape_like(substring));
            params.push(rusqlite::types::Value::Text(pattern));
            Some(format!(
                "json_extract(data, '$.{}') LIKE ?",
                json_path_escape(key)
            ))
        }
        FilterExpr::StartsWith { key, prefix } => {
            if key == cu_fields::ID {
                params.push(rusqlite::types::Value::Text(format!("{}%", prefix)));
                return Some("id LIKE ?".to_string());
            }
            let pattern = format!("{}%", escape_like(prefix));
            params.push(rusqlite::types::Value::Text(pattern));
            Some(format!(
                "json_extract(data, '$.{}') LIKE ?",
                json_path_escape(key)
            ))
        }
        FilterExpr::Relation {
            key: relation,
            value,
        } => {
            // 所有关系（含 is_a）在 JSON 中存储为顶级数组字段，如 $.is_a、$.causes
            let path = format!("$.{}", json_path_escape(relation));
            if let Some(prefix) = value.strip_suffix("::*") {
                params.push(rusqlite::types::Value::Text(format!(
                    "{}%",
                    escape_like(prefix)
                )));
                Some(format!(
                    "EXISTS (SELECT 1 FROM json_each(json_extract(data, '{}')) e WHERE e.value LIKE ?)",
                    path
                ))
            } else {
                params.push(rusqlite::types::Value::Text(value.clone()));
                Some(format!(
                    "EXISTS (SELECT 1 FROM json_each(json_extract(data, '{}')) e WHERE e.value = ?)",
                    path
                ))
            }
        }
        // Semantic 无法下推到 SQL，返回 None 让上层处理
        FilterExpr::Semantic { .. } => None,
        FilterExpr::And(exprs) => {
            let mut parts = Vec::new();
            for e in exprs {
                if let Some(p) = compile_filter_inner(e, params) {
                    parts.push(p);
                } else {
                    return Some("1=1".to_string());
                }
            }
            if parts.is_empty() {
                Some("1=1".to_string())
            } else {
                Some(format!("({})", parts.join(" AND ")))
            }
        }
        FilterExpr::Or(exprs) => {
            let mut parts = Vec::new();
            for e in exprs {
                if let Some(p) = compile_filter_inner(e, params) {
                    parts.push(p);
                } else {
                    return Some("1=1".to_string());
                }
            }
            if parts.is_empty() {
                Some("0=1".to_string())
            } else {
                Some(format!("({})", parts.join(" OR ")))
            }
        }
        FilterExpr::Not(inner) => {
            if let Some(p) = compile_filter_inner(inner, params) {
                Some(format!("NOT ({})", p))
            } else {
                Some("1=1".to_string())
            }
        }
    }
}

fn json_to_sql_value(v: &serde_json::Value) -> rusqlite::types::Value {
    use rusqlite::types::Value as SqlValue;
    match v {
        serde_json::Value::Null => SqlValue::Null,
        serde_json::Value::Bool(b) => SqlValue::Integer(if *b { 1 } else { 0 }),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                SqlValue::Integer(i)
            } else if let Some(u) = n.as_u64() {
                SqlValue::Integer(u as i64)
            } else if let Some(f) = n.as_f64() {
                SqlValue::Real(f)
            } else {
                SqlValue::Text(n.to_string())
            }
        }
        serde_json::Value::String(s) => SqlValue::Text(s.clone()),
        _ => SqlValue::Text(serde_json::to_string(v).unwrap_or_default()),
    }
}

fn json_path_escape(key: &str) -> String {
    key.replace('"', "\\\"")
}

fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

pub(crate) fn build_fts5_query(input: &str) -> String {
    let mut terms: Vec<String> = Vec::new();
    for raw in input.split_whitespace() {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        let escaped = trimmed.replace('"', "\"\"");
        terms.push(format!("\"{}\"*", escaped));
    }
    if terms.is_empty() {
        return "\"\"".to_string();
    }
    terms.join(" AND ")
}

// ── sqlite-vec 全局注册 ──
static SQLITE_VEC_INIT: std::sync::Once = std::sync::Once::new();

fn ensure_sqlite_vec_registered() {
    SQLITE_VEC_INIT.call_once(|| {
        unsafe {
            // sqlite-vec 提供的 C 入口签名与 sqlite3_auto_extension 期望的入口一致，
            // 通过 transmute 把 *const () 转为函数指针。rust 1.80 默认会提示"transmute 缺少
            // 安全注解"，用 #[allow] 显式说明这是受信任的第三方 FFI 调用。
            #[allow(clippy::missing_transmute_annotations)]
            let init_fn: Option<
                unsafe extern "C" fn(
                    *mut rusqlite::ffi::sqlite3,
                    *mut *mut i8,
                    *const rusqlite::ffi::sqlite3_api_routines,
                ) -> i32,
            > = Some(std::mem::transmute(
                sqlite_vec::sqlite3_vec_init as *const (),
            ));
            rusqlite::ffi::sqlite3_auto_extension(init_fn);
        }
    });
}

// ── embedding 常量 ──
const EMBED_QUEUE_CAPACITY: usize = 1024;
const EMBED_DIM: usize = 512; // bge-small-zh-v1.5

/// 将 f32 向量转为小端字节序列（sqlite-vec 格式）
fn f32_vec_to_bytes(v: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(v.len() * 4);
    for f in v {
        bytes.extend_from_slice(&f.to_le_bytes());
    }
    bytes
}

/// 从 CognitiveUnit 中提取文本用于 embedding
fn get_text_for_embedding(unit: &CognitiveUnit) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(name) = unit.name() {
        if !name.is_empty() {
            parts.push(name.to_string());
        }
    }
    if let Some(desc) = unit.description() {
        if !desc.is_empty() {
            parts.push(desc.to_string());
        }
    }
    if let Some(content) = unit.content() {
        if !content.is_empty() {
            parts.push(content.to_string());
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" "))
    }
}

// ── 核心结构体 ──

pub(crate) struct SqliteStorage {
    db_path: PathBuf,
    conn: Arc<tokio::sync::OnceCell<Connection>>,
    // ── embedding 语义搜索 ──
    embed_service: Option<Arc<dyn EmbeddingService>>,
    embed_tx: Option<mpsc::Sender<String>>,
}

impl SqliteStorage {
    pub fn new(base_path: &Path) -> Self {
        let db_path = base_path.join("units.db");
        Self {
            db_path,
            conn: Arc::new(tokio::sync::OnceCell::new()),
            embed_service: None,
            embed_tx: None,
        }
    }

    /// 工厂方法：供 store 注册表使用
    pub fn create(
        _config: &crate::plugins::agent::core::AgentConfig,
        agent_dir: &std::path::Path,
    ) -> crate::plugins::agent::store::BoxFuture<
        Result<Arc<dyn AgentStore>, crate::plugins::agent::core::StoreError>,
    > {
        let agent_dir = agent_dir.to_path_buf();
        Box::pin(async move {
            let mut storage = Self::new(&agent_dir);
            // 通过 submit_object_creator 机制获取 EmbeddingService
            let ctx: Arc<dyn crate::symbio_core::InvokeRequest> =
                Arc::new(crate::symbio_core::SimpleRequest::new(None, None));
            if let Some(embed_service) =
                crate::symbio_core::create_object::<dyn EmbeddingService>("fastembed", ctx)
            {
                let (embed_tx, embed_rx) = mpsc::channel::<String>(EMBED_QUEUE_CAPACITY);
                storage.embed_service = Some(embed_service.clone());
                storage.embed_tx = Some(embed_tx);
                let storage = Arc::new(storage);
                Self::spawn_embed_worker(
                    Arc::clone(&storage) as Arc<dyn AgentStore>,
                    embed_service,
                    embed_rx,
                );
                Ok(storage as Arc<dyn AgentStore>)
            } else {
                Ok(Arc::new(storage) as Arc<dyn AgentStore>)
            }
        })
    }

    /// 异步 embed worker
    fn spawn_embed_worker(
        store: Arc<dyn AgentStore>,
        embed_service: Arc<dyn EmbeddingService>,
        mut embed_rx: mpsc::Receiver<String>,
    ) {
        tokio::spawn(async move {
            loop {
                let Some(unit_id) = embed_rx.recv().await else {
                    return;
                };
                let Ok(Some(mut unit)) = store.get(&unit_id).await else {
                    continue;
                };
                if unit.get_embedding().is_some() {
                    continue;
                }
                if let Some(text) = get_text_for_embedding(&unit) {
                    if let Some(emb) = embed_service.embed(&text).await {
                        unit.set_embedding(emb);
                        let _ = store.update(&unit).await;
                    }
                }
            }
        });
    }

    /// 生成 embedding 并存入 sqlite-vec 虚拟表
    async fn upsert_vec_embedding(&self, unit: &CognitiveUnit) -> Result<(), StoreError> {
        let Some(emb) = unit.get_embedding() else {
            return Ok(());
        };
        let conn = self.get_conn().await?;
        let id = unit.id().to_string();
        let emb_bytes = f32_vec_to_bytes(&emb);
        conn.call(move |c| {
            // 先删除旧记录（rowid = units.rowid 需要关联）
            // sqlite-vec vec0 使用 rowid 关联，先按 id 查到 units.rowid
            let rowid: Option<i64> = c
                .query_row("SELECT rowid FROM units WHERE id = ?", [&id], |r| r.get(0))
                .ok();
            if let Some(rid) = rowid {
                c.execute("DELETE FROM units_vec WHERE rowid = ?", [rid])
                    .ok();
                c.execute(
                    "INSERT INTO units_vec(rowid, embedding) VALUES (?, ?)",
                    rusqlite::params![rid, emb_bytes],
                )
                .map_err(|e| tokio_rusqlite::Error::Other(Box::new(e)))?;
            }
            Ok(())
        })
        .await
        .map_err(|e| StoreError::Backend(format!("upsert_vec: {}", e)))?;
        Ok(())
    }

    /// 语义搜索：sqlite-vec KNN
    async fn vec_knn_search(
        &self,
        query_text: &str,
        constraint: Option<&FilterExpr>,
        page: &PageRequest,
    ) -> Result<PageResult, StoreError> {
        let embed_service = match &self.embed_service {
            Some(svc) => svc,
            None => {
                return Ok(PageResult {
                    items: vec![],
                    total: 0,
                    offset: page.offset,
                    limit: page.limit,
                    scores: None,
                })
            }
        };
        let query_embedding = match embed_service.embed(query_text).await {
            Some(emb) => emb,
            None => {
                return Ok(PageResult {
                    items: vec![],
                    total: 0,
                    offset: page.offset,
                    limit: page.limit,
                    scores: None,
                })
            }
        };
        let limit = page.limit.max(1);
        let candidate_pool = (limit * 4).max(32);
        let conn = self.get_conn().await?;
        let emb_bytes = f32_vec_to_bytes(&query_embedding);

        // KNN 查询：从 units_vec 获取最近邻，JOIN units 拉取完整数据
        let candidates: Vec<(String, f32, String)> = conn.call(move |c| {
            let mut stmt = c.prepare(
                "SELECT u.id, v.distance, u.data FROM units_vec v JOIN units u ON u.rowid = v.rowid WHERE v.embedding MATCH ? ORDER BY v.distance LIMIT ?"
            )?;
            let rows = stmt.query_map(
                rusqlite::params![emb_bytes, candidate_pool as i64],
                |row| {
                    let id: String = row.get(0)?;
                    let dist: f32 = row.get(1)?;
                    let data: String = row.get(2)?;
                    Ok((id, dist, data))
                },
            )?;
            let mut out = Vec::with_capacity(candidate_pool);
            for r in rows.flatten() {
                out.push(r);
            }
            Ok(out)
        }).await.map_err(|e| StoreError::Backend(format!("vec knn failed: {}", e)))?;

        // 应用约束 + 分数转换（distance → similarity）
        let mut scored: Vec<(CognitiveUnit, f32)> = Vec::new();
        for (_id, dist, data) in candidates {
            let unit: CognitiveUnit = match serde_json::from_str(&data) {
                Ok(u) => u,
                Err(_) => continue,
            };
            if let Some(f) = constraint {
                if !evaluate_filter(&unit, f) {
                    continue;
                }
            }
            // cosine distance → similarity
            let similarity = 1.0 - dist;
            scored.push((unit, similarity));
        }

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let total = scored.len();
        let start = page.offset.min(total);
        let end = (start + page.limit).min(total);
        let items: Vec<CognitiveUnit> = scored[start..end].iter().map(|(u, _)| u.clone()).collect();
        let scores: Vec<f32> = scored[start..end].iter().map(|(_, s)| *s).collect();

        Ok(PageResult {
            items,
            total,
            offset: page.offset,
            limit: page.limit,
            scores: Some(scores),
        })
    }

    pub(crate) async fn get_conn(&self) -> Result<&Connection, StoreError> {
        self.conn.get_or_try_init(|| async {
            if let Some(parent) = self.db_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }

            // 注册 sqlite-vec 扩展（全局一次）
            ensure_sqlite_vec_registered();

            let conn = Connection::open(&self.db_path).await
                .map_err(|e| StoreError::Backend(format!("Failed to open SQLite: {}", e)))?;

            conn.call(|c| {
                let sql = "CREATE TABLE IF NOT EXISTS units (
                        id TEXT PRIMARY KEY,
                        data TEXT NOT NULL,
                        version INTEGER NOT NULL
                    );
                    CREATE INDEX IF NOT EXISTS idx_units_version ON units(version);
                    CREATE VIRTUAL TABLE IF NOT EXISTS units_fts USING fts5(
                        id UNINDEXED,
                        name,
                        description,
                        content,
                        content='',
                        tokenize = 'unicode61 remove_diacritics 2'
                    );
                    CREATE TRIGGER IF NOT EXISTS units_fts_MODEL AFTER INSERT ON units BEGIN
                        INSERT INTO units_fts(rowid, id, name, description, content)
                        VALUES (new.rowid, new.id,
                                COALESCE(json_extract(new.data, '$.name'), ''),
                                COALESCE(json_extract(new.data, '$.description'), ''),
                                COALESCE(json_extract(new.data, '$.content'), ''));
                    END;
                    CREATE TRIGGER IF NOT EXISTS units_fts_ad AFTER DELETE ON units BEGIN
                        INSERT INTO units_fts(units_fts, rowid) VALUES ('delete', old.rowid);
                    END;
                    CREATE TRIGGER IF NOT EXISTS units_fts_au AFTER UPDATE ON units BEGIN
                        INSERT INTO units_fts(units_fts, rowid) VALUES ('delete', old.rowid);
                        INSERT INTO units_fts(rowid, id, name, description, content)
                        VALUES (new.rowid, new.id,
                                COALESCE(json_extract(new.data, '$.name'), ''),
                                COALESCE(json_extract(new.data, '$.description'), ''),
                                COALESCE(json_extract(new.data, '$.content'), ''));
                    END;";
                    c.execute_batch(sql)
                        .map_err(|e| tokio_rusqlite::Error::Other(Box::new(e)))?;
                    // sqlite-vec 虚拟表用 format! 动态生成维度
                    let vec_ddl = format!(
                        "CREATE VIRTUAL TABLE IF NOT EXISTS units_vec USING vec0(embedding float[{}])",
                        EMBED_DIM
                    );
                    c.execute_batch(&vec_ddl)
                        .map_err(|e| tokio_rusqlite::Error::Other(Box::new(e)))?;
                    Ok(())
            }).await.map_err(|e| StoreError::Backend(format!("Failed to create table: {}", e)))?;

            Ok(conn)
        }).await
    }
}

// ── 测试 ──
#[cfg(test)]
mod tests;

// ── 自注册 ──
crate::submit_store_backend!(
    crate::plugins::agent::core::StorageBackendType::Sqlite,
    SqliteStorage::create
);
