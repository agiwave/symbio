//! `crates/symbio-plugin-composite/src/composite.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

#[allow(unused_imports)]
// 只为把兄弟插件的工厂注册（inventory）链进本测试二进制，见 Cargo.toml 的 dev-dependencies 注。
use symbio as _;

use super::*;

use indexmap::IndexMap;
use std::sync::{Mutex, RwLock};
use symbio_plugin_api::PluginSimpleRequest;

///  在本作用域是泛型别名，测试里的 trait impl 需要具体化
type PluginInvokeResponse = symbio_plugin_api::PluginInvokeResponse<PluginPayload>;

// ==================== 嵌套容器的父地址转发 ====================
//
// 容器可嵌套容器（子智能体子树即此形态：外层系统容器 → 内层子智能体容器
// → 内层子插件）。每次跨挂载边界的转发（route / traverse）都必须把
// **当前父地址**从 ctx 已携带的值续接下去，子插件拿到的才是完整挂载点。

/// 探针插件：route / traverse 时把 ctx 的父地址记进共享桶
struct Probe {
    seen_route: Mutex<Vec<String>>,
    seen_traverse: Mutex<Vec<String>>,
}

impl Probe {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            seen_route: Mutex::new(Vec::new()),
            seen_traverse: Mutex::new(Vec::new()),
        })
    }

    fn routes(&self) -> Vec<String> {
        self.seen_route.lock().unwrap().clone()
    }

    fn traverses(&self) -> Vec<String> {
        self.seen_traverse.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl Plugin for Probe {
    fn meta(&self) -> PluginMeta {
        PluginMeta::new("probe", "probe")
    }

    async fn route(self: Arc<Self>, ctx: Arc<dyn PluginInvokeRequest>) -> PluginInvokeResponse {
        self.seen_route
            .lock()
            .unwrap()
            .push(ctx.get(VDFS_PARENT_ADDR).unwrap_or_default());
        Ok(PluginPayload::new(&serde_json::json!({"ok": true})))
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse {
        self.seen_traverse
            .lock()
            .unwrap()
            .push(ctx.get(VDFS_PARENT_ADDR).unwrap_or_default());
        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

/// 实例表：容器与它的 VDFS 视图共用同一份（唯一持有者是 `PluginRegistry`）
fn plugin_map(
    entries: Vec<(&str, Arc<dyn Plugin>)>,
) -> Arc<RwLock<IndexMap<String, Arc<dyn Plugin>>>> {
    Arc::new(RwLock::new(
        entries
            .into_iter()
            .map(|(n, p)| (n.to_string(), p))
            .collect::<IndexMap<_, _>>(),
    ))
}

/// 造一个容器：与装配期**同一条路**——实例表进注册表，容器与它的 VDFS 因此共用
/// 同一份（不再有「容器一份、视图另一份」的构造方式）
fn composite_of(entries: Vec<(&str, Arc<dyn Plugin>)>) -> Arc<Composite> {
    let registry = Arc::new(PluginRegistry::with_instances(
        plugin_map(entries),
        // 测试容器：根给一个临时目录，不经任何全局系统根。
        std::env::temp_dir().join("symbio-composite-test"),
        Vec::new(),
    ));
    Arc::new(Composite::with_registry(registry))
}

fn host() -> Arc<dyn PluginInvokeRequest> {
    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
    ctx
}

// ==================== 全局收尾：整树 stop(Shutdown) ====================
//
// CLI / Tauri 的退出路径对本根调一次 `stop(Shutdown)`（D-033 的第三态收尾）。
// 遍历 owner 是 `PluginRegistry::shutdown_all`；这里的探针钉三件事：
// **每个子插件恰一次**、**理由可读**（`Shutdown`，不是 Disabled/Uninstalled）、
// **单家失败不拦其余**（收尾是对全表的广播，一家报错不等于其余不用落盘）。

/// 记录自己被 stop 的理由与次数；可配置为「stop 时报错」
struct StopProbe {
    stops: Mutex<Vec<&'static str>>,
    fail_on_stop: bool,
}

impl StopProbe {
    fn new(fail_on_stop: bool) -> Arc<Self> {
        Arc::new(Self {
            stops: Mutex::new(Vec::new()),
            fail_on_stop,
        })
    }

    fn reasons(&self) -> Vec<String> {
        self.stops
            .lock()
            .unwrap()
            .iter()
            .map(|s| s.to_string())
            .collect()
    }
}

#[async_trait::async_trait]
impl Plugin for StopProbe {
    fn meta(&self) -> PluginMeta {
        PluginMeta::new("stop-probe", "stop-probe")
    }

    async fn route(self: Arc<Self>, _ctx: Arc<dyn PluginInvokeRequest>) -> PluginInvokeResponse {
        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse {
        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }

    async fn stop(self: Arc<Self>, reason: PluginStopReason) -> Result<(), PluginError> {
        self.stops.lock().unwrap().push(match reason {
            PluginStopReason::Disabled => "Disabled",
            PluginStopReason::Uninstalled => "Uninstalled",
            PluginStopReason::Shutdown => "Shutdown",
        });
        if self.fail_on_stop {
            Err(PluginError::InternalError("探针故意失败".into()))
        } else {
            Ok(())
        }
    }
}

/// 整树收尾：`stop(Shutdown)` 广播到**每个**子插件、各恰一次、理由是 `Shutdown`
#[tokio::test]
async fn shutdown_broadcasts_to_every_child_exactly_once() {
    let a = StopProbe::new(false);
    let b = StopProbe::new(false);
    let composite = composite_of(vec![
        ("a", Arc::clone(&a) as Arc<dyn Plugin>),
        ("b", Arc::clone(&b) as Arc<dyn Plugin>),
    ]);

    composite
        .clone()
        .stop(PluginStopReason::Shutdown)
        .await
        .unwrap();

    assert_eq!(a.reasons(), vec!["Shutdown"], "a 应恰收一次 Shutdown");
    assert_eq!(b.reasons(), vec!["Shutdown"], "b 应恰收一次 Shutdown");
}

/// 单家收尾失败**不拦其余**：广播继续，容器自身仍返回 Ok
#[tokio::test]
async fn shutdown_continues_past_a_failing_child() {
    let bad = StopProbe::new(true);
    let good = StopProbe::new(false);
    let composite = composite_of(vec![
        ("bad", Arc::clone(&bad) as Arc<dyn Plugin>),
        ("good", Arc::clone(&good) as Arc<dyn Plugin>),
    ]);

    composite
        .clone()
        .stop(PluginStopReason::Shutdown)
        .await
        .unwrap();

    assert_eq!(bad.reasons().len(), 1, "失败的那家也收到了 stop");
    assert_eq!(good.reasons(), vec!["Shutdown"], "后面的插件照常收尾");
}

/// `Disabled` / `Uninstalled` 是对**单个插件**的处置，容器不把它们广播成整树停机
#[tokio::test]
async fn non_shutdown_reasons_do_not_broadcast() {
    let a = StopProbe::new(false);
    let composite = composite_of(vec![("a", Arc::clone(&a) as Arc<dyn Plugin>)]);

    composite
        .clone()
        .stop(PluginStopReason::Disabled)
        .await
        .unwrap();
    composite
        .clone()
        .stop(PluginStopReason::Uninstalled)
        .await
        .unwrap();

    assert!(
        a.reasons().is_empty(),
        "容器收到非 Shutdown 理由时不应向子插件广播（实际: {:?}）",
        a.reasons()
    );
}

/// route 转发链：外层容器 → 内层容器 → 探针，父地址逐级续接成完整挂载点
#[tokio::test]
async fn route_chain_extends_parent_addr_per_level() {
    let probe = Probe::new();
    let inner = composite_of(vec![("probe", Arc::clone(&probe) as Arc<dyn Plugin>)]);
    let outer = composite_of(vec![("inner", Arc::clone(&inner) as Arc<dyn Plugin>)]);

    let ctx = host();
    ctx.set(PATH, "inner/probe".to_string());
    ctx.set(VDFS_PARENT_ADDR, "sys-root".to_string());
    outer.route(ctx).await.unwrap();

    let seen = probe.routes();
    assert_eq!(seen.len(), 1, "探针恰好被路由一次");
    assert_eq!(
        seen[0], "sys-root/inner/probe",
        "父地址 = 上级父地址 + 逐级目录名（不落回系统根，根名无关）"
    );
}

/// traverse 转发链：嵌套容器收集能力时同样逐级续接
#[tokio::test]
async fn traverse_chain_extends_parent_addr_per_level() {
    let probe = Probe::new();
    let inner = composite_of(vec![("probe", Arc::clone(&probe) as Arc<dyn Plugin>)]);
    let outer = composite_of(vec![("inner", Arc::clone(&inner) as Arc<dyn Plugin>)]);

    let ctx = host();
    ctx.set(PATH, TRAVERSE_AVAILABLE_TOOLS.to_string());
    // 模拟外层容器自身挂在 `sys-root` 之下（嵌套装配时上级写入）
    ctx.set(VDFS_PARENT_ADDR, "sys-root".to_string());
    outer.traverse(String::new(), ctx).await.unwrap();

    let seen = probe.traverses();
    assert_eq!(seen.len(), 1, "探针恰好被收集一次");
    assert_eq!(seen[0], "sys-root/inner/probe", "收集期父地址 = 完整挂载点");
}

/// 收集是**广播**：不参与的插件回答 `NotFound`，那是「我不贡献」，不是失败。
///
/// 判反的代价（实测）：启动期每个子插件各刷两遍
/// 「子插件能力收集失败 …：未知遍历路径: available_options」，真正的失败
/// 反而被噪音埋掉。故这条判据单独成函数、单独钉住。
#[test]
fn collect_declines_are_not_failures() {
    assert!(
        collect_declined(&PluginError::NotFound(
            "未知遍历路径: available_options".to_string()
        )),
        "不认识这个收集端点 = 不参与，不该报警"
    );
    assert!(
        !collect_declined(&PluginError::InternalError("配置读不出来".to_string())),
        "参与了却炸了 = 真失败，必须留痕"
    );
    assert!(!collect_declined(&PluginError::Timeout), "超时同理：真失败");
}

/// 顶层数据点：无上级父地址时，子插件落在**静态声明的根**之下
/// （不写死具体名字——断言非空且以本容器目录名结尾）
#[tokio::test]
async fn top_level_container_falls_back_to_declared_root() {
    let probe = Probe::new();
    let outer = composite_of(vec![("probe", Arc::clone(&probe) as Arc<dyn Plugin>)]);

    let ctx = host();
    ctx.set(PATH, "probe".to_string());
    outer.route(ctx).await.unwrap();

    let seen = probe.routes();
    assert_eq!(seen.len(), 1);
    assert!(
        !seen[0].is_empty() && seen[0].ends_with("/probe"),
        "顶层 = 声明根 + 子名（根是数据，形态不作假设）: {}",
        seen[0]
    );
}

// ==================== 挂载行由容器派生 ====================
//
// 「谁能提供 VDFS 资源」只有一个答案：`Plugin::get_vfs_provider`。容器的组合视图
// 按这一个判据决定哪些子插件在资源树里占一格——插件自己不必 `put` 任何东西，
// 漏接一处不再可能静默。行表那份复述已经删掉（见 `capability/mount.rs` 的模块
// 文档），所以下面钉的是**视图本身**，不再经行表中转。

/// 只有视图、`traverse` 什么都不注册的插件：资源树里仍须有它这一格
struct SilentView;

#[async_trait::async_trait]
impl symbio_plugin_api::VdfsProvider for SilentView {
    async fn dispatch(
        &self,
        _ctx: &symbio_plugin_api::VdfsContext,
        _path: &str,
        _req: symbio_plugin_api::VdfsRequest,
    ) -> symbio_plugin_api::VdfsResult<symbio_plugin_api::VdfsResponse> {
        Ok(symbio_plugin_api::VdfsResponse::List(Vec::new()))
    }
}

#[async_trait::async_trait]
impl Plugin for SilentView {
    fn meta(&self) -> PluginMeta {
        PluginMeta::new("silent", "silent")
    }

    async fn route(self: Arc<Self>, _ctx: Arc<dyn PluginInvokeRequest>) -> PluginInvokeResponse {
        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse {
        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }

    fn get_vfs_provider(self: Arc<Self>) -> Option<Arc<dyn VdfsProvider>> {
        Some(Arc::new(SilentView))
    }
}

/// 资源树的成员由 `get_vfs_provider` 单条判据决定，不是插件在 `traverse` 里自己报
#[tokio::test]
async fn resource_tree_membership_is_derived_from_providers() {
    let composite = composite_of(vec![
        ("silent", Arc::new(SilentView) as Arc<dyn Plugin>),
        ("plain", Probe::new() as Arc<dyn Plugin>),
    ]);
    let mounts = mount_names(&composite).await;
    assert_eq!(
        mounts,
        vec!["silent".to_string()],
        "有视图 ⇔ 资源树里占一格；没视图的插件不占一格"
    );

    // 那一格背后正是子插件交出的 provider：往目录上发一次 List 就落到它身上
    let provider = composite
        .clone()
        .get_vfs_provider()
        .expect("容器有组合视图");
    assert!(
        provider
            .dispatch(&vdfs_context(&host()), "silent", VdfsRequest::List {})
            .await
            .is_ok(),
        "silent 那一格可寻址，且应答来自它自己的视图"
    );
}

// ==================== 运行期停用一片插件（D-077 守卫后半） ====================
//
// 「切出一个插件」的**兑现判据**不是目录搬对了，而是：停用它 ⇒ 少掉的正好是它这一
// 个功能，其余一切照旧。这条判据跨插件，故两侧各钉一半——本插件自己那半
// （`traverse` put 两行 + `stop` ⇒ 循环退出）在 `crates/symbio-plugin-heartbeat/src/plugin.test.rs`；
// 这里钉的是**容器侧**：经**工厂 id + 真实 `mount_all`** 装配，不 import 兄弟模块
// （E-009），停用的动作走注册表的 `set_enabled`（= 前端那个开关背后的同一个动词）。

use std::path::{Path, PathBuf};
use symbio_plugin_api::{
    family, vdfs_context, PluginStopReason, Registry, VdfsContent, VdfsRequest, VdfsResponse,
    OPTION_VISITOR, PLUGIN_FILE, TRAVERSE_AVAILABLE_OPTIONS, TRAVERSE_AVAILABLE_TOOLS,
};
use symbio_providers::DefaultToolVisitor;
use symbio_testkit::capability_option_fields;

/// 一次性插件根：判据的来源是磁盘（装配位写在 `PLUGIN.yml`），不能拿内存替身测
fn guard_root(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "symbio-composite-guard-{tag}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).expect("临时插件根建得起来");
    p
}

fn put_plugin_dir(root: &Path, name: &str, provider: &str) {
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).expect("插件目录建得起来");
    std::fs::write(
        dir.join(PLUGIN_FILE),
        format!(r#"{{"plugin_provider": "{provider}"}}"#),
    )
    .expect("装配位写得出去");
}

/// 广播一次收集，回收集器（端点决定贡献方注册哪一族）
async fn collect_at(composite: &Arc<Composite>, endpoint: &str) -> Arc<dyn Registry> {
    let ctx = host();
    ctx.set(PATH, endpoint.to_string());
    let visitor: Arc<dyn Registry> = Arc::new(DefaultToolVisitor::new());
    if endpoint == TRAVERSE_AVAILABLE_TOOLS {
        ctx.set(CAPABILITY_VISITOR, visitor.clone());
    } else {
        ctx.set(OPTION_VISITOR, visitor.clone());
    }
    composite
        .clone()
        .traverse(String::new(), ctx)
        .await
        .expect("容器的收集广播不该失败");
    visitor
}

async fn tool_names(composite: &Arc<Composite>) -> Vec<String> {
    let visitor = collect_at(composite, TRAVERSE_AVAILABLE_TOOLS).await;
    let mut names: Vec<String> = visitor
        .list(family::TOOL)
        .await
        .into_iter()
        .map(|row| row.name().to_string())
        .collect();
    names.sort();
    names
}

/// 资源树的成员名（= 暴露了 VDFS 的已挂载子插件）。
///
/// 判据与 `CompositeVdfs::children_of` 同源：`Plugin::get_vfs_provider` 有视图才算
/// 一格。这里不去 `dispatch(List)` 取——根列表会按 `hidden` 过滤，而判据要的是
/// 「装没装出这一格」，与「界面上显不显示」是两件事（`loop` 就是隐藏的）。
async fn mount_names(composite: &Arc<Composite>) -> Vec<String> {
    let mut names: Vec<String> = composite
        .registry
        .snapshot()
        .into_iter()
        .filter(|(_, p)| p.clone().get_vfs_provider().is_some())
        .map(|(name, _)| name)
        .collect();
    names.sort();
    names
}

async fn option_keys(composite: &Arc<Composite>) -> Vec<String> {
    let visitor = collect_at(composite, TRAVERSE_AVAILABLE_OPTIONS).await;
    capability_option_fields(&*visitor)
        .await
        .into_iter()
        .map(|f| f.key)
        .collect()
}

/// 停用 `heartbeat` ⇒ 它的两行一起消失，会话那一侧一行不少
#[tokio::test]
async fn disabling_a_slice_removes_its_rows_and_keeps_the_rest() {
    let root = guard_root("slice");
    // 两个**真**插件目录，provider 写工厂 id（`submit_object_creator!` 注册的那个）
    put_plugin_dir(&root, "heartbeat", "heartbeat");
    put_plugin_dir(&root, "session", "session");

    let registry = Arc::new(PluginRegistry::with_instances(
        plugin_map(Vec::new()),
        root.clone(),
        Vec::new(),
    ));
    registry.mount_all();
    assert_eq!(
        registry.snapshot().len(),
        2,
        "两个工厂都真实构造出来了（守卫的前提是装配本身跑通）"
    );
    let composite = Arc::new(Composite::with_registry(Arc::clone(&registry)));

    // —— 停用前：本插件那两行都在，会话的行也都在 ——
    assert!(
        tool_names(&composite)
            .await
            .contains(&"heartbeat".to_string()),
        "工具族里有 heartbeat"
    );
    assert!(
        option_keys(&composite)
            .await
            .contains(&"heartbeat".to_string()),
        "选项族里有 heartbeat"
    );

    // —— 停用（前端那个开关背后的同一个动词）——
    registry
        .set_enabled("heartbeat", false)
        .await
        .expect("可停用插件（非界面底座）应当停用成功");

    // —— 停用后：Tool 行与 Option 行的 heartbeat 都不在 ——
    assert!(
        !tool_names(&composite)
            .await
            .contains(&"heartbeat".to_string()),
        "停用后工具族里不该再有 heartbeat（模型调不到它才是「停用」）"
    );
    let keys = option_keys(&composite).await;
    assert!(
        !keys.contains(&"heartbeat".to_string()),
        "停用后选项栏里不该再有 heartbeat 那一格"
    );
    // —— 而其余行照旧：会话自有的三项、会话挂载点 ——
    for expected in ["workdir", "risk_level", "mode"] {
        assert!(
            keys.contains(&expected.to_string()),
            "停用一片插件不该动会话自己的选项：缺 {expected}"
        );
    }
    assert!(
        mount_names(&composite)
            .await
            .contains(&"session".to_string()),
        "会话的 VDFS 挂载点仍在（少掉的只是心跳这一格）"
    );

    // —— 会话侧读写照常：经**容器视图**，与心跳插件用的是同一条通道 ——
    let provider = composite
        .clone()
        .get_vfs_provider()
        .expect("容器有组合视图");
    let vctx = vdfs_context(&host());
    let mut create = VdfsContent::text("");
    create.create = true;
    provider
        .dispatch(
            &vctx,
            "session/guard-1",
            VdfsRequest::Write { content: create },
        )
        .await
        .expect("新建会话照常");
    match provider
        .dispatch(&vctx, "session", VdfsRequest::List {})
        .await
        .expect("清单照常")
    {
        VdfsResponse::List(items) => assert!(
            items.iter().any(|i| i.node.name == "guard-1"),
            "新建的会话在清单里"
        ),
        _ => panic!("vdfs/list 未返回清单"),
    }
    provider
        .dispatch(
            &vctx,
            "session/guard-1/inbox",
            VdfsRequest::Write {
                content: VdfsContent::text("停用心跳之后这条照发"),
            },
        )
        .await
        .expect("收件箱写臂照常（它不认识心跳在不在装配里）");

    // 收尾：整树停机（会话的收件箱消费者是常驻任务）
    Plugin::stop(composite.clone(), PluginStopReason::Shutdown)
        .await
        .expect("收尾不该失败");
    let _ = std::fs::remove_dir_all(&root);
}

use symbio_plugin_api::VdfsError;

/// 反向用例：**必需**插件卸不掉（可以停用）——「可卸载」不是「什么都能删」。
#[tokio::test]
async fn uninstalling_a_required_plugin_is_refused_and_the_directory_survives() {
    let root = guard_root("required");
    put_plugin_dir(&root, "session", "session");
    put_plugin_dir(&root, "loop", "loop");
    let registry = Arc::new(PluginRegistry::with_instances(
        plugin_map(Vec::new()),
        root.clone(),
        vec!["session".to_string()],
    ));
    registry.mount_all();
    assert_eq!(registry.snapshot().len(), 2, "两个工厂都构造出来了");
    let composite = Arc::new(Composite::with_registry(Arc::clone(&registry)));
    let provider = composite
        .clone()
        .get_vfs_provider()
        .expect("容器有组合视图");

    let err = provider
        .dispatch(
            &vdfs_context(&host()),
            "session",
            VdfsRequest::Delete { recursive: true },
        )
        .await
        .expect_err("必需插件必须拒绝卸载");
    match &err {
        VdfsError::Invalid(msg) => assert!(
            msg.message.contains("必需插件"),
            "拒绝的理由要说明它是必需的，实际：{msg:?}"
        ),
        other => panic!("错的错误形状：应当是 Invalid，实际 {other:?}"),
    }
    assert!(
        root.join("session").exists(),
        "拒绝之后目录必须还在（否则拒绝只是句空话）"
    );
    assert_eq!(registry.snapshot().len(), 2, "装配表一项都没少");

    Plugin::stop(composite.clone(), PluginStopReason::Shutdown)
        .await
        .expect("收尾不该失败");
    let _ = std::fs::remove_dir_all(&root);
}
