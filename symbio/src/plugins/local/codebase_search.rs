//! 语义代码检索工具（对应 Trae 的 SearchCodebase）
//!
//! 复用 `EmbeddingService`（fastembed 本地模型）对源码分块嵌入，
//! 对查询做近似最近邻（余弦相似度）检索。
//! 若嵌入服务不可用，自动降级为正则关键词检索（ripgrep 库，非外部可执行文件）。
//!
//! 建索引较重，索引按工作区缓存于进程内；传入 rebuild=true 可强制重建。

use super::policy::SecurityPolicy;
use crate::symbio_core::providers::EmbeddingService;
use crate::symbio_core::{
    create_object, Capability, CapabilityMeta, InvokeRequest, InvokeRequestExt, InvokeResponse,
    PluginError, PluginPayload, SimpleRequest,
};
use async_trait::async_trait;
use grep::regex::RegexMatcherBuilder;
use grep::searcher::sinks::UTF8 as UTF8Sink;
use grep::searcher::SearcherBuilder;
use ignore::WalkBuilder;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const CHUNK_LINES: usize = 40;
const CHUNK_STEP: usize = 20;
const DEFAULT_LIMIT: usize = 8;
const MAX_LIMIT: usize = 30;

/// 纳入索引的源码扩展名
const SOURCE_EXTS: &[&str] = &[
    "rs", "py", "js", "ts", "tsx", "jsx", "go", "java", "c", "cpp", "h", "hpp", "cs", "rb", "php",
    "swift", "kt", "md", "toml", "json", "yml", "yaml", "sql", "html", "css", "sh", "vue",
    "svelte", "txt", "lua", "r", "scala", "dart",
];

/// 一个被嵌入的源码分块
struct Chunk {
    file: String,
    start_line: usize,
    end_line: usize,
    text: String,
    /// 归一化后的嵌入向量，用于余弦相似度（点积）
    norm: Vec<f32>,
}

/// 一个工作区的代码索引
struct CodeIndex {
    chunks: Vec<Chunk>,
}

/// 按工作区路径缓存索引（进程内）
static INDEX_CACHE: std::sync::OnceLock<tokio::sync::Mutex<HashMap<String, Arc<CodeIndex>>>> =
    std::sync::OnceLock::new();

fn cache() -> &'static tokio::sync::Mutex<HashMap<String, Arc<CodeIndex>>> {
    INDEX_CACHE.get_or_init(|| tokio::sync::Mutex::new(HashMap::new()))
}

/// 诊断/检索工具：语义代码检索
#[derive(Clone)]
pub struct CodebaseSearchTool {
    #[allow(dead_code)]
    security: Arc<SecurityPolicy>,
}

impl CodebaseSearchTool {
    pub fn new(security: Arc<SecurityPolicy>) -> Self {
        Self { security }
    }

    async fn execute_inner(&self, args: &Value, workdir: &str) -> InvokeResponse<Value> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if query.is_empty() {
            return Err(PluginError::ValidationError("缺少 query 参数".to_string()));
        }
        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
        let rebuild = args
            .get("rebuild")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_LIMIT as u64)
            .clamp(1, MAX_LIMIT as u64) as usize;

        let workspace_dir = PathBuf::from(shellexpand::tilde(workdir).to_string());

        let ctx: Arc<dyn InvokeRequest> = Arc::new(SimpleRequest::new(None, None));
        let embed = create_object::<dyn EmbeddingService>("fastembed", ctx);

        let mut results: Vec<Value> = Vec::new();
        let mut mode = "keyword_fallback";

        if let Some(embed) = embed.as_ref() {
            if let Some(index) = get_index(&workspace_dir, rebuild, embed).await {
                let sem = semantic_search(&index, &query, embed, limit).await;
                if !sem.is_empty() {
                    results = sem;
                    mode = "semantic";
                }
            }
        }

        if results.is_empty() {
            results = keyword_search(&workspace_dir, &query, limit).await;
            mode = "keyword_fallback";
        }

        if !path.is_empty() {
            results.retain(|r| {
                r.get("file")
                    .and_then(|f| f.as_str())
                    .map(|f| f.contains(path))
                    .unwrap_or(false)
            });
        }

        Ok(json!({
            "query": query,
            "mode": mode,
            "results": results,
            "count": results.len(),
            "message": format!("以 {} 模式返回 {} 条结果。", mode, results.len()),
        }))
    }
}

fn is_source_file(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| SOURCE_EXTS.contains(&e))
        .unwrap_or(false)
}

/// 列出工作区内的源码文件：用 `ignore`（尊重 .gitignore）遍历，并按扩展名白名单过滤。
/// 不再依赖 `rg` 等外部可执行文件，跨平台行为一致。
async fn list_source_files(workdir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut builder = WalkBuilder::new(workdir);
    builder
        .standard_filters(true) // 尊重 .gitignore / 隐藏文件 / 全局 ignore
        .parents(true)
        .require_git(false);
    // 跳过已知的非源码大目录（与旧 `rg --files` 的 !target/!node_modules 等价）
    builder.filter_entry(|e| {
        if e.path().is_dir() {
            let name = e.file_name().to_string_lossy();
            return !matches!(
                name.as_ref(),
                "target" | "node_modules" | ".git" | "dist" | "build"
            );
        }
        true
    });
    for entry in builder.build().flatten() {
        let p = entry.path();
        if p.is_file() && is_source_file(p) {
            files.push(p.to_path_buf());
        }
    }
    files
}

fn normalize(v: &[f32]) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm == 0.0 {
        return v.to_vec();
    }
    v.iter().map(|x| x / norm).collect()
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// 构建（或取缓存）某工作区的代码索引
async fn get_index(
    workdir: &Path,
    rebuild: bool,
    embed: &Arc<dyn EmbeddingService>,
) -> Option<Arc<CodeIndex>> {
    let key = workdir.to_string_lossy().to_string();
    {
        let cache = cache().lock().await;
        if let Some(idx) = cache.get(&key) {
            if !rebuild {
                return Some(Arc::clone(idx));
            }
        }
    }
    let idx = build_index(workdir, embed).await?;
    let arc = Arc::new(idx);
    let mut cache = cache().lock().await;
    cache.insert(key, Arc::clone(&arc));
    Some(arc)
}

/// 读取源码、分块、嵌入，构建索引
async fn build_index(workdir: &Path, embed: &Arc<dyn EmbeddingService>) -> Option<CodeIndex> {
    let files = list_source_files(workdir).await;
    let mut chunks = Vec::new();
    for f in files {
        let text = match tokio::fs::read_to_string(&f).await {
            Ok(t) if !t.is_empty() => t,
            _ => continue,
        };
        let lines: Vec<&str> = text.lines().collect();
        if lines.is_empty() {
            continue;
        }
        let mut start = 0;
        while start < lines.len() {
            let end = (start + CHUNK_LINES).min(lines.len());
            let chunk_text = lines[start..end].join("\n");
            if chunk_text.trim().is_empty() {
                start += CHUNK_STEP;
                if start >= lines.len() {
                    break;
                }
                continue;
            }
            let emb = match embed.embed(&chunk_text).await {
                Some(e) => e,
                None => continue,
            };
            let norm = normalize(&emb);
            let rel = f
                .strip_prefix(workdir)
                .unwrap_or(&f)
                .to_string_lossy()
                .to_string();
            chunks.push(Chunk {
                file: rel,
                start_line: start + 1,
                end_line: end,
                text: chunk_text,
                norm,
            });
            if start + CHUNK_STEP >= lines.len() {
                break;
            }
            start += CHUNK_STEP;
        }
    }
    if chunks.is_empty() {
        return None;
    }
    Some(CodeIndex { chunks })
}

/// 对查询嵌入并与所有分块做余弦相似度排序，返回 top-k
async fn semantic_search(
    index: &CodeIndex,
    query: &str,
    embed: &Arc<dyn EmbeddingService>,
    limit: usize,
) -> Vec<Value> {
    let q = match embed.embed(query).await {
        Some(e) => normalize(&e),
        None => return vec![],
    };
    let mut scored: Vec<(f32, &Chunk)> =
        index.chunks.iter().map(|c| (dot(&q, &c.norm), c)).collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored
        .into_iter()
        .take(limit)
        .map(|(score, c)| {
            json!({
                "file": c.file,
                "start_line": c.start_line,
                "end_line": c.end_line,
                "score": score,
                "snippet": c.text,
            })
        })
        .collect()
}

/// 降级：用 ripgrep 库做正则关键词检索（跨平台，不依赖 `rg` 可执行文件）
async fn keyword_search(workdir: &Path, query: &str, limit: usize) -> Vec<Value> {
    let matcher = match RegexMatcherBuilder::new().case_smart(false).build(query) {
        Ok(m) => m,
        Err(_) => return vec![],
    };
    let mut searcher = SearcherBuilder::new().line_number(true).build();

    let mut builder = WalkBuilder::new(workdir);
    builder
        .standard_filters(true)
        .parents(true)
        .require_git(false);
    builder.filter_entry(|e| {
        if e.path().is_dir() {
            let name = e.file_name().to_string_lossy();
            return !matches!(
                name.as_ref(),
                "target" | "node_modules" | ".git" | "dist" | "build"
            );
        }
        true
    });

    let mut out: Vec<Value> = Vec::new();
    'outer: for entry in builder.build() {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let fp = entry.path();
        if !fp.is_file() || !is_source_file(fp) {
            continue;
        }
        let mut matches: Vec<(u64, String)> = Vec::new();
        let sink = UTF8Sink(|ln: u64, text: &str| {
            matches.push((ln, text.to_string()));
            Ok(true)
        });
        if searcher.search_path(&matcher, fp, sink).is_err() {
            continue;
        }
        let rel = fp
            .strip_prefix(workdir)
            .unwrap_or(fp)
            .to_string_lossy()
            .to_string();
        for (ln, content) in matches {
            out.push(json!({
                "file": rel,
                "start_line": ln,
                "end_line": ln,
                "score": 0.0,
                "snippet": content,
            }));
            if out.len() >= limit {
                break 'outer;
            }
        }
    }
    out
}

#[async_trait]
impl Capability for CodebaseSearchTool {
    fn meta(&self) -> CapabilityMeta {
        CapabilityMeta {
            name: "codebase_search".to_string(),
            description:
                "语义化检索代码库：对查询做向量相似度检索，返回最相关的代码片段（含文件路径与行号）。嵌入服务不可用时自动降级为正则关键词检索。"
                    .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "自然语言或代码片段查询，如 '解析请求参数的函数'" },
                    "path": { "type": "string", "description": "限定检索范围的子目录或文件名（可选）" },
                    "limit": { "type": "integer", "description": "返回结果条数，默认 8，最大 30" },
                    "rebuild": { "type": "boolean", "description": "是否强制重建索引（默认 false，索引按工作区进程内缓存）" }
                },
                "required": ["query"]
            }),
            category: Some(crate::symbio_core::CapabilityCategory::FileOperation),
            examples: Some(vec![
                "query='数据库连接池初始化'".to_string(),
                "query='处理 HTTP 401 的逻辑', limit=5".to_string(),
            ]),
            ..Default::default()
        }
    }

    async fn execute(&self, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload> {
        let args: Value = ctx.payload()?;
        let workdir_str = ctx.get(crate::symbio_core::WORKDIR).ok_or_else(|| {
            PluginError::ValidationError("Missing workdir in context".to_string())
        })?;
        if workdir_str.is_empty() {
            return Err(PluginError::ValidationError(
                "Empty workdir in context".to_string(),
            ));
        }
        let data = self.execute_inner(&args, &workdir_str).await?;
        Ok(PluginPayload::new(&data))
    }
}
