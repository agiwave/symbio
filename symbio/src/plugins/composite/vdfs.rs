//! 容器的 VDFS —— 一个包含子目录（子插件）列表的 provider
//!
//! ## 职责
//!
//! 容器把子插件聚合为一棵目录树：**子插件只要实现 `Plugin::vdfs_dispatch`（或经
//! `Plugin::get_vfs_provider` 暴露自己的 provider），就以实例表里的挂载名（约定 =
//! 插件名）成为本目录下的一个子目录**。未实现该接口的插件不出现在树里，容器也
//! 不需要认识任何具体资源。
//!
//! `get_vfs_provider` / `vdfs_dispatch` 都是 **core 的 `Plugin` 查询接口**，与 LLM
//! 链路的 `CapabilityVisitor`（收集 vfs 根供工具使用）无关——系统视角下「这个插件
//! 自己暴露的 VDFS 视图」由它直接回答，容器与子插件之间因此没有类型耦合。
//!
//! 本模块自身实现 [`VdfsProvider`]（唯一接口 `dispatch`）：
//!
//! | 职责 | 说明 |
//! |---|---|
//! | 列自身目录 | `dispatch(ctx, "", List)` 返回子目录清单（合成，无需子插件参与） |
//! | 路径解析 | 首段 = 子目录名，其余 = 该子插件的**相对路径** |
//! | 全路径回填 | 子节点 / 内容 / 写入响应的 `path` 补成 `<子目录>/<rel>` |
//! | 子目录根守卫 | 子目录根不可读 / 删 / 移，也不可 mkdir（**写不在其中**） |
//! | 跨子目录拒绝 | `move` 只允许在同一子目录内 |
//! | 事件补全 | 子插件报出的相对路径补成树内全路径再交给上层 sink |
//!
//! 子目录节点的**自述**（标题 / 描述 / 访问位 / 隐藏位 / 可新建类型）取自子插件的
//! [`PluginMeta`](crate::symbio_core::PluginMeta)——元数据的唯一来源，provider 上
//! 不再有 `label` / `order` / `root_*` 一族方法。
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
//! 本视图是 `<根>` 的服务者，由两条通道暴露给访问层：
//! - **LLM 链路**：容器在 `traverse` 里经 `CapabilityVisitor::register_vdfs_root` 登记；
//! - **系统 / 前端链路**：容器经 `Plugin::get_vfs_provider` 直接返回（见
//!   `composite.rs` 的 `impl Plugin`），访问层 `resolve_fs` 直接取。
//!
//! 两条通道拿到的是同一个 `CompositeVdfs` 实例；拓扑知识因此不落在访问层。

use super::composite::broadcast_collect;
use crate::symbio_core::vdfs::{descend_addr, host_ctx};
use crate::symbio_core::vdfs_provider::*;
use crate::symbio_core::{
    ConfigurableVisitor, DefaultConfigurableVisitor, InvokeRequestExt, Plugin, PluginMeta,
    CONFIG_VISITOR, PATH, TRAVERSE_AVAILABLE_TOOLS,
};
use async_trait::async_trait;
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

    /// 现场收集：取回各子插件的 `(目录名, 插件)`，按 meta 的 `order` 升序排序
    /// （同序按目录名兜底，使结果不依赖子插件的枚举顺序）。
    ///
    /// 不缓存——子插件集合与注册内容由配置与生命周期决定，每次现取才与容器一致。
    ///
    /// 目录名是路径首段的唯一键：**重名时保留排序后首个**并 `warn`。若不处理，
    /// `list("")` 会给出两个同名节点而 `resolve` 只能命中一个——后来者**完全不可达**。
    /// 因此**先排序、再去重**：胜出者由 `(order, 目录名)` 唯一确定，重名是装配
    /// 错误，必须在日志里可见而不是静默丢弃。
    ///
    /// ## 两条收集通道，各管各的
    ///
    /// - **VDFS 子目录**：经 core 的 [`Plugin::get_vfs_provider`] /
    ///   [`Plugin::vdfs_dispatch`] 直接查询——这是系统链路的视角，目录名 = 子插件在
    ///   容器实例表里的挂载名（约定 = 插件名）。**不走 `CapabilityVisitor`**。
    /// - **配置声明**：仍是独立的 `CONFIG_VISITOR` 通道——逐子插件广播一次 `traverse`，
    ///   子插件把各自的配置文档声明写回本次请求 ctx，被委派的设置插件据此知道「哪些
    ///   插件有配置文档」。
    async fn children_of(&self, ctx: &VdfsContext) -> VdfsResult<Vec<(String, Arc<dyn Plugin>)>> {
        let host = host_ctx(ctx)?;
        let children: Vec<(String, Arc<dyn Plugin>)> = {
            let guard = self.instances.read().await;
            guard
                .iter()
                .map(|(name, p)| (name.clone(), Arc::clone(p)))
                .collect()
        };

        // 可配置声明通道：与 VDFS 无关，仍逐子插件广播一次 `traverse`，但只挂
        // `CONFIG_VISITOR`——配置声明自带目录名，不存在归属歧义。
        let configs: Arc<dyn ConfigurableVisitor> = match host.get(CONFIG_VISITOR) {
            Some(v) => v,
            None => {
                let v: Arc<dyn ConfigurableVisitor> = Arc::new(DefaultConfigurableVisitor::new());
                host.set(CONFIG_VISITOR, v.clone());
                v
            }
        };

        // VDFS 子目录：经 `Plugin::get_vfs_provider` 判定暴露与否（系统链路，
        // 无 CapabilityVisitor）。目录名 = 实例表里的挂载名（约定 = 插件名），
        // 与 `resolve` 的首段键一致。
        let mut collected: Vec<(String, Arc<dyn Plugin>)> = Vec::new();
        for (name, child) in children {
            if child.clone().get_vfs_provider().is_some() {
                collected.push((name.clone(), child.clone()));
            }
            // 配置声明：独立通道，照旧遍历（不改机制）
            let sub = host.fork();
            sub.set(PATH, TRAVERSE_AVAILABLE_TOOLS.to_string());
            sub.set(CONFIG_VISITOR, configs.clone());
            broadcast_collect(child.clone(), sub, &format!("{name} 的配置声明")).await;
        }

        collected.sort_by(|a, b| (a.1.meta().order, &a.0).cmp(&(b.1.meta().order, &b.0)));

        let mut dirs: Vec<(String, Arc<dyn Plugin>)> = Vec::with_capacity(collected.len());
        // 目录名 → 是否已出现（仅用于重名告警）
        let mut owners: HashMap<String, ()> = HashMap::new();
        for (dir, p) in collected {
            if owners.contains_key(&dir) {
                crate::plugin_warn!(
                    "composite",
                    "vdfs: 目录名「{dir}」被多个子插件重复注册，保留首个（其余不可达）"
                );
                continue;
            }
            owners.insert(dir.clone(), ());
            dirs.push((dir, p));
        }
        Ok(dirs)
    }

    /// 自身节点（`""`）——合成的目录节点，子节点为各子插件子目录
    fn self_node(dirs: &[(String, Arc<dyn Plugin>)]) -> VdfsNode {
        let mut n = VdfsNode::dir(
            "",
            "系统",
            VdfsAccess {
                list: true,
                traverse: dirs.iter().any(|(_, p)| p.meta().root_access.traverse),
                ..VdfsAccess::NONE
            },
        );
        n.description = Some("系统资源；子节点为各插件子目录".to_string());
        n
    }

    /// 子目录节点（`<dir>`）——合成的目录节点，静态自述取自子插件的 PluginMeta。
    ///
    /// `new_types`（根可新建类型）**不在其中**：它是根节点的动态清单能力，可能
    /// 依赖运行期汇流（session 的选项定义来自 options 广播），故走 provider 的
    /// async `new_types()` 现场取（见下方调用点）。
    fn dir_node(dir: &str, p: &Arc<dyn Plugin>) -> VdfsNode {
        let meta: PluginMeta = p.meta();
        let mut n = VdfsNode::dir(
            dir.to_string(),
            if meta.name.is_empty() {
                dir.to_string()
            } else {
                meta.name.clone()
            },
            meta.root_access,
        );
        n.path = dir.to_string();
        n.description = meta.description;
        // 子目录节点：它的隐藏属性来自子插件的根声明
        n.hidden = meta.hidden;
        n
    }

    /// 子目录节点 + 动态自述（`new_types`）：异步现场取
    async fn dir_node_full(dir: &str, p: &Arc<dyn Plugin>) -> VdfsNode {
        let mut n = Self::dir_node(dir, p);
        if let Some(provider) = p.clone().get_vfs_provider() {
            n.new_types = provider.new_types().await;
        }
        n
    }

    /// 子插件返回的路径 → 树内全路径。
    ///
    /// 子插件只认**自身子树内的相对路径**，所以它回显的、或它为新条目生成的名字
    /// 都只是 `<rel>`，必须补上 `<子目录>/` 才能交给上层（访问层还要再翻译成展示
    /// 地址）。空串 = 「没填」（例如物理层无从表达新名字）→ 用**请求地址**兜底。
    ///
    /// ⚠️ 不能只在空串时兜底：`write` 的返回值是使用方得知「刚建出来的东西在哪」的
    /// **唯一**途径。漏补即等于新建之后找不到新节点——前端只能停在草稿上。
    fn fill_path(requested: &str, returned: &str) -> String {
        if returned.is_empty() {
            return requested.to_string();
        }
        let dir = split_first(requested).map(|(d, _)| d).unwrap_or("");
        child_path(dir, returned)
    }

    /// 树内路径 → `(子目录名, 子插件, 相对路径)`；自身目录或无匹配时按错误返回
    ///
    /// 目录名匹配**不限定首段**：注册名允许含 `/`（嵌套装配），因此按**最长前缀**
    /// 命中：`agent/x/skill` 要先于 `agent` 被尝试，否则嵌套 provider 永远被
    /// 外层同名前缀遮蔽。注册名之间已由 `children_of` 的重名检测保证唯一，
    /// 这里只需取最长的那个。
    fn resolve<'a>(
        dirs: &'a [(String, Arc<dyn Plugin>)],
        path: &str,
    ) -> VdfsResult<(&'a str, &'a Arc<dyn Plugin>, String)> {
        // 自身目录不是可操作节点——在多段名匹配之前拦截，错误语义才不混入
        // 「目录不存在」（它存在，只是没有相对部分可派发）
        if path.is_empty() {
            return Err(VdfsError::invalid(
                "目录不是可操作节点，请给出 <子目录>/... 路径",
            ));
        }
        let mut best: Option<(&'a str, &'a Arc<dyn Plugin>, String)> = None;
        for (name, p) in dirs {
            let rel = if path == name.as_str() {
                Some(String::new())
            } else {
                path.strip_prefix(&format!("{name}/")).map(str::to_string)
            };
            if let Some(rel) = rel {
                let longer = best.as_ref().is_none_or(|(b, _, _)| name.len() > b.len());
                if longer {
                    best = Some((name.as_str(), p, rel));
                }
            }
        }
        best.ok_or_else(|| {
            let first = split_first(path).map(|(d, _)| d).unwrap_or(path);
            VdfsError::not_found(format!(
                "目录不存在：{first}（现有：{}）",
                Self::names_hint(dirs)
            ))
        })
    }

    /// 派发：解析出（目录名, 子插件, 子树相对地址），并把 ctx 的**当前父
    /// 地址**改写为该目录的挂载点——这就是 vdfs 核心协议把相对地址转发给
    /// 子插件的那一跳。
    ///
    /// 子插件收到的地址仍是子树相对地址（常态）；仅当它需要协议级绝对
    /// 地址时才取 `ctx.parent_addr()` 拼。改写规则见
    /// `symbio_core::vdfs::address`：容器自身的父地址（嵌套派发时由上级写入）
    /// 非空则原地续接，为空则落到静态声明的根。
    async fn dispatch_to(
        &self,
        ctx: &VdfsContext,
        dirs: &[(String, Arc<dyn Plugin>)],
        path: &str,
    ) -> VdfsResult<(String, VdfsContext, Arc<dyn Plugin>, String)> {
        let (dir, p, rel) = Self::resolve(dirs, path)?;
        let sub = ctx
            .clone()
            .with_parent_addr(descend_addr(ctx.parent_addr(), dir));
        Ok((dir.to_string(), sub, p.clone(), rel))
    }

    /// 子目录名清单（报错提示用）
    fn names_hint(dirs: &[(String, Arc<dyn Plugin>)]) -> String {
        if dirs.is_empty() {
            return "（无）".to_string();
        }
        dirs.iter()
            .map(|(d, _)| d.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// 全部操作都是同一件事：现场取子目录清单 → 按 `path` 首段解析 → 委派。
///
/// 路径是**树内相对路径**（`""` = 根）；首段拆分与回填在本文件内完成。
#[async_trait]
impl VdfsProvider for CompositeVdfs {
    async fn dispatch(
        &self,
        ctx: &VdfsContext,
        path: &str,
        req: VdfsRequest,
    ) -> VdfsResult<VdfsResponse> {
        let dirs = self.children_of(ctx).await?;

        // 自身目录：容器是一个**纯目录**——可列 / 可 stat，其余拒绝。
        // 隐藏属性在这里生效：`hidden` 的子目录不出现在清单里，但仍留在 `dirs`
        // 中——目录本身照旧存在，按路径照常可寻址（见 `resolve`）。
        if path.is_empty() {
            return match req {
                VdfsRequest::List { .. } => {
                    let mut out = Vec::with_capacity(dirs.len());
                    for (d, p) in &dirs {
                        out.push(Self::dir_node_full(d, p).await);
                    }
                    Ok(VdfsResponse::List(
                        out.into_iter().filter(|n| !n.hidden).collect(),
                    ))
                }
                VdfsRequest::Stat => Ok(VdfsResponse::Stat(Self::self_node(&dirs))),
                _ => Err(VdfsError::invalid(
                    "目录不是可操作节点，请给出 <子目录>/... 路径",
                )),
            };
        }

        let (dir, sub, p, rel) = self.dispatch_to(ctx, &dirs, path).await?;

        let mismatch = || VdfsError::internal("provider 返回的响应类型与请求不匹配");

        match req {
            VdfsRequest::List { .. } => {
                let mut items = p
                    .clone()
                    .vdfs_dispatch(
                        &sub,
                        &rel,
                        VdfsRequest::List {
                            limit: None,
                            before: None,
                        },
                    )
                    .await?
                    .into_list()
                    .ok_or_else(mismatch)?;
                fill_node_paths(&dir, &rel, &mut items);
                // 隐藏属性是**机制级**的：任何子树里被标为 hidden 的子节点都不出现，
                // 不因它来自哪个插件而异。
                items.retain(|n| !n.hidden);
                Ok(VdfsResponse::List(items))
            }
            VdfsRequest::Stat => {
                if rel.is_empty() {
                    return Ok(VdfsResponse::Stat(Self::dir_node_full(&dir, &p).await));
                }
                let mut n = p
                    .clone()
                    .vdfs_dispatch(&sub, &rel, VdfsRequest::Stat)
                    .await?
                    .into_stat()
                    .ok_or_else(mismatch)?;
                n.path = path.to_string();
                Ok(VdfsResponse::Stat(n))
            }
            VdfsRequest::Read => {
                if rel.is_empty() {
                    return Err(VdfsError::Forbidden(
                        "目录不是可读文件；请读取其子节点".to_string(),
                    ));
                }
                let mut c = p
                    .clone()
                    .vdfs_dispatch(&sub, &rel, VdfsRequest::Read)
                    .await?
                    .into_read()
                    .ok_or_else(mismatch)?;
                c.path = Self::fill_path(path, &c.path);
                Ok(VdfsResponse::Read(c))
            }
            VdfsRequest::Write { content } => {
                // `rel` 为空 = 写在**挂载点目录自身**上。这正是「新建」的机制形态：
                // 使用方只说「建在哪个目录」，不说「叫什么」——名字由子插件生成。
                // 是否支持由子插件判定，容器不做类型特判（与 `action` 对空 `rel`
                // 的处理一致）。
                let mut r = p
                    .clone()
                    .vdfs_dispatch(&sub, &rel, VdfsRequest::Write { content })
                    .await?
                    .into_write()
                    .ok_or_else(mismatch)?;
                // 子插件生成的名字（写在目录自身时）与它回显的相对路径都在这**一次**补齐。
                r.path = Self::fill_path(path, &r.path);
                Ok(VdfsResponse::Write(r))
            }
            VdfsRequest::Delete { recursive } => {
                if rel.is_empty() {
                    return Err(VdfsError::Forbidden("目录不可删除".to_string()));
                }
                p.clone()
                    .vdfs_dispatch(&sub, &rel, VdfsRequest::Delete { recursive })
                    .await?;
                Ok(VdfsResponse::Unit)
            }
            VdfsRequest::Mkdir => {
                if rel.is_empty() {
                    return Err(VdfsError::invalid("目录已存在，无需创建"));
                }
                p.clone()
                    .vdfs_dispatch(&sub, &rel, VdfsRequest::Mkdir)
                    .await?;
                Ok(VdfsResponse::Unit)
            }
            VdfsRequest::Move { to } => {
                // 跨子目录拒绝：`to` 也要先解析出所属子目录
                let (to_dir, _, _, rt) = self.dispatch_to(ctx, &dirs, &to).await?;
                if to_dir != dir {
                    return Err(VdfsError::invalid(format!(
                        "不支持跨目录移动：{dir} → {to_dir}"
                    )));
                }
                if rel.is_empty() || rt.is_empty() {
                    return Err(VdfsError::Forbidden("目录不可移动".to_string()));
                }
                p.clone()
                    .vdfs_dispatch(&sub, &rel, VdfsRequest::Move { to: rt })
                    .await?;
                Ok(VdfsResponse::Unit)
            }
            VdfsRequest::Action { action, payload } => {
                p.clone()
                    .vdfs_dispatch(&sub, &rel, VdfsRequest::Action { action, payload })
                    .await
            }
            VdfsRequest::Watch { sink } => {
                // 子插件报出的相对路径在此补成树内全路径，再交给上层 sink。
                // 用 [`VdfsChange::map_paths`] 一次覆盖全部路径（含 `node` 载荷内的
                // 路径），不逐字段重建——新增字段时不会漏转发。
                let wrapped: VdfsChangeSink =
                    Arc::new(move |c: VdfsChange| sink(c.map_paths(|p| child_path(&dir, p))));
                p.clone()
                    .vdfs_dispatch(&sub, &rel, VdfsRequest::Watch { sink: wrapped })
                    .await?;
                Ok(VdfsResponse::Unit)
            }
            VdfsRequest::Unwatch => {
                p.clone()
                    .vdfs_dispatch(&sub, &rel, VdfsRequest::Unwatch)
                    .await?;
                Ok(VdfsResponse::Unit)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbio_core::vdfs::vdfs_context;
    use crate::symbio_core::{
        InvokeRequest, PluginError, PluginPayload, SimpleRequest, CAPABILITY_VISITOR,
    };

    /// 只暴露一个 `a.txt` 的 provider；目录名 / 顺序 / 隐藏由**假插件的 meta** 决定
    struct LeafProvider;

    #[async_trait]
    impl VdfsProvider for LeafProvider {
        async fn dispatch(
            &self,
            _ctx: &VdfsContext,
            _path: &str,
            req: VdfsRequest,
        ) -> VdfsResult<VdfsResponse> {
            match req {
                VdfsRequest::List { .. } => Ok(VdfsResponse::List(vec![VdfsNode::file(
                    "a.txt",
                    "A",
                    VdfsAccess::READ,
                )])),
                VdfsRequest::Read => Ok(VdfsResponse::Read(VdfsContent::text(
                    _path,
                    format!("leaf:{_path}"),
                ))),
                _ => Err(VdfsError::NotImplemented),
            }
        }
    }

    /// 假子插件：自述来自 meta；`traverse` 时按目录名注册 provider（约定 = 插件名）
    struct FakeChild {
        dir: &'static str,
        label: &'static str,
        order: i32,
        hidden: bool,
    }

    #[async_trait]
    impl Plugin for FakeChild {
        fn meta(&self) -> PluginMeta {
            PluginMeta::new(self.dir, self.label)
                .with_order(self.order)
                .with_hidden(self.hidden)
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
                        .register_vdfs_provider(self.dir, Arc::new(LeafProvider))
                        .await;
                }
            }
            Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
        }

        /// 系统链路：直接暴露自己的 provider（目录名由容器实例表的挂载名给出）
        fn get_vfs_provider(self: Arc<Self>) -> Option<Arc<dyn VdfsProvider>> {
            Some(Arc::new(LeafProvider))
        }
    }

    type InvokeResponse = crate::symbio_core::InvokeResponse<PluginPayload>;

    /// 假子插件：把**给定的** provider 暴露在自己目录名下
    ///
    /// 与 [`FakeChild`] 的区别只有一个：那个固定暴露 [`LeafProvider`]，这个让测试
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

        /// 系统链路：直接暴露给定的 provider（目录名由容器实例表的挂载名给出）
        fn get_vfs_provider(self: Arc<Self>) -> Option<Arc<dyn VdfsProvider>> {
            Some(self.provider.clone())
        }
    }

    fn container(children: Vec<FakeChild>) -> CompositeVdfs {
        let mut map: HashMap<String, Arc<dyn Plugin>> = HashMap::new();
        for c in children {
            map.insert(c.dir.to_string(), Arc::new(c));
        }
        CompositeVdfs::new(Arc::new(RwLock::new(map)))
    }

    /// 只含一个子插件的容器，子插件把 `provider` 暴露在 `dir` 下
    fn container_of(dir: &'static str, provider: Arc<dyn VdfsProvider>) -> CompositeVdfs {
        let mut map: HashMap<String, Arc<dyn Plugin>> = HashMap::new();
        map.insert(dir.to_string(), Arc::new(ProviderChild { dir, provider }));
        CompositeVdfs::new(Arc::new(RwLock::new(map)))
    }

    fn host_ctx() -> VdfsContext {
        let host: Arc<dyn InvokeRequest> = Arc::new(SimpleRequest::new(None, None));
        vdfs_context(&host)
    }

    /// 子插件的子目录以**自己 meta 的 name** 作为标题出现在本目录下，并按 order 升序
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

        let root = vdfs
            .dispatch(
                &ctx,
                "",
                VdfsRequest::List {
                    limit: None,
                    before: None,
                },
            )
            .await
            .unwrap();
        let VdfsResponse::List(root) = root else {
            panic!("应为 List 响应");
        };
        assert_eq!(root.len(), 2);
        assert_eq!(
            root.iter().map(|n| n.name.as_str()).collect::<Vec<_>>(),
            vec!["beta", "alpha"],
            "目录名来自子插件自己的注册；顺序按 meta.order"
        );
        assert_eq!(root[0].path, "beta");
        assert_eq!(root[0].title, "乙");
        assert_eq!(root[0].kind, VDFS_KIND_DIR);

        let VdfsResponse::Stat(root_node) =
            vdfs.dispatch(&ctx, "", VdfsRequest::Stat).await.unwrap()
        else {
            panic!("应为 Stat 响应");
        };
        assert_eq!(root_node.path, "");
        assert!(root_node.is_dir());
    }

    /// 隐藏属性：`meta.hidden` 的子目录不出现在列表里，但**照常可寻址**
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
        let VdfsResponse::List(root) = vdfs
            .dispatch(
                &ctx,
                "",
                VdfsRequest::List {
                    limit: None,
                    before: None,
                },
            )
            .await
            .unwrap()
        else {
            panic!("应为 List 响应");
        };
        assert_eq!(
            root.iter().map(|n| n.name.as_str()).collect::<Vec<_>>(),
            vec!["shown"]
        );

        // 但按路径 stat 照常命中，且**如实报告**隐藏属性
        let VdfsResponse::Stat(n) = vdfs
            .dispatch(&ctx, "masked", VdfsRequest::Stat)
            .await
            .unwrap()
        else {
            panic!("应为 Stat 响应");
        };
        assert_eq!(n.name, "masked");
        assert_eq!(n.title, "看不见");
        assert!(n.hidden, "stat 不该替消费者隐瞒属性");

        // 子树内容也照常可读
        let VdfsResponse::List(items) = vdfs
            .dispatch(
                &ctx,
                "masked",
                VdfsRequest::List {
                    limit: None,
                    before: None,
                },
            )
            .await
            .unwrap()
        else {
            panic!("应为 List 响应");
        };
        assert_eq!(items[0].path, "masked/a.txt");
    }

    /// 隐藏属性是机制级的：子插件交回来的 `list` 里标了 `hidden` 的条目同样不出现
    #[tokio::test]
    async fn hidden_children_from_any_provider_are_filtered() {
        struct MixedProvider;

        #[async_trait]
        impl VdfsProvider for MixedProvider {
            async fn dispatch(
                &self,
                _ctx: &VdfsContext,
                _path: &str,
                req: VdfsRequest,
            ) -> VdfsResult<VdfsResponse> {
                match req {
                    VdfsRequest::List { .. } => {
                        let mut masked = VdfsNode::file("secret.txt", "内部", VdfsAccess::READ);
                        masked.hidden = true;
                        Ok(VdfsResponse::List(vec![
                            VdfsNode::file("a.txt", "A", VdfsAccess::READ),
                            masked,
                        ]))
                    }
                    _ => Err(VdfsError::NotImplemented),
                }
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

            // 系统链路：直接暴露 provider（与 LLM 链路经 `register_vdfs_provider` 注册互不冲突）
            fn get_vfs_provider(
                self: Arc<Self>,
            ) -> Option<Arc<dyn crate::symbio_core::vdfs_provider::VdfsProvider>> {
                Some(Arc::new(MixedProvider))
            }
        }

        let mut map: HashMap<String, Arc<dyn Plugin>> = HashMap::new();
        map.insert("mixed".to_string(), Arc::new(MixedChild));
        let vdfs = CompositeVdfs::new(Arc::new(RwLock::new(map)));
        let VdfsResponse::List(items) = vdfs
            .dispatch(
                &host_ctx(),
                "mixed",
                VdfsRequest::List {
                    limit: None,
                    before: None,
                },
            )
            .await
            .unwrap()
        else {
            panic!("应为 List 响应");
        };
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
            assert_eq!(p.meta().name, label, "插件与目录名一一对应");
        }
    }

    /// 子插件各以**挂载名**作为目录名出现：两个不同挂载名的插件各自一个目录，
    /// 不合并（目录名唯一性由容器实例表的键保证）。列表顺序按 meta.order 升序。
    #[tokio::test]
    async fn distinct_mount_names_each_get_a_dir() {
        let mut map: HashMap<String, Arc<dyn Plugin>> = HashMap::new();
        map.insert(
            "plugin_a".to_string(),
            Arc::new(FakeChild {
                dir: "a",
                label: "甲",
                order: 2,
                hidden: false,
            }),
        );
        map.insert(
            "plugin_b".to_string(),
            Arc::new(FakeChild {
                dir: "b",
                label: "乙",
                order: 1,
                hidden: false,
            }),
        );
        let vdfs = CompositeVdfs::new(Arc::new(RwLock::new(map)));
        let ctx = host_ctx();

        let dirs = vdfs.children_of(&ctx).await.unwrap();
        assert_eq!(dirs.len(), 2, "两个不同挂载名 → 两个目录");
        // 顺序按 meta.order：plugin_b(1) 先于 plugin_a(2)
        assert_eq!(dirs[0].0, "plugin_b");
        assert_eq!(dirs[0].1.meta().name, "乙");
        assert_eq!(dirs[1].0, "plugin_a");
        assert_eq!(dirs[1].1.meta().name, "甲");

        let VdfsResponse::List(root) = vdfs
            .dispatch(
                &ctx,
                "",
                VdfsRequest::List {
                    limit: None,
                    before: None,
                },
            )
            .await
            .unwrap()
        else {
            panic!("应为 List 响应");
        };
        assert_eq!(root.len(), 2);
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

        let VdfsResponse::List(items) = vdfs
            .dispatch(
                &ctx,
                "alpha",
                VdfsRequest::List {
                    limit: None,
                    before: None,
                },
            )
            .await
            .unwrap()
        else {
            panic!("应为 List 响应");
        };
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].path, "alpha/a.txt", "相对路径被回填成树内全路径");

        let VdfsResponse::Read(c) = vdfs
            .dispatch(&ctx, "alpha/a.txt", VdfsRequest::Read)
            .await
            .unwrap()
        else {
            panic!("应为 Read 响应");
        };
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
        let VdfsResponse::List(items) = vdfs
            .dispatch(
                &ctx,
                "",
                VdfsRequest::List {
                    limit: None,
                    before: None,
                },
            )
            .await
            .unwrap()
        else {
            panic!("应为 List 响应");
        };
        assert!(items.is_empty());
        let VdfsResponse::Stat(n) = vdfs.dispatch(&ctx, "", VdfsRequest::Stat).await.unwrap()
        else {
            panic!("应为 Stat 响应");
        };
        assert_eq!(n.path, "");
    }

    /// 自身目录与子目录的守卫：不可读 / 删 / 移，mkdir 报已存在
    ///
    /// ⚠️ **写不在此列**：写子目录根 = 写在**挂载点目录自身**上，那是「新建」的
    /// 机制形态（使用方只说建在哪个目录，不说叫什么）。容器**不做类型特判**，
    /// 一律转发给子插件判定——这里 `LeafProvider` 没实现 `write`，
    /// 所以落到 `NotImplemented`，而不是容器自己抛 `Forbidden`。
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
            vdfs.dispatch(&ctx, "alpha", VdfsRequest::Read)
                .await
                .unwrap_err(),
            VdfsError::Forbidden(_)
        ));
        assert!(
            vdfs.dispatch(
                &ctx,
                "alpha",
                VdfsRequest::Write {
                    content: VdfsContent::text("", "x")
                }
            )
            .await
            .unwrap_err()
            .is_not_implemented(),
            "容器不替子插件判「目录自身能不能写」，只转发"
        );
        assert!(matches!(
            vdfs.dispatch(&ctx, "alpha", VdfsRequest::Delete { recursive: true })
                .await
                .unwrap_err(),
            VdfsError::Forbidden(_)
        ));
        assert!(matches!(
            vdfs.dispatch(&ctx, "alpha", VdfsRequest::Mkdir)
                .await
                .unwrap_err(),
            VdfsError::Invalid(_)
        ));
        // 自身目录也不可操作
        assert!(matches!(
            vdfs.dispatch(&ctx, "", VdfsRequest::Read)
                .await
                .unwrap_err(),
            VdfsError::Invalid(_)
        ));
    }

    /// 写在挂载点目录自身：`rel` 原样（空串）转发，返回的新路径补成树内全路径
    ///
    /// 这是「新建 = 写目录自身」在容器层唯一要做的事——子插件生成名字后
    /// 必须能把新地址交回使用方。
    #[tokio::test]
    async fn dir_root_write_is_forwarded_and_path_is_prefixed() {
        /// 支持「无名字新建」的假 provider：只记录收到的 `rel`，返回自己生成的名字
        struct RootWritableProvider;

        #[async_trait]
        impl VdfsProvider for RootWritableProvider {
            async fn dispatch(
                &self,
                _ctx: &VdfsContext,
                path: &str,
                req: VdfsRequest,
            ) -> VdfsResult<VdfsResponse> {
                let VdfsRequest::Write { .. } = req else {
                    return Err(VdfsError::NotImplemented);
                };
                assert_eq!(path, "", "容器必须原样转发空 rel，而不是替子插件拼名字");
                Ok(VdfsResponse::Write(VdfsWriteResponse {
                    path: "generated-1".to_string(),
                    created: true,
                    etag: None,
                }))
            }
        }

        let vdfs = container_of("rw", Arc::new(RootWritableProvider));
        let VdfsResponse::Write(r) = vdfs
            .dispatch(
                &host_ctx(),
                "rw",
                VdfsRequest::Write {
                    content: VdfsContent::text("", "{}").with_create(),
                },
            )
            .await
            .unwrap()
        else {
            panic!("应为 Write 响应");
        };
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
            async fn dispatch(
                &self,
                _ctx: &VdfsContext,
                path: &str,
                req: VdfsRequest,
            ) -> VdfsResult<VdfsResponse> {
                let VdfsRequest::Read = req else {
                    return Err(VdfsError::NotImplemented);
                };
                Ok(VdfsResponse::Read(VdfsContent::text(path, "{}")))
            }
        }

        let vdfs = container_of("echo", Arc::new(EchoPathProvider));
        let VdfsResponse::Read(c) = vdfs
            .dispatch(&host_ctx(), "echo/item.json", VdfsRequest::Read)
            .await
            .unwrap()
        else {
            panic!("应为 Read 响应");
        };
        assert_eq!(
            c.path, "echo/item.json",
            "provider 回显的相对路径被补成树内全路径"
        );
    }

    /// 未知目录明确报错；跨子目录移动被拒
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

        let err = vdfs
            .dispatch(
                &ctx,
                "nope",
                VdfsRequest::List {
                    limit: None,
                    before: None,
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, VdfsError::NotFound(_)));
        assert!(err.to_string().contains("alpha"), "提示现有目录");

        assert!(matches!(
            vdfs.dispatch(
                &ctx,
                "alpha/a",
                VdfsRequest::Move {
                    to: "beta/a".into()
                }
            )
            .await
            .unwrap_err(),
            VdfsError::Invalid(_)
        ));
    }

    /// 宿主句柄缺失（provider 未随 symbio 上下文调用）→ 明确报错而非静默空树
    #[tokio::test]
    async fn missing_host_ctx_is_an_internal_error() {
        let vdfs = container(Vec::new());
        // Ok 侧不可 `Debug`，故不能用 unwrap_err
        let err = match vdfs
            .dispatch(
                &VdfsContext::empty(),
                "",
                VdfsRequest::List {
                    limit: None,
                    before: None,
                },
            )
            .await
        {
            Ok(_) => panic!("缺宿主句柄时应报错"),
            Err(e) => e,
        };
        assert_eq!(err.code(), "INTERNAL_ERROR");
    }

    // ==================== 嵌套装配：多段注册名 ====================
    //
    // 子智能体子树里的 provider 经 agent 插件的作用域代理（SubAgentVisitor）
    // 以 **多段名** 注册进系统容器：`agent/<agent_id>/<name>`。这里的测试
    // 用合成多段名模拟该形态，钉住三件事：可寻址（resolve 最长前缀）、
    // 派发期父地址 = 完整挂载点、根清单的呈现形态。

    /// 回声 provider：`read` 返回 `<标签>:<父地址>/<path>`，同时验证两件事——
    /// 收到的地址是子树相对路径、上下文父地址已被派发改写为完整挂载点。
    struct EchoProvider {
        label: &'static str,
    }

    #[async_trait]
    impl VdfsProvider for EchoProvider {
        async fn dispatch(
            &self,
            ctx: &VdfsContext,
            path: &str,
            req: VdfsRequest,
        ) -> VdfsResult<VdfsResponse> {
            let VdfsRequest::Read = req else {
                return Err(VdfsError::NotImplemented);
            };
            let parent = ctx.parent_addr();
            Ok(VdfsResponse::Read(VdfsContent::text(
                "",
                format!("{}:{parent}/{path}", self.label),
            )))
        }
    }

    /// 子插件的 provider 经 `Plugin::get_vfs_provider` 直接取回（系统链路），
    /// 目录名 = 实例表的挂载名；与 LLM 链路经 `CapabilityVisitor` 收集互不干扰。
    #[tokio::test]
    async fn sub_provider_fetched_via_trait_method() {
        let mut map: HashMap<String, Arc<dyn Plugin>> = HashMap::new();
        map.insert(
            "agent".to_string(),
            Arc::new(ProviderChild {
                dir: "agent",
                provider: Arc::new(EchoProvider { label: "outer" }),
            }),
        );
        let vdfs = CompositeVdfs::new(Arc::new(RwLock::new(map)));
        let ctx = host_ctx();

        // `get_vfs_provider` 直接给出 provider；`children_of` 用挂载名 "agent" 作目录名
        let dirs = vdfs.children_of(&ctx).await.unwrap();
        assert_eq!(dirs.len(), 1);
        assert_eq!(dirs[0].0, "agent");
        assert_eq!(dirs[0].1.meta().name, "agent");

        // 经组合视图读子 provider：父地址续接、相对路径透传
        let VdfsResponse::Read(c) = vdfs
            .dispatch(&ctx, "agent/x/f.md", VdfsRequest::Read)
            .await
            .unwrap()
        else {
            panic!("应为 Read 响应");
        };
        let text = c.text.as_deref().unwrap();
        assert!(text.starts_with("outer:"), "挂载名命中: {text}");
        assert!(
            text.ends_with("/agent/x/f.md"),
            "父地址 + 相对路径 = 完整挂载点地址（根名无关）: {text}"
        );
    }
}
