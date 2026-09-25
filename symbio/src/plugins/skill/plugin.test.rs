//! `plugin.rs` 的单元测试（skill Provider 的 VDFS 行为）。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）：
//! `plugin.rs` 只保留生产代码，测试全部放本文件。

use super::*;

/// 路径末段才是 id：`<根>/skill/<id>.skill` 与裸 `<id>` 同解
#[test]
fn id_of_strips_presentation_extension() {
    assert_eq!(id_of("demo"), "demo");
    assert_eq!(id_of("demo.skill"), "demo");
}

/// 导入的**目标名**取自包的文件名（不是目标地址）——去掉 `.zip` 即得。
///
/// 导入改走详情页动作后，地址不再承载「导入到哪个名字下」（动作打在挂载根上），
/// 名字改由载荷里的 `filename` 推导；`VdfsUnpack::name_of` 是这条推导的唯一实现。
#[test]
fn unpack_name_comes_from_the_filename() {
    let pack = |f: &str| crate::providers::vdfs_service::VdfsUnpack {
        filename: f.into(),
        b64: String::new(),
    };
    assert_eq!(pack("demo.zip").name_of(PLUGIN_SKILL), "demo");
    assert_eq!(pack("demo.skill").name_of(PLUGIN_SKILL), "demo");
    assert_eq!(pack("demo").name_of(PLUGIN_SKILL), "demo");
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

/// 无 frontmatter：标题回落条目 id、无摘要（不猜标题行）
#[test]
fn node_without_frontmatter_falls_back_to_the_id() {
    let md = "# 没有 frontmatter\n\n正文\n";
    let n = node_of("old", Some(md));
    assert_eq!(n.title, "old");
    assert!(n.description.is_none());
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
    let s = DirVdfs::at(tmp.path().join("skill"), PLUGIN_SKILL, MANIFEST);
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
        s.dispatch(
            &ctx,
            "",
            VdfsRequest::List {
                limit: None,
                before: None
            }
        )
        .await
        .unwrap()
        .into_list()
        .unwrap()[0]
            .node
            .is_dir(),
        "条目内部可下钻"
    );
    assert_eq!(
        s.dispatch(&ctx, "demo/SKILL.md", VdfsRequest::Read)
            .await
            .unwrap()
            .into_read()
            .unwrap()
            .as_text(),
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
///
/// 类型位**只描述落成后的样子**：整包导入是详情页的一条动作
/// （`VDFS_ACTION_IMPORT`），不是这里的一个字段——故类型上没有任何导入痕迹。
#[tokio::test]
async fn new_type_declares_the_landing_detail() {
    let plugin = SkillPlugin {
        config: Arc::new(RwLock::new(SkillConfig::default())),
        dir: PluginDir::at(std::env::temp_dir().join("symbio-test/skill"), PLUGIN_SKILL),
    };
    // 「根下可新建类型」挂在**根节点自己的自述**上（不在同步的 `PluginMeta` 上）：
    // 走 `Stat("")`，与更深层节点同一条通道（`VdfsNode::new_type`）。
    let root = plugin
        .dispatch(&VdfsContext::empty(), "", VdfsRequest::Stat)
        .await
        .unwrap()
        .into_stat()
        .expect("根节点自述");
    let t = root.new_type.expect("根下可新建「技能」");
    assert_eq!(t.ext, PLUGIN_SKILL, "呈现扩展名是技能自己的，不是包的");
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
    assert!(id.starts_with("skill-"), "带类别前缀便于人读：{id}");
    assert_eq!(id.len(), "skill-".len() + 8, "随机段定长：{id}");
}

/// `{HOMEDIR}/skills` 必须解析为**本作用域**的系统根，而不是全局 homedir。
///
/// 回归：子 Agent 里 skill 挂在 `<agent dir>/skill`，作用域根是 `<agent dir>`；
/// 早先这里读全局 `HomedirRegistry`，于是解析成系统级 `<homedir>/skills`——
/// 父/系统作用域的技能目录，与 `load_skills_for_tool` 的「子树自包含」相抵。
#[test]
fn homedir_placeholder_resolves_to_scope_root() {
    let scope_root = std::path::Path::new("scope_root");
    let expected = scope_root.join("skills").to_string_lossy().to_string();
    let mut dirs = vec!["{HOMEDIR}/skills".to_string(), "other".to_string()];
    SkillPlugin::resolve_skill_dirs_template(&mut dirs, scope_root);
    assert_eq!(dirs[0], expected, "占位符必须落在传入的作用域根下");
    assert_eq!(dirs[1], "other", "无占位符的条目原样保留");
}
