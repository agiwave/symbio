use super::*;

#[test]
fn embedded_index_is_always_available() {
    // 真产物或占位页都必须有 index.html —— serve 的 SPA 回退依赖它。
    let index = get("index.html").expect("embedded index.html");
    assert!(!index.is_empty());
    let text = String::from_utf8_lossy(index);
    assert!(text.contains("<html") || text.contains("<HTML"));
}

#[test]
fn root_path_falls_back_to_index() {
    assert_eq!(get("/"), get("index.html"));
    assert_eq!(get(""), get("index.html"));
}

#[test]
fn content_type_distinguishes_immutable_assets_from_html() {
    assert_eq!(content_type("index.html"), "text/html; charset=utf-8");
    assert_eq!(
        content_type("assets/app.js"),
        "text/javascript; charset=utf-8"
    );
    assert_eq!(content_type("assets/app.css"), "text/css; charset=utf-8");
}

#[test]
fn placeholder_state_is_exposed_to_runtime() {
    // 只是确认接口可读；真值由构建时 dist 是否存在决定，不强绑。
    let _ = is_placeholder();
}
