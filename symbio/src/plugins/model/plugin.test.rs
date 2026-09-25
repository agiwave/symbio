//! `plugin.rs` 的单元测试（model Provider 的 VDFS 行为与启动加载）。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）：
//! `plugin.rs` 只保留生产代码，测试全部放本文件。

use super::*;

/// 路径末段才是 id：`<根>/model/<id>.model` 与裸 `<id>` 同解
#[test]
fn id_of_strips_presentation_extension() {
    assert_eq!(ModelPlugin::id_of("openai-1"), "openai-1");
    assert_eq!(ModelPlugin::id_of("openai-1.model"), "openai-1");
    assert_eq!(ModelPlugin::id_of("a/b/openai-1.model"), "openai-1");
}

/// 新建的最小清单必须带 `skip_validation`：此时用户还没填 key / base，
/// 走连接校验必然失败（新建态不该被校验挡住）。
#[test]
fn new_manifest_skips_validation() {
    let v = ModelPlugin::default().new_manifest("p1", "p1");
    assert_eq!(v.get("id").and_then(|x| x.as_str()), Some("p1"));
    assert_eq!(
        v.get("skip_validation").and_then(|x| x.as_bool()),
        Some(true)
    );
}

fn sample() -> ModelProviderConfig {
    ModelProviderConfig {
        id: "openai-1".into(),
        name: "我的 OpenAI".into(),
        provider: "openai".into(),
        api_base: "https://api.openai.com/v1".into(),
        model: "gpt-4o".into(),
        ..Default::default()
    }
}

/// `sample()` 的 JSON 形态（落盘原文 / 前端下发的 manifest 都是这个形状）
fn sample_value() -> Value {
    serde_json::to_value(sample()).unwrap()
}

/// 呈现差异（标题 / 副标题 / 状态 / 详情定义）都落在节点上，
/// 且 `ext = form` 时 `schema` 必须随节点下发——否则前端渲染不出表单。
#[test]
fn node_carries_presentation_differential() {
    let n = node_of(&sample(), Some(123));
    assert_eq!(n.name, "openai-1");
    assert_eq!(n.title, "我的 OpenAI");
    assert_eq!(n.kind, PLUGIN_MODEL);
    assert_eq!(n.ext.as_deref(), Some(VDFS_EXT_FORM));
    assert_eq!(n.status, VDFS_STATUS_ACTIVE);
    assert_eq!(n.description.as_deref(), Some("gpt-4o"));
    assert_eq!(n.updated_at, Some(123));
    assert!(n.schema.is_some(), "详情定义必须随节点下发");
    // 停用态映射成 disabled（机制只看 status 字符串，不看 kind）
    let mut off = sample();
    off.enabled = false;
    assert_eq!(node_of(&off, None).status, VDFS_STATUS_DISABLED);
}

/// 清单缺 `id` 时以磁盘段名补全（编辑链路的不变量），已有值原样保留
#[test]
fn config_of_falls_back_to_the_path_segment() {
    let bare = serde_json::json!({
        "provider": "openai",
        "api_base": "https://api.openai.com/v1",
        "model": "gpt-4o",
    })
    .to_string();
    let p = config_of("seg-1", &bare).expect("补 id 后应解析成功");
    assert_eq!(p.id, "seg-1");
    // `name` 的 serde 缺省是 "Default"；显式空串才回落磁盘段名
    assert_eq!(p.name, "Default");
    let blank = serde_json::json!({
        "provider": "openai",
        "api_base": "https://api.openai.com/v1",
        "api_key": null,
        "model": "gpt-4o",
        "name": "",
    })
    .to_string();
    assert_eq!(config_of("seg-2", &blank).unwrap().name, "seg-2");
    // 已有值原样保留
    let full = serde_json::to_string(&sample()).unwrap();
    assert_eq!(config_of("other", &full).unwrap().id, "openai-1");
    assert_eq!(config_of("other", &full).unwrap().name, "我的 OpenAI");
    // 坏清单不是配置：不产出节点（列表宁可少一项）
    assert!(config_of("x", "not json").is_none());
    // 写入路径同样补 id（否则「编辑已有 Provider 保存」必报 missing field `id`）
    assert_eq!(
        with_id(&serde_json::json!({ "provider": "openai" }), "p9")
            .get("id")
            .and_then(|v| v.as_str()),
        Some("p9")
    );
    assert_eq!(
        with_id(&sample_value(), "p9")
            .get("id")
            .and_then(|v| v.as_str()),
        Some("openai-1"),
        "已有 id 不被路径段覆盖"
    );
}

/// 内存镜像是 `list` 的唯一来源：灌进去的条目才列得出来
#[tokio::test]
async fn list_comes_from_the_memory_mirror() {
    let plugin = ModelPlugin::default();
    let ctx = VdfsContext::empty();
    assert!(plugin
        .dispatch(
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
        .unwrap()
        .is_empty());

    plugin
        .entries
        .set("openai-1", serde_json::to_string(&sample()).unwrap());
    // 坏条目降级：列不出来，而不是列一个空壳
    plugin.entries.set("broken", "}}}");
    let nodes = plugin
        .dispatch(
            &ctx,
            "",
            VdfsRequest::List {
                limit: None,
                before: None,
            },
        )
        .await
        .unwrap()
        .into_list()
        .unwrap();
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].node.name, "openai-1");
    assert_eq!(nodes[0].node.title, "我的 OpenAI");
    assert!(
        plugin
            .dispatch(
                &ctx,
                "openai-1",
                VdfsRequest::List {
                    limit: None,
                    before: None
                }
            )
            .await
            .is_err(),
        "叶子资源无子项"
    );
}

/// 「点新建」必须直接进入该类型的详情页（用户第 1 / 2 点）。
///
/// 类型清单里的 `ext` 是**呈现扩展名**（`model`——`id_of` 按它剥地址后缀），
/// 而落成后的节点 `ext = form`。两者不同，所以类型必须显式声明：
///
/// - `node_ext` = 落成后的渲染器键（漏了它，草稿详情页落到 `fallback` 兜底，
///   而不是同一张表单）；
/// - `schema`    = 表单定义（没有它，`form` 渲染器渲染不出任何字段）。
#[tokio::test]
async fn new_type_declares_the_landing_detail() {
    // 「根下可新建类型」挂在**根节点自己的自述**上（不在同步的 `PluginMeta` 上）：
    // 走 `Stat("")`，与更深层节点同一条通道（`VdfsNode::new_type`）。
    let root = ModelPlugin::default()
        .dispatch(&VdfsContext::empty(), "", VdfsRequest::Stat)
        .await
        .unwrap()
        .into_stat()
        .expect("根节点自述");
    let t = root.new_type.expect("根下可新建「模型」");
    assert_eq!(t.ext, PLUGIN_MODEL, "呈现扩展名不变：id_of 仍按它剥后缀");
    assert_eq!(
        t.node_ext.as_deref(),
        Some(VDFS_EXT_FORM),
        "草稿必须与落成后用同一个渲染器"
    );
    assert!(t.schema.is_some(), "没有 schema，表单渲染不出任何字段");
}

/// 无名字新建 = 写挂载点目录自身：id 由本插件生成（用户第 3 点）。
///
/// 目录自身没有可覆盖的目标 ⇒ 必须带 `create` 意图；带了就必须**建得出来**，
/// 不能像原先那样一律 `Invalid("不支持在挂载根上写入")`——那会让「点新建 → 在
/// 详情页填好 → 保存」在 model 这一栏永远失败（前端只能停在草稿上）。
#[test]
fn nameless_write_generates_an_id() {
    // 有名字：末段即 id（呈现扩展名由 `id_of` 剥掉）
    assert_eq!(
        ModelPlugin::resolve_id("openai-1.model", false).unwrap(),
        "openai-1"
    );
    // 无名字又没有 create 意图 → 明确报错，而不是静默建一个
    assert!(
        ModelPlugin::resolve_id("", false).is_err(),
        "目录自身没有可覆盖的目标，必须显式表达 create 意图"
    );
    // 无名字 + create → 生成带前缀、定长、安全的 id
    let id = ModelPlugin::resolve_id("", true).unwrap();
    assert!(id.starts_with("model-"), "带类别前缀便于人读：{id}");
    assert_eq!(id.len(), "model-".len() + 8, "随机段定长：{id}");
}

/// `load_from_storage` 的唯一来源是**本插件自己目录**下的条目：
/// 注册表与内存镜像一次填满，`default_provider_id` 按 `is_default` 标记推定。
#[tokio::test]
async fn load_fills_registry_and_mirror_from_its_own_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let plugin = ModelPlugin::new(
        ModelProvidersConfig::default(),
        PluginDir::at(tmp.path().join("model"), "model"),
    );
    let store = plugin.store();
    let mut marked = sample_value();
    marked["is_default"] = serde_json::json!(true);
    store
        .write_text("openai-1", &serde_json::to_string_pretty(&marked).unwrap())
        .await
        .unwrap();
    // 坏条目：跳过而不是让整批加载失败
    store.write_text("broken", "}}}").await.unwrap();

    let ctx: std::sync::Arc<dyn crate::symbio_core::PluginInvokeRequest> =
        std::sync::Arc::new(crate::symbio_core::PluginSimpleRequest::new(None, None));
    plugin.load_from_storage(&ctx).await;

    assert_eq!(plugin.entries.ids(), vec!["openai-1".to_string()]);
    let providers = plugin.providers.read().await;
    assert_eq!(providers.providers.len(), 1);
    assert_eq!(providers.default_provider_id.as_deref(), Some("openai-1"));
}
