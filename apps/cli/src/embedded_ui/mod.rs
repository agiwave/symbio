//! CLI 专属的编译期内嵌 UI 资源（`packages/ui/dist`）。
//!
//! 内嵌只属于 CLI 这个交付物入口：桌面启动器不内嵌 UI，它拉起的 CLI serve 提供同一份 UI，
//! 共享 runtime 不依赖本模块，因此桌面二进制不会被迫多嵌一份 dist。
//! 与「运行时从磁盘读 `--frontend-dist`」并存：默认走内嵌资源，二进制自带 UI；
//! 显式指定外部目录时才回退到磁盘。两者由 `symbio-runtime` 统一成同一套静态响应。

include!(concat!(env!("OUT_DIR"), "/embedded_assets.rs"));

/// 当前二进制内嵌的是占位页而不是真实前端产物；serve 启动时据此给出显式告警。
pub fn is_placeholder() -> bool {
    PLACEHOLDER
}

/// 按 URL 路径取资源；`/` 与目录路径落到 `index.html`。
///
/// 路径规范化在调用方（runtime 的 ingress 已做 `Component::Normal` 校验与 URL 解码），
/// 这里只负责查表：查不到返回 `None`，由调用方决定 404 还是 SPA 回退。
pub fn get(path: &str) -> Option<&'static [u8]> {
    let normalized = path.trim_start_matches('/');
    let key = if normalized.is_empty() {
        "index.html"
    } else {
        normalized
    };
    ASSETS.get_file(key).map(|f| f.contents())
}

/// 内嵌资源的 MIME 类型（按扩展名），与 runtime 的磁盘分支共用口径。
pub fn content_type(path: &str) -> &'static str {
    let ext = path.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    match ext {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "woff2" => "font/woff2",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
#[path = "mod.test.rs"]
mod tests;
