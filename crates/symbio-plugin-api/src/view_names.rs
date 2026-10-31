//! 跨插件可读视图名 —— **由 `scripts/facts/views-rs.mjs` 生成，请勿手改**。
//!
//! 事实源：各插件自己目录里的 `PLUGIN.yml` 的 `shared_views:` 行。名字、版本、
//! 扫描成本与求值函数仍住在 owner 自己的 `ViewDecl` 里；本表只回答「哪几条视图公开
//! 给别的插件按名字读」。读口见 `symbio_plugin_api::views`（DE-025 D-221）。
//!
//! 手改本文件会被 `gen-facts --check` 判红。

/// 一条公开给别的插件按名字读的视图：名字 + owner。
///
/// `ver` / `cost` / 求值函数不在这里：它们随 owner 的 `ViewDecl` 走，读口在
/// 冻结声明表里查——本表多抄一份就会与 owner 漂移。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SharedViewDecl {
    pub name: &'static str,
    pub owner: &'static str,
}

/// 跨插件可读视图（按名字排序）。
#[rustfmt::skip]
pub const SHARED_VIEWS: &[SharedViewDecl] = &[
    SharedViewDecl { name: "notes.digest", owner: "notes" },
    SharedViewDecl { name: "notes.insights", owner: "notes" },
    SharedViewDecl { name: "notes.recent", owner: "notes" },
    SharedViewDecl { name: "skill.retreat", owner: "skill" },
    SharedViewDecl { name: "turn.recalled", owner: "loop" },
];
