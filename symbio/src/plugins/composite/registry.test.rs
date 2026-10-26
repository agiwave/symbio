//! `symbio/src/plugins/composite/registry.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。
//!
//! 注册表是装配态的**权威**（三条判据 + 启停 / 安装 / 卸载）。这些测试因此都在
//! **真目录**上做：判据的来源就是磁盘，用内存替身去测它等于把被测对象换掉。
//! 每个用例自建一棵一次性插件根（[`temp_root`]），跑完删掉。

use super::*;

use crate::assembly::{PLUGIN_ID_LOCAL, PLUGIN_ID_MANAGER, PLUGIN_ID_SESSION};
use crate::symbio_core::{PluginError, PluginPayload};
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
        Arc::new(RwLock::new(IndexMap::new())),
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

/// **身份来自 manifest，不依赖被构造**（D-032）：未挂载也有名字
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

// ==================== 装配失败的三类归因 ====================
//
// 「插件不见了」原本是**一种**现象、**三种**成因，而日志之外谁也分不出来：
// 工厂没编进这个构建 / 工厂编进来了但它不是插件 / 工厂是插件而它构造时 panic。
// 这三条各有一处判据（前两处读工厂表，第三处在调用点接 panic），
// 各自都要在条目上留下自己的那句话——`mount_error` 就是那三个落点。

/// 未注册：`PLUGIN.yml` 指向一个本构建没有的工厂（旧构建留下的目录是最常见的来路）
#[tokio::test]
async fn unregistered_provider_leaves_a_reason_on_the_entry() {
    let root = temp_root("fail-unregistered");
    put_plugin(
        &root,
        "alpha",
        r#"{"plugin_provider": "no_such_factory_in_this_build"}"#,
    );

    let reg = registry_at(&root, Vec::new());
    reg.mount_all();

    let entries = reg.entries();
    let entry = &entries[0];
    assert!(!entry.mounted, "没挂上");
    let reason = entry.mount_error.clone().unwrap_or_default();
    assert!(
        reason.contains("未注册"),
        "原因要说是未注册，而不是「请查看日志」：{reason}"
    );

    // 停用之后再读：它是「停用」，不是「失败」——压根没被尝试过的东西没有失败原因
    reg.set_enabled("alpha", false).await.unwrap();
    let entries = reg.entries();
    let entry = &entries[0];
    assert!(!entry.enabled);
    assert_eq!(entry.mount_error, None, "停用位的条目不投影失败原因");

    let _ = std::fs::remove_dir_all(&root);
}

/// 产物不是插件：id 在工厂表里，但它申报的是别的类型族（同一张表也住模型协议）
#[tokio::test]
async fn a_non_plugin_factory_is_told_apart_from_an_unregistered_one() {
    let root = temp_root("fail-wrong-kind");
    // `openai_chat` 是**真**工厂 id，但它注册的是 `dyn ModelProtocol`（见
    // `plugins/model/protocols/mod.rs`）——正好是「工厂在、产物不是插件」那一条。
    put_plugin(&root, "alpha", r#"{"plugin_provider": "openai_chat"}"#);

    let reg = registry_at(&root, Vec::new());
    reg.mount_all();

    let entries = reg.entries();
    let entry = &entries[0];
    let reason = entry.mount_error.clone().unwrap_or_default();
    assert!(
        reason.contains("不是插件"),
        "「工厂存在但装不出插件」要说成另一类，不能混进未注册：{reason}"
    );
    assert!(
        !reason.contains("未注册"),
        "两类归因不能共用一条措辞：{reason}"
    );

    // 运行期按下「启用」同一条款要回给用户（而不是让他去翻日志）
    let err = reg.set_enabled("alpha", true).await.unwrap_err();
    assert!(err.contains("不是插件"), "启用的报错复用同一条归因：{err}");

    let _ = std::fs::remove_dir_all(&root);
}

/// 构造 panic：接住它，换一个插件的一句失败原因——容器不跟着倒
#[tokio::test]
async fn a_panicking_factory_is_caught_and_attributed() {
    let root = temp_root("fail-panic");
    put_plugin(
        &root,
        "alpha",
        r#"{"plugin_provider": "registry-test-panic-factory"}"#,
    );

    let reg = registry_at(&root, Vec::new());
    reg.mount_all();

    let entries = reg.entries();
    let entry = &entries[0];
    assert!(!entry.mounted, "panic 的插件不该进实例表");
    let reason = entry.mount_error.clone().unwrap_or_default();
    assert!(reason.contains("构造时 panic"), "第三类归因：{reason}");
    assert!(
        reason.contains("故意 panic"),
        "panic 的原文要带出来，否则等于没说：{reason}"
    );

    // 同一个注册表里其余目录照常装配：一家 panic 不是整棵树的理由
    put_plugin(
        &root,
        "beta",
        r#"{"plugin_provider": "no_such_factory_in_this_build"}"#,
    );
    reg.mount_all();
    let names: Vec<String> = reg.entries().into_iter().map(|e| e.name).collect();
    assert!(names.iter().any(|n| n == "beta"), "失败一家不拦其余目录");

    let _ = std::fs::remove_dir_all(&root);
}

/// 故意 panic 的工厂：只为第三条归因而存在（`inventory` 只在测试构建里收集它）
fn build_panics(_ctx: Arc<dyn PluginInvokeRequest>) -> Arc<dyn Plugin> {
    panic!("测试工厂故意 panic")
}

crate::submit_object_creator!("registry-test-panic-factory", build_panics, dyn Plugin);

// ==================== 装配期：出厂身份落位（D-032） ====================

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
    let mut instances: IndexMap<String, Arc<dyn Plugin>> = IndexMap::new();
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

// ==================== 快照次序 = 能力收集的广播序 ====================

/// `snapshot()` 的次序是**挂载次序**，不是哈希迭代序。
///
/// 消费方是 `Composite::traverse`：它按这张表的次序向子插件广播收集，而
/// 系统提示词行的「谁在前谁在后 = 注册顺序」直接吃这条序。
/// 表写成 `HashMap` 时迭代序按进程随机化 ⇒ **系统提示词里各记忆小节的相对顺序每起一个进程
/// 重洗一次**（`e2e/cases/t46` 的形状基线实测：同一份装配跑 20 次，13 次是另一种顺序）。
///
/// 判据因此钉在最小的那一步：插一张**逆着字母序**的表，读回来还得是这个顺序。
/// 哪天这里换回哈希表、或给 `snapshot()` 加了排序，本条先红。
#[test]
fn snapshot_keeps_mount_order_not_hash_order() {
    let root = temp_root("snapshot-order");
    let mut instances: IndexMap<String, Arc<dyn Plugin>> = IndexMap::new();
    for name in ["zebra", "mid", "alpha"] {
        instances.insert(name.to_string(), Arc::new(NopPlugin));
    }
    let reg = PluginRegistry::with_instances(Arc::new(RwLock::new(instances)), root, Vec::new());

    let names: Vec<String> = reg.snapshot().into_iter().map(|(n, _)| n).collect();
    assert_eq!(
        names,
        vec!["zebra".to_string(), "mid".to_string(), "alpha".to_string()],
        "快照次序 = 挂载次序（字母序说明被重排过，其它顺序说明是哈希迭代序）"
    );
    // 停用 / 卸载摘除走的也是保序的 `shift_remove`（见 `set_enabled` / `uninstall`）：
    // 那两条要落配置位、删目录，不是本条判据的对象，故不在这里演一遍。
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

    let mut instances: IndexMap<String, Arc<dyn Plugin>> = IndexMap::new();
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

    let mut instances: IndexMap<String, Arc<dyn Plugin>> = IndexMap::new();
    instances.insert("alpha".to_string(), Arc::new(NopPlugin));
    let reg =
        PluginRegistry::with_instances(Arc::new(RwLock::new(instances)), root.clone(), Vec::new());

    reg.uninstall("alpha").await.unwrap();
    assert!(!root.join("alpha").exists(), "插件目录随之消失");
    assert!(!lock_read(reg.instances()).contains_key("alpha"));

    let _ = std::fs::remove_dir_all(&root);
}
