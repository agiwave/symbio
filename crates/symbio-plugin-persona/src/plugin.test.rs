//! `plugin.rs` 的单元测试 —— 人格段**只在填过时**出现，且改一行配置就变。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。
//!
//! 钉住四件事：
//!
//! - 填了 ⇒ 交出一条段（段名、正文都在），空 ⇒ 一条都不交（平凡值）；
//! - 段名**不与挂载行同名**——能力表的键是行名、族不参与键，同键会把挂载行顶掉；
//! - 配置是**每轮现读**的：改完下一轮就生效，不需要重建插件；
//! - 无自有协议：`route` 恒 `NotFound`，`traverse` 只认一个路径。

use super::*;
use std::path::Path;
use std::sync::Arc;
use symbio_plugin_api::{
    CapabilityFamily, PluginConfigMount, PluginInvokeRequestExt, PluginSimpleRequest, Registry,
};
use tempfile::TempDir;
use tokio::sync::RwLock;

/// 最小登记表：只装行、按族过滤。测试不需要审批闸与单槽移交（本插件不注册工具 /
/// 模型行），因此**不引 `symbio-providers`**——那会把一整条工具链拖进本 crate 的
/// 测试依赖里，而这里要验的只是「往表里放了什么」。
struct FakeRegistry {
    rows: RwLock<Vec<Arc<CapabilityRow>>>,
}

impl FakeRegistry {
    fn new() -> Self {
        Self {
            rows: RwLock::new(Vec::new()),
        }
    }
}

#[async_trait]
impl Registry for FakeRegistry {
    async fn put(&self, row: CapabilityRow) {
        self.rows.write().await.push(Arc::new(row));
    }

    async fn get(&self, name: &str) -> Option<Arc<CapabilityRow>> {
        self.rows
            .read()
            .await
            .iter()
            .rev()
            .find(|r| r.name() == name)
            .cloned()
    }

    async fn list(&self, family: CapabilityFamily) -> Vec<Arc<CapabilityRow>> {
        self.rows
            .read()
            .await
            .iter()
            .filter(|r| r.family() == family)
            .cloned()
            .collect()
    }
}

/// 遍历上下文：两个访问器都挂上（提示词走 `CAPABILITY_VISITOR`，
/// 可配置声明走 `CONFIGURABLE_VISITOR`——它们是两个键，只挂一个会静默少一样）
fn ctx() -> (Arc<dyn PluginInvokeRequest>, Arc<FakeRegistry>) {
    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
    ctx.set(PATH, TRAVERSE_AVAILABLE_TOOLS.to_string());
    let visitor = Arc::new(FakeRegistry::new());
    ctx.set(
        CAPABILITY_VISITOR,
        Arc::clone(&visitor) as Arc<dyn Registry>,
    );
    ctx.set(
        symbio_plugin_api::CONFIGURABLE_VISITOR,
        Arc::clone(&visitor) as Arc<dyn Registry>,
    );
    (ctx, visitor)
}

fn plugin_dir(dir: &Path) -> PluginDir {
    PluginDir::at(dir.join(PLUGIN_ID_PERSONA), PLUGIN_ID_PERSONA)
}

fn plugin_in(dir: &Path, config: PersonaConfig) -> Arc<PersonaPlugin> {
    Arc::new(PersonaPlugin::new(plugin_dir(dir), config))
}

fn with_persona(text: &str) -> PersonaConfig {
    PersonaConfig {
        persona: Some(text.to_string()),
    }
}

/// 收集本插件这一轮交出的**提示词段**（段名, 正文）
async fn segments_of(p: Arc<PersonaPlugin>) -> Vec<(String, String)> {
    let (ctx, visitor) = ctx();
    p.traverse(String::new(), ctx)
        .await
        .expect("traverse 必须成功");
    visitor
        .list(symbio_plugin_api::family::PROMPT)
        .await
        .iter()
        .filter_map(|r| {
            r.prompt_text()
                .map(|t| (r.name().to_string(), t.to_string()))
        })
        .collect()
}

// ==================== 有配置 ⇒ 一条段 ====================

#[tokio::test]
async fn configured_persona_yields_one_segment() {
    let tmp = TempDir::new().expect("临时目录");
    let p = plugin_in(tmp.path(), with_persona("你说话简短，不寒暄。"));

    let segs = segments_of(p).await;
    assert_eq!(segs.len(), 1, "填了人格必须恰好交一条段：{segs:?}");
    let (name, text) = &segs[0];
    assert_eq!(name, SEGMENT_NAME, "段名是覆盖键，不能随手改");
    assert!(
        text.contains("你说话简短，不寒暄。"),
        "正文必须原样带上：{text}"
    );
    // 标题让模型知道「这一段比后文更不该被改写」——同一份提示词里还躺着记忆与技能段
    assert!(text.contains("【人格】"), "段应带标题：{text}");
}

/// 段名与挂载行同名会把挂载行顶掉（`Registry::put` 的跨族同名覆盖）。
/// 这条断言是**结构性**的：只要有人把段名改成 `persona`，这里立刻红。
#[test]
fn segment_name_differs_from_mount_row() {
    assert_ne!(
        SEGMENT_NAME, PLUGIN_ID_PERSONA,
        "段名与插件目录名同键会顶掉挂载行（表键是全表唯一的名字空间，族不参与键）"
    );
}

// ==================== 平凡值 ====================

#[tokio::test]
async fn unconfigured_persona_yields_nothing() {
    let tmp = TempDir::new().expect("临时目录");
    let p = plugin_in(tmp.path(), PersonaConfig::default());
    assert!(
        segments_of(p).await.is_empty(),
        "空人格不该往系统提示词里塞一个空标题"
    );
}

#[tokio::test]
async fn blank_is_the_same_as_absent() {
    let tmp = TempDir::new().expect("临时目录");
    let p = plugin_in(tmp.path(), with_persona("   \n\t "));
    assert!(segments_of(p).await.is_empty(), "纯空白不是人格");
}

// ==================== 每轮现读配置 ====================

/// 面板上改完，**下一轮**就该生效。构造期快照会让「改了但还在用旧值」无从排查。
#[tokio::test]
async fn config_change_takes_effect_next_round() {
    let tmp = TempDir::new().expect("临时目录");
    let p = plugin_in(tmp.path(), with_persona("旧人格"));

    assert!(segments_of(p.clone()).await[0].1.contains("旧人格"));

    *p.config_slot().write().await = with_persona("新人格");
    let segs = segments_of(p).await;
    assert!(
        segs[0].1.contains("新人格") && !segs[0].1.contains("旧人格"),
        "第二轮应读新值：{segs:?}"
    );
}

/// 清空 ⇒ 段整个消失（不是变成一条空段）
#[tokio::test]
async fn clearing_config_removes_the_segment() {
    let tmp = TempDir::new().expect("临时目录");
    let p = plugin_in(tmp.path(), with_persona("会被清掉的人格"));
    assert_eq!(segments_of(p.clone()).await.len(), 1);

    *p.config_slot().write().await = PersonaConfig::default();
    assert!(segments_of(p).await.is_empty(), "清空后不该留一条空段");
}

// ==================== 边界：无自有协议 ====================

#[tokio::test]
async fn route_is_always_not_found() {
    let tmp = TempDir::new().expect("临时目录");
    let c: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
    c.set(PATH, "persona/tone".to_string());

    let r = plugin_in(tmp.path(), PersonaConfig::default())
        .route(c)
        .await;
    assert!(
        matches!(r, Err(PluginError::NotFound(_))),
        "人格没有自有协议：编辑面是 VDFS，不是路由"
    );
}

#[tokio::test]
async fn traverse_rejects_unknown_path() {
    let tmp = TempDir::new().expect("临时目录");
    let (ctx, _visitor) = ctx();
    ctx.set(PATH, "available_options".to_string());

    let r = plugin_in(tmp.path(), PersonaConfig::default())
        .traverse(String::new(), ctx)
        .await;
    assert!(
        matches!(r, Err(PluginError::NotFound(_))),
        "traverse 只认 available_tools"
    );
}

/// 配置文档要**声明出来**，设置页才列得到它
#[tokio::test]
async fn announces_its_config_document() {
    let tmp = TempDir::new().expect("临时目录");
    let (ctx, visitor) = ctx();
    plugin_in(tmp.path(), PersonaConfig::default())
        .traverse(String::new(), ctx)
        .await
        .expect("traverse");

    let rows = visitor.list(symbio_plugin_api::family::CONFIGURABLE).await;
    assert_eq!(rows.len(), 1, "本插件恰好一份配置文档");
    // 设置页那一项叫**插件名**（`capability_entry_of` 把文件名换成插件名，供列表按
    // 名字指到 `<根>/<插件名>`）。这条断言同时是「段名不能叫 `persona`」的现场证据：
    // 表键是全表唯一的名字空间，`persona` 这一个名字已经被这一行（和挂载行）占了。
    assert_eq!(
        rows[0].name(),
        PLUGIN_ID_PERSONA,
        "配置项的名字是插件名，不是 PLUGIN.yml"
    );
}

/// 挂载点存在（人格的编辑面是 VDFS，不是路由）
#[tokio::test]
async fn exposes_a_vdfs_mount() {
    let tmp = TempDir::new().expect("临时目录");
    let p = plugin_in(tmp.path(), PersonaConfig::default());
    assert!(
        Arc::clone(&p).get_vfs_provider().is_some(),
        "配置面必须经 VDFS 可达（route 恒 NotFound 的那些事都走这里）"
    );
}
