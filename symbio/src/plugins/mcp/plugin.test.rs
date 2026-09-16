//! `plugin.rs` 的单元测试（MCP Provider 的 VDFS 行为）。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）：
//! `plugin.rs` 只保留生产代码，测试全部放本文件。

use super::*;

/// 「点新建」必须直接进入该类型的详情页（用户第 1 / 2 点）。
///
/// 类型清单里的 `ext` 是**呈现扩展名**（`mcp`——`id_of` 按它剥地址后缀），
/// 而落成后的节点 `ext = form`。两者不同，所以类型必须显式声明：
///
/// - `node_ext` = 落成后的渲染器键（漏了它，草稿详情页落到 `fallback` 兜底，
///   而不是同一张表单）；
/// - `schema`    = 表单定义（没有它，`form` 渲染器渲染不出任何字段）。
#[test]
fn new_type_declares_the_landing_detail() {
    let types = McpPlugin::default().root_new_types();
    let form = types
        .iter()
        .find(|t| t.ext == PLUGIN_MCP)
        .expect("根下可新建 MCP Server");
    assert_eq!(
        form.node_ext.as_deref(),
        Some(VDFS_EXT_FORM),
        "草稿必须与落成后用同一个渲染器"
    );
    assert!(form.schema.is_some(), "没有 schema，表单渲染不出任何字段");

    // 整包导入那条不进详情页（内容来自本地文件），因此不需要 node_ext
    let pack = types
        .iter()
        .find(|t| t.ext == VDFS_EXT_ZIP)
        .expect("可导入整包");
    assert_eq!(pack.source.as_deref(), Some(VDFS_NEW_SOURCE_FILE));
    assert!(pack.node_ext.is_none() && pack.schema.is_none());
}

/// 无名字新建 = 写挂载点目录自身：id 由本插件生成（用户第 3 点）。
///
/// 目录自身没有可覆盖的目标 ⇒ 必须带 `create` 意图；带了就必须建得出来，
/// 否则「点新建 → 在详情页填好 → 保存」会停在草稿上。
#[test]
fn nameless_write_generates_an_id() {
    assert_eq!(resolve_id("demo", false).unwrap(), "demo");
    assert!(
        resolve_id("", false).is_err(),
        "目录自身没有可覆盖的目标，必须显式表达 create 意图"
    );
    let id = resolve_id("", true).unwrap();
    assert!(id.starts_with("mcp-"), "带类别前缀便于人读：{id}");
    assert_eq!(id.len(), "mcp-".len() + 8, "随机段定长：{id}");
}
