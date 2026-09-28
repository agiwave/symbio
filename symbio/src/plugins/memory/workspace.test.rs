//! `workspace.rs` 的单元测试 —— 工作区这一腿的**个性**（落位 / 挂载名 / 闸门注入）。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。
//!
//! 共享实现的行为（读写、拒绝、截断、片段排版）已在 `providers/memory/tests.rs` 钉住，
//! 这里**刻意不重复**。本文件只回答「工作区这一腿」特有的问题——尤其是
//! **挂载名 ≠ 物理名**这条本插件合并后才存在的约定。

use super::*;
use crate::symbio_core::absolute_addr;
use crate::symbio_core::{
    PluginInvokeRequest, PluginInvokeRequestExt, PluginSimpleRequest, VDFS_PARENT_ADDR,
};
use std::sync::Arc;
use tempfile::TempDir;

/// 合成父地址：真实值由容器转发时写入上下文，本文件不依赖真实挂载名
const PARENT: &str = "@vfs/memory";

/// 落位是**行业约定**：工作区根目录的 `AGENTS.md`（不属于 symbio 私有数据）
#[test]
fn memory_is_the_workspace_root_agents_file() {
    let path = memory_path("/w");
    assert_eq!(path, Path::new("/w").join(WORK_MEMORY_FILE));
    assert_eq!(path.file_name().unwrap(), "AGENTS.md");
    // 挂载里的地址是挂载名（WORKSPACE.md），不是物理名
    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
    ctx.set(VDFS_PARENT_ADDR, PARENT.to_string());
    assert_eq!(
        absolute_addr(&ctx, rel_path()),
        format!("{PARENT}/{}", MOUNT_FILE)
    );
}

/// 挂载名与物理名**刻意不同**：两个作用域的物理文件同名（都叫 `AGENTS.md`），
/// 挂载里必须能区分「这份记忆是谁的」
#[test]
fn mount_name_differs_from_the_physical_name() {
    assert_eq!(WORK_MEMORY_FILE, "AGENTS.md");
    assert_eq!(MOUNT_FILE, "WORKSPACE.md");
    assert_ne!(
        WORK_MEMORY_FILE, MOUNT_FILE,
        "同一挂载点里两个同名条目 = 模型与用户都分不清归属"
    );
    assert_eq!(rel_path(), MOUNT_FILE, "挂载内的相对路径 = 挂载名");
}

/// 无工作区 = 无作用域（闸门在 [`store`] 一处收口，调用点不重复判断）
#[test]
fn no_workspace_means_no_scope() {
    let m = store(None, 1024, 256);
    assert!(!m.has_scope());
    assert!(m.path().is_none());
    assert_eq!(m.inject().unwrap(), None, "无工作区不注入任何记忆段落");
    assert_eq!(
        m.segment(&segment_spec("@vfs/memory/WORKSPACE.md"))
            .unwrap(),
        None
    );

    // 空串与纯空白同样视为「没有工作区」
    for blank in ["", "   "] {
        assert!(!store(Some(blank), 1024, 256).has_scope());
    }
}

/// 有工作区时落位就在工作区根，且两道闸门原样交给共享实现
#[test]
fn scope_carries_the_two_gates_into_the_shared_impl() {
    let tmp = TempDir::new().unwrap();
    let wd = tmp.path().to_string_lossy().to_string();
    let m = store(Some(&wd), 1234, 321);

    let expected = memory_path(&wd);
    assert!(m.has_scope());
    assert_eq!(m.path(), Some(expected.as_path()));
    assert_eq!(m.write_max_bytes(), 1234);
    assert_eq!(m.inject_max_bytes(), 321);
    assert_eq!(
        m.file_name(),
        Some(WORK_MEMORY_FILE),
        "物理文件名 = 行业约定的 AGENTS.md"
    );
}

/// 节点名必须是**挂载名**：`node_spec` 用 `name` 覆盖真实文件名，
/// 否则与智能体记忆在同一个列表里撞名
#[test]
fn node_name_is_the_mount_name_not_the_physical_name() {
    let tmp = TempDir::new().unwrap();
    let wd = tmp.path().to_string_lossy().to_string();
    let m = store(Some(&wd), 1024, 256);
    let n = m.node(&node_spec());
    assert_eq!(n.name, MOUNT_FILE, "挂载列表里的名字必须是 WORKSPACE.md");
    assert_eq!(n.title, SEGMENT_TITLE);
    assert_eq!(n.kind, "workspace", "场景 kind 只承载语义");
}

/// 片段规格 = 本层的全部「个性」：标题 / 地址 / 互属说明 / 空提示
#[test]
fn segment_spec_is_the_workspace_layer_personality() {
    let addr = "@vfs/memory/WORKSPACE.md";
    let s = segment_spec(addr);
    assert_eq!(s.title, "工作区记忆");
    assert_eq!(s.address, addr);
    assert!(
        s.note.unwrap_or_default().contains("智能体记忆"),
        "两个作用域同住一个挂载点，片段要点明互属关系"
    );
    assert!(s.empty_hint.contains("暂无内容"));
}

/// 端到端：落位 → 写入 → 片段（四要素齐全）
#[test]
fn segment_round_trips_through_the_real_file() {
    let tmp = TempDir::new().unwrap();
    let wd = tmp.path().to_string_lossy().to_string();
    let m = store(Some(&wd), 1024, 256);
    m.write("用户偏好中文回答。").unwrap();

    let seg = m
        .segment(&segment_spec("@vfs/memory/WORKSPACE.md"))
        .unwrap()
        .unwrap();
    assert!(seg.contains("【工作区记忆】"));
    assert!(
        seg.contains("@vfs/memory/WORKSPACE.md"),
        "地址要下发（挂载名，不是物理名）: {seg}"
    );
    assert!(seg.contains("1024"), "上限来自本层配置: {seg}");
    assert!(seg.contains("用户偏好中文回答。"), "正文原样带上: {seg}");
    assert!(seg.contains("vdfs_read") && seg.contains("vdfs_write"));
}

/// 记忆就躺在工作区里——能被 `git` 版本化、能被协作者读到
#[test]
fn memory_is_a_plain_file_inside_the_workspace() {
    let tmp = TempDir::new().unwrap();
    let wd = tmp.path().to_string_lossy().to_string();
    let m = store(Some(&wd), 1024, 256);
    m.write("x").unwrap();

    let p = memory_path(&wd);
    assert!(p.is_file());
    assert_eq!(
        p.parent().unwrap(),
        tmp.path(),
        "直接落在工作区根，不另建子目录"
    );
}
