//! `tool_executor` 模块的单元测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）：
//! `tool_executor.rs` 只保留生产代码，测试全部放本文件。

use super::*;
use crate::symbio_core::SimpleRequest;
use std::sync::atomic::AtomicBool;

fn test_ctx() -> Arc<dyn InvokeRequest> {
    Arc::new(SimpleRequest::new(None, None))
}

/// 需求 2 回归：工具调用 id 缺失/非法时不得跳过，必须作为工具调用失败处理——
/// 生成错误结果子节点（喂回 LLM）+ 父节点失败补丁，保证已落库的 ToolCall
/// 节点不会成为"无结果 tool_call"（下轮请求 400）。
#[tokio::test]
async fn missing_tool_call_id_is_recorded_as_failure() {
    let (_host, mut plugin_chan) = PluginChannel::pair(64);
    let abort = Arc::new(AtomicBool::new(false));
    let tcs = vec![ToolCallInfo {
        id: None,
        name: Some("vdfs_list".into()),
        arguments: json!({ "path": "." }),
    }];

    let (msgs, updates) =
        process_tool_calls_async(tcs, &None, &mut plugin_chan, &abort, test_ctx()).await;

    assert_eq!(msgs.len(), 1, "必须生成失败结果子节点（而非跳过）");
    assert_eq!(msgs[0].role, Some(MessageRole::Tool));
    assert_eq!(msgs[0].status, Some(MessageStatus::Completed));
    assert!(
        msgs[0]
            .content
            .as_ref()
            .map(|c| c.to_text().contains("Error:"))
            .unwrap_or(false),
        "结果内容应携带错误信息"
    );
    assert!(!msgs[0].parent_id.as_deref().unwrap_or("").is_empty());

    assert_eq!(updates.len(), 1, "必须生成父节点失败补丁");
    assert_eq!(updates[0].status, Some(MessageStatus::Completed));
    assert_eq!(
        updates[0]
            .meta
            .as_ref()
            .and_then(|m| m.get("failure_kind"))
            .and_then(|v| v.as_str()),
        Some("error")
    );
}

/// 空串 id 与缺失同等对待（"不合法"）。
#[tokio::test]
async fn empty_tool_call_id_is_recorded_as_failure() {
    let (_host, mut plugin_chan) = PluginChannel::pair(64);
    let abort = Arc::new(AtomicBool::new(false));
    let tcs = vec![ToolCallInfo {
        id: Some(String::new()),
        name: Some("vdfs_list".into()),
        arguments: json!({}),
    }];

    let (msgs, updates) =
        process_tool_calls_async(tcs, &None, &mut plugin_chan, &abort, test_ctx()).await;

    assert_eq!(msgs.len(), 1);
    assert_eq!(updates.len(), 1);
}

/// 工具名缺失/非法 → 同样作为失败处理（结果挂到已知 id 上，结构完整）。
#[tokio::test]
async fn missing_tool_name_is_recorded_as_failure() {
    let (_host, mut plugin_chan) = PluginChannel::pair(64);
    let abort = Arc::new(AtomicBool::new(false));
    let tcs = vec![ToolCallInfo {
        id: Some("tc-known".into()),
        name: Some(String::new()),
        arguments: json!({}),
    }];

    let (msgs, updates) =
        process_tool_calls_async(tcs, &None, &mut plugin_chan, &abort, test_ctx()).await;

    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].parent_id.as_deref(), Some("tc-known"));
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].id, "tc-known");
}

/// 回归（真实事故）：中文参数摘要不得在字节边界上 panic。
///
/// 事故现场：`write_file` 传入含中文的 `content`，`args_summary(.., 200)`
/// 的 `&s[..200]` 落在 `'的'`（bytes 199..202）内部 →
/// `tokio-rt-worker` panic → ChatLoop join 失败 → 会话异常终止。
#[test]
fn args_summary_handles_multibyte_args() {
    use serde_json::json;

    // 构造必然跨越 200 字节边界的中文参数（'的' 3 字节）
    let payload = "参数摘要：截断到 max_chars，用于日志打印（避免超长参数刷屏）。".repeat(8);
    let args = json!({ "path": "a.rs", "content": payload });

    // 任意 max 都必须安全返回（旧实现会在部分 max 上 panic）
    for max in [1usize, 2, 3, 199, 200, 201, 512] {
        let s = args_summary(&args, max);
        assert!(
            s.chars().count() <= max + 32,
            "max={max} 摘要过长：{} 字节",
            s.len()
        );
    }

    // 超长时带长度标注，便于日志排查
    let long = args_summary(&args, 200);
    assert!(long.ends_with(&format!("(len={})", args.to_string().len())));
    assert!(long.contains('…'));

    // 短参数原样返回
    assert_eq!(args_summary(&json!({"a": 1}), 200), "{\"a\":1}");
}
