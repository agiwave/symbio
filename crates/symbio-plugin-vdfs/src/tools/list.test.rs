//! `vdfs_list` 的结果形状契约。
//!
//! 三条判据都来自实跑观察，不是设想：
//! 1. **目录在前**——provider 的 `list` 本就按「目录优先、名升序」排好，本工具再排一次。
//!    此前那一次比较器写反（`tb.cmp(ta)`），线上输出一直是**文件在前**，而注释与工具
//!    描述都写着目录在前；没有用例锁形状，反向排序发了很久没人发现。
//! 2. **回执自带账目**——`message` 报「N 个目录 + M 个文件」。实测会话
//!    cli1a175d1ea440：输出 28 条、文件排在前的那一段先被读完，模型只复述了它读到的
//!    前 17 条，然后宣布剩下的「是第二层」。条数写在回执上它就知道自己该说几个。
//! 3. **根的写法**——工作目录根是 `'.'`，回执也照它印。`'/'` 不是它的别名：
//!    首部的 `/` 是操作系统的绝对锚点（见 D-172），把 `'/'` 当根写法会让模型下一次
//!    照着回执去列磁盘根。此前描述与示例互相打脸（`PATH_DESC` 说 `'/'` 会被剥掉，
//!    示例却在用 `'/'` 当根），这里把「示例的形式真的能用」变成用例。

use super::*;
use symbio_providers::DefaultToolVisitor;
use symbio_core::{
    Capability, ExecEnv, PluginInvokeRequest, PluginInvokeRequestExt, PluginSimpleRequest, WORKDIR,
};
use std::sync::Arc;

fn empty_provider() -> Arc<ToolVdfs> {
    Arc::new(ToolVdfs::new(Arc::new(DefaultToolVisitor::new())))
}

/// 在临时工作目录里造一棵最小树：两个目录、三个文件。
fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir(root.join("docs")).unwrap();
    std::fs::create_dir(root.join("src")).unwrap();
    std::fs::write(root.join("README.md"), "hello\n").unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\n").unwrap();
    std::fs::write(root.join("LICENSE"), "mit\n").unwrap();
    dir
}

async fn list(path: &str, workdir: &str, ignore: Option<Value>) -> Value {
    let req = PluginSimpleRequest::new(None, None);
    req.set(WORKDIR, workdir.to_string());
    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(req);
    let env = ExecEnv::from_request(&*ctx);
    let mut args = json!({ "path": path });
    if let Some(ig) = ignore {
        args["ignore"] = ig;
    }
    ListTool::new(empty_provider())
        .execute(args, &env, ctx)
        .await
        .expect("列举应成功")
}

/// 目录排在文件之前，各自按名称升序；`message` 把两类条数分别报出来。
#[tokio::test]
async fn directories_come_first_and_the_receipt_counts_both_kinds() {
    let dir = fixture();
    let data = list(".", &dir.path().to_string_lossy(), None).await;

    let names: Vec<&str> = data["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        vec!["docs", "src", "Cargo.toml", "LICENSE", "README.md"],
        "目录必须在前（名升序），文件在后（名升序）；实际: {names:?}"
    );
    assert_eq!(data["count"].as_u64(), Some(5));
    let message = data["message"].as_str().unwrap();
    assert!(
        message.contains("2 个目录 + 3 个文件"),
        "回执要自带账目，弱模型才不会把没复述的条目当成不存在（实际: {message}）"
    );
}

/// `'.'` 是工作目录根，回执照原样印它——`'/'` 已经不是根写法（首部的 `/` 是绝对
/// 锚点，`PATH_DESC` 与 `normalize_addr` 同一口径，见 D-172）。
#[tokio::test]
async fn the_workdir_root_is_named_by_a_dot_not_by_a_slash() {
    let wd = fixture();
    let workdir = wd.path().to_string_lossy().to_string();
    let data = list(".", &workdir, None).await;
    let names: Vec<&str> = data["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert!(
        names.starts_with(&["docs", "src"]),
        "`.` 应列出工作目录根且目录在前（实际: {names:?}）"
    );
}

/// `ignore` 按名称 glob 过滤，计数随之变小（过滤发生在排序与计数之前）。
#[tokio::test]
async fn ignore_glob_filters_before_the_count_is_reported() {
    let dir = fixture();
    let data = list(
        ".",
        &dir.path().to_string_lossy(),
        Some(json!(["*.toml", "src"])),
    )
    .await;
    let names: Vec<&str> = data["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        vec!["docs", "LICENSE", "README.md"],
        "被 ignore 命中的目录与文件都不该出现（实际: {names:?}）"
    );
    assert!(
        data["message"]
            .as_str()
            .unwrap()
            .contains("1 个目录 + 2 个文件"),
        "账目按过滤后的条目算：{}",
        data["message"]
    );
}
