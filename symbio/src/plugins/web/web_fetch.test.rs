//! `symbio/src/plugins/web/web_fetch.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

fn tool() -> WebFetchTool {
    WebFetchTool::new(Arc::new(RwLock::new(WebConfig::default())))
}

/// `web_fetch` 必须与 `http_request` 用**同一套**出站判据：
/// 只校验 scheme 时，同一内网地址会「一个工具能取、另一个不能」。
#[tokio::test]
async fn private_and_local_targets_are_rejected_before_any_request() {
    let t = tool();
    for bad in [
        "http://127.0.0.1/admin",
        "http://127.0.0.2/",
        "http://[::1]:8080/",
        "http://0.0.0.0/",
        "http://10.1.2.3/",
        "http://192.168.1.1/",
        "http://169.254.169.254/latest/meta-data/",
        "http://localhost/",
        "http://foo.localhost/",
        "http://printer.local/",
        "http://db.internal/",
    ] {
        let err = t
            .execute_inner(json!({ "url": bad }))
            .await
            .expect_err(&format!("应拒绝：{bad}"));
        assert!(
            err.to_string().contains("阻止本地/私有主机"),
            "{bad} 的拒绝理由应是私网判定（实际: {err}）"
        );
    }
}

/// 非法 scheme 与无法解析的 URL 在守卫之前就被挡下，同样不发请求。
#[tokio::test]
async fn invalid_scheme_is_rejected() {
    let t = tool();
    for bad in ["ftp://example.com", "file:///etc/passwd"] {
        let err = t
            .execute_inner(json!({ "url": bad }))
            .await
            .expect_err(&format!("应拒绝：{bad}"));
        assert!(err.to_string().contains("只支持 HTTP/HTTPS"));
    }
}

/// 元数据头是模型区分 200 / 404 / 截断的唯一一行——形状由单测钉住
/// （无外网 CI 上 e2e 拿不到真实响应）。
#[test]
fn response_meta_header_carries_status_and_truncation() {
    let ok = content_with_meta(
        200,
        "text/html",
        false,
        "https://example.com/",
        "<h1>hi</h1>",
    );
    assert_eq!(
        ok.lines().next().unwrap(),
        "[web_fetch] status=200 · content_type=text/html · truncated=false · url=https://example.com/"
    );
    assert!(ok.ends_with("\n\n<h1>hi</h1>"), "正文必须紧跟在空行之后");

    let missing = content_with_meta(
        404,
        "text/plain",
        true,
        "https://example.com/x",
        "not found",
    );
    assert!(missing.starts_with("[web_fetch] status=404 · "));
    assert!(missing.contains("· truncated=true ·"));
}
