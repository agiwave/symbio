//! codebase_search.rs 单元测试
//!
//! 对应源文件: `codebase_search.rs`
//!
//! 重点在两件容易"看起来对、实际错"的事：
//!
//! 1. **增量判据**——什么情况下复用旧块、什么情况下必须重嵌。这里直接用构造出来的
//!    [`ScannedFile`]（指纹由测试指定）驱动 [`refresh_index`]，不依赖真实文件系统的
//!    `mtime` 粒度：在 Windows 上两次写盘可能落在同一个时间戳刻度里，
//!    用真实 `mtime` 写的测试会**偶尔**通过，那种测试比没有更糟。
//! 2. **二进制格式的读回**——索引是纯派生数据，读坏了正确的行为是"丢弃重建"，
//!    而不是 panic 或读出错位的垃圾向量。

use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

/// 计数 + 确定性输出的嵌入桩：不跑真模型，但调用次数可断言
struct CountingEmbed {
    calls: AtomicUsize,
}

impl CountingEmbed {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicUsize::new(0),
        })
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn as_service(self: &Arc<Self>) -> Arc<dyn EmbeddingService> {
        Arc::clone(self) as Arc<dyn EmbeddingService>
    }
}

#[async_trait]
impl EmbeddingService for CountingEmbed {
    async fn embed(&self, text: &str) -> Option<Vec<f32>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        // 内容决定向量：复用错了会在这里露出来
        Some(vec![text.len() as f32, text.lines().count() as f32, 1.0])
    }
}

fn fp(secs: i64, size: u64) -> Fingerprint {
    Fingerprint {
        mtime_secs: secs,
        mtime_nanos: 0,
        size,
    }
}

/// 在 `dir` 下写一个文件，并返回一个指纹由调用方指定的 [`ScannedFile`]
async fn scanned(dir: &Path, name: &str, content: &str, fingerprint: Fingerprint) -> ScannedFile {
    tokio::fs::write(dir.join(name), content)
        .await
        .expect("写测试文件");
    ScannedFile {
        abs: dir.join(name),
        rel: name.to_string(),
        fingerprint,
    }
}

// ==================== 增量判据 ====================

/// 文件集与指纹都没变 ⇒ 一个块都不重嵌（"零嵌入"快路径）
#[tokio::test]
async fn unchanged_scan_reuses_every_chunk_and_embeds_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let embed = CountingEmbed::new();

    let first = vec![
        scanned(dir.path(), "a.rs", "fn a() {}\n", fp(100, 10)).await,
        scanned(dir.path(), "b.rs", "fn b() {}\n", fp(100, 10)).await,
    ];
    let (idx1, s1) = refresh_index(&first, None, &embed.as_service())
        .await
        .unwrap();
    assert_eq!(s1.embedded_files, 2);
    assert_eq!(s1.reused_files, 0);
    assert!(idx1.matches_scan(&first), "刚建出来的索引必须与扫描一致");

    let before = embed.calls();
    let (idx2, s2) = refresh_index(&first, Some(&idx1), &embed.as_service())
        .await
        .unwrap();
    assert_eq!(embed.calls() - before, 0, "指纹未变却跑了嵌入");
    assert_eq!(s2.embedded_files, 0);
    assert_eq!(s2.reused_files, 2);
    assert_eq!(s2.chunks, s1.chunks);
    assert!(idx2.matches_scan(&first));
}

/// 只有指纹变了的那个文件重嵌，其余复用
#[tokio::test]
async fn only_the_changed_file_is_re_embedded() {
    let dir = tempfile::tempdir().unwrap();
    let embed = CountingEmbed::new();

    let old = vec![
        scanned(dir.path(), "a.rs", "fn a() {}\n", fp(100, 10)).await,
        scanned(dir.path(), "b.rs", "fn b() {}\n", fp(100, 10)).await,
        scanned(dir.path(), "c.rs", "fn c() {}\n", fp(100, 10)).await,
    ];
    let (idx1, _) = refresh_index(&old, None, &embed.as_service())
        .await
        .unwrap();
    let a_chunks_before = idx1.files[0].chunks[0].text.clone();

    // 只改 b 的指纹与内容
    let new = vec![
        scanned(dir.path(), "a.rs", "fn a() {}\n", fp(100, 10)).await,
        scanned(dir.path(), "b.rs", "fn b() { /* 改了 */ }\n", fp(200, 30)).await,
        scanned(dir.path(), "c.rs", "fn c() {}\n", fp(100, 10)).await,
    ];
    let before = embed.calls();
    let (idx2, s) = refresh_index(&new, Some(&idx1), &embed.as_service())
        .await
        .unwrap();

    assert_eq!(s.embedded_files, 1, "只应重嵌 b");
    assert_eq!(s.reused_files, 2);
    assert_eq!(embed.calls() - before, 1);
    // a 的块是原样搬过来的（文本一致即证）
    assert_eq!(idx2.files[0].chunks[0].text, a_chunks_before);
    assert!(idx2.files[1].chunks[0].text.contains("改了"));
}

/// 已删除的文件整条丢掉，不留残影
#[tokio::test]
async fn deleted_file_is_dropped_from_the_index() {
    let dir = tempfile::tempdir().unwrap();
    let embed = CountingEmbed::new();

    let old = vec![
        scanned(dir.path(), "a.rs", "fn a() {}\n", fp(100, 10)).await,
        scanned(dir.path(), "gone.rs", "fn gone() {}\n", fp(100, 15)).await,
    ];
    let (idx1, _) = refresh_index(&old, None, &embed.as_service())
        .await
        .unwrap();
    assert_eq!(idx1.files.len(), 2);

    // gone.rs 从磁盘上消失（扫描结果里不再有它）
    tokio::fs::remove_file(dir.path().join("gone.rs"))
        .await
        .unwrap();
    let new = vec![scanned(dir.path(), "a.rs", "fn a() {}\n", fp(100, 10)).await];

    let before = embed.calls();
    let (idx2, s) = refresh_index(&new, Some(&idx1), &embed.as_service())
        .await
        .unwrap();

    assert_eq!(idx2.files.len(), 1);
    assert_eq!(idx2.files[0].rel, "a.rs");
    assert_eq!(s.embedded_files, 0, "删除文件不该触发任何嵌入");
    assert_eq!(embed.calls() - before, 0);
}

/// 新增文件只嵌它自己
#[tokio::test]
async fn new_file_is_embedded_alone() {
    let dir = tempfile::tempdir().unwrap();
    let embed = CountingEmbed::new();

    let old = vec![scanned(dir.path(), "a.rs", "fn a() {}\n", fp(100, 10)).await];
    let (idx1, _) = refresh_index(&old, None, &embed.as_service())
        .await
        .unwrap();

    let new = vec![
        scanned(dir.path(), "a.rs", "fn a() {}\n", fp(100, 10)).await,
        scanned(dir.path(), "b.rs", "fn b() {}\n", fp(300, 10)).await,
    ];
    let before = embed.calls();
    let (idx2, s) = refresh_index(&new, Some(&idx1), &embed.as_service())
        .await
        .unwrap();

    assert_eq!(s.embedded_files, 1);
    assert_eq!(s.reused_files, 1);
    assert_eq!(embed.calls() - before, 1);
    assert_eq!(idx2.files.len(), 2);
}

/// `rebuild`（`previous = None`）时全量重嵌，一个都不复用
#[tokio::test]
async fn full_rebuild_ignores_previous_entirely() {
    let dir = tempfile::tempdir().unwrap();
    let embed = CountingEmbed::new();

    let scan = vec![
        scanned(dir.path(), "a.rs", "fn a() {}\n", fp(100, 10)).await,
        scanned(dir.path(), "b.rs", "fn b() {}\n", fp(100, 10)).await,
    ];
    let (idx1, _) = refresh_index(&scan, None, &embed.as_service())
        .await
        .unwrap();

    let before = embed.calls();
    let (_, s) = refresh_index(&scan, None, &embed.as_service())
        .await
        .unwrap();
    assert_eq!(s.embedded_files, 2, "previous=None 必须全量重嵌");
    assert_eq!(s.reused_files, 0);
    assert!(embed.calls() - before >= 2);
    // 顺带确认 idx1 没被用上（只是避免 unused 警告）
    assert_eq!(idx1.files.len(), 2);
}

// ==================== 二进制格式 ====================

fn sample_index() -> CodeIndex {
    CodeIndex {
        files: vec![
            IndexedFile {
                rel: "src/中文 路径.rs".to_string(), // 非 ASCII 路径要走 UTF-8 字节长度
                fingerprint: fp(1_700_000_000, 12345),
                chunks: vec![
                    Chunk {
                        start_line: 1,
                        end_line: 40,
                        text: "fn a() {}\n".repeat(3),
                        norm: vec![0.5, -0.25, 0.125],
                    },
                    Chunk {
                        start_line: 21,
                        end_line: 60,
                        text: String::new(),
                        norm: vec![1.0],
                    },
                ],
            },
            IndexedFile {
                rel: "b.rs".to_string(),
                fingerprint: fp(0, 0),
                chunks: Vec::new(),
            },
        ],
    }
}

/// 编码 → 解码必须逐字段还原（含非 ASCII 路径、空文本、空块列表）
#[test]
fn encode_decode_roundtrip_is_lossless() {
    let idx = sample_index();
    let bytes = encode_index(&idx);
    let back = decode_index(&bytes).expect("应能解回");

    assert_eq!(back.files.len(), 2);
    assert_eq!(back.files[0].rel, "src/中文 路径.rs");
    assert_eq!(back.files[0].fingerprint, fp(1_700_000_000, 12345));
    assert_eq!(back.files[0].chunks.len(), 2);
    assert_eq!(back.files[0].chunks[0].start_line, 1);
    assert_eq!(back.files[0].chunks[0].end_line, 40);
    assert_eq!(back.files[0].chunks[0].text, "fn a() {}\n".repeat(3));
    assert_eq!(back.files[0].chunks[0].norm, vec![0.5, -0.25, 0.125]);
    assert_eq!(back.files[0].chunks[1].text, "");
    assert_eq!(back.files[1].chunks.len(), 0);
    assert_eq!(back.files[1].fingerprint, fp(0, 0));

    // 再编一次应得到完全相同的字节（编码只取决于内容，不取决于内存布局）
    assert_eq!(encode_index(&back), bytes);
}

/// 魔数不符 ⇒ 丢弃（而不是继续按新格式解）
#[test]
fn decode_rejects_wrong_magic() {
    let mut bytes = encode_index(&sample_index());
    bytes[0] = b'X';
    assert!(decode_index(&bytes).is_none());
}

/// 版本不符 ⇒ 丢弃重建。这条是防"字段增删后按旧布局读出错位垃圾"的唯一屏障。
#[test]
fn decode_rejects_version_mismatch() {
    let mut bytes = encode_index(&sample_index());
    // 版本在 magic 之后，小端 u32
    bytes[8] = (INDEX_FORMAT_VERSION + 1) as u8;
    assert!(decode_index(&bytes).is_none());
}

/// 截断的索引不能 panic，只能返回 None
#[test]
fn decode_rejects_truncated_input_without_panicking() {
    let bytes = encode_index(&sample_index());
    for cut in [0, 1, 7, 8, 11, 12, 20, bytes.len() / 2, bytes.len() - 1] {
        assert!(
            decode_index(&bytes[..cut]).is_none(),
            "截到 {cut} 字节应返回 None"
        );
    }
    // 完整的不该被误判
    assert!(decode_index(&bytes).is_some());
}

/// 长度字段被改成天文数字（损坏 / 恶意）也不能 panic、不能巨额分配
#[test]
fn decode_rejects_absurd_length_prefix() {
    let mut bytes = encode_index(&sample_index());
    // file_count 在 magic(8) + version(4) 之后
    bytes[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(decode_index(&bytes).is_none());
}

// ==================== 落盘与读回 ====================

/// 落盘 → 读回，内容一致；且落盘是"临时文件 + rename"，不留 `.tmp` 残骸
#[tokio::test]
async fn index_round_trips_through_disk() {
    let dir = tempfile::tempdir().unwrap();
    let idx = sample_index();

    save_to_disk(dir.path(), &idx).await.expect("落盘");
    let path = index_path(dir.path());
    assert!(path.exists(), "索引文件应存在于 {}", path.display());

    let back = load_from_disk(dir.path()).await.expect("读回");
    assert_eq!(back.files.len(), 2);
    assert_eq!(back.files[0].chunks[0].norm, vec![0.5, -0.25, 0.125]);

    assert!(
        !path.with_extension("bin.tmp").exists(),
        "rename 之后不该留下临时文件"
    );
}

/// 索引文件不存在 ⇒ None（不是错误，只是"还没有索引"）
#[tokio::test]
async fn missing_index_file_is_not_an_error() {
    let dir = tempfile::tempdir().unwrap();
    assert!(load_from_disk(dir.path()).await.is_none());
}

/// 磁盘上的索引损坏 ⇒ None，让上层重建
#[tokio::test]
async fn corrupt_index_file_loads_as_none() {
    let dir = tempfile::tempdir().unwrap();
    let path = index_path(dir.path());
    tokio::fs::create_dir_all(path.parent().unwrap())
        .await
        .unwrap();
    tokio::fs::write(&path, b"not an index at all")
        .await
        .unwrap();
    assert!(load_from_disk(dir.path()).await.is_none());
}

/// 落盘路径必须是**工作区内**的 `.symbio/cache/`——这是"工作区级数据落在工作区里"
/// 那条约定，也是"索引不会把自己索引进去"（`.symbio` 是隐藏目录，会被 WalkBuilder 跳过）的前提
#[test]
fn index_lives_under_the_workspace_dot_symbio() {
    let p = index_path(Path::new("/tmp/some-workspace"));
    assert!(p.ends_with(
        INDEX_REL_PATH
            .replace('/', std::path::MAIN_SEPARATOR_STR)
            .as_str()
    ));
    assert!(p.to_string_lossy().contains(".symbio"));
    // `.bin` 不在 SOURCE_EXTS 里 —— 第二重保险
    assert!(!SOURCE_EXTS.contains(&"bin"));
}

/// 从盘上读来的索引**必须进进程内缓存**。
///
/// 否则每次调用都会 cache-miss → 重读并解码整个索引文件（本仓 25 MB），
/// 一个不报错、只让每次检索都慢一截的漏洞。
///
/// 判据：把盘上的索引删掉之后再调一次——若快路径没进缓存，这次会 cache-miss 且
/// 读不到盘，于是退化成全量重建（`embedded_files > 0`）；进了缓存则零嵌入。
#[tokio::test]
async fn disk_loaded_index_is_cached_for_subsequent_calls() {
    let dir = tempfile::tempdir().unwrap();
    let embed = CountingEmbed::new();
    let key = dir.path().to_string_lossy().to_string();

    tokio::fs::write(
        dir.path().join("a.rs"),
        "fn a() {}
",
    )
    .await
    .unwrap();

    // 建一次（rebuild 绕过任何既有状态），落盘 + 进缓存
    let (built, s1) = get_index(dir.path(), true, &embed.as_service())
        .await
        .expect("首次构建");
    let chunks = built.chunk_count();
    assert_eq!(s1.embedded_files, 1);
    assert!(index_path(dir.path()).exists(), "应已落盘");

    // 模拟"进程重启"：清掉进程内缓存，只留盘上的索引
    cache().lock().await.remove(&key);

    let before = embed.calls();
    let (from_disk, s2) = get_index(dir.path(), false, &embed.as_service())
        .await
        .expect("读盘");
    assert_eq!(s2.embedded_files, 0, "指纹未变，不该重嵌");
    assert_eq!(from_disk.chunk_count(), chunks);
    assert_eq!(embed.calls() - before, 0);

    // 关键一步：把盘上的索引删掉。此时若上一次没进缓存，就会 cache-miss + 无盘可读
    // → 全量重建。这正是要防的那条路径。
    tokio::fs::remove_file(index_path(dir.path()))
        .await
        .unwrap();

    let before = embed.calls();
    let (again, s3) = get_index(dir.path(), false, &embed.as_service())
        .await
        .expect("第三次");
    assert_eq!(
        s3.embedded_files, 0,
        "盘上索引已删，却仍在重嵌 ⇒ 上次读盘的结果没进进程内缓存"
    );
    assert_eq!(embed.calls() - before, 0);
    assert_eq!(again.chunk_count(), chunks);
}

// ==================== 生成物闸门 ====================

/// 生成物不进索引：锁文件 / 压缩产物按名字挡掉
#[test]
fn generated_files_are_not_indexed() {
    assert!(!is_source_file(Path::new("tauri/package-lock.json")));
    assert!(!is_source_file(Path::new("symbio/Cargo.lock")));
    assert!(!is_source_file(Path::new("a/yarn.lock")));
    assert!(!is_source_file(Path::new("dist/app.min.js")));
    assert!(!is_source_file(Path::new("dist/app.min.css")));
    assert!(!is_source_file(Path::new("a/b.exe"))); // 不在扩展名白名单

    // 正常文件不受影响 —— 别把闸门开成"只留 .rs"
    assert!(is_source_file(Path::new("symbio/src/lib.rs")));
    assert!(is_source_file(Path::new("tauri/src/stores/sessions.ts")));
    assert!(is_source_file(Path::new("docs/CHANGELOG.md")));
    assert!(is_source_file(Path::new("tauri/package.json"))); // 非 lock 的 json 仍要
    assert!(is_source_file(Path::new("examples/x.yaml")));
}

/// 超长文件（生成物）不产生任何块，也不跑嵌入
#[tokio::test]
async fn overlong_file_produces_no_chunks() {
    let dir = tempfile::tempdir().unwrap();
    let embed = CountingEmbed::new();

    let over = "x\n".repeat(MAX_INDEXED_LINES + 1);
    tokio::fs::write(dir.path().join("vocab.json"), &over)
        .await
        .unwrap();
    let chunks = embed_file(&dir.path().join("vocab.json"), &embed.as_service()).await;
    assert!(chunks.is_empty(), "超长文件不该产出块");
    assert_eq!(embed.calls(), 0, "不该为超长文件跑任何嵌入");

    // 恰好在上限内照常索引（阈值是 `>` 而不是 `>=`）
    let ok = "x\n".repeat(MAX_INDEXED_LINES);
    tokio::fs::write(dir.path().join("ok.json"), &ok)
        .await
        .unwrap();
    let chunks = embed_file(&dir.path().join("ok.json"), &embed.as_service()).await;
    assert!(!chunks.is_empty(), "上限内的文件应照常索引");
}
