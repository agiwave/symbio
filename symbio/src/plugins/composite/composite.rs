//! Composite 插件实现——结构体 + Plugin trait impl
//!
//! 工厂逻辑通过 `Composite::build` 静态方法 + `submit_object_creator!` 自注册。
//!
//! ## 子项从哪来：**插件目录**
//!
//! 容器的子插件由**扫描插件目录**得到（见 [`PluginRegistry::mount_all`]），
//! 而不是由父插件塞一张配置表进来。于是：
//!
//! - 「有哪些插件」= 插件根下有哪些目录，用户放一个目录就多一个插件；
//! - 「插件怎么配」= 那个目录里的 `PLUGIN.yml`，**插件自己读写**；
//! - 容器只做两件事：把构造者声明的必需插件目录补出来、把合格目录构造出来并把
//!   **自身目录**告知被构造的插件。
//!
//! 父插件因此**不认识任何子插件的配置**——不再有合并规则、不再有分发规则。
//! 容器也**不内置任何插件清单**：它是通用容器（可嵌套另一个容器），「哪些插件
//! 必须存在」由构造者经 [`REQUIRED_PLUGINS`] 随构造传入。
//!
//! ## 装配与运行期是同一份实现
//!
//! 「插件根 / 必需清单 / 构造上下文 / 实例表」收在 [`PluginRegistry`] 里，
//! 装配期（本文件 `build`）与运行期（启停 / 安装 / 卸载，见 `vdfs.rs` 的根动词）
//! 走的是**同一批方法**：装配不是一条特殊路径，只是「把当下该挂的都挂上」这一次
//! 调用。于是「运行期改完要不要重启」这个问题不存在——实例表就是「谁在装配中」，
//! 改它即刻生效（`route` / `traverse` / 资源树都现取）。
//!
//! ## 容器自己的目录：系统根
//!
//! 容器是 `home` 的**动态内置替身**，父插件把系统根（`<homedir>`）告知它；它据此
//! 定位自己管辖的插件根（= 系统根本身，一层目录 = 一个插件），因此不依赖任何
//! 全局路径常量。系统根下的 `PLUGIN.yml` 属于 `home`——容器没有配置，不写 manifest。

use super::registry::PluginRegistry;
use super::vdfs::CompositeVdfs;
use crate::symbio_core::vdfs::descend_addr;
use crate::symbio_core::{
    lock_read, InvokeRequest, InvokeRequestExt, InvokeResponse, Plugin, PluginError, PluginMeta,
    PluginPayload, VdfsProvider, CAPABILITY_VISITOR, PATH, PLUGIN_COMPOSITE,
    TRAVERSE_AVAILABLE_TOOLS, VDFS_PARENT_ADDR,
};

use std::sync::{Arc, Weak};

pub struct Composite {
    /// 插件集合（插件根 / 必需清单 / 构造上下文 / 实例表 + 运行期增删）
    registry: Arc<PluginRegistry>,
    /// VDFS 组合视图：容器是虚拟根 `/` 的拥有者（见 `vdfs` 子模块）
    vdfs: Arc<CompositeVdfs>,
}

impl Composite {
    /// 用给定的注册表装配一个容器（装配期与测试的唯一构造入口）
    fn with_registry(registry: Arc<PluginRegistry>) -> Self {
        let vdfs = Arc::new(CompositeVdfs::new(Arc::clone(&registry)));
        Self { registry, vdfs }
    }

    fn parse_path(path: &str) -> Option<(&str, &str)> {
        let path = path.trim_start_matches('/');
        if path.is_empty() {
            return None;
        }
        match path.find('/') {
            Some(idx) => Some((&path[..idx], &path[idx + 1..])),
            None => Some((path, "")),
        }
    }

    /// 静态工厂：从 InvokeRequest 构造 Plugin 实例
    pub fn build(ctx: Arc<dyn InvokeRequest>) -> Arc<dyn Plugin> {
        // 子插件的父引用要指向**本容器自己**，而它在 `Arc::new_cyclic` 之前还不存在
        // ——这正是那个构造器存在的理由（子插件经 `ctx.parent()` 回到容器；
        // 运行期挂载新插件时同样要用它，见 `PluginRegistry::mount_child`）。
        let composite: Arc<Self> = Arc::new_cyclic(|me| {
            // 先把 `me` 落到具体类型上再转 trait object：直接写
            // `let parent: Weak<dyn Plugin> = me.clone()` 会让 `new_cyclic` 的
            // 类型参数被推断成 `dyn Plugin`（闭包于是要返回一个未定大小的值）。
            let me: Weak<Composite> = me.clone();
            let parent: Weak<dyn Plugin> = me;
            Self::with_registry(Arc::new(PluginRegistry::new(Arc::clone(&ctx), parent)))
        });

        // 装配：补出必需插件的目录，再把当下该挂的都挂上。
        // 这两步与运行期启停/安装走的是**同一批方法**——装配不是特殊路径。
        composite.registry.ensure_required();
        composite.registry.mount_all();

        composite as Arc<dyn Plugin>
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new(PLUGIN_COMPOSITE, "通用插件容器")
            .with_description("通用的插件容器，可以管理子插件实例，支持嵌套")
            .with_version("0.1.0")
    }
}

crate::submit_object_creator!(PLUGIN_COMPOSITE, Composite::build, dyn Plugin);

/// 向子插件**广播**一次收集；只有**真失败**才留痕。
///
/// 收集是广播（契约见 `symbio_core/option.rs`）：宿主对每个子插件发一次问，由插件
/// 按 `ctx[PATH]` 自己决定贡不贡献。**不参与**的插件回答
/// `NotFound("未知遍历路径: …")`，那是它的正常答复，不是失败——每个子插件都 warn
/// 一次会把真正的失败埋进噪音里（启动期实测：同一条消息每个子插件各来两遍）。
///
/// 真正的收集期失败另有**专门通道**：`capability_error.rs` 的 `report_error` /
/// `take_errors`（session 编排方在收集结束后统一裁决）。拿 `traverse` 的返回值当
/// 失败信号，是把「路由层的回答」误当成「收集层的结果」——这两层不该由同一个
/// `Err` 表达。
///
/// 能力收集与配置声明两处共用它，是为了让这条判据**只写一次**：分散成两处时，
/// 第三处出现时最容易照抄错的那一半。
pub(crate) async fn broadcast_collect(
    plugin: Arc<dyn Plugin>,
    ctx: Arc<dyn InvokeRequest>,
    who: &str,
) {
    match plugin.traverse(String::new(), ctx).await {
        Ok(_) => {}
        Err(e) if collect_declined(&e) => {}
        Err(e) => crate::plugin_warn!("composite", "收集 {who} 失败：{e}"),
    }
}

/// 这个错误是否只是「本插件**不参与**这次收集」。
///
/// 抽成函数是为了让这条判据**可被测试钉住**：它正是「启动期每个子插件刷一条
/// WARN」的根因判断，埋在 `broadcast_collect` 里就只能靠人记得。
pub(crate) fn collect_declined(e: &PluginError) -> bool {
    matches!(e, PluginError::NotFound(_))
}

#[async_trait::async_trait]
impl Plugin for Composite {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    async fn route(self: Arc<Self>, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload> {
        let path = ctx.get(PATH).unwrap_or_default();
        let path = path.strip_prefix('/').unwrap_or(&path);

        // 子插件分发
        if let Some((name, rest)) = Self::parse_path(path) {
            // 快照取完即释放锁——随后要走 `await`（子插件路由），持锁跨 await
            // 会让一次慢请求阻塞整个容器。
            let plugin_opt = lock_read(self.registry.instances()).get(name).cloned();

            if let Some(plugin) = plugin_opt {
                let child_ctx = ctx.fork();
                child_ctx.set(PATH, rest.to_string());
                // 跨挂载边界的转发：改写子上下文的**当前父地址**——从 ctx 已携带
                // 的父地址续接（嵌套容器自动得到 `<根>/…/<名字>` 的完整挂载点；
                // 见 `symbio_core::vdfs::address` 的改写规则）
                child_ctx.set(
                    VDFS_PARENT_ADDR,
                    descend_addr(&ctx.get(VDFS_PARENT_ADDR).unwrap_or_default(), name),
                );
                return plugin.route(child_ctx).await;
            }
        }

        Err(PluginError::NotFound(format!(
            "Composite: 路径 '{path}' 无法识别或子插件未挂载"
        )))
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        ctx: Arc<dyn InvokeRequest>,
    ) -> InvokeResponse<PluginPayload> {
        // 装配安排：容器把自己的 vdfs 视图登记进访问层的单槽位（LLM 链路用）。
        // 这不是「容器是根」——composite 只是恰好包含若干子目录的 provider，
        // 能否出现在那里取决于装配，不是本模块的属性。系统链路取同一个根走的是
        // `Plugin::get_vfs_provider`（见下），与 `CapabilityVisitor` 无关。
        if ctx.get(PATH).as_deref() == Some(TRAVERSE_AVAILABLE_TOOLS) {
            if let Some(visitor) = ctx.get(CAPABILITY_VISITOR) {
                let root: Arc<dyn VdfsProvider> = self.vdfs.clone();
                visitor.register_vdfs_root(root).await;
            }
        }

        for (name, plugin) in self.registry.snapshot() {
            let req_ctx = ctx.fork();
            // 能力收集同样是跨挂载边界的转发：子插件在收集期拼协议级绝对地址
            // （如提示词片段里的可编辑地址），靠的就是这里的当前父地址。
            // 从 **ctx 已携带的父地址续接**而非落回系统根——本容器自身可能是
            // 嵌套装配（如子智能体子树挂在 `<根>/agent/<id>` 下），落回系统根
            // 会让子插件收集期拼出与实际挂载不符的地址。
            req_ctx.set(
                VDFS_PARENT_ADDR,
                descend_addr(&ctx.get(VDFS_PARENT_ADDR).unwrap_or_default(), &name),
            );
            // 收集是广播：不参与的插件回答 `NotFound`，那不是失败。判据与理由在
            // `broadcast_collect` 里，两处收集共用同一份，避免各写一半。
            broadcast_collect(plugin, req_ctx, &name).await;
        }

        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }

    /// 系统链路：`Composite` 直接暴露自己的组合视图（[`CompositeVdfs`]）。
    ///
    /// 容器把子插件的 vfs provider 聚合进它，子智能体的 `agent/<id>` 挂载点也经它
    /// 往下钻——`agent` 插件拿到子 composite 的 `Arc<dyn Plugin>` 后调本方法即可取回
    /// 同一个 provider，无需任何类型耦合（见 `plugins/agent/host/vdfs.rs` 的 `sub_vfs`）。
    ///
    /// 插件管理插件也走这条：它经 `ctx.parent()` 拿到本容器，取回这个 provider，
    /// 再以注册表动词读写插件集合（见 `plugins/plugin_manager/plugin.rs`）。
    fn get_vfs_provider(self: Arc<Self>) -> Option<Arc<dyn VdfsProvider>> {
        let root: Arc<dyn VdfsProvider> = self.vdfs.clone();
        Some(root)
    }
}

#[cfg(test)]
#[path = "composite.test.rs"]
mod tests;
