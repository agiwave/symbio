//! `symbio/src/plugins/plugin_manager/plugin.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。
//!
//! 本插件是注册表的**视图**：它的每一个动词都转发给所在容器。因此测试的两端是
//! 「本插件合成/转发了什么」与「容器收到了什么」——用 [`FakeRegistry`] 当容器的
//! 替身，把后者记下来断言。这样这些测试不依赖真的装配一个容器，也不依赖磁盘。

use super::*;

use crate::symbio_core::schemas::detail::DetailDefinition;
use crate::symbio_core::vdfs_provider::{
    VdfsActionResult, VdfsNewType, VdfsWriteResponse, VDFS_STATUS_NONE,
};
use std::collections::HashMap;
use std::sync::Mutex;

// ==================== 容器替身 ====================

/// 容器的 VDFS 替身：只答本插件会问的那几个动词，并把收到的请求记下来。
///
/// 它模拟的正是 `CompositeVdfs` 根上的契约（见 `composite/vdfs.rs`）：
/// `Action(plugins)` 给注册表、`Action(enable|disable)` 作用在载荷里的插件名上、
/// 根上的 `Write` 是安装、`Delete(<插件名>)` 是卸载。
struct FakeRegistry {
    entries: Vec<PluginEntry>,
    /// 收到的启停动作：`(action, payload)`
    acts: Mutex<Vec<(String, Option<Value>)>>,
    /// 收到的卸载路径
    deletes: Mutex<Vec<String>>,
    /// 收到的安装写：`(path, 文本)`
    writes: Mutex<Vec<(String, String)>>,
    /// `Read(<路径>)` 的正文（按路径给；缺省 = `NotFound`）
    configs: HashMap<String, String>,
    /// 安装时回包的名字（= 容器生成的插件名）
    install_name: String,
}

impl FakeRegistry {
    fn new(entries: Vec<PluginEntry>) -> Self {
        Self {
            entries,
            acts: Mutex::new(Vec::new()),
            deletes: Mutex::new(Vec::new()),
            writes: Mutex::new(Vec::new()),
            configs: HashMap::new(),
            install_name: "telegram".to_string(),
        }
    }

    fn with_config(mut self, path: &str, text: &str) -> Self {
        self.configs.insert(path.to_string(), text.to_string());
        self
    }
}

#[async_trait::async_trait]
impl VdfsProvider for FakeRegistry {
    async fn dispatch(
        &self,
        _ctx: &VdfsContext,
        path: &str,
        req: VdfsRequest,
    ) -> VdfsResult<VdfsResponse> {
        match req {
            VdfsRequest::Action { action, payload } => {
                if action == VDFS_ACTION_PLUGINS {
                    return Ok(VdfsResponse::Action(VdfsActionResult {
                        action,
                        ok: true,
                        message: format!("共 {} 个插件", self.entries.len()),
                        data: Some(serde_json::json!({ VDFS_PLUGINS_FIELD: self.entries })),
                    }));
                }
                self.acts.lock().unwrap().push((action.clone(), payload));
                Ok(VdfsResponse::Action(VdfsActionResult {
                    action,
                    ok: true,
                    message: "ok".to_string(),
                    data: None,
                }))
            }
            VdfsRequest::Read => self
                .configs
                .get(path)
                .cloned()
                .map(|t| VdfsResponse::Read(VdfsContent::text(t).with_mime("application/json")))
                .ok_or_else(|| VdfsError::not_found(format!("没有这份配置：{path}"))),
            VdfsRequest::Write { content } => {
                self.writes.lock().unwrap().push((
                    path.to_string(),
                    content.as_text().unwrap_or_default().to_string(),
                ));
                Ok(VdfsResponse::Write(VdfsWriteResponse {
                    name: Some(self.install_name.clone()),
                    created: true,
                    etag: None,
                }))
            }
            VdfsRequest::Delete { .. } => {
                self.deletes.lock().unwrap().push(path.to_string());
                Ok(VdfsResponse::Unit)
            }
            VdfsRequest::Stat => {
                // 容器根的可新建类型：安装表单由**执行安装的那一层**声明，本插件转发它
                let mut n = VdfsNode::dir("", "系统", VdfsAccess::LIST);
                let mut t = VdfsNewType::new(PLUGIN_MANAGER, "插件");
                t.description = Some("从已注册的插件工厂里选一个装进本智能体".to_string());
                n.new_type = Some(Box::new(t));
                Ok(VdfsResponse::Stat(n))
            }
            _ => Err(VdfsError::NotImplemented),
        }
    }
}

// ==================== 夹具 ====================

/// 一条注册表条目（未构造的插件：标题 / 描述 / 版本可为空）
fn entry(name: &str, provider: &str, required: bool, enabled: bool) -> PluginEntry {
    PluginEntry {
        name: name.to_string(),
        provider: provider.to_string(),
        title: format!("{name} 的名字"),
        description: None,
        version: Some("1.0.0".to_string()),
        order: 10,
        required,
        enabled,
        mounted: enabled,
    }
}

fn plugin_with(registry: Arc<FakeRegistry>) -> PluginManagerPlugin {
    PluginManagerPlugin::with_registry_view(Some(registry))
}

/// 无容器可回的插件（未装配 / 单测）
fn plugin_without_registry() -> PluginManagerPlugin {
    PluginManagerPlugin::with_registry_view(None)
}

fn vctx() -> VdfsContext {
    VdfsContext::empty()
}

async fn list(p: &PluginManagerPlugin, ctx: &VdfsContext) -> Vec<VdfsItem> {
    p.dispatch(
        ctx,
        "",
        VdfsRequest::List {
            limit: None,
            before: None,
        },
    )
    .await
    .unwrap()
    .into_list()
    .unwrap()
}

async fn stat(p: &PluginManagerPlugin, ctx: &VdfsContext, path: &str) -> VdfsNode {
    p.dispatch(ctx, path, VdfsRequest::Stat)
        .await
        .unwrap()
        .into_stat()
        .unwrap()
}

/// 条目节点带的定义（`entry_node` 恒写入 `schema`）
fn definition_of(node: &VdfsNode) -> DetailDefinition {
    serde_json::from_value(node.schema.clone().expect("条目应带定义")).expect("定义应可解析")
}

fn action_of<'a>(d: &'a DetailDefinition, id: &str) -> &'a DetailAction {
    d.actions
        .iter()
        .find(|a| a.id == id)
        .unwrap_or_else(|| panic!("条目定义里应有动作 {id}"))
}

/// 读条目，取回注入过投影键的模型
async fn read_model(p: &PluginManagerPlugin, ctx: &VdfsContext, name: &str) -> Value {
    let resp = p
        .dispatch(ctx, name, VdfsRequest::Read)
        .await
        .unwrap()
        .into_read()
        .unwrap();
    serde_json::from_str(resp.as_text().unwrap_or_default()).expect("条目正文应是 JSON")
}

// ==================== 自述 ====================

/// 插件身份：挂载名 = `plugin_manager`，标题与图标供导航使用
#[tokio::test]
async fn metadata_is_the_plugin_manager() {
    let p = plugin_without_registry();
    let meta = p.meta();
    assert_eq!(PLUGIN_MANAGER, "plugin_manager", "插件管理插件的挂载名");
    assert_eq!(meta.name, "插件管理");
    assert_eq!(meta.icon.as_deref(), Some("settings"));
    assert_eq!(meta.order, 6);

    // provider 自述不含挂载名——挂载名由使用方在注册时选定
    let root = stat(&p, &vctx(), "").await;
    assert_eq!(root.name, "", "provider 不知道自己的挂载名");
    assert!(root.is_dir());
}

/// 根可新建 = 添加插件：类型原样转发容器（安装表单只有执行安装的那层说得出来）
#[tokio::test]
async fn root_new_type_is_forwarded_from_the_container() {
    let fake = Arc::new(FakeRegistry::new(Vec::new()));
    let p = plugin_with(Arc::clone(&fake));

    let root = stat(&p, &vctx(), "").await;
    let t = root.new_type.expect("容器声明了可新建类型，应转发过来");
    assert_eq!(t.ext, PLUGIN_MANAGER);
    assert_eq!(t.title, "插件");

    // 容器不可达时按「不可新建」处理：少一个入口，好过给一个必然报错的入口
    let orphan = plugin_without_registry();
    assert!(stat(&orphan, &vctx(), "").await.new_type.is_none());
}

// ==================== 清单 ====================

/// 清单 = 各插件条目在前 + 自有分区垫后
#[tokio::test]
async fn list_puts_plugin_entries_before_own_sections() {
    let fake = Arc::new(FakeRegistry::new(vec![
        entry("alpha", "alpha_factory", false, true),
        entry("beta", "beta_factory", false, false),
    ]));
    let p = plugin_with(fake);

    let items = list(&p, &vctx()).await;
    let names: Vec<&str> = items.iter().map(|it| it.node.name.as_str()).collect();
    assert_eq!(names, vec!["alpha", "beta", "appearance", "about"]);

    // 条目是定义驱动的表单文档，不是目录
    assert_eq!(items[0].node.kind, PLUGIN_MANAGER);
    assert_eq!(items[0].node.ext.as_deref(), Some(VDFS_EXT_FORM));
    assert!(!items[0].node.is_dir(), "条目是文档，不是目录");

    // 自有分区：`ext` 即分区 id（前端按 `ext → 渲染器` 回退到专属面板）
    assert_eq!(items[2].node.ext.as_deref(), Some("appearance"));
    assert_eq!(items[3].node.ext.as_deref(), Some("about"));
}

/// 装配态即节点状态：启用 / 停用各画各的
#[tokio::test]
async fn entry_status_follows_assembly_state() {
    let fake = Arc::new(FakeRegistry::new(vec![
        entry("alpha", "alpha_factory", false, true),
        entry("beta", "beta_factory", false, false),
    ]));
    let p = plugin_with(fake);

    assert_eq!(stat(&p, &vctx(), "alpha").await.status, VDFS_STATUS_ACTIVE);
    assert_eq!(
        stat(&p, &vctx(), "beta").await.status,
        VDFS_STATUS_DISABLED,
        "停用的插件在清单里仍在，但状态是停用"
    );
}

/// 没有配置文档的插件也有一张只读概览——否则点开一片空白，连装配按钮都没有落脚处
#[tokio::test]
async fn entries_without_config_get_a_readonly_overview() {
    let fake = Arc::new(FakeRegistry::new(vec![entry(
        "beta",
        "beta_factory",
        false,
        true,
    )]));
    let p = plugin_with(fake);

    let d = definition_of(&stat(&p, &vctx(), "beta").await);
    assert_eq!(d.binding, "info", "概览是只读的：没有可保存的东西");
    let fields: Vec<&str> = d
        .sections
        .iter()
        .flat_map(|s| s.fields.iter())
        .map(|f| f.key.as_str())
        .collect();
    assert_eq!(fields, vec![KEY_NAME, KEY_PROVIDER, KEY_VERSION]);

    // 标题回落链：配置标题（无）→ 插件元数据的名字
    assert_eq!(stat(&p, &vctx(), "beta").await.title, "beta 的名字");
}

/// 有配置文档的插件沿用**拥有者给的**定义（标题 / 表单都在里面），本插件只追加装配动作
#[tokio::test]
async fn declared_config_defines_the_entry_form() {
    let fake = Arc::new(FakeRegistry::new(vec![entry(
        "web",
        "web_factory",
        false,
        true,
    )]));
    let p = plugin_with(fake);
    let ctx = ctx_with_configs().await;

    let node = stat(&p, &ctx, "web").await;
    assert_eq!(
        node.title, "网络工具",
        "标题取自配置声明（优先于插件元数据的名字）"
    );
    let d = definition_of(&node);
    assert_eq!(d.binding, "option", "沿用拥有者的定义，而非合成的概览");
    // 拥有者声明的动作照旧在（`save`），装配动作追加在其后
    assert_eq!(d.actions[0].id, "save");
    assert_eq!(d.actions.len(), 4, "保存 + 启用 / 停用 / 卸载");
}

/// 配置声明缺失时条目照样列得出——声明通道是增益，缺了只是少一份表单定义
#[tokio::test]
async fn missing_declaration_falls_back_to_the_overview() {
    let fake = Arc::new(FakeRegistry::new(vec![entry(
        "web",
        "web_factory",
        false,
        true,
    )]));
    let p = plugin_with(fake);

    let node = stat(&p, &vctx(), "web").await;
    assert_eq!(node.title, "web 的名字", "回落链第二级：插件元数据的名字");
    assert_eq!(definition_of(&node).binding, "info");
}

// ==================== 装配动作 ====================

/// 三个装配动作恒在定义里（只靠条件显隐），且 `delete` 用机制同名 id
///
/// 条件恒在而不是按状态增删：定义是节点的一部分，而状态一变调用方只重读正文、
/// 不重新取节点——按状态增删会让按钮停在旧状态上（停用之后连「启用」都不出现）。
#[tokio::test]
async fn entry_actions_cover_enable_disable_and_uninstall() {
    let fake = Arc::new(FakeRegistry::new(vec![entry(
        "alpha",
        "alpha_factory",
        false,
        true,
    )]));
    let p = plugin_with(fake);

    let d = definition_of(&stat(&p, &vctx(), "alpha").await);
    let ids: Vec<&str> = d.actions.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(
        ids,
        vec![VDFS_ACTION_ENABLE, VDFS_ACTION_DISABLE, "delete"],
        "`delete` 与前端机制动作同名——因此既换了文案，也顶掉了兜底那一个"
    );

    assert_eq!(
        action_of(&d, VDFS_ACTION_ENABLE).when,
        Some(cond_equals(KEY_ENABLED, Value::Bool(false)))
    );
    assert_eq!(
        action_of(&d, VDFS_ACTION_DISABLE).when,
        Some(cond_all(vec![
            cond_equals(KEY_ENABLED, Value::Bool(true)),
            cond_equals(KEY_CAN_DISABLE, Value::Bool(true)),
        ]))
    );
    assert_eq!(
        action_of(&d, "delete").when,
        Some(cond_equals(KEY_REQUIRED, Value::Bool(false)))
    );
    assert_eq!(action_of(&d, "delete").label, "卸载");
}

/// 读条目叠投影键：装配态因此能被 `DetailAction.when` 求值（定义只认表单模型）
#[tokio::test]
async fn read_injects_projection_keys() {
    let fake = Arc::new(FakeRegistry::new(vec![entry(
        "alpha",
        "alpha_factory",
        false,
        true,
    )]));
    let p = plugin_with(fake);

    let m = read_model(&p, &vctx(), "alpha").await;
    assert_eq!(m[KEY_NAME], Value::String("alpha".to_string()));
    assert_eq!(m[KEY_PROVIDER], Value::String("alpha_factory".to_string()));
    assert_eq!(m[KEY_ENABLED], Value::Bool(true));
    assert_eq!(m[KEY_REQUIRED], Value::Bool(false));
    assert_eq!(m[KEY_CAN_DISABLE], Value::Bool(true));
    assert_eq!(m[KEY_VERSION], Value::String("1.0.0".to_string()));
}

/// 配置正文照常转发：条目不是第二种东西，它就是那份配置在管理页里的入口
#[tokio::test]
async fn read_forwards_the_owners_config_text() {
    let fake = Arc::new(
        FakeRegistry::new(vec![entry("alpha", "alpha_factory", false, true)]).with_config(
            "alpha/PLUGIN.yml",
            r#"{"plugin_provider": "alpha_factory", "mode": "fast"}"#,
        ),
    );
    let p = plugin_with(fake);

    let m = read_model(&p, &vctx(), "alpha").await;
    assert_eq!(
        m["mode"],
        Value::String("fast".to_string()),
        "拥有者的字段原样带出"
    );
    assert_eq!(m[KEY_ENABLED], Value::Bool(true), "投影键叠在其上");
}

/// 界面底座插件：可停用与否由**投影键**表达，删除与否由必需位表达
///
/// 插件管理插件自己就是底座（[`UNDISABLABLE_PLUGINS`]）：它必需、不可停用、
/// 因而两个破坏性动作都不出现——但定义里**恒在**，只是条件不成立。
#[tokio::test]
async fn undisablable_required_plugin_hides_both_destructive_actions() {
    let fake = Arc::new(FakeRegistry::new(vec![entry(
        PLUGIN_MANAGER,
        PLUGIN_MANAGER,
        true,
        true,
    )]));
    let p = plugin_with(fake);

    let m = read_model(&p, &vctx(), PLUGIN_MANAGER).await;
    assert_eq!(m[KEY_REQUIRED], Value::Bool(true));
    assert_eq!(m[KEY_CAN_DISABLE], Value::Bool(false));

    // 条件对这份模型求值：停用与卸载都不显示（`holds` 的判据见 `schemas/detail.rs`）
    let d = definition_of(&stat(&p, &vctx(), PLUGIN_MANAGER).await);
    let disable = action_of(&d, VDFS_ACTION_DISABLE).when.clone().unwrap();
    let delete = action_of(&d, "delete").when.clone().unwrap();
    assert!(!disable.holds(&m), "底座插件不可停用");
    assert!(!delete.holds(&m), "必需插件不可删除");
}

/// 停用：插件名进**载荷**（注册表是根上的视图，停用的插件在资源树里没有位置）
#[tokio::test]
async fn disable_forwards_the_plugin_name_in_the_payload() {
    let fake = Arc::new(FakeRegistry::new(vec![entry(
        "beta",
        "beta_factory",
        false,
        true,
    )]));
    let p = plugin_with(Arc::clone(&fake));

    p.dispatch(
        &vctx(),
        "beta",
        VdfsRequest::Action {
            action: VDFS_ACTION_DISABLE.to_string(),
            payload: None,
        },
    )
    .await
    .unwrap();

    let acts = fake.acts.lock().unwrap();
    assert_eq!(acts.len(), 1);
    assert_eq!(acts[0].0, VDFS_ACTION_DISABLE);
    assert_eq!(
        acts[0].1.as_ref().unwrap()[VDFS_PLUGIN_NAME_FIELD],
        Value::String("beta".to_string())
    );
}

/// 卸载：转发容器的 `Delete(<插件名>)`——「删掉这个目录」与「卸载这个插件」是一件事
#[tokio::test]
async fn uninstall_forwards_delete_to_the_container() {
    let fake = Arc::new(FakeRegistry::new(vec![entry(
        "beta",
        "beta_factory",
        false,
        true,
    )]));
    let p = plugin_with(Arc::clone(&fake));

    p.dispatch(&vctx(), "beta", VdfsRequest::Delete { recursive: false })
        .await
        .unwrap();
    assert_eq!(*fake.deletes.lock().unwrap(), vec!["beta".to_string()]);
}

/// 安装：转发到**容器根上的写**，新插件的名字由容器生成并经回包交回
#[tokio::test]
async fn install_forwards_write_to_the_container_root() {
    let fake = Arc::new(FakeRegistry::new(Vec::new()));
    let p = plugin_with(Arc::clone(&fake));

    let content = VdfsContent::text(r#"{"provider": "telegram"}"#).with_create();
    let w = p
        .dispatch(&vctx(), "", VdfsRequest::Write { content })
        .await
        .unwrap()
        .into_write()
        .unwrap();
    assert_eq!(w.name.as_deref(), Some("telegram"));

    let writes = fake.writes.lock().unwrap();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].0, "", "安装落在容器根上");
    assert!(writes[0].1.contains("telegram"), "表单值原样转过去");
}

// ==================== 自有分区 ====================

/// 分区是叶子：无正文、不参与树遍历，也不接受新建
#[tokio::test]
async fn sections_are_leaves_without_read_or_write() {
    let p = plugin_without_registry();
    let s = stat(&p, &vctx(), "appearance").await;
    assert!(!s.is_dir(), "分区是叶子文档");
    assert_eq!(s.kind, PLUGIN_MANAGER);
    assert_eq!(s.status, VDFS_STATUS_NONE, "静态分区没有「运行中」可言");

    // 数据在前端 store：读写都明确拒绝（而非静默返回空）
    assert!(matches!(
        p.dispatch(&vctx(), "appearance", VdfsRequest::Read).await,
        Err(VdfsError::Forbidden(_))
    ));
    assert!(matches!(
        p.dispatch(
            &vctx(),
            "appearance",
            VdfsRequest::Write {
                content: VdfsContent::text("{}")
            }
        )
        .await,
        Err(VdfsError::Forbidden(_))
    ));
    // 分区不接受新建，也不可列
    assert!(p
        .dispatch(
            &vctx(),
            "appearance",
            VdfsRequest::List {
                limit: None,
                before: None
            }
        )
        .await
        .is_err());
}

/// 未知条目：`NotFound`（区分「不存在」与「不可读写」）
#[tokio::test]
async fn unknown_entry_is_not_found() {
    let fake = Arc::new(FakeRegistry::new(vec![entry(
        "alpha",
        "alpha_factory",
        false,
        true,
    )]));
    let p = plugin_with(fake);

    assert!(matches!(
        p.dispatch(&vctx(), "nope", VdfsRequest::Stat).await,
        Err(VdfsError::NotFound(_))
    ));
    // 多段路径在本子树里没有对应的东西：如实报未知，而不是硬拆首段
    assert!(matches!(
        p.dispatch(&vctx(), "alpha/inner", VdfsRequest::Stat).await,
        Err(VdfsError::NotFound(_))
    ));
}

// ==================== 无容器可回 ====================

/// 未装配（取不到容器）时：注册表类操作如实报错，自有分区照常可用
#[tokio::test]
async fn missing_container_view_reports_internal_but_sections_still_work() {
    let p = plugin_without_registry();

    let err = p
        .dispatch(
            &vctx(),
            "",
            VdfsRequest::List {
                limit: None,
                before: None,
            },
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, VdfsError::Internal(_)),
        "取不到容器视图是内部状态问题，不是「路径不对」：{err:?}"
    );

    assert!(stat(&p, &vctx(), "appearance").await.kind == PLUGIN_MANAGER);
}

// ==================== 配置声明通道 ====================

/// 造一份带可配置声明的请求 ctx（声明通常由容器在广播中收集，这里直接给）
async fn ctx_with_configs() -> VdfsContext {
    use crate::symbio_core::{
        entry_of, vdfs::vdfs_context, ConfigFile, ConfigurableVisitor, DefaultConfigurableVisitor,
        PluginDir, SimpleRequest, CONFIG_VISITOR,
    };

    let visitor: Arc<dyn ConfigurableVisitor> = Arc::new(DefaultConfigurableVisitor::new());
    visitor
        .register_configurable(entry_of(&ConfigFile::new(
            PluginDir::at(std::env::temp_dir(), "web"),
            "网络工具",
            DetailDefinition::form("配置", Vec::new()),
        )))
        .await;

    let host: Arc<dyn InvokeRequest> = Arc::new(SimpleRequest::new(None, None));
    host.set(CONFIG_VISITOR, visitor);
    vdfs_context(&host)
}
