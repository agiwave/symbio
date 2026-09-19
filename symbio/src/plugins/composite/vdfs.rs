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
//! | 子目录根守卫 | 子目录根不可读 / 删 / 移，也不可 mkdir（**写不在其中**） |
//! | 跨子目录拒绝 | `move` 只允许在同一子目录内 |
//! | 事件补全 | 子 provider 报出的相对路径补成树内全路径再交给上层 sink |
//!
//! ## 它不是根，也没有任何「根」的概念
//!
//! composite 只是一个**恰好包含若干子目录的 provider**——它可以被别的目录包含，
//! 子插件本身也可以是另一个 composite（嵌套时它同样只是普通 provider）。当前它
//! 充当整个 `<根>` 的服务者，**只是装配时的安排**（使用方把它登记进了
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
//! 本视图在容器的 `traverse` 里被登记为 `<根>` 的服务者（`register_vdfs_root`）。
//! vdfs 插件只取这个登记项再转发，不认识本模块——拓扑知识因此不落在访问层。

use crate::symbio_core::vdfs::{descend_addr, host_ctx};
use crate::symbio_core::vdfs_provider::*;
use crate::symbio_core::{
    CapabilityVisitor, ConfigurableVisitor, DefaultConfigurableVisitor, DefaultToolVisitor,
    InvokeRequestExt, Plugin, CAPABILITY_VISITOR, CONFIG_VISITOR, PATH, TRAVERSE_AVAILABLE_TOOLS,
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

        // 可配置声明通道（第三条收集通道）：与 VDFS provider 共用这次广播，但用
        // **共享收集器**——声明自带目录名，不存在归属歧义，所有子插件注册进同一个。
        // 收集结果**写回请求 ctx**：同一次请求里稍后被委派的子 provider（如设置插件）
        // 据此知道「哪些插件有配置文档」，无需自己反查插件目录、也无需硬编码清单。
        let configs: Arc<dyn ConfigurableVisitor> = match host.get(CONFIG_VISITOR) {
            Some(v) => v,
            None => {
                let v: Arc<dyn ConfigurableVisitor> = Arc::new(DefaultConfigurableVisitor::new());
                host.set(CONFIG_VISITOR, v.clone());
                v
            }
        };

        // `(注册它的子插件, 目录名, provider)`——带上归属，重名告警才点得出双方
        let mut collected: Vec<(String, String, DynVdfsProvider)> = Vec::new();
        for (plugin, child) in children {
            let visitor: Arc<dyn CapabilityVisitor> = Arc::new(DefaultToolVisitor::new());
            let sub = host.fork();
            sub.set(PATH, TRAVERSE_AVAILABLE_TOOLS.to_string());
            sub.set(CAPABILITY_VISITOR, visitor.clone());
            sub.set(CONFIG_VISITOR, configs.clone());

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
        // 子目录节点：它的隐藏属性来自子 provider 的根声明
        n.hidden = p.root_hidden();
        n
    }

    /// 子 provider 返回的路径 → 树内全路径。
    ///
    /// 子 provider 只认**自身子树内的相对路径**（见 `VdfsProvider::write`），所以它
    /// 回显的、或它为新条目生成的名字都只是 `<rel>`，必须补上 `<子目录>/` 才能交给上层
    /// （访问层还要再翻译成展示地址）。空串 = 「没填」（例如物理层无从表达新名字）
    /// → 用**请求地址**兜底。
    ///
    /// ⚠️ 不能只在空串时兜底：`write` 的返回值是使用方得知「刚建出来的东西在哪」的
    /// **唯一**途径（见 `VdfsProvider::write` 的返回值一节）。漏补即等于新建之后
    /// 找不到新节点——前端只能停在草稿上。
    fn fill_path(requested: &str, returned: &str) -> String {
        if returned.is_empty() {
            return requested.to_string();
        }
        let dir = split_first(requested).map(|(d, _)| d).unwrap_or("");
        child_path(dir, returned)
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

    /// 派发：解析出（目录名, 子 provider, 子树相对地址），并把 ctx 的**当前父
    /// 地址**改写为该目录的挂载点——这就是 vdfs 核心协议把相对地址转发给
    /// provider 的那一跳。
    ///
    /// 子 provider 收到的地址仍是子树相对地址（常态）；仅当它需要协议级绝对
    /// 地址时才取 `ctx.parent_addr()` 拼。改写规则见
    /// `symbio_core::vdfs::address`：容器自身的父地址（嵌套派发时由上级写入）
    /// 非空则原地续接，为空则落到静态声明的根。
    async fn dispatch(
        &self,
        ctx: &VdfsContext,
        dirs: &[(String, DynVdfsProvider)],
        path: &str,
    ) -> VdfsResult<(String, VdfsContext, DynVdfsProvider, String)> {
        let (dir, p, rel) = Self::resolve(dirs, path)?;
        let sub = ctx
            .clone()
            .with_parent_addr(descend_addr(ctx.parent_addr(), dir));
        Ok((dir.to_string(), sub, p.clone(), rel))
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
            // 自身目录：子目录清单（合成，无需子 provider 参与）。
            // **隐藏属性在这里生效**：`hidden` 的子目录不出现在清单里，
            // 但仍留在 `dirs` 中——目录本身照旧存在，按路径照常可寻址（见 `resolve`）。
            None => Ok(dirs
                .iter()
                .map(|(d, p)| Self::dir_node(d, p))
                .filter(|n| !n.hidden)
                .collect()),
            Some(_) => {
                let (dir, sub, p, rel) = self.dispatch(ctx, &dirs, path).await?;
                let mut items = p.list(&sub, &rel).await?;
                fill_node_paths(&dir, &rel, &mut items);
                // 隐藏属性是**机制级**的：任何子树里被标为 hidden 的子节点都不出现，
                // 不因它来自哪个 provider 而异。
                items.retain(|n| !n.hidden);
                Ok(items)
            }
        }
    }

    async fn stat(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<VdfsNode> {
        let dirs = self.children_of(ctx).await?;
        let Some((dir, _rel)) = split_first(path) else {
            return Ok(Self::self_node(&dirs));
        };
        let (_, sub, p, rel) = self.dispatch(ctx, &dirs, path).await?;
        if rel.is_empty() {
            return Ok(Self::dir_node(dir, &p));
        }
        let mut n = p.stat(&sub, &rel).await?;
        n.path = path.to_string();
        Ok(n)
    }

    async fn read(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<VdfsContent> {
        let dirs = self.children_of(ctx).await?;
        let (_, sub, p, rel) = self.dispatch(ctx, &dirs, path).await?;
        if rel.is_empty() {
            return Err(VdfsError::Forbidden(
                "目录不是可读文件；请读取其子节点".to_string(),
            ));
        }
        let mut c = p.read(&sub, &rel).await?;
        c.path = Self::fill_path(path, &c.path);
        Ok(c)
    }

    async fn write(
        &self,
        ctx: &VdfsContext,
        path: &str,
        content: &VdfsContent,
    ) -> VdfsResult<VdfsWriteResponse> {
        let dirs = self.children_of(ctx).await?;
        let (_, sub, p, rel) = self.dispatch(ctx, &dirs, path).await?;
        // `rel` 为空 = 写在**挂载点目录自身**上。这正是「新建」的机制形态：
        // 使用方只说「建在哪个目录」，不说「叫什么」——名字由 provider 生成
        // （见 `VdfsProvider::write` 的文档）。是否支持由 provider 判定，
        // 容器不做类型特判（与 `action` 对空 `rel` 的处理一致）。
        let mut r = p.write(&sub, &rel, content).await?;
        // provider 生成的名字（写在目录自身时）与它回显的相对路径都在这**一次**补齐。
        r.path = Self::fill_path(path, &r.path);
        Ok(r)
    }

    async fn delete(&self, ctx: &VdfsContext, path: &str, recursive: bool) -> VdfsResult<()> {
        let dirs = self.children_of(ctx).await?;
        let (_, sub, p, rel) = self.dispatch(ctx, &dirs, path).await?;
        if rel.is_empty() {
            return Err(VdfsError::Forbidden("目录不可删除".to_string()));
        }
        p.delete(&sub, &rel, recursive).await
    }

    async fn mkdir(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<()> {
        let dirs = self.children_of(ctx).await?;
        let (_, sub, p, rel) = self.dispatch(ctx, &dirs, path).await?;
        if rel.is_empty() {
            return Err(VdfsError::invalid("目录已存在，无需创建"));
        }
        p.mkdir(&sub, &rel).await
    }

    async fn move_item(&self, ctx: &VdfsContext, from: &str, to: &str) -> VdfsResult<()> {
        let dirs = self.children_of(ctx).await?;
        let (from_dir, sub, pf, rf) = self.dispatch(ctx, &dirs, from).await?;
        let (to_dir, _, _, rt) = self.dispatch(ctx, &dirs, to).await?;
        if from_dir != to_dir {
            return Err(VdfsError::invalid(format!(
                "不支持跨目录移动：{from_dir} → {to_dir}"
            )));
        }
        if rf.is_empty() || rt.is_empty() {
            return Err(VdfsError::Forbidden("目录不可移动".to_string()));
        }
        pf.move_item(&sub, &rf, &rt).await
    }

    async fn action(
        &self,
        ctx: &VdfsContext,
        path: &str,
        action: &str,
        payload: Option<&Value>,
    ) -> VdfsResult<VdfsActionResult> {
        let dirs = self.children_of(ctx).await?;
        let (_, sub, p, rel) = self.dispatch(ctx, &dirs, path).await?;
        p.action(&sub, &rel, action, payload).await
    }

    /// 子 provider 报出的相对路径在此补成树内全路径，再交给上层 sink。
    ///
    /// 用 [`VdfsChange::map_paths`] 一次覆盖全部路径（含 `node` 载荷内的路径），
    /// 不逐字段重建——新增字段时不会漏转发。
    async fn watch(&self, ctx: &VdfsContext, path: &str, sink: VdfsChangeSink) -> VdfsResult<()> {
        let dirs = self.children_of(ctx).await?;
        let (dir, sub, p, rel) = self.dispatch(ctx, &dirs, path).await?;
        let wrapped: VdfsChangeSink =
            Arc::new(move |c: VdfsChange| sink(c.map_paths(|p| child_path(&dir, p))));
        p.watch(&sub, &rel, wrapped).await
    }

    async fn unwatch(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<()> {
        let dirs = self.children_of(ctx).await?;
        let (_, sub, p, rel) = self.dispatch(ctx, &dirs, path).await?;
        p.unwatch(&sub, &rel).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbio_core::vdfs::vdfs_context;
    use crate::symbio_core::{
        InvokeRequest, PluginError, PluginMeta, PluginPayload, SimpleRequest,
    };

    /// 只暴露一个 `a.txt` 的 provider；标签 / 顺序 / 隐藏可配，便于断言归属
    struct LeafProvider {
        label: &'static str,
        order: i32,
        hidden: bool,
    }

    #[async_trait]
    impl VdfsProvider for LeafProvider {
        fn label(&self) -> Option<&str> {
            Some(self.label)
        }

        fn order(&self) -> i32 {
            self.order
        }

        fn root_hidden(&self) -> bool {
            self.hidden
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
        hidden: bool,
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
                                hidden: self.hidden,
                            }),
                        )
                        .await;
                }
            }
            Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
        }
    }

    type InvokeResponse = crate::symbio_core::InvokeResponse<PluginPayload>;

    /// 假子插件：`traverse` 时把**给定的** provider 注册到自己目录名下
    ///
    /// 与 [`FakeChild`] 的区别只有一个：那个固定注册 [`LeafProvider`]，这个让测试
    /// 自带一个只关心某一两个方法的 provider（如「只实现 `write`」的替身）。
    struct ProviderChild {
        dir: &'static str,
        provider: Arc<dyn VdfsProvider>,
    }

    #[async_trait]
    impl Plugin for ProviderChild {
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
                        .register_vdfs_provider(self.dir, self.provider.clone())
                        .await;
                }
            }
            Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
        }
    }

    fn container(children: Vec<FakeChild>) -> CompositeVdfs {
        let mut map: HashMap<String, Arc<dyn Plugin>> = HashMap::new();
        for c in children {
            map.insert(c.dir.to_string(), Arc::new(c));
        }
        CompositeVdfs::new(Arc::new(RwLock::new(map)))
    }

    /// 只含一个子插件的容器，子插件把 `provider` 注册在 `dir` 下
    fn container_of(dir: &'static str, provider: Arc<dyn VdfsProvider>) -> CompositeVdfs {
        let mut map: HashMap<String, Arc<dyn Plugin>> = HashMap::new();
        map.insert(dir.to_string(), Arc::new(ProviderChild { dir, provider }));
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
                hidden: false,
            },
            FakeChild {
                dir: "beta",
                label: "乙",
                order: 10,
                hidden: false,
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
        assert_eq!(root[0].kind, VDFS_KIND_DIR);

        let root_node = vdfs.stat(&ctx, "").await.unwrap();
        assert_eq!(root_node.path, "");
        assert!(root_node.is_dir());
    }

    /// 隐藏属性：`root_hidden` 的子目录不出现在列表里，但**照常可寻址**
    ///
    /// 与文件系统的隐藏属性同义——隐藏只影响列表，不是权限也不是卸载。
    #[tokio::test]
    async fn hidden_dirs_are_filtered_from_listing_but_still_reachable() {
        let vdfs = container(vec![
            FakeChild {
                dir: "shown",
                label: "看得见",
                order: 1,
                hidden: false,
            },
            FakeChild {
                dir: "masked",
                label: "看不见",
                order: 2,
                hidden: true,
            },
        ]);
        let ctx = host_ctx();

        // 列表里只有未隐藏的那个
        let root = vdfs.list(&ctx, "").await.unwrap();
        assert_eq!(
            root.iter().map(|n| n.name.as_str()).collect::<Vec<_>>(),
            vec!["shown"]
        );

        // 但按路径 stat 照常命中，且**如实报告**隐藏属性
        let n = vdfs.stat(&ctx, "masked").await.unwrap();
        assert_eq!(n.name, "masked");
        assert_eq!(n.title, "看不见");
        assert!(n.hidden, "stat 不该替消费者隐瞒属性");

        // 子树内容也照常可读
        let items = vdfs.list(&ctx, "masked").await.unwrap();
        assert_eq!(items[0].path, "masked/a.txt");
    }

    /// 隐藏属性是机制级的：子 provider 交回来的 `list` 里标了 `hidden` 的条目同样不出现
    #[tokio::test]
    async fn hidden_children_from_any_provider_are_filtered() {
        struct MixedProvider;

        #[async_trait]
        impl VdfsProvider for MixedProvider {
            async fn list(&self, _ctx: &VdfsContext, _path: &str) -> VdfsResult<Vec<VdfsNode>> {
                let mut masked = VdfsNode::file("secret.txt", "内部", VdfsAccess::READ);
                masked.hidden = true;
                Ok(vec![VdfsNode::file("a.txt", "A", VdfsAccess::READ), masked])
            }
        }

        struct MixedChild;

        #[async_trait]
        impl Plugin for MixedChild {
            fn meta(&self) -> PluginMeta {
                PluginMeta::new("mixed", "mixed")
            }

            async fn route(self: Arc<Self>, _ctx: Arc<dyn InvokeRequest>) -> InvokeResponse {
                Err(PluginError::NotFound("mixed".into()))
            }

            async fn traverse(
                self: Arc<Self>,
                _path: String,
                ctx: Arc<dyn InvokeRequest>,
            ) -> InvokeResponse {
                if ctx.get(PATH).as_deref() == Some(TRAVERSE_AVAILABLE_TOOLS) {
                    if let Some(visitor) = ctx.get(CAPABILITY_VISITOR) {
                        visitor
                            .register_vdfs_provider("mixed", Arc::new(MixedProvider))
                            .await;
                    }
                }
                Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
            }
        }

        let mut map: HashMap<String, Arc<dyn Plugin>> = HashMap::new();
        map.insert("mixed".to_string(), Arc::new(MixedChild));
        let vdfs = CompositeVdfs::new(Arc::new(RwLock::new(map)));
        let items = vdfs.list(&host_ctx(), "mixed").await.unwrap();
        assert_eq!(
            items.iter().map(|n| n.name.as_str()).collect::<Vec<_>>(),
            vec!["a.txt"],
            "机制级的隐藏属性与 provider 是谁无关"
        );
    }

    /// 每个子插件用**独立**收集器：provider 不会张冠李戴
    #[tokio::test]
    async fn per_child_collection_keeps_ownership() {
        let vdfs = container(vec![
            FakeChild {
                dir: "alpha",
                label: "甲",
                order: 1,
                hidden: false,
            },
            FakeChild {
                dir: "beta",
                label: "乙",
                order: 2,
                hidden: false,
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
                hidden: false,
            }),
        );
        map.insert(
            "plugin_b".to_string(),
            Arc::new(FakeChild {
                dir: "same",
                label: "乙",
                order: 1,
                hidden: false,
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
            hidden: false,
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

    /// 自身目录与子目录的守卫：不可读 / 删 / 移，mkdir 报已存在
    ///
    /// ⚠️ **写不在此列**：写子目录根 = 写在**挂载点目录自身**上，那是「新建」的
    /// 机制形态（使用方只说建在哪个目录，不说叫什么）。容器**不做类型特判**，
    /// 一律转发给子 provider 判定——这里 `LeafProvider` 没实现 `write`，
    /// 所以落到缺省的 `NotImplemented`，而不是容器自己抛 `Forbidden`。
    #[tokio::test]
    async fn guards_self_and_child_dir_roots() {
        let vdfs = container(vec![FakeChild {
            dir: "alpha",
            label: "甲",
            order: 1,
            hidden: false,
        }]);
        let ctx = host_ctx();

        assert!(matches!(
            vdfs.read(&ctx, "alpha").await.unwrap_err(),
            VdfsError::Forbidden(_)
        ));
        assert!(
            vdfs.write(&ctx, "alpha", &VdfsContent::text("", "x"))
                .await
                .unwrap_err()
                .is_not_implemented(),
            "容器不替子 provider 判「目录自身能不能写」，只转发"
        );
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

    /// 写在挂载点目录自身：`rel` 原样（空串）转发，返回的新路径补成树内全路径
    ///
    /// 这是「新建 = 写目录自身」在容器层唯一要做的事——provider 生成名字后
    /// 必须能把新地址交回使用方（见 `VdfsProvider::write` 的返回值一节）。
    #[tokio::test]
    async fn dir_root_write_is_forwarded_and_path_is_prefixed() {
        /// 支持「无名字新建」的假 provider：只记录收到的 `rel`，返回自己生成的名字
        struct RootWritableProvider;

        #[async_trait]
        impl VdfsProvider for RootWritableProvider {
            async fn write(
                &self,
                _ctx: &VdfsContext,
                path: &str,
                _content: &VdfsContent,
            ) -> VdfsResult<VdfsWriteResponse> {
                assert_eq!(path, "", "容器必须原样转发空 rel，而不是替 provider 拼名字");
                Ok(VdfsWriteResponse {
                    path: "generated-1".to_string(),
                    created: true,
                    etag: None,
                })
            }
        }

        let vdfs = container_of("rw", Arc::new(RootWritableProvider));
        let r = vdfs
            .write(
                &host_ctx(),
                "rw",
                &VdfsContent::text("", "{}").with_create(),
            )
            .await
            .unwrap();
        assert!(r.created);
        assert_eq!(r.path, "rw/generated-1", "相对路径被回填成树内全路径");
    }

    /// 读回的内容路径同样补成树内全路径
    ///
    /// 三个 form 型插件（model / mcp / skill）的 `read` 都把收到的相对路径原样回显
    /// （`VdfsContent::text(path, …)`），因此这条不是假想：漏补前缀，上层就会把它翻译
    /// 成 `<根>/<rel>`——一个并不存在的地址。
    #[tokio::test]
    async fn read_content_path_is_prefixed_with_dir() {
        struct EchoPathProvider;

        #[async_trait]
        impl VdfsProvider for EchoPathProvider {
            async fn read(&self, _ctx: &VdfsContext, path: &str) -> VdfsResult<VdfsContent> {
                Ok(VdfsContent::text(path, "{}"))
            }
        }

        let vdfs = container_of("echo", Arc::new(EchoPathProvider));
        let c = vdfs.read(&host_ctx(), "echo/item.json").await.unwrap();
        assert_eq!(
            c.path, "echo/item.json",
            "provider 回显的相对路径被补成树内全路径"
        );
    }

    /// 未知目录明确报错；跨子目录移动被拒；同子目录内移动转发相对路径
    #[tokio::test]
    async fn rejects_unknown_and_cross_dir_moves() {
        let vdfs = container(vec![
            FakeChild {
                dir: "alpha",
                label: "甲",
                order: 1,
                hidden: false,
            },
            FakeChild {
                dir: "beta",
                label: "乙",
                order: 2,
                hidden: false,
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
