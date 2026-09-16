//! `workdir` 模块的单元测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）：
//! `workdir.rs` 只保留生产代码，测试全部放本文件。

use super::*;
use serde_json::json;
use tempfile::TempDir;

/// 世代守卫延迟释放：卸载的 unwatch 迟到时不得误杀重新建立的订阅；
/// 宽限期内无重新订阅才真正释放（tokio paused time 驱动）。
#[tokio::test(start_paused = true)]
async fn release_watch_generation_guard() {
    let mgr = WorkdirWatchManager::default();
    let wd = "D:/work";

    // 第一次进入：订阅（世代 1）
    mgr.ensure_watch(wd, "sess_a");
    assert!(mgr.is_watched(wd));

    // 切走 → unwatch（世代 1 的释放进入宽限期）
    mgr.release_watch(wd, "sess_a");
    assert!(mgr.is_watched(wd), "宽限期内监听保持");

    // 快速返回 → 重新订阅（世代 2）
    mgr.ensure_watch(wd, "sess_a");
    assert!(mgr.is_watched(wd));

    // 宽限期流逝：世代 1 的迟到释放因世代推进被忽略
    advance_until(&mgr, wd, true).await;
    assert!(mgr.is_watched(wd), "迟到的 unwatch 不得误杀重新订阅");

    // 再切走 → 世代 2 的释放正常生效（无重新订阅推进世代）
    mgr.release_watch(wd, "sess_a");
    advance_until(&mgr, wd, false).await;
    assert!(!mgr.is_watched(wd), "无订阅方后监听应停止");
}

/// 在 paused 时钟下推进时间并等待释放任务收敛到目标状态。
///
/// 释放任务经 `tokio::spawn` + `sleep(RELEASE_GRACE)` 调度，定时器在任务
/// 首次被 poll 时才注册；若首次 poll 发生在时钟推进之后，deadline 会顺延
/// 到「当前时刻 + 宽限期」。固定 advance 一次不可靠，故循环推进并让出，
/// 直到目标状态出现（有界，防死循环）。
async fn advance_until(mgr: &WorkdirWatchManager, wd: &str, watched: bool) {
    for _ in 0..10 {
        tokio::time::advance(RELEASE_GRACE).await;
        tokio::task::yield_now().await;
        if mgr.is_watched(wd) == watched {
            return;
        }
    }
}

/// 共享 workdir：多方订阅引用计数，最后一位释放后监听停止
#[tokio::test(start_paused = true)]
async fn shared_workdir_reference_counting() {
    let mgr = WorkdirWatchManager::default();
    let wd = "D:/shared";

    mgr.ensure_watch(wd, "sess_a");
    mgr.ensure_watch(wd, "sess_b");
    assert!(mgr.is_watched(wd));

    mgr.release_watch(wd, "sess_a");
    advance_until(&mgr, wd, true).await;
    assert!(mgr.is_watched(wd), "仍有其他订阅方，监听保持");

    mgr.release_watch(wd, "sess_b");
    advance_until(&mgr, wd, false).await;
    assert!(!mgr.is_watched(wd), "最后一位释放后监听停止");
}

async fn seed(dir: &Path) {
    tokio::fs::create_dir_all(dir.join("src")).await.unwrap();
    tokio::fs::create_dir_all(dir.join(".hidden"))
        .await
        .unwrap();
    tokio::fs::write(dir.join("src/lib.rs"), "fn main() {}")
        .await
        .unwrap();
    tokio::fs::write(dir.join("README.md"), "# demo")
        .await
        .unwrap();
    tokio::fs::write(dir.join(".env"), "SECRET=1")
        .await
        .unwrap();
}

#[tokio::test]
async fn root_children_skip_hidden_and_sort_dirs_first() {
    let tmp = TempDir::new().unwrap();
    seed(tmp.path()).await;
    let nodes = list_children(tmp.path().to_str().unwrap(), None)
        .await
        .unwrap();
    let names: Vec<&str> = nodes.iter().map(|n| n.name.as_str()).collect();
    assert_eq!(names, vec!["src", "README.md"]);
    // 是否可展开只看访问位，不看 kind
    assert!(nodes[0].is_dir(), "目录 → `l`");
    assert!(!nodes[1].is_dir(), "文件 → 不可列");
    assert_eq!(nodes[1].access.flags(), "rw");
}

#[tokio::test]
async fn nested_level_reports_only_its_own_segment() {
    let tmp = TempDir::new().unwrap();
    seed(tmp.path()).await;
    let nodes = list_children(tmp.path().to_str().unwrap(), Some("src"))
        .await
        .unwrap();
    assert_eq!(nodes.len(), 1);
    // 全路径由分发层按请求路径回填，本层只给段名
    assert_eq!(nodes[0].name, "lib.rs");
}

#[tokio::test]
async fn traversal_paths_are_rejected() {
    let tmp = TempDir::new().unwrap();
    seed(tmp.path()).await;
    let wd = tmp.path().to_str().unwrap();
    assert!(list_children(wd, Some("../..")).await.is_err());
    assert!(read_node(wd, "../secrets").await.is_err());
}

/// `stat` 只回答节点形状，正文归 `vdfs/read`（不再内联，也不误读大文件）
#[tokio::test]
async fn stat_shapes_the_node_without_inlining_content() {
    let tmp = TempDir::new().unwrap();
    seed(tmp.path()).await;
    let wd = tmp.path().to_str().unwrap();
    let f = read_node(wd, "README.md").await.unwrap();
    assert_eq!(f.name, "README.md");
    assert_eq!(f.size, Some(6));
    assert_eq!(
        f.attributes.get(ATTR_CONFIG_TYPE).and_then(|v| v.as_str()),
        Some("file")
    );
    let d = read_node(wd, "src").await.unwrap();
    assert!(d.is_dir());
    assert!(d.size.is_none(), "目录没有字节数语义");
    assert_eq!(read_content(wd, "README.md").await.unwrap(), "# demo");
}

/// 工作目录**往返**：写 → 列 → 读 → 删（S6 会话内部链路的真实 IO 闭合）
///
/// VDFS 侧的 `<id>/工作目录[/<rel>]` 最终全部落到这四个函数上，而此前
/// 只有「列 / 读既有文件」的单点测试——写入与删除的实际落盘没有闭合验证。
#[tokio::test]
async fn workdir_roundtrip_write_list_read_delete() {
    let tmp = TempDir::new().unwrap();
    seed(tmp.path()).await;
    let wd = tmp.path().to_str().unwrap();

    // 写入（新建子目录内的文件）
    write_node(wd, "src/new.rs", "pub fn new() {}")
        .await
        .unwrap();
    let nested = list_children(wd, Some("src")).await.unwrap();
    let names: Vec<&str> = nested.iter().map(|n| n.name.as_str()).collect();
    assert_eq!(names, vec!["lib.rs", "new.rs"], "写入后出现在清单中");

    // 读回（VDFS `read` 走 `read_content`）
    assert_eq!(
        read_content(wd, "src/new.rs").await.unwrap(),
        "pub fn new() {}"
    );

    // 覆盖
    write_node(wd, "src/new.rs", "// 改后").await.unwrap();
    assert_eq!(read_content(wd, "src/new.rs").await.unwrap(), "// 改后");
    assert_eq!(
        list_children(wd, Some("src")).await.unwrap().len(),
        2,
        "覆盖不新增"
    );

    // 删除
    delete_node(wd, "src/new.rs").await.unwrap();
    let nested = list_children(wd, Some("src")).await.unwrap();
    assert_eq!(nested.len(), 1);
    assert_eq!(nested[0].name, "lib.rs");

    // 越界写入 / 读取目录内容均被拒（沙箱边界是这条链路的安全底线）
    assert!(write_node(wd, "../escape.rs", "x").await.is_err());
    assert!(read_content(wd, "src").await.is_err(), "目录不可当文件读");
}

#[tokio::test]
async fn workdir_metadata_extracts() {
    let mut s = Session::new("s1");
    assert!(workdir_of(&s).is_none());
    s.metadata["workdir"] = json!("D:/work");
    assert_eq!(workdir_of(&s).as_deref(), Some("D:/work"));
}
