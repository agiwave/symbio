//! `symbio/src/plugins/composite/registry.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。
//!
//! 注册表是装配态的**权威**（三条判据 + 启停 / 安装 / 卸载）。这些测试因此都在
//! **真目录**上做：判据的来源就是磁盘，用内存替身去测它等于把被测对象换掉。
//! 每个用例自建一棵一次性插件根（[`temp_root`]），跑完删掉。

use super::*;

use crate::symbio_core::{
    PluginError, PluginPayload, PLUGIN_ID_LOCAL, PLUGIN_ID_MANAGER, PLUGIN_ID_SESSION,
};
use std::path::Path;

/// 一次性临时插件根（目录名带进程号，避免并行测试互相踩）
fn temp_root(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("symbio-registry-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

/// 在插件根下造一个插件目录（`manifest` = `PLUGIN.yml` 的正文）
fn put_plugin(root: &Path, name: &str, manifest: &str) {
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(PLUGIN_FILE), manifest).unwrap();
}

/// 空实例表的注册表（判据只看磁盘时用它）
fn registry_at(root: &Path, required: Vec<String>) -> PluginRegistry {
    PluginRegistry::with_instances(
        Arc::new(RwLock::new(HashMap::new())),
        root.to_path_buf(),
        required,
    )
}

// ==================== 注册表全表 ====================

/// 「合格」= 有 `PLUGIN.yml` 且声明了非空 `plugin_provider`——**不要求**工厂已注册
///
/// 不合规的目录（没 manifest / 配了一半）不进表：它们既不是插件候选，也没有可显示
/// 的身份。而**已停用**的必须进表——否则用户看不到自己刚停用的那个，也就永远点不回
/// 「启用」。
#[tokio::test]
async fn entries_cover_qualified_dirs_including_disabled_ones() {
    let root = temp_root("entries");
    put_plugin(&root, "alpha", r#"{"plugin_provider": "alpha"}"#);
    put_plugin(
        &root,
        "beta",
        r#"{"plugin_provider": "beta", "plugin_enabled": false}"#,
    );
    put_plugin(&root, "broken", r#"{"plugin_title": "配了一半"}"#);
    std::fs::create_dir_all(root.join("notes")).unwrap();
    std::fs::write(root.join("README.md"), "不是目录").unwrap();

    let entries = registry_at(&root, vec!["alpha".to_string()]).entries();
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["alpha", "beta"],
        "只有合格目录进表，按 (order, 名字) 排"
    );

    assert!(entries[0].enabled, "装配位缺省 = 启用");
    assert!(entries[0].required, "构造者声明的必需插件");
    assert!(!entries[0].mounted, "没有实例 = 未挂载（停用或构造失败）");
    assert!(!entries[1].enabled, "显式停用");
    assert!(!entries[1].required);

    let _ = std::fs::remove_dir_all(&root);
}

/// **身份来自 manifest，不依赖被构造**（ADR-032）：未挂载也有名字
///
/// 这正是「停用后仍能在插件列表里看到它叫什么」的机制——停用不构造，而身份早已
/// 在首次装配时落位。
#[tokio::test]
async fn identity_comes_from_manifest_even_when_not_mounted() {
    let root = temp_root("identity");
    put_plugin(
        &root,
        "alpha",
        r#"{"plugin_provider": "alpha", "plugin_title": "甲",
            "plugin_description": "第一个", "plugin_version": "2.0.0"}"#,
    );

    let entries = registry_at(&root, Vec::new()).entries();
    assert_eq!(entries.len(), 1);
    assert!(!entries[0].mounted, "没有实例 = 未挂载");
    assert_eq!(entries[0].title, "甲", "未挂载也有身份");
    assert_eq!(entries[0].description.as_deref(), Some("第一个"));
    assert_eq!(entries[0].version.as_deref(), Some("2.0.0"));
    assert_eq!(
        entries[0].order,
        PluginMeta::default().order,
        "`order` 是挂载点呈现，未挂载时仍取缺省（与身份不同，这是刻意的）"
    );

    let _ = std::fs::remove_dir_all(&root);
}

/// 从未落位过的目录（manifest 里没有身份键）：身份为空，消费方按目录名兜底
#[tokio::test]
async fn identity_is_empty_for_never_seeded_dirs() {
    let root = temp_root("nometa");
    put_plugin(&root, "alpha", r#"{"plugin_provider": "alpha"}"#);

    let entries = registry_at(&root, Vec::new()).entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].title, "");
    assert_eq!(entries[0].description, None);
    assert_eq!(entries[0].version, None);

    let _ = std::fs::remove_dir_all(&root);
}

// ==================== 装配期：出厂身份落位（ADR-032） ====================

/// 装配期**真的**把出厂身份投影进 manifest
///
/// 只测 `PluginDir::seed_identity` 本身不够——它由容器的装配路径调用，那条链断了，
/// 身份就永远不落位（而「停用插件也有身份」全靠它）。所以这里走**真实**的
/// `mount_all`：真目录 → 真工厂 → 真构造。
#[tokio::test]
async fn mounting_seeds_the_identity_into_the_manifest() {
    let root = temp_root("seed");
    put_plugin(
        &root,
        "seeded",
        r#"{"plugin_provider": "symbio-test-seed"}"#,
    );

    let reg = registry_at(&root, Vec::new());
    reg.mount_all();

    assert!(
        lock_read(reg.instances()).contains_key("seeded"),
        "工厂已注册且未停用 ⇒ 应已构造"
    );
    let id = reg.dir_of("seeded").identity();
    assert_eq!(id.title, "种子插件", "出厂身份落进 manifest");
    assert_eq!(id.version.as_deref(), Some("9.9.9"));

    let _ = std::fs::remove_dir_all(&root);
}

/// 装配期投影用的工厂插件：只为了在**真实** `mount_all` 里被构造一次
struct SeedPlugin;

impl SeedPlugin {
    fn build(_ctx: Arc<dyn PluginInvokeRequest>) -> Arc<dyn Plugin> {
        Arc::new(Self)
    }
}

#[async_trait::async_trait]
impl Plugin for SeedPlugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta::new("symbio-test-seed", "种子插件").with_version("9.9.9")
    }

    async fn route(
        self: Arc<Self>,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> crate::symbio_core::PluginInvokeResponse<PluginPayload> {
        Err(PluginError::NotFound("seed".to_string()))
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> crate::symbio_core::PluginInvokeResponse<PluginPayload> {
        Ok(PluginPayload::Empty)
    }
}

// 测试专用工厂：不占用任何生产 id，`installable()` 的断言因此不受影响
crate::submit_object_creator!("symbio-test-seed", SeedPlugin::build, dyn Plugin);

// ==================== 可安装清单 ====================

/// 可安装 = 已注册、但当前**没有挂载实例**的工厂；系统级工厂排除在外
///
/// 系统级（`home` / `composite`）的目录就是系统根本身，它们不是「装在某个目录下的
/// 插件」；而**停用**的插件在候选里——那正是「添加插件」的语义（把它重新装配进来）。
#[tokio::test]
async fn installable_excludes_mounted_and_system_level() {
    let root = temp_root("installable");
    let mut instances: HashMap<String, Arc<dyn Plugin>> = HashMap::new();
    // 用 `composite` 自己当「已挂载」的实例：它一定已注册，且无需构造具体插件
    instances.insert("local".to_string(), Arc::new(NopPlugin));
    let reg =
        PluginRegistry::with_instances(Arc::new(RwLock::new(instances)), root.clone(), Vec::new());

    let ids = reg.installable();
    for system in SYSTEM_LEVEL_PROVIDERS {
        assert!(
            !ids.contains(system),
            "系统级工厂不可作为子插件装配：{system}"
        );
    }
    assert!(!ids.contains(&PLUGIN_ID_LOCAL), "已挂载的不在候选里");
    assert!(
        ids.contains(&PLUGIN_ID_SESSION),
        "已注册未挂载的工厂应在候选里"
    );
    assert!(
        ids.contains(&PLUGIN_ID_MANAGER),
        "插件管理插件在注册表眼里也是普通工厂：它没被挂载时同样可添加"
    );
    assert!(
        ids.windows(2).all(|w| w[0] <= w[1]),
        "顺序稳定（工厂 id 升序），界面不随枚举顺序抖"
    );

    let _ = std::fs::remove_dir_all(&root);
}

/// 什么都答 `NotFound` 的占位插件（只为了让实例表里有一项）
struct NopPlugin;

#[async_trait::async_trait]
impl Plugin for NopPlugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta::new("nop", "nop")
    }

    async fn route(
        self: Arc<Self>,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> crate::symbio_core::PluginInvokeResponse<crate::symbio_core::PluginPayload> {
        Err(crate::symbio_core::PluginError::NotFound("nop".to_string()))
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> crate::symbio_core::PluginInvokeResponse<crate::symbio_core::PluginPayload> {
        Ok(crate::symbio_core::PluginPayload::new(&Vec::<
            serde_json::Value,
        >::new()))
    }
}

// ==================== 停用：界面底座插件不可关 ====================

/// 界面底座插件（[`ASSEMBLY_UNDISABLABLE_PLUGINS`]）拒绝停用——停掉它就没有界面再把它打开
#[tokio::test]
async fn disabling_an_undisablable_plugin_is_refused() {
    let root = temp_root("undisablable");
    let reg = registry_at(&root, Vec::new());

    for name in ASSEMBLY_UNDISABLABLE_PLUGINS {
        let err = reg.set_enabled(name, false).await.unwrap_err();
        assert!(err.contains("界面底座"), "{name} 应被拒绝停用：{err}");
    }

    let _ = std::fs::remove_dir_all(&root);
}

/// 停用 = 写装配位 + 摘掉实例；**不删任何东西**（删除是卸载的语义）
#[tokio::test]
async fn disabling_writes_the_flag_and_drops_the_instance() {
    let root = temp_root("disable");
    put_plugin(&root, "alpha", r#"{"plugin_provider": "alpha"}"#);

    let mut instances: HashMap<String, Arc<dyn Plugin>> = HashMap::new();
    instances.insert("alpha".to_string(), Arc::new(NopPlugin));
    let reg =
        PluginRegistry::with_instances(Arc::new(RwLock::new(instances)), root.clone(), Vec::new());

    reg.set_enabled("alpha", false).await.unwrap();

    assert!(
        !PluginDir::at(root.join("alpha"), "alpha").enabled(),
        "装配位写回磁盘"
    );
    assert!(
        !lock_read(reg.instances()).contains_key("alpha"),
        "实例表随之摘掉：停用不是「构造了但不显示」"
    );
    assert!(root.join("alpha").exists(), "停用不删目录（那是卸载）");

    let _ = std::fs::remove_dir_all(&root);
}

// ==================== 卸载 ====================

/// 必需插件不可删除（可以停用）；不存在的插件如实报错
#[tokio::test]
async fn uninstall_refuses_required_and_unknown_plugins() {
    let root = temp_root("uninstall");
    put_plugin(&root, "alpha", r#"{"plugin_provider": "alpha"}"#);
    let reg = registry_at(&root, vec!["alpha".to_string()]);

    let err = reg.uninstall("alpha").await.unwrap_err();
    assert!(err.contains("必需插件"), "必需插件只能停用：{err}");
    assert!(root.join("alpha").exists(), "拒绝时不得动目录");

    assert!(reg
        .uninstall("ghost")
        .await
        .unwrap_err()
        .contains("未找到插件"));

    let _ = std::fs::remove_dir_all(&root);
}

/// 非必需插件：卸载 = 摘实例 + **删掉插件目录**（连同里面的配置与数据）
#[tokio::test]
async fn uninstall_removes_the_instance_and_the_directory() {
    let root = temp_root("uninstall-ok");
    put_plugin(&root, "alpha", r#"{"plugin_provider": "alpha"}"#);

    let mut instances: HashMap<String, Arc<dyn Plugin>> = HashMap::new();
    instances.insert("alpha".to_string(), Arc::new(NopPlugin));
    let reg =
        PluginRegistry::with_instances(Arc::new(RwLock::new(instances)), root.clone(), Vec::new());

    reg.uninstall("alpha").await.unwrap();
    assert!(!root.join("alpha").exists(), "插件目录随之消失");
    assert!(!lock_read(reg.instances()).contains_key("alpha"));

    let _ = std::fs::remove_dir_all(&root);
}
