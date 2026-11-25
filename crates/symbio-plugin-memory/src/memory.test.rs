//! `memory/memory.rs` 的单元测试 —— 智能体自身 `MEMORY.md` 的**落位、片段与节点**。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。
//!
//! 共享实现的机制（读写 / 两道闸门 / 排版形状）由 `memory_file` 自己测；
//! 这里钉的是**本层的个性**：目录从哪来、地址怎么拼出来、空文件怎么降级。

use super::*;
use crate::memory_file::fixture;
use crate::memory_file::MemoryFile;
use std::path::{Path, PathBuf};
use symbio_plugin_api::VdfsAccess;

/// 宿主目录 = 本插件目录的父目录（容器的装配规则，不需要额外上下文键）
///
/// 落位规则属于本层测试；生产路径由 provider 持有根，插件不再拼物理路径。
fn host_dir(plugin_dir: &Path) -> PathBuf {
    plugin_dir
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| plugin_dir.to_path_buf())
}

/// 记忆文件的落位：`{宿主目录}/MEMORY.md`
fn file_path(host_dir: &Path) -> PathBuf {
    host_dir.join(MEMORY_FILE)
}

/// 测试夹具：保留「宿主目录 → 物理落位」的落位规则用例
fn store_at(host_dir: &Path, write_max_bytes: usize, inject_max_bytes: usize) -> MemoryFile {
    fixture::at(Some(file_path(host_dir)), write_max_bytes, inject_max_bytes)
}

/// 宿主目录 = 本插件目录的父目录（容器的装配规则，不需要额外上下文键）
///
/// 这条规则同时给出两个作用域：系统树实例落在 `<homedir>`，子树实例落在
/// `<agentdir>`——**同一个实现，不是两份**。
#[tokio::test]
async fn host_dir_is_the_parent_of_the_plugin_dir() {
    let system = Path::new("/homedir").join(PLUGIN_ID_MEMORY);
    assert_eq!(host_dir(&system), Path::new("/homedir"));

    let sub = Path::new("/homedir/agent/reviewer").join(PLUGIN_ID_MEMORY);
    assert_eq!(host_dir(&sub), Path::new("/homedir/agent/reviewer"));

    assert_eq!(
        file_path(&host_dir(&system)),
        Path::new("/homedir").join(MEMORY_FILE)
    );
}

/// 落位就是那个文件：读写走共享实现，两道闸门生效
#[tokio::test]
async fn memory_roundtrips_through_the_shared_impl() {
    let tmp = tempfile::TempDir::new().unwrap();
    let host = tmp.path().join("homedir");
    let m = store_at(&host, 1024, 128);

    // 还没写过 → 空串（不是错误）
    assert_eq!(m.read().await.unwrap(), "");
    assert!(
        m.has_scope(),
        "本层恒有作用域（目录由装配位置决定，不依赖任何选择）"
    );

    m.write("只改必要之处").await.unwrap();
    assert_eq!(m.read().await.unwrap(), "只改必要之处");
    assert_eq!(
        std::fs::read_to_string(file_path(&host)).unwrap(),
        "只改必要之处",
        "落位必须与片段里印的地址同源"
    );

    // 写入闸门在共享实现里（本插件不重复实现）
    let err = m.write(&"x".repeat(2048)).await.unwrap_err();
    assert!(err.contains("超出容量上限"), "{err}");
}

/// 空文件 / 不存在 → **整段省略**（不产出空标题，不往收集期错误桶里塞东西）
#[tokio::test]
async fn empty_memory_is_not_injected() {
    let tmp = tempfile::TempDir::new().unwrap();
    let m = store_at(tmp.path(), 1024, 128);

    assert_eq!(
        segment(&m, "@vfs/memory/MEMORY.md").await.unwrap(),
        None,
        "没写过 → 不注入"
    );

    std::fs::write(file_path(tmp.path()), "   \n\t ").unwrap();
    assert_eq!(
        segment(&m, "@vfs/memory/MEMORY.md").await.unwrap(),
        None,
        "只有空白 → 同样不注入"
    );
}

/// 有内容 → 共享实现排版：标题 + **真实地址** + 上限 + 与本层的关系说明
#[tokio::test]
async fn segment_carries_title_address_and_gates() {
    let tmp = tempfile::TempDir::new().unwrap();
    let m = store_at(tmp.path(), 1024, 128);
    m.write("只改必要之处").await.unwrap();

    let seg = segment(&m, "@vfs/memory/MEMORY.md")
        .await
        .unwrap()
        .expect("有内容必注入");
    assert!(seg.contains("【智能体记忆】"), "{seg}");
    assert!(
        seg.contains("@vfs/memory/MEMORY.md"),
        "地址必须真实可达：{seg}"
    );
    assert!(
        seg.contains("与【工作区记忆】相互独立"),
        "同名不同作用域，必须点明：{seg}"
    );
    assert!(seg.contains("上限：1024字节"), "{seg}");
    assert!(seg.contains("只改必要之处"), "{seg}");
}

/// 注入超预算 → 截断并在片段里指路（截断口径取自共享实现，本层不另写一份）
#[tokio::test]
async fn segment_truncates_over_the_inject_budget() {
    let tmp = tempfile::TempDir::new().unwrap();
    let m = store_at(tmp.path(), 4096, 4);
    m.write("0123456789").await.unwrap();

    let seg = segment(&m, "@vfs/memory/MEMORY.md").await.unwrap().unwrap();
    assert!(seg.contains("已截断至 4 字节"), "{seg}");
    assert!(seg.contains("vdfs_read"), "截断必须指路取全文：{seg}");
}

/// 节点形状由共享实现决定 —— 与另外几层同源，不在这里手搓一份
#[tokio::test]
async fn node_shape_comes_from_the_shared_impl() {
    let tmp = tempfile::TempDir::new().unwrap();
    let m = store_at(tmp.path(), 1024, 128);
    m.write("内容").await.unwrap();

    let n = m.node(&node_spec()).await;
    assert_eq!(n.name, MEMORY_FILE, "节点名 = 真实文件名");
    assert_eq!(n.title, SEGMENT_TITLE);
    assert_eq!(n.kind, PLUGIN_ID_MEMORY, "场景标签用所属插件的场景名");
    assert_eq!(n.size, Some("内容".len() as u64));
    assert!(n.updated_at.is_some(), "节点要用它做排序");
    assert_eq!(n.access, VdfsAccess::READ_WRITE, "记忆文件恒可读写");
}

/// 挂载点内的相对地址就是文件名 —— 绝对地址由调用方用上下文父地址拼
#[tokio::test]
async fn rel_path_is_just_the_file_name() {
    assert_eq!(rel_path(), MEMORY_FILE);
}
