//! `plugin.rs` 的单元测试（skill Provider 的 VDFS 行为）。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）：
//! `plugin.rs` 只保留生产代码，测试全部放本文件。

use super::*;

/// 路径末段才是 id：`.vdfs/skill/<id>.skill` 与裸 `<id>` 同解
#[test]
fn id_of_strips_presentation_extension() {
    assert_eq!(id_of("demo"), "demo");
    assert_eq!(id_of("demo.skill"), "demo");
}

/// 导入建议名：新建地址是 `<name>.zip`，建议名即去掉 `.zip` 的 `<name>`
#[test]
fn import_name_of_strips_zip() {
    assert_eq!(import_name_of("demo.zip"), "demo");
    assert_eq!(import_name_of("demo.skill"), "demo");
}

/// 摘要优先 YAML frontmatter：`name` 作标题、`description` 作摘要，
/// 且详情定义随节点 `schema` 下发（详情页表单预填靠 `read`）
#[test]
fn node_prefers_frontmatter() {
    let md = "---\nname: 演示\ndescription: 一个用于演示的技能\n---\n\n正文\n";
    let n = node_of("demo", Some(md));
    assert_eq!(n.name, "demo");
    assert_eq!(n.title, "演示");
    assert_eq!(n.description.as_deref(), Some("一个用于演示的技能"));
    assert_eq!(n.ext.as_deref(), Some(VDFS_EXT_FORM));
    assert!(n.schema.is_some(), "详情定义必须随节点下发");
}

/// 无 frontmatter 的旧格式回落：首行标题 + Description 行
#[test]
fn node_falls_back_to_legacy_heading() {
    let md = "# 旧技能\n\n**Description** 旧格式描述\n";
    let n = node_of("old", Some(md));
    assert_eq!(n.title, "旧技能");
    assert_eq!(n.description.as_deref(), Some("旧格式描述"));
}

/// 主文件缺失（坏条目）降级为 id 占位，不阻断整张列表
#[test]
fn node_degrades_to_the_id_when_manifest_unreadable() {
    let n = node_of("broken", None);
    assert_eq!(n.name, "broken");
    assert_eq!(n.title, "broken");
    assert!(n.description.is_none());
}

/// 回归：**写进去的必须能被读回来**（写读同源）。
///
/// SKILL.md 是 Markdown，只能走纯文本落盘。若把 manifest 当 JSON 值写入，
/// 落盘会变成 `"---\nname: ...\n"`（外层引号 + `\n` 被转义成字面两字符），
/// 而 `parse_skill_md` 以 `strip_prefix("---\n")` 起手 ⇒ 必然失败：
/// 保存报成功、文件却是坏的，下次加载解析不出 frontmatter。
#[test]
fn validate_manifest_produces_parsable_markdown() {
    let manifest = serde_json::json!({
        "id": "demo",
        "name": "demo",
        "description": "一个用于演示的技能描述",
    });
    let md = validate_manifest("demo", &manifest).expect("BUG-SR6 / SR7 均应满足");
    assert!(
        md.starts_with("---\n"),
        "SKILL.md 必须以 frontmatter 起始标记开头（不是引号），实际开头：{:?}",
        &md[..md.len().min(12)]
    );
    // 关键断言：落盘文本经同一份摘要解析能还原出填写的字段
    let n = node_of("demo", Some(&md));
    assert_eq!(n.title, "demo");
    assert_eq!(n.description.as_deref(), Some("一个用于演示的技能描述"));
}

/// 新建链路：最小清单同样必须产出合法 SKILL.md（否则一新建就是坏文件）
#[test]
fn new_manifest_passes_validation() {
    let m = new_manifest("demo");
    let md = validate_manifest("demo", &m).expect("新建的最小清单必须合法");
    assert!(md.starts_with("---\n"));
    let n = node_of("demo", Some(&md));
    assert_eq!(n.title, "demo");
    assert!(n.description.is_some());
}

/// 集中实现接上真实磁盘：写 → 列 → 读 → 删全程往返
/// （`read` 给出**表单形状**，Markdown 原文走条目内部地址）
#[tokio::test]
async fn store_roundtrip_through_the_dir_impl() {
    let tmp = tempfile::tempdir().unwrap();
    let s = DirVdfs::at(tmp.path().join("plugins/skill"), PLUGIN_SKILL, MANIFEST);
    let ctx = VdfsContext::empty();

    let md = validate_manifest("demo", &new_manifest("demo")).unwrap();
    s.write_text("demo", &md).await.unwrap();

    let entries = s.entries().await.unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(
        node_of(&entries[0].id, entries[0].raw.as_deref()).title,
        "demo"
    );
    assert_eq!(s.read_text("demo").await.unwrap(), md);
    assert!(
        s.list(&ctx, "").await.unwrap()[0].is_dir(),
        "条目内部可下钻"
    );
    assert_eq!(
        s.read(&ctx, "demo/SKILL.md").await.unwrap().as_text(),
        Some(md.as_str()),
        "原文地址读到的就是落盘原文"
    );

    s.remove("demo").await.unwrap();
    assert!(s.entries().await.unwrap().is_empty());
}

/// 「点新建」必须直接进入该类型的详情页（用户第 1 / 2 点）。
///
/// 与 model 同构：`ext = skill` 是**呈现扩展名**（`id_of` 按它剥地址后缀），
/// 落成后的节点 `ext = form` ⇒ 必须显式声明 `node_ext` 与 `schema`。
#[test]
fn new_type_declares_the_landing_detail() {
    let plugin = SkillPlugin {
        config: Arc::new(RwLock::new(SkillConfig::default())),
    };
    let types = plugin.root_new_types();
    let form = types
        .iter()
        .find(|t| t.ext == PLUGIN_SKILL)
        .expect("根下可新建「技能」");
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
    assert!(pack.node_ext.is_none() && pack.schema.is_none());
}
