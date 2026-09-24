//! `symbio/src/symbio_core/plugin_dir.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

use crate::symbio_core::schemas::detail::{DetailField, DetailSection};
use serde_json::json;

fn definition() -> DetailDefinition {
    let mut port = DetailField {
        key: "port".into(),
        label: "端口".into(),
        widget: "number".into(),
        min: Some(1.0),
        max: Some(65535.0),
        ..Default::default()
    };
    port.required = true;
    DetailDefinition {
        sections: vec![DetailSection {
            title: None,
            collapsed: false,
            fields: vec![port],
        }],
        ..Default::default()
    }
}

fn dir_at(tmp: &Path) -> PluginDir {
    PluginDir::at(tmp, "demo")
}

/// 布局：插件目录 + `PLUGIN.yml`，配置与数据同处一处
#[test]
fn plugin_dir_is_the_plugin_home() {
    let tmp = tempfile::TempDir::new().unwrap();
    let d = dir_at(tmp.path());

    assert_eq!(d.dir(), tmp.path());
    assert_eq!(d.config_path(), tmp.path().join(PLUGIN_FILE));
    assert_eq!(d.name(), "demo");
    assert_eq!(d.provider(), "demo");
}

/// 身份字段与配置字段同处一个文件，但**只有配置**参与反序列化
#[test]
fn identity_fields_are_stripped_and_restored() {
    let tmp = tempfile::TempDir::new().unwrap();
    let d = dir_at(tmp.path()).with_provider("demo_provider");

    #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
    struct C {
        port: u32,
    }

    d.save(&C { port: 8080 }).unwrap();

    // 落盘形态：身份字段在前，配置字段平级
    let text = std::fs::read_to_string(d.config_path()).unwrap();
    assert!(text.contains("plugin_provider: demo_provider"), "{text}");
    assert!(text.contains("plugin_name: demo"), "{text}");
    assert!(text.contains("port: 8080"), "{text}");

    // 读回来只有配置——身份字段不会撞进配置结构
    assert_eq!(d.load::<C>().unwrap(), Some(C { port: 8080 }));
}

/// 文件不存在 → `None`；空文件按「没有配置」处理，不算错误
#[test]
fn missing_and_empty_config_are_both_none() {
    let tmp = tempfile::TempDir::new().unwrap();
    let d = dir_at(tmp.path());

    assert_eq!(d.read_manifest().unwrap(), None);
    assert_eq!(d.load::<Map<String, Value>>().unwrap(), None);

    std::fs::write(d.config_path(), "").unwrap();
    assert_eq!(d.read_manifest().unwrap(), None);
}

/// `ensure_manifest` 只补身份字段，**不覆盖**已有配置
#[test]
fn ensure_never_overwrites_existing_config() {
    let tmp = tempfile::TempDir::new().unwrap();
    let d = dir_at(tmp.path());

    #[derive(serde::Serialize, serde::Deserialize, Default, PartialEq, Debug)]
    struct C {
        #[serde(default)]
        port: u32,
    }

    // 目录都不存在时也能补出来；只有身份字段 → 缺省字段由插件自己的 Default 兜底
    d.ensure_manifest().unwrap();
    let manifest = d.read_manifest().unwrap().unwrap();
    assert_eq!(manifest[KEY_PROVIDER], json!("demo"));
    assert_eq!(manifest[KEY_NAME], json!("demo"));
    assert_eq!(d.load::<C>().unwrap(), Some(C::default()));

    d.save(&C { port: 8080 }).unwrap();
    d.ensure_manifest().unwrap();
    assert_eq!(d.load::<C>().unwrap(), Some(C { port: 8080 }));
}

/// `remove_keys`：遗留字段清理后，身份字段仍在；键不存在则不动文件
#[test]
fn remove_keys_drops_legacy_fields_but_keeps_identity() {
    let tmp = tempfile::TempDir::new().unwrap();
    let d = dir_at(tmp.path());

    // 旧形态：资源明细混在配置里
    std::fs::write(
        d.config_path(),
        "plugin_provider: demo\nplugin_name: demo\nservers:\n  a: 1\n_storage: plugins/x\n",
    )
    .unwrap();

    d.remove_keys(&["servers"]).unwrap();
    let text = std::fs::read_to_string(d.config_path()).unwrap();
    assert!(!text.contains("servers"), "{text}");
    assert!(
        text.contains("_storage: plugins/x"),
        "未点名的键不动：{text}"
    );
    assert!(text.contains("plugin_provider: demo"), "{text}");

    // 再摘一个不存在的键：文件不动
    let before = std::fs::read_to_string(d.config_path()).unwrap();
    d.remove_keys(&["nope"]).unwrap();
    assert_eq!(std::fs::read_to_string(d.config_path()).unwrap(), before);
}

/// 节点：真实文件名 + `ext = form` + `rw` + `schema` 即定义
#[test]
fn config_node_is_a_form_document_named_by_the_real_file() {
    let tmp = tempfile::TempDir::new().unwrap();
    let f = ConfigFile::new(dir_at(tmp.path()), "演示设置", definition());
    let n = f.node();

    assert_eq!(n.name, PLUGIN_FILE, "地址就是真实文件名，不是保留段");
    assert_eq!(n.title, "演示设置");
    assert_eq!(n.ext.as_deref(), Some(VDFS_EXT_FORM));
    assert_eq!(n.access.flags(), "rw");
    assert!(!n.is_dir(), "配置是文件而非目录");
    assert_eq!(
        n.schema.as_ref().unwrap()["sections"][0]["fields"][0]["key"],
        "port"
    );
}

/// 解码即校验：越界 / 缺必填 / 非 JSON 都拦在这里，且是**字段级**错误
#[test]
fn decode_validates_through_the_definition() {
    let tmp = tempfile::TempDir::new().unwrap();
    let f = ConfigFile::new(dir_at(tmp.path()), "演示", definition());

    let field_of = |e: VdfsError| match e {
        VdfsError::Invalid(v) => v.fields[0].field.clone(),
        other => panic!("应为字段级校验错误，实得 {other:?}"),
    };

    let bad = VdfsContent::text("", r#"{"port": 70000}"#);
    assert_eq!(field_of(f.decode(&bad).unwrap_err()), "port");

    let missing = VdfsContent::text("", "{}");
    assert_eq!(field_of(f.decode(&missing).unwrap_err()), "port");

    let broken = VdfsContent::text("", "{oops");
    assert!(f.decode(&broken).is_err());

    let ok = VdfsContent::text("", r#"{"port": 8080}"#);
    assert_eq!(f.decode(&ok).unwrap(), json!({ "port": 8080 }));
}

/// 读：配置槽 → JSON 文本内容
#[tokio::test]
async fn read_serializes_the_slot() {
    let tmp = tempfile::TempDir::new().unwrap();
    let f = ConfigFile::new(dir_at(tmp.path()), "演示", definition());
    let slot = RwLock::new(json!({ "port": 8080 }));

    let content = f.read(&slot).await.unwrap();
    let text = content.text.unwrap();
    assert!(text.contains("\"port\": 8080"), "应为 pretty JSON：{text}");
    assert_eq!(content.mime.as_deref(), Some("application/json"));
}

/// 写的一条链：**校验 → 落内存 → 落自己的文件**。
///
/// 锁定本方案的要点：落盘**不再经过父插件**——写的就是自己目录里的
/// `PLUGIN.yml`，校验未过则内存与磁盘都不动。
#[tokio::test]
async fn apply_writes_the_plugins_own_file() {
    let tmp = tempfile::TempDir::new().unwrap();
    let f = ConfigFile::new(dir_at(tmp.path()), "演示", definition());
    let slot = RwLock::new(json!({ "port": 1 }));

    // 校验失败：内存与磁盘都不动
    let bad = VdfsContent::text("", r#"{"port": 70000}"#);
    assert!(f.apply(&slot, &bad).await.is_err());
    assert_eq!(*slot.read().await, json!({ "port": 1 }));
    assert!(!f.dir().config_path().exists(), "校验未过不该落盘");

    // 成功：内存生效 + 文件落在自己的目录里
    let resp = f
        .apply(&slot, &VdfsContent::text("", r#"{"port": 8080}"#))
        .await
        .unwrap();
    assert_eq!(resp.path, PLUGIN_FILE);
    assert_eq!(*slot.read().await, json!({ "port": 8080 }));

    let text = std::fs::read_to_string(f.dir().config_path()).unwrap();
    assert!(text.contains("port: 8080"), "{text}");
    assert!(text.contains("plugin_provider: demo"), "{text}");
}
