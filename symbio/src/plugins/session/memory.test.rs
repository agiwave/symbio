//! `session/memory.rs` 的单元测试 —— 本层的**个性**（落位 / 地址 / 闸门注入）。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。
//!
//! 内核行为（读写、拒绝、截断、片段排版）已在 `symbio_core::memory.test.rs` 钉住，
//! 这里**刻意不重复**。本文件只回答「会话这一层」特有的问题。
//!
//! ⚠️ 只做**路径计算**，不落盘：`memory_path` 走的是真实 homedir，写进去会污染
//! 用户目录。需要真文件的用例一律用 `tempfile` 自建 [`MemoryFile`]。

use super::*;
use crate::symbio_core::absolute_addr;
use crate::symbio_core::{
    MemoryFile, PluginInvokeRequest, PluginInvokeRequestExt, PluginSimpleRequest, PLUGIN_SESSION,
    VDFS_PARENT_ADDR,
};
use std::sync::Arc;
use tempfile::TempDir;

/// 测试用会话存储根（只算路径，不落盘）
fn root() -> std::path::PathBuf {
    std::path::Path::new("/symbio-test").join(PLUGIN_SESSION)
}

/// 落位在会话目录下，与 `session.json` / `messages.json` 同级
#[test]
fn memory_lives_in_the_session_directory() {
    let p = memory_path(&root(), "abc");
    assert_eq!(p.file_name().unwrap(), MEMORY_AGENTS_FILE);
    assert_eq!(p.parent().unwrap().file_name().unwrap(), "abc");
    assert_eq!(
        p.parent().unwrap().parent().unwrap().file_name().unwrap(),
        PLUGIN_SESSION,
        "会话目录的父目录就是 session 存储根"
    );
}

/// 会话 id 经 `safe_id` 归一：路径分隔符不得逃逸出会话目录
#[test]
fn session_id_is_sanitized_before_joining() {
    // `safe_segment` 只替换分隔符与非法字符（`/ \ : * ? " < > |`）与控制字符，
    // **点号保留**——`..` 因此变成 `_.._`（两侧下划线来自两个 `/`），
    // 整串不再是一个可上溯的路径段。
    let p = memory_path(&root(), "../../etc");
    assert_eq!(
        p.parent().unwrap().file_name().unwrap(),
        ".._.._etc",
        "id 里的分隔符被替换，不能穿越目录"
    );
    assert!(
        p.starts_with(
            memory_path(&root(), "x")
                .parent()
                .unwrap()
                .parent()
                .unwrap()
        ),
        "归一后仍落在 session 存储根之下"
    );
}

/// 相对地址 + 上下文父地址 = 绝对地址（封装入口 [`absolute_addr`]）
///
/// 容器转发时把当前父地址写进上下文；本测试直接设一个合成父地址，
/// 于是不依赖真实挂载名（换名不必改测试）。
#[test]
fn address_composes_from_context_parent_and_rel() {
    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
    ctx.set(VDFS_PARENT_ADDR, "@vfs/session".to_string());
    assert_eq!(memory_rel_path("abc"), "abc/AGENTS.md");
    assert_eq!(
        absolute_addr(&ctx, &memory_rel_path("abc")),
        "@vfs/session/abc/AGENTS.md",
        "绝对地址 = 上下文父地址 + 相对地址"
    );
}

/// 无会话 id = 无作用域（闸门在 [`store`] 一处收口）
#[test]
fn no_session_means_no_scope() {
    let m = store(&root(), None, 1024, 256);
    assert!(!m.has_scope());
    assert!(m.path().is_none());
    assert_eq!(m.inject().unwrap(), None, "无会话不注入任何记忆");

    for blank in ["", "   "] {
        assert!(!store(&root(), Some(blank), 1024, 256).has_scope());
    }
}

/// 有会话 id 时落位正确，且两道闸门原样交给内核
#[test]
fn scope_carries_the_two_gates_into_the_kernel() {
    let m = store(&root(), Some("abc"), 1234, 321);
    assert!(m.has_scope());
    assert_eq!(m.path(), Some(memory_path(&root(), "abc").as_path()));
    assert_eq!(m.write_max_bytes(), 1234);
    assert_eq!(m.inject_max_bytes(), 321);
    assert_eq!(m.file_name(), MEMORY_AGENTS_FILE);
}

/// 条目规格：标题 / 地址 / 区分说明 / 空提示四样都在
#[test]
fn segment_spec_is_the_session_layer_personality() {
    // 合成绝对地址（真实值由调用点的 absolute_addr 拼出）
    let address = "@vfs/session/abc/AGENTS.md";
    let s = segment_spec(address);

    assert_eq!(s.title, "会话记忆");
    assert_eq!(s.address, address, "片段地址就是下发给模型的那个可编辑地址");
    assert!(
        s.note
            .unwrap_or_default()
            .contains("与【工作区记忆】【智能体记忆】相互独立"),
        "三层同名不同域，必须点明，否则模型会写错地方"
    );
    assert!(s.empty_hint.contains("暂无内容"));
}

/// 端到端：落位 → 写入 → 条目（四要素齐全），用临时目录自建避免污染真实 homedir
#[test]
fn segment_round_trips_through_a_real_file() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join(MEMORY_AGENTS_FILE);
    let m = MemoryFile::new(Some(path), 1024, 256);
    m.write("本会话约定：所有时间用 UTC。").unwrap();

    let address = "@vfs/session/abc/AGENTS.md";
    let seg = m.segment(&segment_spec(address)).unwrap().unwrap();
    assert!(seg.contains("【会话记忆】"), "{seg}");
    assert!(seg.contains(address), "地址要下发: {seg}");
    assert!(seg.contains("1024"), "上限来自本层配置: {seg}");
    assert!(seg.contains("本会话约定：所有时间用 UTC。"), "{seg}");
    assert!(seg.contains("vdfs_read") && seg.contains("vdfs_write"));
}
