//! `symbio/src/plugins/composite/composite.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

use crate::symbio_core::PluginSimpleRequest;
use std::collections::HashMap;
use std::sync::{Mutex, RwLock};

///  在本作用域是泛型别名，测试里的 trait impl 需要具体化
type PluginInvokeResponse = crate::symbio_core::PluginInvokeResponse<PluginPayload>;

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
) -> Arc<RwLock<HashMap<String, Arc<dyn Plugin>>>> {
    Arc::new(RwLock::new(
        entries
            .into_iter()
            .map(|(n, p)| (n.to_string(), p))
            .collect(),
    ))
}

/// 造一个容器：与装配期**同一条路**——实例表进注册表，容器与它的 VDFS 因此共用
/// 同一份（不再有「容器一份、视图另一份」的构造方式）
fn composite_of(entries: Vec<(&str, Arc<dyn Plugin>)>) -> Arc<Composite> {
    let registry = Arc::new(PluginRegistry::with_instances(
        plugin_map(entries),
        crate::symbio_core::plugins_root(),
        Vec::new(),
    ));
    Arc::new(Composite::with_registry(registry))
}

fn host() -> Arc<dyn PluginInvokeRequest> {
    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
    ctx
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
