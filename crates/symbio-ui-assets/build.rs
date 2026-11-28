//! 编译期内嵌前端产物：把 `packages/ui/dist` 编进 CLI 二进制。
//!
//! ## 为什么 build.rs 生成代码而不是直接 include_dir!
//!
//! `include_dir!` 要求路径是**编译期字面量**；产物目录随仓库位置变化，必须由
//! build.rs 解析绝对路径后写进 OUT_DIR 的生成的 `.rs`。同时 `rerun-if-changed`
//! 让前端重建后 cargo 会重新嵌入——否则 CLI 二进制会带着旧 UI 蒙混过关。
//!
//! ## 产物不存在时
//!
//! 用一份占位页面而不是让编译失败：`msrv` / `verify` 等只编 Rust 的阶段没有
//! npm 产物，不能因为「没跑前端构建」就整体红。占位页明确提示如何构建，且
//! `is_placeholder()` 让运行期能识别——绝不静默当成正常产物。

use std::env;
use std::fs;
use std::path::PathBuf;

// panic-allow crates/symbio-ui-assets/build.rs::main: 构建期不变量——cargo 必须提供 CARGO_MANIFEST_DIR / OUT_DIR，环境变量缺失意味着构建环境损坏，继续只会生成指向错误位置的产物。
// panic-allow crates/symbio-ui-assets/build.rs::main: 构建期不变量——crate 必须位于仓库的 crates/ 下，解析不出仓库根就没法定位 packages/ui/dist。
// panic-allow crates/symbio-ui-assets/build.rs::main: 构建期不变量——OUT_DIR 必须可写，写不出生成代码时后续 include! 必然编译失败，早崩比晚崩更易定位。
fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    // crate 在 <repo>/crates/symbio-ui-assets，向上两级到仓库根。
    let repo = manifest
        .parent()
        .and_then(|p| p.parent())
        .expect("crate must live under <repo>/crates/");
    let dist = repo.join("packages").join("ui").join("dist");

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let generated = out_dir.join("embedded_assets.rs");

    let real = dist.join("index.html").is_file();
    let asset_root = if real {
        println!("cargo:rerun-if-changed={}", dist.display());
        dist
    } else {
        // 占位目录也放 OUT_DIR：不污染源码树，且随 crate 清理一起消失。
        let placeholder = out_dir.join("placeholder-dist");
        fs::create_dir_all(&placeholder).expect("create placeholder dist");
        fs::write(
            placeholder.join("index.html"),
            b"<!doctype html><html><head><meta charset=\"utf-8\"><title>Symbio</title></head><body><h1>Frontend assets not built</h1><p>Run <code>npm run build</code> in <code>packages/ui</code>, then rebuild the CLI.</p></body></html>\n",
        )
        .expect("write placeholder index.html");
        println!("cargo:warning=packages/ui/dist not found: embedding placeholder frontend; run `npm run build` in packages/ui for a complete CLI binary");
        placeholder
    };

    // include_dir! 只接受普通字符串字面量（raw string 不算）；统一成正斜杠避免转义。
    let root = asset_root.to_string_lossy().replace('\\', "/");
    let escaped = root.replace('"', "\\\"");
    let source = format!(
        "pub static ASSETS: ::include_dir::Dir = ::include_dir::include_dir!(\"{escaped}\");\n\
         pub const PLACEHOLDER: bool = {placeholder};\n",
        placeholder = !real,
    );
    fs::write(&generated, source).expect("write embedded_assets.rs");
}
