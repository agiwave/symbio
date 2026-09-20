//! 语义代码检索工具（对应 Trae 的 SearchCodebase）
//!
//! 复用 `EmbeddingService`（local 本地模型，ONNX Runtime via `ort`）对源码分块嵌入，
//! 对查询做近似最近邻（余弦相似度）检索。
//! 若嵌入服务不可用，自动降级为正则关键词检索（ripgrep 库，非外部可执行文件）。
//!
//! ## 索引的时效性
//!
//! 索引是**派生数据**，必须随时跟得上磁盘。三条机制叠在一起：
//!
//! 1. **每次调用都做一次文件集指纹扫描**（`stat` 全部候选文件，几百个文件 ≈ 毫秒级），
//!    与上次索引比对；文件集（相对路径 + `mtime` + 字节数）完全一致就直接复用，
//!    一次嵌入都不做。
//! 2. **有差异时只重嵌改动过的文件**：未变文件的块原样搬过来，新增/改动的文件重新
//!    分块嵌入，已删除的文件整条丢掉。
//! 3. **索引落盘**（`{workdir}/.symbio/cache/codebase-index.bin`）：进程重启后先读盘，
//!    再走上面第 1、2 步，因此冷启动也不必全量重嵌。
//!
//! 之所以敢把"每次调用都扫描"当默认行为：`ort` 下单块嵌入 ≈ 10 ms（`tract` 时代是
//! ≈ 10 s），全量重建本仓 ≈ 23 s，而扫描只是几百次 `stat`。反过来，**静默的过期索引**
//! 比慢更糟——agent 会拿一份不包含自己刚写的代码的索引去搜，且完全无从察觉。
//!
//! 已知边界：指纹用 `mtime` + 字节数，**刻意保留 `mtime` 的写入（如 `rsync -t`）检测不到**。
//! 这是 `mtime` 型增量重建的固有取舍；要绝对可靠就让 LLM 传 `rebuild=true`。
//!
//! ## 为什么索引不放进 `homedir`
//!
//! 索引是**工作区级**的（一个工作区一份），与 `plugins/agent/host/store.rs` 里
//! `{workdir}/.symbio/agent/<id>` 同源——工作区级数据就落在工作区里。放进 homedir
//! 反而要按工作区路径做哈希分片，多一层可能撞键的间接。`.symbio/` 以 `.` 开头，
//! `WalkBuilder` 的 `standard_filters` 会跳过它，`.bin` 也不在 `SOURCE_EXTS` 里，
//! 因此**索引不会把自己索引进去**（两重保险）。

use super::policy::SecurityPolicy;
use crate::symbio_core::providers::EmbeddingService;
use crate::symbio_core::{
    create_object, Capability, CapabilityMeta, InvokeRequest, InvokeRequestExt, InvokeResponse,
    PluginError, PluginPayload, SimpleRequest, EMBEDDING_LOCAL,
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

/// 索引文件相对工作区的路径。与 `agent` 的工作区级落位（`{workdir}/.symbio/agent/`）
/// 同一层约定：**工作区级数据落在工作区里**。
const INDEX_REL_PATH: &str = ".symbio/cache/codebase-index.bin";

/// 索引文件魔数（含一个 0 字节，便于 `file` 之类的工具识别为二进制）
const INDEX_MAGIC: &[u8; 8] = b"SYMCIDX\0";

/// 索引格式版本。
///
/// **任何字段增删都必须 +1**。版本不符时直接丢弃、全量重建——索引是纯派生数据，
/// 重建代价是秒级，不值得为它写迁移代码；而容忍旧版本读出来的**错位数据**
/// （字段错位不会报错，只会静默读出垃圾向量）是这里唯一不能犯的错。
const INDEX_FORMAT_VERSION: u32 = 1;

/// 纳入索引的源码扩展名
const SOURCE_EXTS: &[&str] = &[
    "rs", "py", "js", "ts", "tsx", "jsx", "go", "java", "c", "cpp", "h", "hpp", "cs", "rb", "php",
    "swift", "kt", "md", "toml", "json", "yml", "yaml", "sql", "html", "css", "sh", "vue",
    "svelte", "txt", "lua", "r", "scala", "dart",
];

/// 单文件行数上限：超过这个长度的一律视为**生成物**（锁文件、词表、schema 转储），
/// 不纳入索引。
///
/// 阈值 5000 来自本仓的实测空档：最大的正常源码是 1640 行的 `vdfs_provider.rs`，
/// 而 `tokenizer.json` 是 21,277 行、`package-lock.json` 是 7,249 行——两者之间
/// 隔着数量级，5000 行能干净地把"人写的代码"和"工具吐的数据"分开。
///
/// 索引这类文件不只是浪费（实测 `tokenizer.json` + `package-lock.json` 两个文件
/// 就占掉全库 8104 块里的 1426 块、约 18%）：它们还会**挤占 top-k**——
/// 词表里全是短 token，几乎对任何查询都有中等相似度，把真正相关的代码顶出去。
const MAX_INDEXED_LINES: usize = 5000;

/// 按文件名排除的生成物（精确匹配，零误伤）
///
/// 放在遍历阶段（不读文件内容），因此比行数上限便宜。两类：
/// 锁文件（`package-lock.json` / `Cargo.lock` / `*.lock`）与压缩产物
/// （`*.min.js` / `*.min.css`）——它们的"语义"是依赖图或机器码，不是代码。
fn is_generated_name(name: &str) -> bool {
    name.ends_with("-lock.json")
        || name == "Cargo.lock"
        || name.ends_with(".lock")
        || name.ends_with(".min.js")
        || name.ends_with(".min.css")
}

/// 一个被嵌入的源码分块
///
/// 不带文件名——它属于某个 [`IndexedFile`]，每块各存一份完整路径会让索引里
/// 多出几百份重复字符串。
struct Chunk {
    start_line: usize,
    end_line: usize,
    text: String,
    /// 归一化后的嵌入向量，用于余弦相似度（点积）
    norm: Vec<f32>,
}

/// 文件指纹：判断"这个文件自上次索引以来动过没有"的依据
///
/// `mtime` + 字节数。加字节数是为了兜住"同一秒内改了长度"这类 `mtime` 分辨率问题
/// （Windows NTFS 的 `mtime` 是 100ns 粒度，但很多编辑器/归档工具只写到秒）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Fingerprint {
    mtime_secs: i64,
    mtime_nanos: u32,
    size: u64,
}

impl Fingerprint {
    fn of(meta: &std::fs::Metadata) -> Self {
        let (mtime_secs, mtime_nanos) = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| (d.as_secs() as i64, d.subsec_nanos()))
            .unwrap_or((0, 0));
        Self {
            mtime_secs,
            mtime_nanos,
            size: meta.len(),
        }
    }
}

/// 一个被索引的文件：指纹 + 它的全部块
struct IndexedFile {
    /// 相对工作区的路径（用 `/` 分隔，跨平台一致）
    rel: String,
    fingerprint: Fingerprint,
    chunks: Vec<Chunk>,
}

/// 一个工作区的代码索引
struct CodeIndex {
    files: Vec<IndexedFile>,
}

impl CodeIndex {
    fn chunk_count(&self) -> usize {
        self.files.iter().map(|f| f.chunks.len()).sum()
    }

    /// 文件集是否与本次扫描结果**完全一致**（相对路径 + 指纹逐项相同）
    ///
    /// 这是"零嵌入"快路径的判据。顺序无关——两边都按 `rel` 排序后再比。
    fn matches_scan(&self, scanned: &[ScannedFile]) -> bool {
        self.files.len() == scanned.len()
            && self
                .files
                .iter()
                .zip(scanned)
                .all(|(a, b)| a.rel == b.rel && a.fingerprint == b.fingerprint)
    }
}

/// 扫描到的一个候选文件（尚未分块/嵌入）
struct ScannedFile {
    abs: PathBuf,
    rel: String,
    fingerprint: Fingerprint,
}

/// 本次刷新索引的统计（用于把"索引有没有跟上"变成可观测的输出）
#[derive(Default)]
struct IndexStats {
    files: usize,
    chunks: usize,
    /// 指纹未变、直接复用旧块的文件数
    reused_files: usize,
    /// 本次真正跑过嵌入的文件数（新增 + 改动）
    embedded_files: usize,
}

/// 按工作区路径缓存索引（进程内）
///
/// 注意：**这个缓存不是时效性的来源**。命中它之后仍要走一次文件指纹扫描，
/// 确认磁盘没变才返回——否则进程内缓存会退化成"首次调用那一刻的快照"，
/// 也就是这套机制要修掉的那个 bug。
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
        let embed = create_object::<dyn EmbeddingService>(EMBEDDING_LOCAL, ctx);

        let mut results: Vec<Value> = Vec::new();
        let mut mode = "keyword_fallback";
        let mut index_info = Value::Null;

        if let Some(embed) = embed.as_ref() {
            if let Some((index, stats)) = get_index(&workspace_dir, rebuild, embed).await {
                // 索引统计放进返回体：让"索引是否已跟上磁盘"可见，而不是靠猜。
                // `embedded_files == 0` 就意味着本次一次嵌入都没跑（索引已是最新）。
                index_info = json!({
                    "files": stats.files,
                    "chunks": stats.chunks,
                    "reused_files": stats.reused_files,
                    "embedded_files": stats.embedded_files,
                });
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
            "index": index_info,
            "message": format!("以 {} 模式返回 {} 条结果。", mode, results.len()),
        }))
    }
}

fn is_source_file(p: &Path) -> bool {
    let ok_ext = p
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| SOURCE_EXTS.contains(&e))
        .unwrap_or(false);
    if !ok_ext {
        return false;
    }
    !p.file_name()
        .and_then(|n| n.to_str())
        .map(is_generated_name)
        .unwrap_or(false)
}

/// 列出工作区内的源码文件：用 `ignore`（尊重 .gitignore）遍历，并按扩展名白名单过滤。
/// 不依赖 `rg` 等外部可执行文件，跨平台行为一致。
fn walk_source_files(workdir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut builder = WalkBuilder::new(workdir);
    builder
        .standard_filters(true) // 尊重 .gitignore / 隐藏文件 / 全局 ignore
        .parents(true)
        .require_git(false);
    // 跳过已知的非源码大目录（target / node_modules / dist / build 等）
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

/// 扫描工作区：拿到候选文件 + 各自的指纹，并按相对路径排序
///
/// 排序是**必须**的：增量比对的快路径要求两侧顺序一致。顺带让索引文件的字节内容
/// 只取决于内容（不取决于遍历顺序），同一份代码重复构建得到同一个文件。
async fn scan_files(workdir: &Path) -> Vec<ScannedFile> {
    let mut out: Vec<ScannedFile> = Vec::new();
    for abs in walk_source_files(workdir) {
        let Ok(meta) = tokio::fs::metadata(&abs).await else {
            continue;
        };
        let rel = abs
            .strip_prefix(workdir)
            .unwrap_or(&abs)
            .to_string_lossy()
            .replace('\\', "/");
        out.push(ScannedFile {
            abs,
            rel,
            fingerprint: Fingerprint::of(&meta),
        });
    }
    out.sort_by(|a, b| a.rel.cmp(&b.rel));
    out
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

/// 把一个文件切成块并逐块嵌入。文本读不出来 / 全为空 ⇒ 空 `Vec`（该文件不进索引）
async fn embed_file(path: &Path, embed: &Arc<dyn EmbeddingService>) -> Vec<Chunk> {
    let text = match tokio::fs::read_to_string(path).await {
        Ok(t) if !t.is_empty() => t,
        _ => return Vec::new(),
    };
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return Vec::new();
    }
    // 生成物闸门。放在这里而不是遍历阶段：行数只有读了内容才知道，
    // 而遍历阶段要尽量便宜（几百次 stat 而不是几百次读）。
    // 这类文件留在索引里是**零块**——它仍被指纹跟踪，改了会被重新判定。
    if lines.len() > MAX_INDEXED_LINES {
        tracing::debug!(
            "跳过过长文件（{} 行 > {} 行上限，按生成物处理）：{}",
            lines.len(),
            MAX_INDEXED_LINES,
            path.display()
        );
        return Vec::new();
    }
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < lines.len() {
        let end = (start + CHUNK_LINES).min(lines.len());
        let chunk_text = lines[start..end].join("\n");
        if !chunk_text.trim().is_empty() {
            if let Some(emb) = embed.embed(&chunk_text).await {
                chunks.push(Chunk {
                    start_line: start + 1,
                    end_line: end,
                    text: chunk_text,
                    norm: normalize(&emb),
                });
            }
        }
        if start + CHUNK_STEP >= lines.len() {
            break;
        }
        start += CHUNK_STEP;
    }
    chunks
}

/// 由扫描结果 + 上次索引构建出新的索引，**只重嵌新增/改动的文件**。
///
/// `scanned` 已按 `rel` 排序（[`scan_files`] 保证）；`previous` 为 `None` 时全量重建。
///
/// 返回 `None` 当且仅当扫描结果为空（调用方会退化成关键词检索）。
async fn refresh_index(
    scanned: &[ScannedFile],
    previous: Option<&CodeIndex>,
    embed: &Arc<dyn EmbeddingService>,
) -> Option<(CodeIndex, IndexStats)> {
    if scanned.is_empty() {
        return None;
    }

    // 上一版按 `rel` 建查找表。这里不做"归并"式的花活：HashMap 一次建表
    // 比逐项归并更好读，而几百个文件的建表开销相对嵌入可以忽略。
    let mut old: HashMap<&str, &IndexedFile> = HashMap::new();
    if let Some(prev) = previous {
        for f in &prev.files {
            old.insert(f.rel.as_str(), f);
        }
    }

    let mut files: Vec<IndexedFile> = Vec::with_capacity(scanned.len());
    let mut stats = IndexStats::default();
    for s in scanned {
        let reuse = old
            .get(s.rel.as_str())
            .filter(|f| f.fingerprint == s.fingerprint);
        let (chunks, reused) = match reuse {
            Some(f) => (clone_chunks(&f.chunks), true),
            None => (embed_file(&s.abs, embed).await, false),
        };
        if reused {
            stats.reused_files += 1;
        } else {
            stats.embedded_files += 1;
        }
        stats.chunks += chunks.len();
        files.push(IndexedFile {
            rel: s.rel.clone(),
            fingerprint: s.fingerprint,
            chunks,
        });
    }
    stats.files = files.len();
    Some((CodeIndex { files }, stats))
}

/// 复用旧块（克隆而非移动：旧索引仍被进程内缓存 / `Arc` 持有）
fn clone_chunks(chunks: &[Chunk]) -> Vec<Chunk> {
    chunks
        .iter()
        .map(|c| Chunk {
            start_line: c.start_line,
            end_line: c.end_line,
            text: c.text.clone(),
            norm: c.norm.clone(),
        })
        .collect()
}

/// 取（或刷新）某工作区的代码索引
///
/// 每一次调用都会走一遍「取上次索引 → 扫描指纹 → 有差异才重嵌」，
/// 因此返回的索引**总是当前磁盘内容的视图**（除非写盘方刻意保留了 `mtime`，
/// 见模块文档；那种情况用 `rebuild` 兜底）。
async fn get_index(
    workdir: &Path,
    rebuild: bool,
    embed: &Arc<dyn EmbeddingService>,
) -> Option<(Arc<CodeIndex>, IndexStats)> {
    let key = workdir.to_string_lossy().to_string();

    // 上次的索引：`rebuild` 时一律当作没有（全量重嵌）。
    // 进程内缓存优先，没有再读盘——读盘是为了跨进程重启也能省掉全量重建。
    let previous: Option<Arc<CodeIndex>> = if rebuild {
        None
    } else {
        let cached = { cache().lock().await.get(&key).cloned() };
        match cached {
            Some(idx) => Some(idx),
            None => load_from_disk(workdir).await.map(Arc::new),
        }
    };

    // 扫描放在这里（而不是 `refresh_index` 里），是为了让"零嵌入"快路径也能
    // 直接把上一次的 `Arc` 还回去——不必克隆整个索引。
    let scanned = scan_files(workdir).await;
    if scanned.is_empty() {
        return None;
    }

    // 快路径：文件集与指纹逐项相同 ⇒ 一个块都不用重嵌，索引原样复用。
    //
    // 这里**也必须进缓存**：`previous` 可能是刚从盘上读出来的（进程内缓存未命中），
    // 那份索引不在缓存里。直接 `return` 会让下一次调用再次 cache-miss，
    // 于是**每次调用都重读并解码整个索引文件**（本仓 25 MB）——一个不会报错、
    // 只会让每次检索都慢上几十毫秒的漏洞。
    if let Some(prev) = &previous {
        if prev.matches_scan(&scanned) {
            cache().lock().await.insert(key, Arc::clone(prev));
            let stats = IndexStats {
                files: prev.files.len(),
                chunks: prev.chunk_count(),
                reused_files: prev.files.len(),
                embedded_files: 0,
            };
            return Some((Arc::clone(prev), stats));
        }
    }

    let (idx, stats) = refresh_index(&scanned, previous.as_deref(), embed).await?;
    let arc = Arc::new(idx);

    // 落盘与进缓存。落盘失败不影响本次结果（下次照旧走全量），但要说出来——
    // 否则"索引没落盘"会表现为每次冷启动都慢，且没有任何线索。
    if let Err(e) = save_to_disk(workdir, &arc).await {
        tracing::warn!("代码索引落盘失败（不影响本次检索）：{e}");
    }
    cache().lock().await.insert(key, Arc::clone(&arc));
    Some((arc, stats))
}

// ==================== 索引的二进制格式 ====================
//
// 手写小端格式，不引序列化库：
// - 索引是纯派生数据，格式只需自己读得回来，没有跨版本兼容诉求；
// - 512 维 f32 用 JSON 存会膨胀数倍（每维 ~12 字节文本 vs 4 字节二进制）；
// - 本仓对"为一件小事拉一整条依赖链"一贯保守（zip 只开 deflate 是同一个理由）。
//
// 布局：
//   magic[8] | version:u32 | file_count:u32
//   每文件： rel:str | mtime_secs:i64 | mtime_nanos:u32 | size:u64 | chunk_count:u32
//   每块：   start_line:u32 | end_line:u32 | text:str | dim:u32 | f32*dim
// 其中 str = len:u32 + utf8 字节

/// 越界一律返回 `None` 的读取器：截断或损坏的索引文件不该让进程 panic，
/// 只该让它被丢弃重建。
struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        let s = self.buf.get(self.pos..end)?;
        self.pos = end;
        Some(s)
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }

    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }

    fn i64(&mut self) -> Option<i64> {
        Some(i64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }

    fn string(&mut self) -> Option<String> {
        let n = self.u32()? as usize;
        String::from_utf8(self.take(n)?.to_vec()).ok()
    }

    fn vec_f32(&mut self) -> Option<Vec<f32>> {
        let dim = self.u32()? as usize;
        let bytes = self.take(dim.checked_mul(4)?)?;
        Some(
            bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|c| f32::from_le_bytes(*c))
                .collect(),
        )
    }
}

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_u64(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_i64(out: &mut Vec<u8>, v: i64) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    put_u32(out, s.len() as u32);
    out.extend_from_slice(s.as_bytes());
}

fn put_vec_f32(out: &mut Vec<u8>, v: &[f32]) {
    put_u32(out, v.len() as u32);
    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
}

fn encode_index(idx: &CodeIndex) -> Vec<u8> {
    let mut out = Vec::with_capacity(64 * 1024);
    out.extend_from_slice(INDEX_MAGIC);
    put_u32(&mut out, INDEX_FORMAT_VERSION);
    put_u32(&mut out, idx.files.len() as u32);
    for f in &idx.files {
        put_str(&mut out, &f.rel);
        put_i64(&mut out, f.fingerprint.mtime_secs);
        put_u32(&mut out, f.fingerprint.mtime_nanos);
        put_u64(&mut out, f.fingerprint.size);
        put_u32(&mut out, f.chunks.len() as u32);
        for c in &f.chunks {
            put_u32(&mut out, c.start_line as u32);
            put_u32(&mut out, c.end_line as u32);
            put_str(&mut out, &c.text);
            put_vec_f32(&mut out, &c.norm);
        }
    }
    out
}

fn decode_index(buf: &[u8]) -> Option<CodeIndex> {
    let mut r = Reader::new(buf);
    if r.take(INDEX_MAGIC.len())? != INDEX_MAGIC.as_slice() {
        return None;
    }
    if r.u32()? != INDEX_FORMAT_VERSION {
        return None;
    }
    let file_count = r.u32()? as usize;
    let mut files = Vec::with_capacity(file_count.min(4096));
    for _ in 0..file_count {
        let rel = r.string()?;
        let fingerprint = Fingerprint {
            mtime_secs: r.i64()?,
            mtime_nanos: r.u32()?,
            size: r.u64()?,
        };
        let chunk_count = r.u32()? as usize;
        let mut chunks = Vec::with_capacity(chunk_count.min(4096));
        for _ in 0..chunk_count {
            let start_line = r.u32()? as usize;
            let end_line = r.u32()? as usize;
            let text = r.string()?;
            let norm = r.vec_f32()?;
            chunks.push(Chunk {
                start_line,
                end_line,
                text,
                norm,
            });
        }
        files.push(IndexedFile {
            rel,
            fingerprint,
            chunks,
        });
    }
    Some(CodeIndex { files })
}

fn index_path(workdir: &Path) -> PathBuf {
    workdir.join(INDEX_REL_PATH)
}

/// 从盘上读索引。任何异常（不存在 / 截断 / 版本不符）都只是"没有旧索引"，返回 `None`。
async fn load_from_disk(workdir: &Path) -> Option<CodeIndex> {
    let path = index_path(workdir);
    let buf = tokio::fs::read(&path).await.ok()?;
    match decode_index(&buf) {
        Some(idx) => {
            tracing::debug!(
                "代码索引已从磁盘加载：{} 文件 / {} 块（{}）",
                idx.files.len(),
                idx.chunk_count(),
                path.display()
            );
            Some(idx)
        }
        None => {
            tracing::info!("磁盘上的代码索引无法解析（版本不符或已损坏），将重建");
            None
        }
    }
}

/// 把索引写到盘上。
///
/// **先写临时文件再 `rename`**：`rename` 在同一卷上是原子的，因此读方看到的
/// 要么是完整的新索引、要么是完整的旧索引，不会读到写了一半的文件
/// （那种文件头是好的、块是缺的，靠魔数/版本根本发现不了）。
async fn save_to_disk(workdir: &Path, idx: &CodeIndex) -> std::io::Result<()> {
    let path = index_path(workdir);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let bytes = encode_index(idx);
    let tmp = path.with_extension("bin.tmp");
    tokio::fs::write(&tmp, &bytes).await?;
    tokio::fs::rename(&tmp, &path).await
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
    let mut scored: Vec<(f32, &str, &Chunk)> = index
        .files
        .iter()
        .flat_map(|f| f.chunks.iter().map(move |c| (f.rel.as_str(), c)))
        .map(|(rel, c)| (dot(&q, &c.norm), rel, c))
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored
        .into_iter()
        .take(limit)
        .map(|(score, rel, c)| {
            json!({
                "file": rel,
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
                "语义化检索代码库：对查询做向量相似度检索，返回最相关的代码片段（含文件路径与行号）。索引会按文件 mtime 自动增量更新（每次调用都校验，未改动的文件不重嵌），跨进程重启复用磁盘上的索引。嵌入服务不可用时自动降级为正则关键词检索。"
                    .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "自然语言或代码片段查询，如 '解析请求参数的函数'" },
                    "path": { "type": "string", "description": "限定检索范围的子目录或文件名（可选）" },
                    "limit": { "type": "integer", "description": "返回结果条数，默认 8，最大 30" },
                    "rebuild": { "type": "boolean", "description": "忽略现有索引、全量重建（默认 false）。正常情况不需要——索引每次调用都会按 mtime 增量同步；只在怀疑索引与磁盘不一致时使用" }
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

#[cfg(test)]
#[path = "codebase_search.test.rs"]
mod tests;
