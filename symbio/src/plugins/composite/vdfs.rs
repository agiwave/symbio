//! 容器的 VDFS 组合视图 —— 虚拟根 `/` 的拥有者
//!
//! ## 职责
//!
//! 容器把自己的子插件聚合为一棵 VDFS 子树：**子插件只要在
//! `traverse(TRAVERSE_AVAILABLE_TOOLS)` 中注册了 `VdfsProvider`，就以自己给出的
//! 挂载名（约定 = 插件名）出现在虚拟根下**成为子目录。没有注册的子插件不出现在
//! 树里，容器也不需要认识任何具体资源。
//!
//! 具体的拓扑解析（首段 = 挂载名、全路径回填、挂载根守卫、跨挂载点拒绝、
//! 变更事件补全路径）由 [`VdfsMountTable`] 承担——那是核心里的通用机制，
//! 本模块只负责「**把子插件的挂载收集起来**」这一件容器特有的事。
//!
//! ## 为什么逐子插件单独广播
//!
//! 一次广播把整棵树都收进同一个收集器，就无法区分某个 provider 是谁注册的。
//! 逐个向子插件广播、每个子插件配一个独立收集器，得到的必然是该子插件自己的
//! 注册项——挂载名因此天然归属于「注册它的那个子插件」。
//!
//! ## 访问层
//!
//! 本视图在容器的 `traverse` 里被登记为 VDFS **根**（`register_vdfs_root`）。
//! vdfs 插件只取这个根再转发，不认识本模块——拓扑知识因此不落在访问层。

use crate::symbio_core::vdfs::host_ctx;
use crate::symbio_core::vdfs_provider::*;
use crate::symbio_core::{
    CapabilityVisitor, DefaultToolVisitor, InvokeRequestExt, Plugin, CAPABILITY_VISITOR, PATH,
    TRAVERSE_AVAILABLE_TOOLS,
};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// 容器的 VDFS 组合视图
pub struct CompositeVdfs {
    /// 与容器共享同一份子插件表（容器增删实例即时可见）
    instances: Arc<RwLock<HashMap<String, Arc<dyn Plugin>>>>,
}

impl CompositeVdfs {
    pub fn new(instances: Arc<RwLock<HashMap<String, Arc<dyn Plugin>>>>) -> Self {
        Self { instances }
    }

    /// 现场收集：逐个向子插件广播一次能力收集，取回各自注册的 `(挂载名, provider)`。
    ///
    /// 不缓存——子插件集合与注册内容由配置与生命周期决定，每次现取才与容器一致。
    pub async fn table_of(&self, ctx: &VdfsContext) -> VdfsResult<VdfsMountTable> {
        let host = host_ctx(ctx)?;
        let children: Vec<Arc<dyn Plugin>> = {
            let guard = self.instances.read().await;
            guard.values().cloned().collect()
        };

        let mut mounts: VdfsMounts = Vec::new();
        for child in children {
            let visitor: Arc<dyn CapabilityVisitor> = Arc::new(DefaultToolVisitor::new());
            let sub = host.fork();
            sub.set(PATH, TRAVERSE_AVAILABLE_TOOLS.to_string());
            sub.set(CAPABILITY_VISITOR, visitor.clone());

            if let Err(e) = child.clone().traverse(String::new(), sub).await {
                crate::plugin_warn!("composite", "vdfs: 子插件遍历失败，已跳过其挂载: {e:?}");
            }
            mounts.extend(visitor.list_vdfs_providers().await);
        }
        Ok(VdfsMountTable::new(mounts))
    }
}

/// 九个操作都是同一件事：现场取组合视图 → 委派。
///
/// 路径是**已规范化的全路径**；相对路径的拆分与回填在 [`VdfsMountTable`] 内完成。
#[async_trait]
impl VdfsProvider for CompositeVdfs {
    fn label(&self) -> Option<&str> {
        Some("系统")
    }

    fn description(&self) -> Option<&str> {
        Some("容器聚合视图：虚拟根及其下的各插件子树")
    }

    fn order(&self) -> i32 {
        0
    }

    fn root_access(&self) -> VdfsAccess {
        VdfsAccess {
            list: true,
            traverse: true,
            ..VdfsAccess::NONE
        }
    }

    async fn list(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<Vec<VdfsNode>> {
        self.table_of(ctx).await?.list(ctx, path).await
    }

    async fn stat(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<VdfsNode> {
        self.table_of(ctx).await?.stat(ctx, path).await
    }

    async fn read(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<VdfsContent> {
        self.table_of(ctx).await?.read(ctx, path).await
    }

    async fn write(
        &self,
        ctx: &VdfsContext,
        path: &str,
        content: &VdfsContent,
    ) -> VdfsResult<VdfsWriteResponse> {
        self.table_of(ctx).await?.write(ctx, path, content).await
    }

    async fn delete(&self, ctx: &VdfsContext, path: &str, recursive: bool) -> VdfsResult<()> {
        self.table_of(ctx).await?.delete(ctx, path, recursive).await
    }

    async fn mkdir(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<()> {
        self.table_of(ctx).await?.mkdir(ctx, path).await
    }

    async fn move_item(&self, ctx: &VdfsContext, from: &str, to: &str) -> VdfsResult<()> {
        self.table_of(ctx).await?.move_item(ctx, from, to).await
    }

    async fn watch(&self, ctx: &VdfsContext, path: &str, sink: VdfsChangeSink) -> VdfsResult<()> {
        self.table_of(ctx).await?.watch(ctx, path, sink).await
    }

    async fn unwatch(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<()> {
        self.table_of(ctx).await?.unwatch(ctx, path).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbio_core::vdfs::vdfs_context;
    use crate::symbio_core::{
        InvokeRequest, PluginError, PluginMeta, PluginPayload, SimpleRequest,
    };

    /// 只暴露一个 `a.txt` 的 provider；标签 / 顺序可配，便于断言归属
    struct LeafProvider {
        label: &'static str,
        order: i32,
    }

    #[async_trait]
    impl VdfsProvider for LeafProvider {
        fn label(&self) -> Option<&str> {
            Some(self.label)
        }

        fn order(&self) -> i32 {
            self.order
        }

        async fn list(&self, _ctx: &VdfsContext, _path: &str) -> VdfsResult<Vec<VdfsNode>> {
            Ok(vec![VdfsNode::file("a.txt", "A", VdfsAccess::READ)])
        }

        async fn read(&self, _ctx: &VdfsContext, path: &str) -> VdfsResult<VdfsContent> {
            Ok(VdfsContent::text("", format!("leaf:{path}")))
        }
    }

    /// 假子插件：`traverse` 时按 `mount` 注册自己的 provider（约定 = 插件名）
    struct FakeChild {
        mount: &'static str,
        label: &'static str,
        order: i32,
    }

    #[async_trait]
    impl Plugin for FakeChild {
        fn meta(&self) -> PluginMeta {
            PluginMeta::new(self.mount, self.mount)
        }

        async fn route(self: Arc<Self>, _ctx: Arc<dyn InvokeRequest>) -> InvokeResponse {
            Err(PluginError::NotFound(self.mount.to_string()))
        }

        async fn traverse(
            self: Arc<Self>,
            _path: String,
            ctx: Arc<dyn InvokeRequest>,
        ) -> InvokeResponse {
            if ctx.get(PATH).as_deref() == Some(TRAVERSE_AVAILABLE_TOOLS) {
                if let Some(visitor) = ctx.get(CAPABILITY_VISITOR) {
                    visitor
                        .register_vdfs_provider(
                            self.mount,
                            Arc::new(LeafProvider {
                                label: self.label,
                                order: self.order,
                            }),
                        )
                        .await;
                }
            }
            Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
        }
    }

    type InvokeResponse = crate::symbio_core::InvokeResponse<PluginPayload>;

    fn container(children: Vec<FakeChild>) -> CompositeVdfs {
        let mut map: HashMap<String, Arc<dyn Plugin>> = HashMap::new();
        for c in children {
            map.insert(c.mount.to_string(), Arc::new(c));
        }
        CompositeVdfs::new(Arc::new(RwLock::new(map)))
    }

    fn host_ctx() -> VdfsContext {
        let host: Arc<dyn InvokeRequest> = Arc::new(SimpleRequest::new(None, None));
        vdfs_context(&host)
    }

    /// 子插件的挂载以**自己给的挂载名**出现在根下，并按 order 升序
    #[tokio::test]
    async fn container_lists_children_mounts_by_registered_name() {
        let vdfs = container(vec![
            FakeChild {
                mount: "alpha",
                label: "甲",
                order: 20,
            },
            FakeChild {
                mount: "beta",
                label: "乙",
                order: 10,
            },
        ]);
        let ctx = host_ctx();

        let table = vdfs.table_of(&ctx).await.unwrap();
        assert_eq!(
            table
                .mounts()
                .iter()
                .map(|(m, _)| m.as_str())
                .collect::<Vec<_>>(),
            vec!["beta", "alpha"],
            "挂载名来自子插件自己的注册；顺序按 order"
        );

        let root = vdfs.list(&ctx, VFDS_ROOT).await.unwrap();
        assert_eq!(root.len(), 2);
        assert_eq!(root[0].name, "beta");
        assert_eq!(root[0].path, "/beta");
        assert_eq!(root[0].title, "乙");
        assert_eq!(root[0].kind, VFDS_KIND_MOUNT);
    }

    /// 每个子插件用**独立**收集器：provider 不会张冠李戴
    #[tokio::test]
    async fn per_child_collection_keeps_ownership() {
        let vdfs = container(vec![
            FakeChild {
                mount: "alpha",
                label: "甲",
                order: 1,
            },
            FakeChild {
                mount: "beta",
                label: "乙",
                order: 2,
            },
        ]);
        let ctx = host_ctx();
        let table = vdfs.table_of(&ctx).await.unwrap();

        for (mount, label) in [("alpha", "甲"), ("beta", "乙")] {
            let (name, p) = table
                .mounts()
                .iter()
                .find(|(m, _)| m == mount)
                .expect("该挂载应存在");
            assert_eq!(name, mount);
            assert_eq!(p.label(), Some(label), "provider 与挂载名一一对应");
        }
    }

    /// 根之下的全路径被拆回相对路径交给叶子 provider
    #[tokio::test]
    async fn container_delegates_relative_path_and_fills_full_path() {
        let vdfs = container(vec![FakeChild {
            mount: "alpha",
            label: "甲",
            order: 1,
        }]);
        let ctx = host_ctx();

        let items = vdfs.list(&ctx, "/alpha").await.unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].path, "/alpha/a.txt", "相对路径被回填成全路径");

        let c = vdfs.read(&ctx, "/alpha/a.txt").await.unwrap();
        assert_eq!(
            c.text.as_deref(),
            Some("leaf:a.txt"),
            "provider 收到的是相对路径"
        );
        assert_eq!(c.path, "/alpha/a.txt");
    }

    /// 无子插件 → 空树（虚拟根仍可列出）
    #[tokio::test]
    async fn empty_container_is_an_empty_vfs() {
        let vdfs = container(Vec::new());
        let ctx = host_ctx();
        assert!(vdfs.table_of(&ctx).await.unwrap().is_empty());
        assert!(vdfs.list(&ctx, VFDS_ROOT).await.unwrap().is_empty());
        assert_eq!(vdfs.stat(&ctx, VFDS_ROOT).await.unwrap().path, VFDS_ROOT);
    }

    /// 宿主句柄缺失（provider 未随 symbio 上下文调用）→ 明确报错而非静默空树
    #[tokio::test]
    async fn missing_host_ctx_is_an_internal_error() {
        let vdfs = container(Vec::new());
        // `VdfsMountTable` 持有 trait object，Ok 侧不可 `Debug`，故不能用 unwrap_err
        let err = match vdfs.table_of(&VdfsContext::empty()).await {
            Ok(_) => panic!("缺宿主句柄时应报错"),
            Err(e) => e,
        };
        assert_eq!(err.code(), "INTERNAL_ERROR");
    }
}
