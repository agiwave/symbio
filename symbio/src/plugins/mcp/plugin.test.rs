//! `plugin.rs` 的单元测试（MCP Provider 的 VDFS 行为）。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）：
//! `plugin.rs` 只保留生产代码，测试全部放本文件。

use super::*;

/// 「点新建」必须直接进入该类型的详情页（用户第 1 / 2 点）。
///
/// 类型里的 `ext` 是**呈现扩展名**（`mcp`——`id_of` 按它剥地址后缀），
/// 而落成后的节点 `ext = form`。两者不同，所以类型必须显式声明：
///
/// - `node_ext` = 落成后的渲染器键（漏了它，草稿详情页落到 `fallback` 兜底，
///   而不是同一张表单）；
/// - `schema`    = 表单定义（没有它，`form` 渲染器渲染不出任何字段）。
#[tokio::test]
async fn new_type_declares_the_landing_detail() {
    // 「根下可新建类型」挂在**根节点自己的自述**上（不在同步的 `PluginMeta` 上）：
    // 走 `Stat("")`，与更深层节点同一条通道（`VdfsNode::new_type`）。
    let root = McpPlugin::default()
        .dispatch(&VdfsContext::empty(), "", VdfsRequest::Stat)
        .await
        .unwrap()
        .into_stat()
        .expect("根节点自述");
    let t = root.new_type.expect("根下可新建 MCP Server");
    assert_eq!(t.ext, PLUGIN_ID_MCP, "呈现扩展名是 MCP 自己的，不是包的");
    assert_eq!(
        t.node_ext.as_deref(),
        Some(VDFS_EXT_FORM),
        "草稿必须与落成后用同一个渲染器"
    );
    assert!(t.schema.is_some(), "没有 schema，表单渲染不出任何字段");
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

/// `with_id` 与 `model/plugin.rs` 的同名实现**锁等价**（ADR-011 双份实现）。
///
/// 用例与 `model/plugin.test.rs` 逐字同构：缺 id 补路径段、已有 id 原样保留、
/// 空串 id 视为缺失。任一侧改动行为，这里（或 model 侧）必有一个测试失败，
/// 从而暴露漂移——这是双份实现唯一的漂移守卫。
#[test]
fn with_id_locks_parity_with_model() {
    // 缺 id → 补路径段
    assert_eq!(
        with_id(&serde_json::json!({ "name": "demo" }), "mcp-demo")
            .get("id")
            .and_then(|v| v.as_str()),
        Some("mcp-demo")
    );
    // 已有 id → 原样保留，不被路径段覆盖
    assert_eq!(
        with_id(
            &serde_json::json!({ "id": "existing", "name": "demo" }),
            "mcp-demo"
        )
        .get("id")
        .and_then(|v| v.as_str()),
        Some("existing")
    );
    // 空串 id → 视为缺失，补路径段
    assert_eq!(
        with_id(&serde_json::json!({ "id": "", "name": "demo" }), "mcp-demo")
            .get("id")
            .and_then(|v| v.as_str()),
        Some("mcp-demo")
    );
}
