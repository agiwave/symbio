//! `vdfs_read` 的结果形状契约（/ 04 报告建议 3）。
//!
//! 消费方 `extract_result`（loop 插件）第 1 条判据读 `content`；生产方在这里、
//! 消费方在另一个插件，两侧互不可见。这条用例把约定变成可执行的绿：文本文件必须
//! 给 `content` **字符串**（含行号与分页读数），而不是只给 `total_lines` 之类的
//! 结构化字段让消费方退回整包 JSON。

use super::*;
use crate::providers::DefaultToolVisitor;
use crate::symbio_core::{
    Capability, ExecEnv, PluginInvokeRequest, PluginInvokeRequestExt, PluginSimpleRequest, WORKDIR,
};
use std::sync::Arc;

fn empty_provider() -> Arc<ToolVdfs> {
    Arc::new(ToolVdfs::new(Arc::new(DefaultToolVisitor::new())))
}

/// 文本文件：正文进 `content`，带行号与分页读数，并回显请求地址。
#[tokio::test]
async fn text_result_carries_numbered_content_and_pagination_echo() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("note.txt"), "alpha\nbeta\ngamma\n").unwrap();

    let req = PluginSimpleRequest::new(None, None);
    req.set(WORKDIR, dir.path().to_string_lossy().to_string());
    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(req);
    let env = ExecEnv::from_request(&*ctx);

    let tool = ReadTool::new(empty_provider());
    let data = tool
        .execute(
            serde_json::json!({ "path": "note.txt", "offset": 2, "limit": 1 }),
            &env,
            ctx,
        )
        .await
        .expect("读取应成功");

    let content = data["content"]
        .as_str()
        .expect("正文必须在 content 字符串里（extract_result 第 1 条判据）");
    assert!(
        content.contains("2: beta"),
        "行号必须从 1 起且与请求窗口对齐（实际: {content:?}）"
    );
    assert!(
        !content.contains("alpha"),
        "窗口外的行不该出现（实际: {content:?}）"
    );
    assert!(
        content.contains("[行 2-2，共 3 行]"),
        "分页读数缺失——模型靠它决定下一次 offset（实际: {content:?}）"
    );
    assert_eq!(data["total_lines"].as_u64(), Some(3));
    assert_eq!(
        data["path"].as_str(),
        Some("note.txt"),
        "回显的是请求地址，不是解析后的绝对路径"
    );
}

/// 读一个目录：**错误文案必须是可执行的下一步，不能像一个地址**。
///
/// 实测会话 cli1a175d2ff0：文案写着 `目录不可读：<工作目录>\docs`，模型下一跳就交了
/// `vdfs_read {"path":"<那条真实路径>"}`——把诊断串当成地址，同一错误连撞三轮。
/// 现在文案①以调用方交来的**地址**为主语、②附一份可直接复制的 `vdfs_list` 参数、
/// ③通篇不出现物理路径（物理位置只在不存在的分支里以「工作目录 =」的形式出现）。
#[tokio::test]
async fn reading_a_directory_answers_with_the_next_call_not_with_a_path_lookalike() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    let workdir = dir.path().to_string_lossy().to_string();

    let req = PluginSimpleRequest::new(None, None);
    req.set(WORKDIR, workdir.clone());
    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(req);
    let env = ExecEnv::from_request(&*ctx);

    let err = ReadTool::new(empty_provider())
        .execute(serde_json::json!({ "path": "sub" }), &env, ctx)
        .await
        .expect_err("目录没有正文，应报错");
    let text = err.to_string();

    // 文案里的是**归一化后的地址**（复制过去必然可用），主语用它、不用物理路径
    assert!(
        text.contains("'sub'"),
        "主语必须是本次调用的地址（实际: {text}）"
    );
    assert!(
        text.contains(r#"vdfs_list"#) && text.contains(r#"{"path":"sub"}"#),
        "要给出原样可用的下一次调用——模型改的是**工具**才算真的换了路（实际: {text}）"
    );
    assert!(
        !text.contains(&workdir),
        "物理路径不许出现在文案里：它长得像地址，就会被当地址重交（实际: {text}）"
    );
}

/// 路径不存在：以地址为主语，物理位置标明「在工作目录 … 下找」——上下文，不是候选地址。
#[tokio::test]
async fn a_missing_path_names_the_address_and_labels_the_workdir_as_context() {
    let dir = tempfile::tempdir().unwrap();
    let req = PluginSimpleRequest::new(None, None);
    req.set(WORKDIR, dir.path().to_string_lossy().to_string());
    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(req);
    let env = ExecEnv::from_request(&*ctx);

    let err = ReadTool::new(empty_provider())
        .execute(serde_json::json!({ "path": "nope.md" }), &env, ctx)
        .await
        .expect_err("不存在的文件应报错");
    let text = err.to_string();
    assert!(
        text.contains("'nope.md'"),
        "文案要点名失败的那个地址（实际: {text}）"
    );
    assert!(
        text.contains("在工作目录"),
        "物理位置要作为「在哪找的」出现，而不是光秃秃一条路径（实际: {text}）"
    );
    assert!(
        !text.contains("插件未找到"),
        "一次路径拼错不该读成「工具没装」（实际: {text}）"
    );
}
