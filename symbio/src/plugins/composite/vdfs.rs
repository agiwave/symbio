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
//! | 地址翻译 | 把子插件的**相对地址**放进 `ctx` 的当前父地址，供其拼绝对地址 |
//! | 子目录根守卫 | 子目录根不可读 / 移，也不可 mkdir；**删 = 卸载**（转注册表） |
//! | 跨子目录拒绝 | `move` 只允许在同一子目录内 |
//! | 事件补全 | 子插件报出的相对路径补成树内全路径再交给上层 sink |
//!
//! **本层不填条目地址**：地址是「某一份列表」的定位（[`VdfsItem::path`]），
//! 按 `<父地址>/<name>` 推导即可，由访问层（`plugins/vdfs/host.rs::fill_paths`）
//! 用**请求地址**统一回填。本层若自己填，就必须知道自己的挂载前缀——而它不知道
//! （composite 可被另一个 composite 包含），填出来的会缺前缀。
//!
//! 子目录节点的**自述**（标题 / 描述 / 访问位 / 隐藏位 / 可新建类型）取自子插件的
//! [`PluginMeta`](crate::symbio_core::PluginMeta)——元数据的唯一来源，provider 上
//! 不再有 `label` / `order` / `root_*` 一族方法。
//!
//! ## 资源树与插件注册表：两个视图，刻意不共用 `List`
//!
//! 容器的根同时是两样东西，而它们**答的不是同一个问题**：
//!
//! - **资源树**（`List`）：`<根>` 下有哪些**可用**资源域。只有「已挂载**且**暴露
//!   VDFS」的子插件在里面——`telegram` 没有 VDFS 视图，它不该在资源树里占一格；
//! - **插件注册表**（`Action(plugins)`）：这个智能体由**哪些插件**组成。它要的是
//!   全集：含没有 VDFS 视图的、含**已停用**的（否则用户看不到自己刚停用的那个，
//!   也就永远点不回「启用」）。
//!
//! 两件事的成员集合不同，因此不能共用一个动词——把注册表塞进 `List`，资源树就会
//! 多出一些点进去什么都没有的格子；把 `List` 收窄成注册表，没有 VDFS 的插件就会
//! 从资源树里消失。动作名与回包形状见 `symbio_core::vdfs` 的
//! `VDFS_ACTION_PLUGINS` / `VDFS_PLUGINS_FIELD`。
//!
//! ## 它不是根，也没有任何「根」的概念
//!
//! composite 只是一个**恰好包含若干子目录的 provider**——它可以被别的目录包含，
//! 子插件本身也可以是另一个 composite（嵌套时它同样只是普通 provider）。当前它
//! 充当整个 `<根>` 的服务者，**只是装配时的安排**（使用方把它登记进了
//! `register_vdfs_root` 槽位），不是本模块的属性；本文件不出现任何「根级别」
//! 的概念与代码（`path == ""` 只是「本目录自身」，任何目录都可能是别的目录的子目录）。
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
use super::registry::PluginRegistry;
use crate::symbio_core::schemas::detail::{DetailDefinition, DetailField, DetailOption};
use crate::symbio_core::{descend_addr, host_ctx};
use crate::symbio_core::{
    ConfigurableVisitor, DefaultConfigurableVisitor, Plugin, PluginInvokeRequestExt, PluginMeta,
    CONFIG_VISITOR, PATH, PLUGIN_MANAGER, TRAVERSE_AVAILABLE_TOOLS,
};
use crate::symbio_core::{
    VdfsAccess, VdfsActionResult, VdfsChange, VdfsChangeSink, VdfsContent, VdfsContext, VdfsError,
    VdfsNewType, VdfsNode, VdfsProvider, VdfsRequest, VdfsResponse, VdfsResult, VdfsWriteResponse,
    PLUGIN_PROVIDER_FIELD, VDFS_ACTION_DISABLE, VDFS_ACTION_ENABLE, VDFS_ACTION_PLUGINS,
    VDFS_EXT_FORM, VDFS_PLUGINS_FIELD, VDFS_PLUGIN_NAME_FIELD,
};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

/// 容器的 VDFS（包含子目录列表的 provider，见模块文档）
pub struct CompositeVdfs {
    /// 插件集合：目录清单（资源树与注册表）与实例表都在它那里，
    /// 运行期启停/安装/卸载也经它执行——本视图因此没有自己的状态。
    registry: Arc<PluginRegistry>,
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

impl CompositeVdfs {
    pub fn new(registry: Arc<PluginRegistry>) -> Self {
        Self { registry }
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
    ///   子插件把各自的配置文档声明写回本次请求 ctx，插件管理插件据此知道「哪些
    ///   插件有配置文档」。
    async fn children_of(&self, ctx: &VdfsContext) -> VdfsResult<Vec<(String, Arc<dyn Plugin>)>> {
        let host = host_ctx(ctx)?;
        // 实例表快照（`std::sync::RwLock`：临界区只有一次遍历，没有任何 await）
        let children: Vec<(String, Arc<dyn Plugin>)> = self.registry.snapshot();

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
    ///
    /// 它同时声明**根可新建什么**：一个插件（见 [`Self::install_new_type`]）。
    /// 「根接受的东西 = 一个插件」不是新增的界面概念，而是 `new_type` 机制的本来
    /// 用途——根就是插件根，在它下面新建一个东西，就是把一个插件装进来。
    fn self_node(&self, dirs: &[(String, Arc<dyn Plugin>)]) -> VdfsNode {
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
        n.new_type = Some(Box::new(self.install_new_type()));
        n
    }

    /// 根可新建的类型：**一个插件**（安装表单）
    ///
    /// 候选 = 已注册但当前未挂载的工厂（见 [`PluginRegistry::installable`]），落成
    /// 动作就是根上的 `Write`（见 [`Self::install_plugin`]）——表单声明与执行者同在
    /// 一处，因此**只有这一份**安装表单的定义；插件管理插件原样转发它（它的根是
    /// 插件根在界面上的门面），不另写一张。
    fn install_new_type(&self) -> VdfsNewType {
        let options: Vec<DetailOption> = self
            .registry
            .installable()
            .into_iter()
            .map(|id| DetailOption {
                value: id.to_string(),
                label: id.to_string(),
                description: None,
            })
            .collect();
        let mut t = VdfsNewType::new(PLUGIN_MANAGER, "插件");
        t.description = Some("从已注册的插件工厂里选一个装进本智能体".to_string());
        // 落成后是一个**定义驱动的表单**节点（与新建态同一张详情，见 `VdfsNewType`）。
        t.node_ext = Some(VDFS_EXT_FORM.to_string());
        t.schema = serde_json::to_value(DetailDefinition::form(
            "添加插件",
            vec![DetailField::select(
                PLUGIN_PROVIDER_FIELD,
                "插件工厂",
                options,
                "",
            )],
        ))
        .ok();
        t
    }

    /// 子目录节点（`<dir>`）——合成的目录节点。
    ///
    /// 两类字段**两个来源**（ADR-032）：
    ///
    /// - **身份**（`title` / `description`）来自该插件目录的 `PLUGIN.yml`
    ///   （[`PluginRegistry::dir_of`] → `PluginDir::identity`）——与插件列表同一份，
    ///   且停用也读得到（它只是不被构造）；
    /// - **挂载点呈现**（`root_access` / `hidden`）来自子插件的 `PluginMeta`：
    ///   它们描述「这个挂载点长什么样」，只有挂载了才成立。
    ///
    /// `new_type`（根可新建类型）**不在其中**：它要运行期取（见
    /// [`Self::dir_node_full`]）。
    fn dir_node(registry: &PluginRegistry, dir: &str, p: &Arc<dyn Plugin>) -> VdfsNode {
        let meta: PluginMeta = p.meta();
        let identity = registry.dir_of(dir).identity();
        let mut n = VdfsNode::dir(
            dir.to_string(),
            if identity.title.is_empty() {
                dir.to_string()
            } else {
                identity.title
            },
            meta.root_access,
        );
        n.description = identity.description;
        // 子目录节点：它的隐藏属性来自子插件的根声明
        n.hidden = meta.hidden;
        n
    }

    /// 子目录节点 + **动态自述**（`new_type`）：问 provider「你的根长什么样」。
    ///
    /// 取法是向该 provider 发一次 `Stat`（空路径 = 它自己的根）——这与「更深层的
    /// 节点在自己的 `list` 结果里带 [`VdfsNode::new_type`]」是**同一条通道**，
    /// 使用方不必先知道某个节点是不是根，才能问它「你能新建什么」。
    ///
    /// 只取 `new_type`：其余字段仍以 [`dir_node`](Self::dir_node) 为准。provider
    /// 没答上来时按「根下不可新建」处理——与 `new_type: None` 的语义一致。
    async fn dir_node_full(
        registry: &PluginRegistry,
        sub: &VdfsContext,
        dir: &str,
        p: &Arc<dyn Plugin>,
    ) -> VdfsNode {
        let mut n = Self::dir_node(registry, dir, p);
        if let Ok(resp) = p.clone().vdfs_dispatch(sub, "", VdfsRequest::Stat).await {
            if let Some(root_node) = resp.into_stat() {
                n.new_type = root_node.new_type;
            }
        }
        n
    }

    /// 子目录自己的 ctx（父地址续接成该挂载点）——`dispatch_to` 与根列表共用一处
    fn sub_ctx(ctx: &VdfsContext, dir: &str) -> VdfsContext {
        ctx.clone()
            .with_parent_addr(descend_addr(ctx.parent_addr(), dir))
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
        let sub = Self::sub_ctx(ctx, dir);
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

    // ==================== 插件注册表（运行期） ====================
    //
    // 注册表的四个动词全部落在**本目录自己**（`path == ""`）上：列（`Action(plugins)`）、
    // 启停（`Action(enable|disable)`）、安装（`Write`）。卸载落在**插件目录自身**
    // （`Delete` 且相对路径为空）——「删掉这个目录」与「卸载这个插件」是同一件事。
    //
    // 为什么都挤在根上：容器的根就是**装配面**（插件根），根之下每一个子目录才是
    // 一个插件。把动词挂到子目录上会与「该插件自己的动作」抢名字（`enable` 是装配
    // 方的词，不是插件自己的动作）。
    //
    // **不发变更广播**：注册表是**装配态**，改它的人就是当前这个请求的发起者，
    // 而调用方（前端 `runAction` / `write` / `removeSelected`）在动作返回后一律
    // 重拉当前目录——它本来就会看到新状态。跨窗口同步装配态没有需求，为它接一条
    // 容器级的订阅通道，是给机制加一份没人要的能力。

    /// `Action(plugins)` —— 列出插件注册表（回包 `data[VDFS_PLUGINS_FIELD]`）
    fn list_registry(&self, action: String) -> VdfsResult<VdfsResponse> {
        let entries = self.registry.entries();
        let total = entries.len();
        let enabled = entries.iter().filter(|e| e.enabled).count();
        Ok(VdfsResponse::Action(VdfsActionResult {
            action,
            ok: true,
            message: format!("共 {total} 个插件，其中 {enabled} 个已启用"),
            data: Some(serde_json::json!({ VDFS_PLUGINS_FIELD: entries })),
        }))
    }

    /// `Action(enable|disable)` —— 启停一个插件（`payload.name` = 插件名）
    ///
    /// 名称取自**载荷**而不是路径：这个动作是「对注册表里某一项做点什么」，
    /// 而注册表是根上的视图——路径上没有这一项（停用的插件不在资源树里，
    /// 见模块文档「两个视图」）。
    async fn set_plugin_enabled(
        &self,
        action: String,
        payload: Option<&serde_json::Value>,
        enabled: bool,
    ) -> VdfsResult<VdfsResponse> {
        let name = payload
            .and_then(|p| p.get(VDFS_PLUGIN_NAME_FIELD))
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        if name.is_empty() {
            return Err(VdfsError::invalid(format!(
                "「{action}」需要在载荷里给出插件名（{{\"{VDFS_PLUGIN_NAME_FIELD}\": \"…\"}}）"
            )));
        }
        self.registry
            .set_enabled(&name, enabled)
            .await
            .map_err(VdfsError::invalid)?;
        Ok(VdfsResponse::Action(VdfsActionResult {
            action,
            ok: true,
            message: format!("已{}「{name}」", if enabled { "启用" } else { "停用" }),
            data: None,
        }))
    }

    /// 根上的 `Write` —— **安装**一个插件（内容 = 安装表单的字段值）
    ///
    /// 与「新建一项资源」是同一条机制形态（写目录自身、名字由 provider 生成）：
    /// 使用方只说「往这个目录里加一个插件」，加的是谁由表单字段（`provider`）决定，
    /// 落成后的名字经 [`VdfsWriteResponse::name`] 交回。
    fn install_plugin(&self, content: &VdfsContent) -> VdfsResult<VdfsResponse> {
        // 与「新建一项资源」同一条判据：写挂载点目录自身没有任何「已存在的目标」
        // 可覆盖，`create` 是唯一说得通的意思（见 `VdfsContent::create`）。
        if !content.create {
            return Err(VdfsError::invalid(
                "写插件根需要 create 意图：目录自身没有可覆盖的目标",
            ));
        }
        let text = content
            .as_text()
            .ok_or_else(|| VdfsError::invalid("安装插件需要文本（JSON）内容"))?;
        let value: serde_json::Value = serde_json::from_str(text)
            .map_err(|e| VdfsError::invalid(format!("安装表单不是合法 JSON：{e}")))?;
        let provider = value
            .get(PLUGIN_PROVIDER_FIELD)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        if provider.is_empty() {
            return Err(VdfsError::invalid("请选择一个插件工厂"));
        }
        let name = self
            .registry
            .install(&provider)
            .map_err(VdfsError::invalid)?;
        Ok(VdfsResponse::Write(VdfsWriteResponse {
            name: Some(name),
            created: true,
            etag: None,
        }))
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

        // 自身目录：可列 / 可 stat / 可写（= 安装一个插件）/ 收注册表动词；
        // 其余拒绝。隐藏属性在这里生效：`hidden` 的子目录不出现在清单里，但仍留在
        // `dirs` 中——目录本身照旧存在，按路径照常可寻址（见 `resolve`）。
        if path.is_empty() {
            return match req {
                VdfsRequest::List { .. } => {
                    let mut out = Vec::with_capacity(dirs.len());
                    for (d, p) in &dirs {
                        out.push(
                            Self::dir_node_full(
                                self.registry.as_ref(),
                                &Self::sub_ctx(ctx, d),
                                d,
                                p,
                            )
                            .await,
                        );
                    }
                    Ok(VdfsResponse::list(out.into_iter().filter(|n| !n.hidden)))
                }
                VdfsRequest::Stat => Ok(VdfsResponse::Stat(self.self_node(&dirs))),
                // 注册表动词（见上文「插件注册表（运行期）」）：列 / 启停 / 安装。
                // 三者的**判据都是载荷里的插件名**，不是路径——注册表是根上的视图，
                // 停用的插件在资源树里没有位置（模块文档「两个视图」）。
                VdfsRequest::Action { action, .. } if action == VDFS_ACTION_PLUGINS => {
                    self.list_registry(action)
                }
                VdfsRequest::Action { action, payload } if action == VDFS_ACTION_ENABLE => {
                    self.set_plugin_enabled(action, payload.as_ref(), true)
                        .await
                }
                VdfsRequest::Action { action, payload } if action == VDFS_ACTION_DISABLE => {
                    self.set_plugin_enabled(action, payload.as_ref(), false)
                        .await
                }
                // 根上的写 = **安装**：与「新建一项资源」同形（往目录里加一个东西，
                // 名字由 provider 生成并经 `VdfsWriteResponse::name` 交回）。
                VdfsRequest::Write { content } => self.install_plugin(&content),
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
                // 地址与 `ext` / `title` 不在这里补——访问层按**请求地址**统一回填
                // （见模块文档「本层不填条目地址」）。
                // 隐藏属性是**机制级**的：任何子树里被标为 hidden 的子节点都不出现，
                // 不因它来自哪个插件而异。
                items.retain(|it| !it.node.hidden);
                Ok(VdfsResponse::List(items))
            }
            VdfsRequest::Stat => {
                if rel.is_empty() {
                    return Ok(VdfsResponse::Stat(
                        Self::dir_node_full(self.registry.as_ref(), &sub, &dir, &p).await,
                    ));
                }
                let n = p
                    .clone()
                    .vdfs_dispatch(&sub, &rel, VdfsRequest::Stat)
                    .await?
                    .into_stat()
                    .ok_or_else(mismatch)?;
                Ok(VdfsResponse::Stat(n))
            }
            VdfsRequest::Read => {
                if rel.is_empty() {
                    return Err(VdfsError::Forbidden(
                        "目录不是可读文件；请读取其子节点".to_string(),
                    ));
                }
                let c = p
                    .clone()
                    .vdfs_dispatch(&sub, &rel, VdfsRequest::Read)
                    .await?
                    .into_read()
                    .ok_or_else(mismatch)?;
                Ok(VdfsResponse::Read(c))
            }
            VdfsRequest::Write { content } => {
                // `rel` 为空 = 写在**挂载点目录自身**上。这正是「新建」的机制形态：
                // 使用方只说「建在哪个目录」，不说「叫什么」——名字由子插件生成，
                // 并经 [`VdfsWriteResponse::name`] 交回（容器不代拼地址：调用方本来
                // 就知道它请求的是哪个目录）。是否支持由子插件判定，容器不做类型特判
                // （与 `action` 对空 `rel` 的处理一致）。
                p.clone()
                    .vdfs_dispatch(&sub, &rel, VdfsRequest::Write { content })
                    .await
            }
            VdfsRequest::Delete { recursive } => {
                if rel.is_empty() {
                    // 「删掉这个目录」与「卸载这个插件」是同一件事——插件目录存在与否
                    // 就是它装没装的**唯一凭据**（见 `registry.rs`）。必需插件由注册表
                    // 拒绝：可以停用，但不可删除。`recursive` 不参与判定：卸载本就是整棵
                    // 子树的事，注册表一次做完，调用方不需要先知道这棵树有多深。
                    self.registry
                        .uninstall(&dir)
                        .await
                        .map_err(VdfsError::invalid)?;
                    return Ok(VdfsResponse::Unit);
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
#[path = "vdfs.test.rs"]
mod tests;
