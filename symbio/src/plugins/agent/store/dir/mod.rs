use crate::plugins::agent::core::types::cu_fields;
use crate::plugins::agent::core::StorageFormat;
use crate::plugins::agent::core::{
    cu_from_json, evaluate_filter, AgentStore, CognitiveUnit, EmbeddingService, FilterExpr,
    PageRequest, PageResult, StoreError,
};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs;
use tokio::sync::{mpsc, RwLock};
use tokio_util::sync::CancellationToken;

// ── embedding 常量 ──
const EMBED_QUEUE_CAPACITY: usize = 1024;
const BUCKET_BITS: usize = 8;
const BUCKETS: usize = 1 << BUCKET_BITS; // 256
const BUCKET_NEIGHBOR_HOPS: u8 = 1;

/// 计算 embedding 的 8-bit 桶码（量化）
pub(super) fn quantize_embedding(emb: &[f32]) -> u8 {
    let dim = emb.len();
    if dim == 0 {
        return 0;
    }
    let segment_len = dim / BUCKET_BITS;
    if segment_len == 0 {
        return 0;
    }
    let rem = dim % BUCKET_BITS;
    let mut code: u8 = 0;
    let mut cursor = 0usize;
    for bit_idx in 0..BUCKET_BITS {
        let this_len = segment_len + if bit_idx < rem { 1 } else { 0 };
        let start = cursor;
        let end = cursor + this_len;
        cursor = end;
        if start >= end || end > dim {
            continue;
        }
        let sum: f32 = emb[start..end].iter().sum();
        let mean = sum / (end - start) as f32;
        if mean > 0.0 {
            code |= 1 << bit_idx;
        }
    }
    code
}

/// 枚举与 code 汉明距离 ≤ 1 的所有桶码（含自身）
pub(super) fn bucket_neighbors(code: u8) -> Vec<u8> {
    let mut out = Vec::with_capacity(BUCKET_NEIGHBOR_HOPS as usize * 8 + 1);
    out.push(code);
    for bit in 0..BUCKET_BITS {
        out.push(code ^ (1 << bit));
    }
    out
}

/// v8 迁移：灵活反序列化 CognitiveUnit
///
/// 优先尝试严格反序列化（CognitiveUnit 字段完全匹配）。
/// 失败时回退到 cu_from_json 宽松解析（保留所有未知字段到 properties），
/// 兼容旧数据中 is_a 为字符串、含未建模字段等情况。
fn deserialize_cu(content: &str, format: StorageFormat) -> Option<CognitiveUnit> {
    let value: Value = match format {
        StorageFormat::Json => serde_json::from_str(content).ok()?,
        StorageFormat::Yaml => serde_yml::from_str(content).ok()?,
    };
    if matches!(value, Value::Array(_)) {
        return None; // 不是单个 unit
    }
    if let Ok(unit) = serde_json::from_value::<CognitiveUnit>(value.clone()) {
        return Some(unit);
    }
    Some(cu_from_json(value))
}

/// 带缓存的目录存储
///
/// 缓存策略：
/// - 读取时加载到内存缓存
/// - 写入时同时更新缓存和磁盘
/// - 使用 RwLock 支持并发读取
pub(crate) struct DirStorage {
    base_path: PathBuf,
    format: StorageFormat,
    is_single_file: bool,
    /// 内存缓存，避免重复读取磁盘
    cache: Arc<RwLock<Option<HashMap<String, CognitiveUnit>>>>,
    // ── embedding 语义搜索 ──
    embed_service: Option<Arc<dyn EmbeddingService>>,
    // S-002 修复: vector_index/bucket_index 由 std::sync::RwLock 改为 tokio::sync::RwLock
    // 原因：这些字段在 async fn `semantic_search` 内部被 .read() 持锁，会阻塞 tokio worker。
    // 改为 tokio::sync::RwLock 后，.read().await 异步等待，不会阻塞 worker。
    // 锁内操作仅 HashMap 克隆，无 await 边界，无持锁跨 await 的反模式。
    vector_index: Arc<tokio::sync::RwLock<HashMap<String, Vec<f32>>>>,
    bucket_index: Arc<tokio::sync::RwLock<HashMap<u8, Vec<String>>>>,
    embed_tx: Option<mpsc::Sender<String>>,
    cancel_token: CancellationToken,
}

impl DirStorage {
    pub fn new(base_path: &Path, format: StorageFormat) -> Self {
        let is_single_file = base_path.is_file();
        Self {
            base_path: base_path.to_path_buf(),
            format,
            is_single_file,
            cache: Arc::new(RwLock::new(None)),
            embed_service: None,
            vector_index: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            bucket_index: Arc::new(tokio::sync::RwLock::new(HashMap::with_capacity(BUCKETS))),
            embed_tx: None,
            cancel_token: CancellationToken::new(),
        }
    }

    /// 工厂方法：供 store 注册表使用
    pub fn create(
        _config: &crate::plugins::agent::core::AgentConfig,
        agent_dir: &Path,
    ) -> crate::plugins::agent::store::BoxFuture<
        Result<Arc<dyn AgentStore>, crate::plugins::agent::core::StoreError>,
    > {
        let format = crate::plugins::agent::store::detect_format(agent_dir);
        let agent_dir = agent_dir.to_path_buf();
        Box::pin(async move {
            let mut storage = Self::new(&agent_dir, format);
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
                    Arc::clone(&storage.vector_index),
                    Arc::clone(&storage.bucket_index),
                    embed_rx,
                    storage.cancel_token.clone(),
                );
                Self::schedule_background_rebuild(
                    Arc::clone(&storage) as Arc<dyn AgentStore>,
                    Arc::clone(&storage.vector_index),
                    Arc::clone(&storage.bucket_index),
                    storage.cancel_token.clone(),
                );
                Ok(storage as Arc<dyn AgentStore>)
            } else {
                Ok(Arc::new(storage) as Arc<dyn AgentStore>)
            }
        })
    }

    fn get_units_dir(&self) -> PathBuf {
        if self.is_single_file {
            self.base_path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .to_path_buf()
        } else {
            self.base_path.clone()
        }
    }

    async fn ensure_dir(&self) -> std::io::Result<()> {
        if self.is_single_file {
            Ok(())
        } else {
            let units_dir = self.get_units_dir();
            if !units_dir.exists() {
                fs::create_dir_all(&units_dir).await?;
            }
            Ok(())
        }
    }

    /// 把 CU id 安全化，用作文件名（防止目录遍历）
    ///
    /// 之前只过滤了 `::` 和 `/`，但仍存在以下风险：
    /// - `\` (Windows 分隔符) 可绕过 Linux 检查
    /// - `..` 可向上越级
    /// - 空字符 `\0` 在某些 FS 上截断
    /// - 长度无上界，可构造超长路径触发 OS 错误
    /// - 路径名含 NUL/C0 控制字符在 Windows 上是 illegal
    ///
    /// 现在：先把 `..` / `.` / 路径分隔符 / NUL / 控制字符全部替换为 `_`，
    /// 然后用 `canonicalize` 后验证结果仍在 base_path 内（最后防线）。
    fn sanitize_id_for_filename(id: &str) -> String {
        const MAX_LEN: usize = 200; // 单文件名超过 200 几乎一定是异常

        // 1. 替换高危字符（含 `\` `..` `.` `/` NUL C0 控制字符）
        // 一次性遍历 + 一次分配，避免多次中间 String 分配
        let mut sanitized: String = id
            .chars()
            .map(|c| {
                if c == '\0' || c == '/' || c == '\\' || c == ':' || c.is_control() {
                    '_'
                } else {
                    c
                }
            })
            .collect();

        // 2. 处理 `..`：把整段 `..`（连续两个点）替换
        if sanitized.contains("..") {
            sanitized = sanitized.replace("..", "__");
        }

        // 3. 截断到 MAX_LEN
        if sanitized.len() > MAX_LEN {
            sanitized.truncate(MAX_LEN);
        }

        // 4. 兜底：空字符串 → 占位符（避免写入隐藏文件 ".yaml"）
        if sanitized.is_empty() || sanitized == "." {
            sanitized = "_empty".to_string();
        }

        sanitized
    }

    fn get_au_path(&self, id: &str) -> PathBuf {
        if self.is_single_file {
            self.base_path.clone()
        } else {
            let safe_id = Self::sanitize_id_for_filename(id);
            let ext = match self.format {
                StorageFormat::Yaml => "yaml",
                StorageFormat::Json => "json",
            };
            let units_dir = self.get_units_dir();
            let path = units_dir.join(format!("{safe_id}.{ext}"));

            // 最后一道防线：把"我们以为安全"的路径规范化后，
            // 确认仍在 units_dir 之内。这一步在正常路径下是 no-op，
            // 但如果 sanitize 漏了某个 corner case，仍能拦截目录遍历。
            // 注意：被文件尚未创建时 canonicalize 失败，fallback 到 path.starts_with
            if let (Ok(canonical_base), Ok(canonical_path)) =
                (units_dir.canonicalize(), path.canonicalize())
            {
                if !canonical_path.starts_with(&canonical_base) {
                    crate::plugin_warn!(
                        "agent",
                        "[DirStorage] Path traversal attempt detected for id '{}', \
                         falling back to sanitized name",
                        id
                    );
                    // 二次 sanitize：把所有分隔符强制替换
                    let mut fallback = safe_id
                        .chars()
                        .map(|c| if c == '/' || c == '\\' { '_' } else { c })
                        .collect::<String>();
                    if fallback.is_empty() {
                        fallback = "_fallback".to_string();
                    }
                    return units_dir.join(format!("{fallback}.{ext}"));
                }
            }

            path
        }
    }

    async fn save_internal(&self, au: &CognitiveUnit) -> Result<(), String> {
        if self.is_single_file {
            return Err("Cannot save single unit to single file mode".to_string());
        }

        self.ensure_dir().await.map_err(|e| e.to_string())?;
        let id = au.id();
        if id.is_empty() {
            return Err("认知单元没有 id".to_string());
        }
        let path = self.get_au_path(id);

        let content = match self.format {
            StorageFormat::Yaml => serde_yml::to_string(au).map_err(|e| e.to_string())?,
            StorageFormat::Json => serde_json::to_string_pretty(au).map_err(|e| e.to_string())?,
        };

        fs::write(path, content).await.map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn read_single_file(&self) -> HashMap<String, CognitiveUnit> {
        let mut units = HashMap::new();

        if !self.base_path.exists() {
            return units;
        }

        let content = match fs::read_to_string(&self.base_path).await {
            Ok(c) => c,
            Err(_) => return units,
        };

        // v8 迁移：直接反序列化为 CognitiveUnit
        // 优先尝试数组（含 typed 结构）→ 单个 → 退化宽松解析
        if let Ok(items) = serde_json::from_str::<Vec<CognitiveUnit>>(&content) {
            for au in items {
                let id = au.id();
                if !id.is_empty() {
                    units.insert(id.to_string(), au);
                }
            }
            return units;
        }

        if let Ok(au) = serde_json::from_str::<CognitiveUnit>(&content) {
            let id = au.id();
            if !id.is_empty() {
                units.insert(id.to_string(), au);
            }
            return units;
        }

        if let Ok(items) = serde_yml::from_str::<Vec<CognitiveUnit>>(&content) {
            for au in items {
                let id = au.id();
                if !id.is_empty() {
                    units.insert(id.to_string(), au);
                }
            }
            return units;
        }

        if let Ok(au) = serde_yml::from_str::<CognitiveUnit>(&content) {
            let id = au.id();
            if !id.is_empty() {
                units.insert(id.to_string(), au);
            }
            return units;
        }

        // v8 迁移：兼容旧数据（is_a 为字符串、含未建模字段等）
        // 先解析为 Value，再通过 cu_from_json 宽松构造
        if let Ok(value) = serde_yml::from_str::<Value>(&content) {
            match value {
                Value::Array(arr) => {
                    for v in arr {
                        if let Some(id) = v.get(cu_fields::ID).and_then(|x| x.as_str()) {
                            units.insert(id.to_string(), cu_from_json(v));
                        }
                    }
                },
                Value::Object(_) => {
                    let unit = cu_from_json(value);
                    let id = unit.id();
                    if !id.is_empty() {
                        units.insert(id.to_string(), unit);
                    }
                },
                _ => {},
            }
        } else if let Ok(value) = serde_json::from_str::<Value>(&content) {
            match value {
                Value::Array(arr) => {
                    for v in arr {
                        if let Some(id) = v.get(cu_fields::ID).and_then(|x| x.as_str()) {
                            units.insert(id.to_string(), cu_from_json(v));
                        }
                    }
                },
                Value::Object(_) => {
                    let unit = cu_from_json(value);
                    let id = unit.id();
                    if !id.is_empty() {
                        units.insert(id.to_string(), unit);
                    }
                },
                _ => {},
            }
        }

        units
    }

    async fn write_single_file(
        &self,
        units: &HashMap<String, CognitiveUnit>,
    ) -> Result<(), String> {
        let content = match self.format {
            StorageFormat::Yaml => serde_yml::to_string(&units).map_err(|e| e.to_string())?,
            StorageFormat::Json => {
                serde_json::to_string_pretty(&units).map_err(|e| e.to_string())?
            },
        };

        if let Some(parent) = self.base_path.parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|e| e.to_string())?;
        }

        fs::write(&self.base_path, content)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// 使缓存失效
    async fn invalidate_cache(&self) {
        let mut cache = self.cache.write().await;
        *cache = None;
    }

    // ── embedding 辅助方法 ──

    /// 把单元文本拼接为嵌入输入
    fn get_text_for_embedding(unit: &CognitiveUnit) -> Option<String> {
        let mut parts = Vec::new();
        if let Some(name) = unit.name() {
            parts.push(name.to_string());
        }
        if let Some(desc) = unit.description() {
            parts.push(desc.to_string());
        }
        if let Some(content) = unit.content() {
            parts.push(content.to_string());
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join(" "))
        }
    }

    /// 异步 embed worker
    /// S-002 修复: vector_index/bucket_index 参数类型由 std::sync::RwLock 改为 tokio::sync::RwLock
    fn spawn_embed_worker(
        store: Arc<dyn AgentStore>,
        embed_service: Arc<dyn EmbeddingService>,
        vector_index: Arc<tokio::sync::RwLock<HashMap<String, Vec<f32>>>>,
        bucket_index: Arc<tokio::sync::RwLock<HashMap<u8, Vec<String>>>>,
        mut embed_rx: mpsc::Receiver<String>,
        cancel_token: CancellationToken,
    ) {
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    biased;
                    _ = cancel_token.cancelled() => { return; }
                    maybe_id = embed_rx.recv() => {
                        let Some(unit_id) = maybe_id else { return; };
                        let Ok(Some(mut unit)) = store.get(&unit_id).await else { continue; };
                        if unit.get_embedding().is_some() {
                            Self::index_unit_inplace(&vector_index, &bucket_index, &unit).await;
                            continue;
                        }
                        if let Some(text) = Self::get_text_for_embedding(&unit) {
                            if let Some(emb) = embed_service.embed(&text).await {
                                // Race-condition fix: 之前这里调 `store.update(&unit).await`
                                // 会把 worker 拿到的旧 unit（来自 worker 启动前的 store.get）写回
                                // 磁盘，覆盖掉调用方随后做的 update 业务数据修改。
                                // 例如 test: insert(系统管家) → enqueue → user update(张三) → worker
                                // 才从 store.get 拿到 "系统管家"，embed 后调用 update 把"张三"覆盖。
                                // 现在 worker 只更新内存向量索引；embedding 的持久化由调用方的下一次
                                // update/insert 经 enqueue_or_sync_embed 同步路径完成。
                                unit.set_embedding(emb);
                                Self::index_unit_inplace(&vector_index, &bucket_index, &unit).await;
                            }
                        }
                    }
                }
            }
        });
    }

    /// 把 unit 加入向量索引 + 桶索引
    /// S-002 修复: 由同步 fn 改为 async fn，因为现在 vector_index/bucket_index 是
    /// tokio::sync::RwLock，.write().await 必须 await 释放 worker。锁内仅 HashMap 写入，
    /// 无 await 边界，不会出现持锁跨 await 的反模式。
    /// 注: tokio::sync::RwLock::write() 直接返回 RwLockWriteGuard（无 PoisonError，
    /// 也不需要 .unwrap()），所以不能用 `if let Ok(...) = ...` 模式。
    async fn index_unit_inplace(
        vector_index: &Arc<tokio::sync::RwLock<HashMap<String, Vec<f32>>>>,
        bucket_index: &Arc<tokio::sync::RwLock<HashMap<u8, Vec<String>>>>,
        unit: &CognitiveUnit,
    ) {
        let id = unit.id();
        if id.is_empty() {
            return;
        }
        let Some(emb) = unit.get_embedding() else {
            return;
        };
        let new_code = quantize_embedding(&emb);
        let mut idx = vector_index.write().await;
        let old_code = idx.get(id).map(|old| quantize_embedding(old.as_slice()));
        idx.insert(id.to_string(), emb);
        let mut bkt = bucket_index.write().await;
        if let Some(old) = old_code {
            if let Some(v) = bkt.get_mut(&old) {
                v.retain(|x| x.as_str() != id);
            }
        }
        bkt.entry(new_code).or_default().push(id.to_string());
    }

    /// 从索引移除
    /// S-002 修复: 由同步 fn 改为 async fn，与 index_unit_inplace 保持一致。
    /// 注: tokio::sync::RwLock::write() 直接返回 RwLockWriteGuard，无 PoisonError。
    async fn unindex_embedding(&self, id: &str) {
        let mut idx = self.vector_index.write().await;
        if let Some(old_emb) = idx.remove(id) {
            let code = quantize_embedding(&old_emb);
            let mut bkt = self.bucket_index.write().await;
            if let Some(v) = bkt.get_mut(&code) {
                v.retain(|x| x != id);
            }
        }
    }

    /// 尝试入队异步 embed
    /// S-002 修复: 由同步 fn 改为 async fn（因 index_unit_inplace 现在是 async）。
    async fn enqueue_or_sync_embed(&self, unit: &mut CognitiveUnit) -> bool {
        if unit.get_embedding().is_some() {
            // 已有 embedding，直接索引
            Self::index_unit_inplace(&self.vector_index, &self.bucket_index, unit).await;
            return true;
        }
        if let Some(tx) = &self.embed_tx {
            match tx.try_send(unit.id().to_string()) {
                Ok(()) => true,
                Err(_) => false,
            }
        } else {
            false
        }
    }

    /// 后台重建向量索引
    /// S-002 修复: vector_index/bucket_index 参数类型由 std::sync::RwLock 改为 tokio::sync::RwLock，
    /// 内部 .write() 改 .write().await
    fn schedule_background_rebuild(
        store: Arc<dyn AgentStore>,
        vector_index: Arc<tokio::sync::RwLock<HashMap<String, Vec<f32>>>>,
        bucket_index: Arc<tokio::sync::RwLock<HashMap<u8, Vec<String>>>>,
        cancel_token: CancellationToken,
    ) {
        tokio::spawn(async move {
            let mut new_index = HashMap::new();
            let mut new_buckets: HashMap<u8, Vec<String>> = HashMap::with_capacity(BUCKETS);
            let page_size = 500;
            let mut offset = 0;
            loop {
                if cancel_token.is_cancelled() {
                    return;
                }
                let page = crate::plugins::agent::core::PageRequest::new(offset, page_size);
                let batch = match store
                    .query(&crate::plugins::agent::core::FilterExpr::match_all(), &page)
                    .await
                {
                    Ok(b) => b,
                    Err(_) => return,
                };
                let has_next = batch.has_next();
                for unit in batch.items {
                    let id = unit.id().to_string();
                    if let Some(emb) = unit.get_embedding() {
                        let code = quantize_embedding(&emb);
                        new_buckets.entry(code).or_default().push(id.clone());
                        new_index.insert(id, emb);
                    }
                }
                if !has_next {
                    break;
                }
                offset += page_size;
            }
            let count = new_index.len();
            // tokio::sync::RwLock::write() 直接返回 RwLockWriteGuard
            *vector_index.write().await = new_index;
            *bucket_index.write().await = new_buckets;
            crate::plugin_info!(
                "agent",
                "[DirStorage] Background vector index ready: {} entries",
                count
            );
        });
    }

    /// 语义搜索：桶式 ANN
    async fn semantic_search(
        &self,
        query_text: &str,
        filter: &FilterExpr,
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
            },
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
            },
        };
        let min_score = 0.1;

        // 冷启动降级：索引为空时全量扫描
        // S-002 修复: tokio::sync::RwLock 用 .read().await 异步等待，不阻塞 worker
        // tokio::sync::RwLock::read() 直接返回 RwLockReadGuard（无 PoisonError），
        // 所以把 std 的 `match guard { Ok(g) => g.clone(), Err(_) => ... }` 模式
        // 简化为 `guard.clone()`。
        let vector_size = self.vector_index.read().await.len();
        if vector_size == 0 {
            return self
                .degraded_semantic_search(&query_embedding, filter, page, min_score)
                .await;
        }

        // 桶式 ANN 候选
        let query_code = quantize_embedding(&query_embedding);
        let bkt_snapshot = self.bucket_index.read().await.clone();
        if bkt_snapshot.is_empty() {
            return self
                .degraded_semantic_search(&query_embedding, filter, page, min_score)
                .await;
        }
        let neighbors = bucket_neighbors(query_code);
        let mut seen = HashMap::new();
        for code in neighbors {
            if let Some(ids) = bkt_snapshot.get(&code) {
                for id in ids {
                    seen.entry(id.clone()).or_insert(());
                }
            }
        }
        let candidate_ids: Vec<String> = seen.into_keys().collect();

        // 对候选打分
        let idx_snapshot = {
            let guard = self.vector_index.read().await;
            candidate_ids
                .iter()
                .filter_map(|id| guard.get(id).map(|v| (id.clone(), v.clone())))
                .collect::<HashMap<_, _>>()
        };
        if idx_snapshot.is_empty() {
            return self
                .degraded_semantic_search(&query_embedding, filter, page, min_score)
                .await;
        }
        let mut scored: Vec<(String, f32)> = candidate_ids
            .into_iter()
            .filter_map(|id| {
                idx_snapshot.get(&id).map(|emb| {
                    let score =
                        crate::plugins::agent::core::cosine_similarity(&query_embedding, emb);
                    (id, score)
                })
            })
            .filter(|(_, s)| *s >= min_score)
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // 拉取完整 CU 并应用 filter
        let mut results: Vec<(CognitiveUnit, f32)> = Vec::new();
        for (id, score) in &scored {
            if let Ok(Some(unit)) = self.get(id).await {
                if evaluate_filter(&unit, filter) {
                    results.push((unit, *score));
                }
            }
        }

        let total = results.len();
        let start = page.offset.min(total);
        let end = (page.offset + page.limit).min(total);
        let page_items: Vec<CognitiveUnit> =
            results[start..end].iter().map(|(u, _)| u.clone()).collect();
        let page_scores: Vec<f32> = results[start..end].iter().map(|(_, s)| *s).collect();

        Ok(PageResult {
            items: page_items,
            total,
            offset: page.offset,
            limit: page.limit,
            scores: Some(page_scores),
        })
    }

    /// 降级语义搜索：全量扫描 + 余弦打分
    async fn degraded_semantic_search(
        &self,
        query_embedding: &[f32],
        filter: &FilterExpr,
        page: &PageRequest,
        min_score: f32,
    ) -> Result<PageResult, StoreError> {
        let all = self.load_all().await;
        let mut scored: Vec<(CognitiveUnit, f32)> = Vec::new();
        for (_, mut unit) in all {
            if !evaluate_filter(&unit, filter) {
                continue;
            }
            // 尝试同步计算 embedding
            if unit.get_embedding().is_none() {
                if let Some(svc) = &self.embed_service {
                    if let Some(text) = Self::get_text_for_embedding(&unit) {
                        if let Some(emb) = svc.embed(&text).await {
                            unit.set_embedding(emb);
                        }
                    }
                }
            }
            if let Some(emb) = unit.get_embedding() {
                let score = crate::plugins::agent::core::cosine_similarity(query_embedding, &emb);
                if score >= min_score {
                    scored.push((unit, score));
                }
            }
        }
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let total = scored.len();
        let start = page.offset.min(total);
        let end = (page.offset + page.limit).min(total);
        let items = scored[start..end].iter().map(|(u, _)| u.clone()).collect();
        let scores = scored[start..end].iter().map(|(_, s)| *s).collect();
        Ok(PageResult {
            items,
            total,
            offset: page.offset,
            limit: page.limit,
            scores: Some(scores),
        })
    }

    async fn load_all(&self) -> HashMap<String, CognitiveUnit> {
        // 检查缓存（快速路径）
        {
            let cache = self.cache.read().await;
            if let Some(cached) = cache.as_ref() {
                return cached.clone();
            }
        }

        // 缓存未命中，获取写锁再次检查并加载
        let mut cache = self.cache.write().await;

        // 双重检查锁定：获取写锁后再次检查缓存
        if let Some(cached) = cache.as_ref() {
            return cached.clone();
        }

        // 从磁盘加载
        let result = if self.is_single_file {
            self.read_single_file().await
        } else {
            let mut units = HashMap::new();
            let units_dir = self.get_units_dir();
            if !units_dir.exists() {
                *cache = Some(units.clone());
                return units;
            }

            if let Ok(mut entries) = fs::read_dir(units_dir).await {
                while let Ok(Some(entry)) = entries.next_entry().await {
                    let path = entry.path();
                    if path.is_file() {
                        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
                        if ext == "json" || ext == "yaml" {
                            if let Ok(content) = fs::read_to_string(&path).await {
                                let format = if ext == "yaml" {
                                    StorageFormat::Yaml
                                } else {
                                    StorageFormat::Json
                                };
                                // v8：优先严格反序列化，失败时回退到宽松解析
                                let parse_value: fn(&str) -> Option<Value> = match ext {
                                    "yaml" => |s| serde_yml::from_str(s).ok(),
                                    _ => |s| serde_json::from_str(s).ok(),
                                };
                                let au = parse_value(&content).and_then(|v| {
                                    if matches!(v, Value::Array(_)) {
                                        None
                                    } else {
                                        serde_json::from_value::<CognitiveUnit>(v)
                                            .ok()
                                            .or_else(|| deserialize_cu(&content, format))
                                    }
                                });
                                if let Some(au) = au {
                                    let id = au.id();
                                    if !id.is_empty() {
                                        units.insert(id.to_string(), au);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            units
        };

        // 更新缓存
        *cache = Some(result.clone());
        result
    }
}

mod store_impl;

#[cfg(test)]
mod tests;

// ── 自注册 ──
crate::submit_store_backend!(
    crate::plugins::agent::core::StorageBackendType::Dir,
    DirStorage::create
);
