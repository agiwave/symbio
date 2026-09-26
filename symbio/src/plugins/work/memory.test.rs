//! `work/memory.rs` 的单元测试 —— 本层的**个性**（落位 / 地址 / 闸门注入）。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。
//!
//! 共享实现的行为（读写、拒绝、截断、片段排版）已在 `providers/memory/tests.rs` 钉住，
//! 这里**刻意不重复**——重复测一遍只会让共享实现改口径时要改两处，那正是抽共享实现
//! 要消灭的成本。本文件只回答「工作区这一层」特有的问题。

use super::*;
use crate::symbio_core::absolute_addr;
use crate::symbio_core::{
    PluginInvokeRequest, PluginInvokeRequestExt, PluginSimpleRequest, VDFS_PARENT_ADDR,
};
use std::sync::Arc;
use tempfile::TempDir;

/// 合成父地址：真实值由容器转发时写入上下文，本文件不依赖真实挂载名
const PARENT: &str = "@vfs/work";

/// 落位是**行业约定**：工作区根目录的 `AGENTS.md`（不属于 symbio 私有数据）
#[test]
fn memory_is_the_workspace_root_agents_file() {
    let path = memory_path("/w");
    assert_eq!(path, Path::new("/w").join(WORK_MEMORY_FILE));
    assert_eq!(path.file_name().unwrap(), "AGENTS.md");
    // 相对地址就是文件名本身（挂在挂载点自身）；绝对地址 = 上下文父地址 + 相对
    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
    ctx.set(VDFS_PARENT_ADDR, PARENT.to_string());
    assert_eq!(
        absolute_addr(&ctx, WORK_MEMORY_FILE),
        format!("{PARENT}/AGENTS.md")
    );
}

/// 无工作区 = 无作用域（闸门在 [`store`] 一处收口，调用点不重复判断）
#[test]
fn no_workspace_means_no_scope() {
    let m = store(None, 1024, 256);
    assert!(!m.has_scope());
    assert!(m.path().is_none());
    assert_eq!(m.inject().unwrap(), None, "无工作区不注入任何记忆段落");
    assert_eq!(
        m.segment(&segment_spec("@vfs/work/AGENTS.md")).unwrap(),
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
    assert_eq!(m.file_name(), Some(WORK_MEMORY_FILE), "节点名 = 真实文件名");
}

/// 片段规格 = 本层的全部「个性」：标题 / 地址 / 空提示，且**没有**多余附加说明
#[test]
fn segment_spec_is_the_workspace_layer_personality() {
    let addr = "@vfs/work/AGENTS.md";
    let s = segment_spec(addr);
    assert_eq!(s.title, "工作区记忆");
    assert_eq!(s.address, addr);
    assert!(s.note.is_none(), "工作区层没有需要区分的邻居");
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
        .segment(&segment_spec("@vfs/work/AGENTS.md"))
        .unwrap()
        .unwrap();
    assert!(seg.contains("【工作区记忆】"));
    assert!(seg.contains("@vfs/work/AGENTS.md"), "地址要下发: {seg}");
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
