//! 容器的 VDFS —— 一个包含子目录（子 vdfs_provider）列表的 provider
//!
//! ## 职责
//!
//! 容器把子插件聚合为一棵目录树：**子插件只要在
//! `traverse(TRAVERSE_AVAILABLE_TOOLS)` 中注册了 `VdfsProvider`，就以自己给出的
//! 目录名（约定 = 插件名）成为本目录下的一个子目录**。没有注册的子插件不出现在
//! 树里，容器也不需要认识任何具体资源。
//!
//! 本模块自身实现 [`VdfsProvider`]（没有任何中间结构）：
//!
//! | 职责 | 说明 |
//! |---|---|
//! | 列自身目录 | `list("")` 返回子目录清单（合成，无需子 provider 参与） |
//! | 路径解析 | 首段 = 子目录名，其余 = 该子 provider 的**相对路径** |
//! | 全路径回填 | 子节点 / 内容 / 写入响应的 `path` 补成 `<子目录>/<rel>` |
//! | 子目录根守卫 | 子目录根不可读 / 写 / 删，也不可 mkdir / move |
//! | 跨子目录拒绝 | `move` 只允许在同一子目录内 |
//! | 事件补全 | 子 provider 报出的相对路径补成树内全路径再交给上层 sink |
//!
//! ## 它不是根，也没有任何「根」的概念
//!
//! composite 只是一个**恰好包含若干子目录的 provider**——它可以被别的目录包含，
//! 子插件本身也可以是另一个 composite（嵌套时它同样只是普通 provider）。当前它
//! 充当整个 `.vdfs` 的服务者，**只是装配时的安排**（使用方把它登记进了
//! `register_vdfs_root` 槽位），不是本模块的属性；本文件不出现任何「根级别」
//! 的概念与代码。
//!
//! ## 为什么逐子插件单独广播
//!
//! 一次广播把整棵树都收进同一个收集器，就无法区分某个 provider 是谁注册的。
//! 逐个向子插件广播、每个子插件配一个独立收集器，得到的必然是该子插件自己的
//! 注册项——目录名因此天然归属于「注册它的那个子插件」。
//!
//! ## 访问层
//!
//! 本视图在容器的 `traverse` 里被登记为 `.vdfs` 的服务者（`register_vdfs_root`）。
//! vdfs 插件只取这个登记项再转发，不认识本模块——拓扑知识因此不落在访问层。

use crate::symbio_core::vdfs::host_ctx;
use crate::symbio_core::vdfs_provider::*;
use crate::symbio_core::{
    CapabilityVisitor, DefaultToolVisitor, InvokeRequestExt, Plugin, CAPABILITY_VISITOR, PATH,
    TRAVERSE_AVAILABLE_TOOLS,
};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// 容器的 VDFS（包含子目录列表的 provider，见模块文档）
pub struct CompositeVdfs {
    /// 与容器共享同一份子插件表（容器增删实例即时可见）
    instances: Arc<RwLock<HashMap<String, Arc<dyn Plugin>>>>,
}

/// 拆分树内路径：`(首段, 其余)`；根（空路径）返回 `None`
fn split_first(path: &str) -> Option<(&str, &str)> {
    match path.find('/') {
        Some(i) => Some((&path[..i], &path[i + 1..])),
        None if path.is_empty() => None,
        None => Some((path, "")),
    }
}

/// 子树内全路径 = `<子目录>/…`；任一段为空则跳过（不产生空段或多余分隔符）
fn child_path(dir: &str, rel: &str) -> String {
    match (dir.is_empty(), rel.is_empty()) {
        (true, true) => String::new(),
        (true, false) => rel.to_string(),
        (false, true) => dir.to_string(),
        (false, false) => format!("{dir}/{rel}"),
    }
}

/// 回填机制级字段：`path`（树内全路径）、`ext`（缺省由 `name` 推导）、`title`（缺省同 name）
fn fill_node_paths(dir: &str, base_rel: &str, nodes: &mut [VdfsNode]) {
    for n in nodes.iter_mut() {
        if n.path.is_empty() {
            n.path = child_path(dir, &child_path(base_rel, &n.name));
        }
        if n.ext.is_none() {
            n.ext = derive_ext(&n.name);
        }
        if n.title.is_empty() {
            n.title = n.name.clone();
        }
    }
}

impl CompositeVdfs {
    pub fn new(instances: Arc<RwLock<HashMap<String, Arc<dyn Plugin>>>>) -> Self {
        Self { instances }
    }

    /// 现场收集：逐个向子插件广播一次能力收集，取回各自注册的 `(目录名, provider)`，
    /// 按 `order` 升序排序（同序按目录名、插件名兜底，使结果不依赖子插件的枚举顺序）。
    ///
    /// 不缓存——子插件集合与注册内容由配置与生命周期决定，每次现取才与容器一致。
    ///
    /// 目录名是路径首段的唯一键：**重名时保留排序后首个**并 `warn`。若不处理，
    /// `list("")` 会给出两个同名节点而 `resolve` 只能命中一个——后来者**完全不可达**。
    /// 因此**先排序、再去重**：胜出者由 `(order, 目录名, 插件名)` 唯一确定，重名是装配
    /// 错误，必须在日志里可见而不是静默丢弃。
    async fn children_of(&self, ctx: &VdfsContext) -> VdfsResult<Vec<(String, DynVdfsProvider)>> {
        let host = host_ctx(ctx)?;
        let children: Vec<(String, Arc<dyn Plugin>)> = {
            let guard = self.instances.read().await;
            guard
                .iter()
                .map(|(name, p)| (name.clone(), Arc::clone(p)))
                .collect()
        };

        // `(注册它的子插件, 目录名, provider)`——带上归属，重名告警才点得出双方
        let mut collected: Vec<(String, String, DynVdfsProvider)> = Vec::new();
        for (plugin, child) in children {
            let visitor: Arc<dyn CapabilityVisitor> = Arc::new(DefaultToolVisitor::new());
            let sub = host.fork();
            sub.set(PATH, TRAVERSE_AVAILABLE_TOOLS.to_string());
            sub.set(CAPABILITY_VISITOR, visitor.clone());

            if let Err(e) = child.clone().traverse(String::new(), sub).await {
                crate::plugin_warn!("composite", "vdfs: 子插件遍历失败，已跳过其子目录: {e:?}");
            }
            collected.extend(
                visitor
                    .list_vdfs_providers()
                    .await
                    .into_iter()
                    .map(|(dir, p)| (plugin.clone(), dir, p)),
            );
        }
        collected.sort_by(|a, b| (a.2.order(), &a.1, &a.0).cmp(&(b.2.order(), &b.1, &b.0)));

        let mut dirs: Vec<(String, DynVdfsProvider)> = Vec::with_capacity(collected.len());
        // 目录名 → 已胜出的注册者（仅用于重名告警）
        let mut owners: HashMap<String, String> = HashMap::new();
        for (plugin, dir, p) in collected {
            if let Some(prev) = owners.get(&dir) {
                crate::plugin_warn!(
                    "composite",
                    "vdfs: 目录名「{dir}」被 {prev} 与 {plugin} 重复注册，保留 {prev}（{plugin} 不可达）"
                );
                continue;
            }
            owners.insert(dir.clone(), plugin);
            dirs.push((dir, p));
        }
        Ok(dirs)
    }

    /// 自身节点（`""`）——合成的目录节点，子节点为各子插件子目录
    fn self_node(dirs: &[(String, DynVdfsProvider)]) -> VdfsNode {
        let mut n = VdfsNode::dir(
            "",
            "系统",
            VdfsAccess {
                list: true,
                traverse: dirs.iter().any(|(_, p)| p.root_access().traverse),
                ..VdfsAccess::NONE
            },
        );
        n.description = Some("系统资源；子节点为各插件子目录".to_string());
        n
    }

    /// 子目录节点（`<dir>`）——合成的目录节点，携带子 provider 的自述
    fn dir_node(dir: &str, p: &DynVdfsProvider) -> VdfsNode {
        let mut n = VdfsNode::dir(
            dir.to_string(),
            p.label().unwrap_or(dir).to_string(),
            p.root_access(),
        );
        n.path = dir.to_string();
        n.status = p.root_status().to_string();
        n.description = p.description().map(str::to_string);
        n.new_types = p.root_new_types();
        n
    }

    /// 树内路径 → `(子目录名, provider, 相对路径)`；自身目录或无匹配时按错误返回
    fn resolve<'a>(
        dirs: &'a [(String, DynVdfsProvider)],
        path: &str,
    ) -> VdfsResult<(&'a str, &'a DynVdfsProvider, String)> {
        let Some((dir, rel)) = split_first(path) else {
            return Err(VdfsError::invalid(
                "目录不是可操作节点，请给出 <子目录>/... 路径",
            ));
        };
        let (d, p) = dirs.iter().find(|(name, _)| name == dir).ok_or_else(|| {
            VdfsError::not_found(format!(
                "目录不存在：{dir}（现有：{}）",
                Self::names_hint(dirs)
            ))
        })?;
        Ok((d.as_str(), p, rel.to_string()))
    }

    /// 子目录名清单（报错提示用）
    fn names_hint(dirs: &[(String, DynVdfsProvider)]) -> String {
        if dirs.is_empty() {
            return "（无）".to_string();
        }
        dirs.iter()
            .map(|(d, _)| d.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// 九个操作都是同一件事：现场取子目录清单 → 解析 → 委派。
///
/// 路径是**树内相对路径**（`""` = 根）；首段拆分与回填在本文件内完成。
#[async_trait]
impl VdfsProvider for CompositeVdfs {
    fn label(&self) -> Option<&str> {
        Some("系统")
    }

    fn description(&self) -> Option<&str> {
        Some("系统资源；子节点为各插件子目录")
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
        let dirs = self.children_of(ctx).await?;
        match split_first(path) {
            // 自身目录：子目录清单（合成，无需子 provider 参与）
            None => Ok(dirs.iter().map(|(d, p)| Self::dir_node(d, p)).collect()),
            Some(_) => {
                let (dir, p, rel) = Self::resolve(&dirs, path)?;
                let mut items = p.list(ctx, &rel).await?;
                fill_node_paths(dir, &rel, &mut items);
                Ok(items)
            }
        }
    }

    async fn stat(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<VdfsNode> {
        let dirs = self.children_of(ctx).await?;
        let Some((dir, _rel)) = split_first(path) else {
            return Ok(Self::self_node(&dirs));
        };
        let (_, p, rel) = Self::resolve(&dirs, path)?;
        if rel.is_empty() {
            return Ok(Self::dir_node(dir, p));
        }
        let mut n = p.stat(ctx, &rel).await?;
        n.path = path.to_string();
        Ok(n)
    }

    async fn read(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<VdfsContent> {
        let dirs = self.children_of(ctx).await?;
        let (_, p, rel) = Self::resolve(&dirs, path)?;
        if rel.is_empty() {
            return Err(VdfsError::Forbidden(
                "目录不是可读文件；请读取其子节点".to_string(),
            ));
        }
        let mut c = p.read(ctx, &rel).await?;
        if c.path.is_empty() {
            c.path = path.to_string();
        }
        Ok(c)
    }

    async fn write(
        &self,
        ctx: &VdfsContext,
        path: &str,
        content: &VdfsContent,
    ) -> VdfsResult<VdfsWriteResponse> {
        let dirs = self.children_of(ctx).await?;
        let (_, p, rel) = Self::resolve(&dirs, path)?;
        if rel.is_empty() {
            return Err(VdfsError::Forbidden("目录不可写".to_string()));
        }
        let mut r = p.write(ctx, &rel, content).await?;
        if r.path.is_empty() {
            r.path = path.to_string();
        }
        Ok(r)
    }

    async fn delete(&self, ctx: &VdfsContext, path: &str, recursive: bool) -> VdfsResult<()> {
        let dirs = self.children_of(ctx).await?;
        let (_, p, rel) = Self::resolve(&dirs, path)?;
        if rel.is_empty() {
            return Err(VdfsError::Forbidden("目录不可删除".to_string()));
        }
        p.delete(ctx, &rel, recursive).await
    }

    async fn mkdir(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<()> {
        let dirs = self.children_of(ctx).await?;
        let (_, p, rel) = Self::resolve(&dirs, path)?;
        if rel.is_empty() {
            return Err(VdfsError::invalid("目录已存在，无需创建"));
        }
        p.mkdir(ctx, &rel).await
    }

    async fn move_item(&self, ctx: &VdfsContext, from: &str, to: &str) -> VdfsResult<()> {
        let dirs = self.children_of(ctx).await?;
        let (from_dir, pf, rf) = Self::resolve(&dirs, from)?;
        let (to_dir, _, rt) = Self::resolve(&dirs, to)?;
        if from_dir != to_dir {
            return Err(VdfsError::invalid(format!(
                "不支持跨目录移动：{from_dir} → {to_dir}"
            )));
        }
        if rf.is_empty() || rt.is_empty() {
            return Err(VdfsError::Forbidden("目录不可移动".to_string()));
        }
        pf.move_item(ctx, &rf, &rt).await
    }

    async fn action(
        &self,
        ctx: &VdfsContext,
        path: &str,
        action: &str,
        payload: Option<&Value>,
    ) -> VdfsResult<VdfsActionResult> {
        let dirs = self.children_of(ctx).await?;
        let (_, p, rel) = Self::resolve(&dirs, path)?;
        p.action(ctx, &rel, action, payload).await
    }

    /// 子 provider 报出的相对路径在此补成树内全路径，再交给上层 sink。
    ///
    /// 用 [`VdfsChange::map_paths`] 一次覆盖全部路径（含 `node` 载荷内的路径），
    /// 不逐字段重建——新增字段时不会漏转发。
    async fn watch(&self, ctx: &VdfsContext, path: &str, sink: VdfsChangeSink) -> VdfsResult<()> {
        let dirs = self.children_of(ctx).await?;
        let (dir, p, rel) = Self::resolve(&dirs, path)?;
        let dir = dir.to_string();
        let wrapped: VdfsChangeSink =
            Arc::new(move |c: VdfsChange| sink(c.map_paths(|p| child_path(&dir, p))));
        p.watch(ctx, &rel, wrapped).await
    }

    async fn unwatch(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<()> {
        let dirs = self.children_of(ctx).await?;
        let (_, p, rel) = Self::resolve(&dirs, path)?;
        p.unwatch(ctx, &rel).await
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

    /// 假子插件：`traverse` 时按目录名注册自己的 provider（约定 = 插件名）
    struct FakeChild {
        dir: &'static str,
        label: &'static str,
        order: i32,
    }

    #[async_trait]
    impl Plugin for FakeChild {
        fn meta(&self) -> PluginMeta {
            PluginMeta::new(self.dir, self.dir)
        }

        async fn route(self: Arc<Self>, _ctx: Arc<dyn InvokeRequest>) -> InvokeResponse {
            Err(PluginError::NotFound(self.dir.to_string()))
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
                            self.dir,
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
            map.insert(c.dir.to_string(), Arc::new(c));
        }
        CompositeVdfs::new(Arc::new(RwLock::new(map)))
    }

    fn host_ctx() -> VdfsContext {
        let host: Arc<dyn InvokeRequest> = Arc::new(SimpleRequest::new(None, None));
        vdfs_context(&host)
    }

    /// 子插件的子目录以**自己给的目录名**出现在本目录下，并按 order 升序
    #[tokio::test]
    async fn self_listing_shows_child_dirs_by_registered_name() {
        let vdfs = container(vec![
            FakeChild {
                dir: "alpha",
                label: "甲",
                order: 20,
            },
            FakeChild {
                dir: "beta",
                label: "乙",
                order: 10,
            },
        ]);
        let ctx = host_ctx();

        let root = vdfs.list(&ctx, "").await.unwrap();
        assert_eq!(root.len(), 2);
        assert_eq!(
            root.iter().map(|n| n.name.as_str()).collect::<Vec<_>>(),
            vec!["beta", "alpha"],
            "目录名来自子插件自己的注册；顺序按 order"
        );
        assert_eq!(root[0].path, "beta");
        assert_eq!(root[0].title, "乙");
        assert_eq!(root[0].kind, VFDS_KIND_DIR);

        let root_node = vdfs.stat(&ctx, "").await.unwrap();
        assert_eq!(root_node.path, "");
        assert!(root_node.is_dir());
    }

    /// 每个子插件用**独立**收集器：provider 不会张冠李戴
    #[tokio::test]
    async fn per_child_collection_keeps_ownership() {
        let vdfs = container(vec![
            FakeChild {
                dir: "alpha",
                label: "甲",
                order: 1,
            },
            FakeChild {
                dir: "beta",
                label: "乙",
                order: 2,
            },
        ]);
        let ctx = host_ctx();
        let dirs = vdfs.children_of(&ctx).await.unwrap();

        for (dir, label) in [("alpha", "甲"), ("beta", "乙")] {
            let (name, p) = dirs.iter().find(|(n, _)| n == dir).expect("该子目录应存在");
            assert_eq!(name, dir);
            assert_eq!(p.label(), Some(label), "provider 与目录名一一对应");
        }
    }

    /// 两个子插件注册同一目录名 → 只保留排序后首个，不产生同名节点（后者本不可达）
    #[tokio::test]
    async fn duplicate_dir_names_keep_the_first_only() {
        // 插件名与目录名解耦：两个**不同**插件注册同一个目录名
        let mut map: HashMap<String, Arc<dyn Plugin>> = HashMap::new();
        map.insert(
            "plugin_a".to_string(),
            Arc::new(FakeChild {
                dir: "same",
                label: "甲",
                order: 2,
            }),
        );
        map.insert(
            "plugin_b".to_string(),
            Arc::new(FakeChild {
                dir: "same",
                label: "乙",
                order: 1,
            }),
        );
        let vdfs = CompositeVdfs::new(Arc::new(RwLock::new(map)));
        let ctx = host_ctx();

        let dirs = vdfs.children_of(&ctx).await.unwrap();
        assert_eq!(dirs.len(), 1, "重名只保留一份，否则会列出两个同名子目录");
        assert_eq!(dirs[0].0, "same");
        assert_eq!(
            dirs[0].1.label(),
            Some("乙"),
            "胜出者由 order 决定（1 < 2），不依赖 HashMap 的枚举顺序"
        );

        let root = vdfs.list(&ctx, "").await.unwrap();
        assert_eq!(root.len(), 1);
        assert_eq!(root[0].name, "same");
    }

    /// 子树内的路径被拆回相对路径交给叶子 provider，返回项回填树内全路径
    #[tokio::test]
    async fn delegates_relative_path_and_fills_full_path() {
        let vdfs = container(vec![FakeChild {
            dir: "alpha",
            label: "甲",
            order: 1,
        }]);
        let ctx = host_ctx();

        let items = vdfs.list(&ctx, "alpha").await.unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].path, "alpha/a.txt", "相对路径被回填成树内全路径");

        let c = vdfs.read(&ctx, "alpha/a.txt").await.unwrap();
        assert_eq!(
            c.text.as_deref(),
            Some("leaf:a.txt"),
            "provider 收到的是相对路径"
        );
        assert_eq!(c.path, "alpha/a.txt");
    }

    /// 无子插件 → 空树（自身目录仍可列出）
    #[tokio::test]
    async fn empty_container_is_an_empty_vfs() {
        let vdfs = container(Vec::new());
        let ctx = host_ctx();
        assert!(vdfs.list(&ctx, "").await.unwrap().is_empty());
        assert_eq!(vdfs.stat(&ctx, "").await.unwrap().path, "");
    }

    /// 自身目录与子目录的守卫：不可读 / 写 / 删 / 移，mkdir 报已存在
    #[tokio::test]
    async fn guards_self_and_child_dir_roots() {
        let vdfs = container(vec![FakeChild {
            dir: "alpha",
            label: "甲",
            order: 1,
        }]);
        let ctx = host_ctx();

        assert!(matches!(
            vdfs.read(&ctx, "alpha").await.unwrap_err(),
            VdfsError::Forbidden(_)
        ));
        assert!(matches!(
            vdfs.write(&ctx, "alpha", &VdfsContent::text("", "x"))
                .await
                .unwrap_err(),
            VdfsError::Forbidden(_)
        ));
        assert!(matches!(
            vdfs.delete(&ctx, "alpha", true).await.unwrap_err(),
            VdfsError::Forbidden(_)
        ));
        assert!(matches!(
            vdfs.mkdir(&ctx, "alpha").await.unwrap_err(),
            VdfsError::Invalid(_)
        ));
        // 自身目录也不可操作
        assert!(matches!(
            vdfs.read(&ctx, "").await.unwrap_err(),
            VdfsError::Invalid(_)
        ));
    }

    /// 未知目录明确报错；跨子目录移动被拒；同子目录内移动转发相对路径
    #[tokio::test]
    async fn rejects_unknown_and_cross_dir_moves() {
        let vdfs = container(vec![
            FakeChild {
                dir: "alpha",
                label: "甲",
                order: 1,
            },
            FakeChild {
                dir: "beta",
                label: "乙",
                order: 2,
            },
        ]);
        let ctx = host_ctx();

        let err = vdfs.list(&ctx, "nope").await.unwrap_err();
        assert!(matches!(err, VdfsError::NotFound(_)));
        assert!(err.to_string().contains("alpha"), "提示现有目录");

        assert!(matches!(
            vdfs.move_item(&ctx, "alpha/a", "beta/a").await.unwrap_err(),
            VdfsError::Invalid(_)
        ));
    }

    /// 宿主句柄缺失（provider 未随 symbio 上下文调用）→ 明确报错而非静默空树
    #[tokio::test]
    async fn missing_host_ctx_is_an_internal_error() {
        let vdfs = container(Vec::new());
        // Ok 侧不可 `Debug`，故不能用 unwrap_err
        let err = match vdfs.children_of(&VdfsContext::empty()).await {
            Ok(_) => panic!("缺宿主句柄时应报错"),
            Err(e) => e,
        };
        assert_eq!(err.code(), "INTERNAL_ERROR");
    }
}
