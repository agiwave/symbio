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
