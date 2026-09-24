//! VDFS 挂载点（`<挂载点名>/agent`）—— 本插件**直接实现 `VdfsProvider`**。
//!
//! ## 与其它资源插件的分工差异
//!
//! Agent 是**目录自管型**资源：它的落盘由 [`AgentDirStore`] 负责（工作区级 +
//! 全局级双层、zip-slip 防护、版本硬门槛），**不经 `vdfs_service`**——
//! `vdfs_service` 的三种拓扑都是「`<本插件目录>/<条目 id>/…`」这一固定落位，
//! 而 Agent 要同时看见工作目录与系统目录两层，寻址规则本身是 Agent 语义的一部分。
//!
//! 相同的是**广播**：落盘后一律走 `vdfs::notify_change`，与 `vdfs_service` 三个
//! 实现投的是同一条频道，订阅方无需区分资源住在哪儿。
//!
//! ## 呈现：整棵目录树，不再分类
//!
//! v1 在这里把 Agent 内部拆成「提示词 / 技能 / MCP」三类容器，每类有自己的路径
//! 白名单、新建模板与默认正文——那是宿主在替能力目录解释语义，改一处要改三处，
//! 且与宿主的技能系统 / MCP 客户端天然不同步。
//!
//! v2 直接把 Agent 目录**整棵呈现**：`<条目 id>/<任意相对路径>`。唯一的例外是根下的
//! `AGENTS.md`（§6 人格与记忆），它走内核的 `MemoryFile::node`（带容量闸门），
//! 与工作区记忆同一口径。
//!
//! ## 挂载根只列「装进来的智能体」
//!
//! 挂载根清单 = 各 agent 目录（装进来的子智能体），与 session / model 列表同一口径。
//! `<挂载点名>/agent/AGENTS.md` 也挂在这棵树上，但它**不在清单里**——它是**本应用
//! （系统智能体）自身**的指令（目录名由各模块按自身挂载规则决定），
//! 属于「本 agent 的修改」，入口在**设置页**（`traverse` 里经 `ConfigurableVisitor`
//! 注册，读写仍落在本插件的地址上），混在 agent 列表里会被读成某个包。
//!
//! 与 agent 目录无关的那三个字母 `AGENTS.md` 因此是挂载根下的**保留名**；agent id 的
//! 字符集要求首字符是小写字母或数字，不可能与之相撞（§5.1）。
//!
//! 外部访问一律走 `<根>/agent/…`。

use super::instruction;
use super::memory;
use super::plugin::AgentPlugin;
use super::store::{AgentDirRecord, AgentDirStore, FileEntry};
use crate::providers::vdfs_service;
use crate::symbio_core::vdfs::{
    descend_addr, host_ctx, notify_change, unwatch_changes, watch_changes,
};
use crate::symbio_core::vdfs_provider::{
    VdfsAccess, VdfsActionResult, VdfsChange, VdfsChangeSink, VdfsContent, VdfsContext, VdfsError,
    VdfsNewType, VdfsNode, VdfsProvider, VdfsRequest, VdfsResponse, VdfsResult, VdfsWriteResponse,
    VDFS_ACTION_EXPORT, VDFS_EXT_FORM, VDFS_EXT_ZIP, VDFS_NEW_SOURCE_FILE,
};
use crate::symbio_core::{dir_from_ctx, InvokeRequest, AGENTS_FILE, PLUGIN_AGENT, PLUGIN_FILE};
use async_trait::async_trait;
use std::sync::Arc;

const LABEL: &str = "智能体";

// ==================== 路径解析 ====================

/// 挂载点内相对路径：`<id>` 之后是 Agent 目录内的**任意**相对路径
#[derive(Debug)]
enum RelPath<'a> {
    Root,
    /// 系统智能体自身的指令：挂载根下的 `AGENTS.md`（与 agent 目录无关，见模块文档）
    Instruction,
    /// Agent 本身（`<id>`）
    Agent {
        id: &'a str,
    },
    /// 人格与记忆：`<条目 id>/AGENTS.md`（§6，走内核记忆门面，不是普通文件）
    Memory {
        id: &'a str,
    },
    /// Agent 目录内的文件 / 子目录；`rel` 可含 `/`
    File {
        id: &'a str,
        rel: &'a str,
    },
}

fn parse_rel_path(path: &str) -> RelPath<'_> {
    let p = path.trim_matches('/');
    if p.is_empty() {
        return RelPath::Root;
    }
    // 保留名：挂载根下的 `AGENTS.md` 是**本应用自身**的指令，不是名为它的 agent 目录
    // （agent id 首字符必须是小写字母或数字，两者不可能相撞）
    if p == AGENTS_FILE {
        return RelPath::Instruction;
    }
    match p.split_once('/') {
        None => RelPath::Agent { id: p },
        // 第二段是记忆文件名 → 记忆，而不是「名为 AGENTS.md 的普通文件」
        Some((id, rest)) if rest == AGENTS_FILE => RelPath::Memory { id },
        Some((id, rest)) => RelPath::File { id, rel: rest },
    }
}

/// 路径末段 → 条目 id（去掉 `.agent` 呈现扩展名）
fn id_of(path: &str) -> String {
    vdfs_service::entry::id_of(path, PLUGIN_AGENT)
}

/// 把子 composite 返回的**子树相对路径**提升为本插件空间内的路径。
///
/// 与 `composite::child_path` 同义（这里不复用，避免在插件间引入依赖）。子 composite
/// 内部只认自身子树内的相对路径（`work/AGENTS.md`），因此委托方必须补上挂载段。
///
/// ⚠️ `mount_rel` 是**树内相对**前缀（本插件空间里的首段，见 `sub_vfs`），不是
/// 地址空间里的绝对地址——带上根名会拼出 `<根>/<根>/…`（展示地址只在 `UnifiedFs`
/// 出口翻译一次）。
fn mount_path(mount_rel: &str, rel: &str) -> String {
    match (mount_rel.is_empty(), rel.is_empty()) {
        (true, true) => String::new(),
        (true, false) => rel.to_string(),
        (false, true) => mount_rel.to_string(),
        (false, false) => format!("{mount_rel}/{rel}"),
    }
}

// ==================== 节点合成 ====================

/// Agent 概览（`read` 与详情表单共用的信息载荷）
fn agent_dir_info(r: &AgentDirRecord, store: &AgentDirStore) -> serde_json::Value {
    // 「装了哪些能力」由**目录**回答（§4.1：目录里有就表示已安装），而不是
    // 宿主按类别点数——那是 v1 的做法，与真实的能力来源两份真相。
    let capabilities: Vec<String> = store
        .list_files(&r.manifest.id, "")
        .unwrap_or_default()
        .into_iter()
        .filter(|e| e.is_dir)
        .map(|e| e.path)
        .collect();
    serde_json::json!({
        // config_type = "agent 目录"：项级图标 / 详情分发的顶层键（VdfsNode attributes flatten）
        "config_type": "agent_dir",
        "version": r.manifest.version,
        "spec": r.manifest.spec,
        "requires_spec": r.manifest.requires.spec,
        "scope": r.source.as_str(),
        "dir": r.dir.to_string_lossy(),
        "capabilities": capabilities.join("、"),
    })
}

/// Agent 记录 → VDFS 节点（只读概览表单：`ext = form` + 定义随 `schema` 下发）
///
/// Agent 同时是容器（内部可浏览整棵目录），但**有详情定义**，故呈现为表单文件
/// ——「浏览内部」走 `enter(<id>/…)` 的目录语义，与详情页互不影响。
fn agent_dir_node(r: &AgentDirRecord, store: &AgentDirStore) -> VdfsNode {
    let id = r.manifest.id.clone();
    let title = if r.manifest.name.is_empty() {
        id.clone()
    } else {
        r.manifest.name.clone()
    };
    let mut n = VdfsNode::file(id, title, VdfsAccess::READ);
    n.kind = PLUGIN_AGENT.to_string();
    n.ext = Some(VDFS_EXT_FORM.to_string());
    n.schema = serde_json::to_value(super::detail::agent_detail_definition()).ok();
    if !r.manifest.description.is_empty() {
        n.description = Some(r.manifest.description.clone());
    }
    n.attributes = agent_dir_info(r, store)
        .as_object()
        .cloned()
        .unwrap_or_default();
    n
}

/// Agent 目录内的一条条目 → VDFS 节点
///
/// 节点名 = 目录内相对路径的**末段**（与 `plugins/vdfs/physical.rs` 同款写法）：
/// 框架经 `fill_node_paths` 用挂载点（`agent`）+ 父地址（`<id>`）重建树内全路径，
fn entry_node(e: &FileEntry) -> VdfsNode {
    let name = e.path.rsplit('/').next().unwrap_or(&e.path).to_string();
    let mut n = if e.is_dir {
        VdfsNode::dir(name.clone(), name.clone(), VdfsAccess::LIST_TRAVERSE)
    } else {
        VdfsNode::file(name.clone(), name.clone(), VdfsAccess::READ_WRITE)
    };
    if !e.is_dir {
        n.ext = e.path.rsplit_once('.').map(|(_, ext)| ext.to_string());
        n.size = Some(e.size);
    }
    n
}

impl AgentPlugin {
    /// 依请求上下文构造 AgentDirStore（每次请求独立，与 route 入口一致）
    ///
    /// agent 目录根 = **本插件自己的目录**，取自父插件经 `PLUGIN_DIR` 传下的目录
    /// （`dir_from_ctx`；缺省退回常规落位）——与 `AgentPlugin::build` 同源，
    /// 这里不另拼一份 `<homedir>/…/agent`。
    fn store_of(ctx: &Arc<dyn InvokeRequest>) -> AgentDirStore {
        let dir = dir_from_ctx(&**ctx, PLUGIN_AGENT);
        AgentDirStore::new(dir.dir())
    }

    /// 系统智能体自身指令 → VDFS 节点（`list` 与 `stat` 共用同一份形状）
    /// 系统智能体自身指令 → VDFS 节点（`list` 与 `stat` 共用同一份形状）。
    ///
    /// `traverse` 也用它拼设置页条目：调用方拿到节点后改 `path` 为真实地址即可。
    pub(crate) async fn instruction_node(&self) -> VdfsNode {
        self.instruction_store()
            .await
            .node(&instruction::node_spec())
    }

    /// 穿过挂载点：取子智能体（`<id>`）的 VDFS 视图。
    ///
    /// 与文件系统的「`<挂载点名>/<条目 id>` 是一个挂载点、底下挂的是子 composite 的虚拟
    /// 文件系统」同义——子 composite 与系统根用的是**同一份** `CompositeVfs`，
    /// 因此钻进子智能体后看到的左栏（会话 / 模型 / 智能体 / MCP / 技能 / 设置）
    /// 与父智能体完全一致，且 `mcp`/`skill` 等点进去是各自 provider 的富列表，
    /// 而不是裸磁盘目录。
    ///
    /// 取回方式走 **core 查询接口 `Plugin::get_vfs_provider`**，不依赖 composite 插件的
    /// 具体类型、也不走 `CapabilityVisitor`（`CapabilityVisitor` 是 LLM 链路的能力收集
    /// 通道，与系统 VDFS 发现无关）。`agent` 与 `composite` 之间因此没有类型耦合。
    ///
    /// 返回 `Err` = 该 `<id>` 不是可装配的 v2 子 Agent（目录缺失 / manifest 非
    /// `agent-dir/v2`）——调用方据此**回退到 raw-dir 行为**（v1 / legacy 约定目录）。
    ///
    /// 元组第三项 `mount_rel` 是挂载点的**树内相对前缀**——本插件空间里的首段
    /// `<id>`，**不含地址空间根名**。子 composite 返回的路径只认自身子树相对地址
    /// （`work/AGENTS.md`），故由本插件补上挂载段；根字面量只在 `UnifiedFs` 出口统一
    /// 翻译一次（`to_display`），这里带上就会拼错（展示地址只在 `UnifiedFs`）。
    ///
    /// ⚠️ 它与**地址空间里的当前父地址**（`VDFS_PARENT_ADDR`）是两回事：后者含根名
    /// （地址字面量只由 vdfs 插件自己决定，外部注释里也不写死——见
    /// `plugins/vdfs/fs.rs` 的 `VDFS_ADDR_ROOT`），供子 provider 计算给模型 / 用户看的
    /// 展示地址。两者都由本函数给出：前者用于**路径回填**，后者经子上下文传下去。
    ///
    /// 分形：`<id>` 只是本级的挂载名，任意层级都一样——子智能体内部的 `agent` 实例
    /// 再钻一层时，收到的路径本就是它自己空间里的 `<sub-id>/…`。
    async fn sub_vfs(
        &self,
        ctx: &VdfsContext,
        id: &str,
    ) -> VdfsResult<(Arc<dyn VdfsProvider>, VdfsContext, String)> {
        let host = host_ctx(ctx)?;
        let tree = self
            .sub_agent(id, &host)
            .await
            .ok_or_else(|| VdfsError::not_found(format!("未找到{LABEL}「{id}」")))?;
        // 系统链路：直接经 core 查询接口取子 composite 暴露的 VDFS provider（无 CapabilityVisitor）。
        let provider = tree
            .get_vfs_provider()
            .ok_or_else(|| VdfsError::internal(format!("子智能体「{id}」未暴露 VDFS 根")))?;

        // 地址空间：父地址续接 `<id>`（与 composite 派发的改写规则同一份，含根名）。
        let sub_ctx = ctx
            .clone()
            .with_parent_addr(descend_addr(ctx.parent_addr(), id));
        // 路径回填：树内相对前缀 = 本插件空间里的首段（`<id>`），见上方文档。
        Ok((provider, sub_ctx, id.to_string()))
    }
}

#[async_trait]
impl VdfsProvider for AgentPlugin {
    /// 根下只有一种新建方式：整包导入（zip）。
    ///
    /// 留在 provider 上而不进同步的 `PluginMeta` 的理由与 session 相同——它是挂载
    /// 点的**动态自述**，由容器合成根节点时现场取。
    async fn new_types(&self) -> Vec<VdfsNewType> {
        vec![VdfsNewType::new(VDFS_EXT_ZIP, format!("{LABEL}包"))
            .with_description(format!("导入{LABEL}整包（.zip）——整目录覆盖同名条目"))
            .with_source(VDFS_NEW_SOURCE_FILE)]
    }

    /// 唯一入口：**先按 `path` 定位资源域，再按 `req` 执行操作**。
    ///
    /// 配置文档（`PLUGIN.yml`）按真实文件名可达——先判路径再分流操作；其余全部经
    /// [`parse_rel_path`] 按 path 形状定域，各域逻辑收敛在下方私有方法里
    /// （派发面与实现面分离：本方法只做路由，域内怎么落盘是各方法自己的事）。
    async fn dispatch(
        &self,
        ctx: &VdfsContext,
        path: &str,
        req: VdfsRequest,
    ) -> VdfsResult<VdfsResponse> {
        // 配置文档按**真实文件名**可达（列表里不并列，进设置走 ConfigurableVisitor）
        if path.trim_matches('/') == PLUGIN_FILE {
            return match req {
                VdfsRequest::Stat => Ok(VdfsResponse::Stat(self.config_file().node())),
                VdfsRequest::Read => Ok(VdfsResponse::Read(
                    self.config_file().read(self.config_slot()).await?,
                )),
                VdfsRequest::Write { content } => Ok(VdfsResponse::Write(
                    self.config_file()
                        .apply(self.config_slot(), &content)
                        .await?,
                )),
                _ => Err(VdfsError::invalid(format!(
                    "该路径是文件，不支持此操作：{path}"
                ))),
            };
        }
        match req {
            VdfsRequest::List { .. } => Ok(VdfsResponse::List(self.list_at(ctx, path).await?)),
            VdfsRequest::Stat => Ok(VdfsResponse::Stat(self.stat_at(ctx, path).await?)),
            VdfsRequest::Read => Ok(VdfsResponse::Read(self.read_at(ctx, path).await?)),
            VdfsRequest::Write { content } => Ok(VdfsResponse::Write(
                self.write_at(ctx, path, &content).await?,
            )),
            VdfsRequest::Delete { recursive } => {
                self.delete_at(ctx, path, recursive).await?;
                Ok(VdfsResponse::Unit)
            }
            VdfsRequest::Mkdir => {
                self.mkdir_at(ctx, path).await?;
                Ok(VdfsResponse::Unit)
            }
            VdfsRequest::Move { to } => {
                self.move_at(ctx, path, &to).await?;
                Ok(VdfsResponse::Unit)
            }
            VdfsRequest::Action { action, payload } => Ok(VdfsResponse::Action(
                self.action_at(ctx, path, &action, payload.as_ref()).await?,
            )),
            VdfsRequest::Watch { sink } => {
                self.watch_at(ctx, path, sink).await?;
                Ok(VdfsResponse::Unit)
            }
            VdfsRequest::Unwatch => {
                self.unwatch_at(ctx, path).await?;
                Ok(VdfsResponse::Unit)
            }
        }
    }
}

impl AgentPlugin {
    /// 挂载根 = **装进来的智能体清单**；可挂载子智能体穿过挂载点看子 composite 视图
    async fn list_at(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<Vec<VdfsNode>> {
        let host = host_ctx(ctx)?;
        let store = Self::store_of(&host);
        match parse_rel_path(path) {
            // 挂载根 = **装进来的智能体清单**，一样别的都没有。
            //
            // 本应用自身的指令（`<根>/agent/AGENTS.md`）也挂在这棵树上，但它是
            // **本应用自身的设置**，不是装进来的智能体——混在这张列表里会让人
            // 把它读成某个包。它的入口在设置页（见 `super::plugin` 的 `traverse`），
            // 地址（`<根>/agent/AGENTS.md`）照旧可达，只是不在这里列出。
            RelPath::Root => Ok(store
                .list()
                .into_iter()
                .map(|r| agent_dir_node(&r, &store))
                .collect()),
            // 指令是叶子节点
            RelPath::Instruction => Err(VdfsError::invalid(format!(
                "该路径是文件，不可列举：{path}"
            ))),
            RelPath::Agent { id } => {
                let id = id_of(id);
                // 可挂载的 v2 子智能体 → 穿过挂载点，列出**子 composite 的根视图**。
                // 子根与父（系统）根是同一份 `CompositeVfs`，因此同样按 `hidden`
                // 只显示可见插件（gateway/web/telegram/local/work 等配置型挂载点不会
                // 出现在侧边栏），父子两侧栏完全一致。
                if let Ok((p, sub, mount_rel)) = self.sub_vfs(ctx, &id).await {
                    let mut items = p
                        .dispatch(
                            &sub,
                            "",
                            VdfsRequest::List {
                                limit: None,
                                before: None,
                            },
                        )
                        .await?
                        .into_list()
                        .ok_or_else(mismatch)?;
                    for n in &mut items {
                        n.path = mount_path(&mount_rel, &n.path);
                    }
                    return Ok(items);
                }
                // 回退（v1 / legacy 约定目录）：原始 agent 目录罗列
                // 存在性校验：不存在的条目应报 NotFound 而非给出空清单
                store
                    .get(&id)
                    .ok_or_else(|| VdfsError::not_found(format!("未找到{LABEL}「{id}」")))?;
                // 记忆节点：形状来自内核（`MemoryFile::node`），与 `stat` 同源
                let memory_node = self
                    .memory_store(&store, &id)
                    .await
                    .node(&memory::node_spec());
                let mut nodes = vec![memory_node];
                // 其余按目录原样呈现（根 `AGENTS.md` 已由记忆节点代表，不重复）
                nodes.extend(
                    store
                        .list_files(&id, "")
                        .map_err(|e| VdfsError::not_found(format!("列出目录失败：{e}")))?
                        .into_iter()
                        .filter(|e| e.path != AGENTS_FILE)
                        .map(|e| entry_node(&e)),
                );
                Ok(nodes)
            }
            // 记忆是叶子节点
            RelPath::Memory { .. } => Err(VdfsError::not_found(format!(
                "智能体记忆是叶子节点，没有子项：{path}"
            ))),
            RelPath::File { id, rel } => {
                let id = id_of(id);
                // 可挂载的子智能体 → 穿过挂载点，列出子 composite 内对应子树
                if let Ok((p, sub, mount_rel)) = self.sub_vfs(ctx, &id).await {
                    let mut items = p
                        .dispatch(
                            &sub,
                            rel,
                            VdfsRequest::List {
                                limit: None,
                                before: None,
                            },
                        )
                        .await?
                        .into_list()
                        .ok_or_else(mismatch)?;
                    for n in &mut items {
                        n.path = mount_path(&mount_rel, &n.path);
                    }
                    return Ok(items);
                }
                let e = store
                    .stat_item(&id, rel)
                    .map_err(|e| VdfsError::not_found(format!("未找到路径「{path}」：{e}")))?;
                if !e.is_dir {
                    return Err(VdfsError::invalid(format!(
                        "该路径是文件，不可列举：{path}"
                    )));
                }
                Ok(store
                    .list_files(&id, rel)
                    .map_err(|e| VdfsError::not_found(format!("列出目录失败：{e}")))?
                    .into_iter()
                    .map(|e| entry_node(&e))
                    .collect())
            }
        }
    }

    /// `path` 域的节点元数据（配置文档已在 [`Self::dispatch`] 按路径先行分流）
    async fn stat_at(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<VdfsNode> {
        let host = host_ctx(ctx)?;
        let store = Self::store_of(&host);
        match parse_rel_path(path) {
            // 自身根：**名字留空**——provider 不知道自己的挂载名，由使用方回填
            RelPath::Root => Ok(VdfsNode::dir("", LABEL, VdfsAccess::LIST_TRAVERSE)),
            // 本应用自身的指令（`{homedir}/AGENTS.md`）
            RelPath::Instruction => Ok(self.instruction_node().await),
            RelPath::Agent { id } => {
                let id = id_of(id);
                let r = store
                    .get(&id)
                    .ok_or_else(|| VdfsError::not_found(format!("未找到{LABEL}「{id}」")))?;
                Ok(agent_dir_node(&r, &store))
            }
            RelPath::Memory { id } => {
                let id = id_of(id);
                let memory = self.memory_store(&store, &id).await;
                if !memory.has_scope() {
                    return Err(VdfsError::not_found(format!("未找到{LABEL}「{id}」")));
                }
                Ok(memory.node(&memory::node_spec()))
            }
            RelPath::File { id, rel } => {
                let id = id_of(id);
                // 可挂载的子智能体 → 穿过挂载点，stat 子 composite 内对应条目
                if let Ok((p, sub, mount_rel)) = self.sub_vfs(ctx, &id).await {
                    let mut n = p
                        .dispatch(&sub, rel, VdfsRequest::Stat)
                        .await?
                        .into_stat()
                        .ok_or_else(mismatch)?;
                    n.path = mount_path(&mount_rel, &n.path);
                    return Ok(n);
                }
                let e = store
                    .stat_item(&id, rel)
                    .map_err(|e| VdfsError::not_found(format!("未找到路径「{path}」：{e}")))?;
                Ok(entry_node(&e))
            }
        }
    }

    /// `path` 域的内容读取
    async fn read_at(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<VdfsContent> {
        let host = host_ctx(ctx)?;
        let store = Self::store_of(&host);
        match parse_rel_path(path) {
            // 本应用自身的指令（`{homedir}/AGENTS.md`）
            RelPath::Instruction => {
                let text = self
                    .instruction_store()
                    .await
                    .read()
                    .map_err(|e| VdfsError::not_found(format!("读取系统指令失败：{e}")))?;
                Ok(VdfsContent::text(path, text))
            }
            // Agent 目录内的文件 / 子路径
            RelPath::File { id, rel } => {
                let id = id_of(id);
                // 可挂载的子智能体 → 穿过挂载点：读的是**子 composite 的视图**
                // （`<挂载点名>/<条目 id>/work/AGENTS.md` 读的是那个子树里 work 的记忆），
                // 而不是裸 agent 目录里的同名物理文件。判定按**路径前缀**统一发生，
                // 不按操作逐个枚举——漏一个操作就会出现「列得出、读不到」。
                if let Ok((p, sub, mount_rel)) = self.sub_vfs(ctx, &id).await {
                    let mut c = p
                        .dispatch(&sub, rel, VdfsRequest::Read)
                        .await?
                        .into_read()
                        .ok_or_else(mismatch)?;
                    c.path = mount_path(&mount_rel, &c.path);
                    return Ok(c);
                }
                // 非可挂载目录（v1 / legacy 约定目录）：直读（沙箱在 store 里）
                let text = store
                    .read_item(&id, rel)
                    .map_err(|e| VdfsError::not_found(format!("读取失败：{e}")))?;
                Ok(VdfsContent::text(path, text))
            }
            // 智能体记忆：Agent 目录下的 `AGENTS.md`
            RelPath::Memory { id } => {
                let id = id_of(id);
                let text = self
                    .memory_store(&store, &id)
                    .await
                    .read()
                    .map_err(|e| VdfsError::not_found(format!("读取智能体记忆失败：{e}")))?;
                Ok(VdfsContent::text(path, text))
            }
            // Agent 条目本身：读的是**概览**（详情表单 `binding: info` 的输入）
            RelPath::Agent { id } => {
                let id = id_of(id);
                let r = store
                    .get(&id)
                    .ok_or_else(|| VdfsError::not_found(format!("未找到{LABEL}「{id}」")))?;
                let text = serde_json::to_string_pretty(&agent_dir_info(&r, &store))
                    .map_err(|e| VdfsError::internal(format!("概览序列化失败：{e}")))?;
                Ok(VdfsContent::text(path, text).with_mime("application/json"))
            }
            RelPath::Root => Err(VdfsError::invalid(format!(
                "该路径是目录，不可读取内容：{path}"
            ))),
        }
    }

    /// `path` 域的写入（配置文档已在 [`Self::dispatch`] 按路径先行分流）
    async fn write_at(
        &self,
        ctx: &VdfsContext,
        path: &str,
        content: &VdfsContent,
    ) -> VdfsResult<VdfsWriteResponse> {
        let host = host_ctx(ctx)?;
        let store = Self::store_of(&host);
        // 系统智能体自身的指令写回（容量闸门在内核里，本插件不重复实现）
        if matches!(parse_rel_path(path), RelPath::Instruction) {
            if content.binary {
                return Err(VdfsError::invalid("AGENTS.md 是文本文件，不接受二进制内容"));
            }
            let instr = self.instruction_store().await;
            let existed = instr.exists();
            let text = content.text.as_deref().unwrap_or_default();
            instr.write(text).map_err(VdfsError::invalid)?;
            notify_change(PLUGIN_AGENT, path);
            return Ok(VdfsWriteResponse {
                path: path.to_string(),
                created: !existed,
                etag: None,
            });
        }
        // 智能体记忆写回（容量闸门在内核里，本插件不重复实现）
        if let RelPath::Memory { id } = parse_rel_path(path) {
            if content.binary {
                return Err(VdfsError::invalid("智能体记忆是文本文件，不接受二进制内容"));
            }
            let id = id_of(id);
            let memory = self.memory_store(&store, &id).await;
            if !memory.has_scope() {
                return Err(VdfsError::not_found(format!("未找到{LABEL}「{id}」")));
            }
            let existed = memory.exists();
            let text = content.text.as_deref().unwrap_or_default();
            memory.write(text).map_err(VdfsError::invalid)?;
            notify_change(PLUGIN_AGENT, path);
            return Ok(VdfsWriteResponse {
                path: path.to_string(),
                created: !existed,
                etag: None,
            });
        }
        // 整包导入：Agent **唯一的创建方式**（id 取自包内 manifest，忽略建议名）
        if content.binary {
            if !matches!(parse_rel_path(path), RelPath::Agent { .. }) {
                return Err(VdfsError::invalid(format!(
                    "{LABEL}整包只能导入到挂载根下：{path}"
                )));
            }
            let bytes = vdfs_service::decode_b64(content.b64.as_deref().unwrap_or_default())
                .map_err(|e| VdfsError::invalid(e.0))?;
            let r = store
                .import(&bytes, true)
                .map_err(|e| VdfsError::invalid(format!("导入失败：{e}")))?;
            notify_change(PLUGIN_AGENT, &r.id);
            return Ok(VdfsWriteResponse {
                path: r.id,
                created: !r.replaced,
                etag: None,
            });
        }
        // Agent 目录内的文件 / 子路径写入
        if let RelPath::File { id, rel } = parse_rel_path(path) {
            let id = id_of(id);
            // 可挂载的子智能体 → 穿过挂载点：写进**子 composite 的视图**。
            // 这是「报成功却落进裸 agent 目录」那个回归的修复点——绕过挂载点会让
            // 写入既污染智能体包、又让子 composite 的 provider 完全没参与。
            if let Ok((p, sub, mount_rel)) = self.sub_vfs(ctx, &id).await {
                let mut r = p
                    .dispatch(
                        &sub,
                        rel,
                        VdfsRequest::Write {
                            content: content.clone(),
                        },
                    )
                    .await?
                    .into_write()
                    .ok_or_else(mismatch)?;
                r.path = mount_path(&mount_rel, &r.path);
                return Ok(r);
            }
            // 非可挂载目录（v1 / legacy 约定目录）：直写（路径沙箱 + 容量闸门在 store 里）
            let text = content.text.as_deref().unwrap_or("");
            let existed = store.stat_item(&id, rel).is_ok();
            let max_bytes = self.item_max_bytes().await;
            store
                .write_item(&id, rel, text, max_bytes)
                .map_err(|e| VdfsError::invalid(format!("写入失败：{e}")))?;
            notify_change(PLUGIN_AGENT, path);
            return Ok(VdfsWriteResponse {
                path: path.to_string(),
                created: !existed,
                etag: None,
            });
        }
        // Agent 条目本身不可表单新建 / 覆盖（无「先建空壳」形态）
        Err(VdfsError::Forbidden(format!(
            "{LABEL}只支持整包导入，不支持表单写入：{path}"
        )))
    }

    /// 删除：系统指令与智能体记忆不可删（要清空就写入空内容）
    async fn delete_at(&self, ctx: &VdfsContext, path: &str, recursive: bool) -> VdfsResult<()> {
        if path.is_empty() {
            return Err(VdfsError::Forbidden(format!("不可删除挂载点：{path}")));
        }
        let host = host_ctx(ctx)?;
        let store = Self::store_of(&host);
        // 系统指令不可删除（与各层记忆同一口径）：要清空就写入空内容
        if matches!(parse_rel_path(path), RelPath::Instruction) {
            return Err(VdfsError::Forbidden(format!(
                "系统指令不可删除（删除即丢失全部指令）。\
                 如需清空，请向 `{AGENTS_FILE}` 写入空内容。"
            )));
        }
        // 智能体记忆不可删除（与工作区记忆同一口径）：要清空就写入空内容
        if matches!(parse_rel_path(path), RelPath::Memory { .. }) {
            return Err(VdfsError::Forbidden(format!(
                "智能体记忆不可删除（删除即丢失全部长期记忆）。\
                 如需清空，请向 `{AGENTS_FILE}` 写入空内容。"
            )));
        }
        // Agent 目录内的文件 / 子目录
        if let RelPath::File { id, rel } = parse_rel_path(path) {
            let id = id_of(id);
            // 可挂载的子智能体 → 穿过挂载点，删除子 composite 内对应条目
            if let Ok((p, sub, _mount_rel)) = self.sub_vfs(ctx, &id).await {
                return p
                    .dispatch(&sub, rel, VdfsRequest::Delete { recursive })
                    .await?
                    .into_unit()
                    .ok_or_else(mismatch);
            }
            let e = store
                .stat_item(&id, rel)
                .map_err(|e| VdfsError::not_found(format!("未找到路径「{path}」：{e}")))?;
            if e.is_dir {
                std::fs::remove_dir_all(store.item_path(&id, rel).map_err(VdfsError::invalid)?)
                    .map_err(|e| VdfsError::invalid(format!("删除目录失败：{e}")))?;
            } else {
                store
                    .delete_item(&id, rel)
                    .map_err(|e| VdfsError::invalid(format!("删除失败：{e}")))?;
            }
            notify_change(PLUGIN_AGENT, path);
            return Ok(());
        }
        let id = id_of(path);
        store
            .delete(&id)
            .map_err(|e| VdfsError::invalid(format!("删除{LABEL}失败：{e}")))?;
        notify_change(PLUGIN_AGENT, &id);
        Ok(())
    }

    /// 新建目录：**只在可挂载的子智能体内部生效**。
    ///
    /// 裸 agent 目录（v1 / legacy）不支持在包内造目录——`AgentDirStore` 本身没有
    /// 这个能力（目录即配置，见模块文档），保持 `NotImplemented`。
    /// 子树则可以：挂载点路径由运行期规则凭借挂载前缀决定，由那个子树的 provider 决定怎么落。
    async fn mkdir_at(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<()> {
        if let RelPath::File { id, rel } = parse_rel_path(path) {
            let id = id_of(id);
            if let Ok((p, sub, _mount_rel)) = self.sub_vfs(ctx, &id).await {
                return p
                    .dispatch(&sub, rel, VdfsRequest::Mkdir)
                    .await?
                    .into_unit()
                    .ok_or_else(mismatch);
            }
        }
        Err(VdfsError::NotImplemented)
    }

    /// 移动 / 重命名：**只在同一个可挂载的子智能体内部**生效。
    ///
    /// 跨挂载点移动没有意义（两侧是不同的 provider，甚至不同的存储），与
    /// `CompositeVfs` 拒跨子目录同一口径；裸 agent 目录仍不支持。
    async fn move_at(&self, ctx: &VdfsContext, from: &str, to: &str) -> VdfsResult<()> {
        let (
            RelPath::File {
                id: from_id,
                rel: from_rel,
            },
            RelPath::File {
                id: to_id,
                rel: to_rel,
            },
        ) = (parse_rel_path(from), parse_rel_path(to))
        else {
            return Err(VdfsError::NotImplemented);
        };
        let (from_id, to_id) = (id_of(from_id), id_of(to_id));
        if from_id != to_id {
            return Err(VdfsError::invalid(format!(
                "不支持跨{LABEL}移动：{from} → {to}"
            )));
        }
        if let Ok((p, sub, _mount_rel)) = self.sub_vfs(ctx, &from_id).await {
            return p
                .dispatch(
                    &sub,
                    from_rel,
                    VdfsRequest::Move {
                        to: to_rel.to_string(),
                    },
                )
                .await?
                .into_unit()
                .ok_or_else(mismatch);
        }
        Err(VdfsError::NotImplemented)
    }

    /// 节点动作：「导出」把 agent 目录打成 zip 随 `data` 回传
    /// （与二进制写入的整包导入互为逆向）。
    ///
    /// 条目内部的子路径若落在可挂载的子智能体里，同样穿过挂载点交给该子树
    /// （动作是 provider 自持的动词，容器/委托方只负责把地址转发到位）。
    async fn action_at(
        &self,
        ctx: &VdfsContext,
        path: &str,
        action: &str,
        payload: Option<&serde_json::Value>,
    ) -> VdfsResult<VdfsActionResult> {
        if let RelPath::File { id, rel } = parse_rel_path(path) {
            let id = id_of(id);
            if let Ok((p, sub, _mount_rel)) = self.sub_vfs(ctx, &id).await {
                return p
                    .dispatch(
                        &sub,
                        rel,
                        VdfsRequest::Action {
                            action: action.to_string(),
                            payload: payload.cloned(),
                        },
                    )
                    .await?
                    .into_action()
                    .ok_or_else(mismatch);
            }
            return Err(VdfsError::NotImplemented);
        }
        if action != VDFS_ACTION_EXPORT {
            return Err(VdfsError::NotImplemented);
        }
        if !matches!(parse_rel_path(path), RelPath::Agent { .. }) {
            return Err(VdfsError::invalid(format!(
                "「导出」只对{LABEL}条目可用：{path}"
            )));
        }
        let host = host_ctx(ctx)?;
        let store = Self::store_of(&host);
        let id = id_of(path);
        let bytes = store
            .export(&id)
            .map_err(|e| VdfsError::not_found(format!("导出失败：{e}")))?;
        let pack = vdfs_service::VdfsPack::new(&id, &bytes);
        let data = serde_json::to_value(&pack)
            .map_err(|e| VdfsError::internal(format!("导出结果序列化失败: {e}")))?;
        Ok(VdfsActionResult {
            action: VDFS_ACTION_EXPORT.to_string(),
            ok: true,
            message: format!("已打包「{}」", pack.filename),
            data: Some(data),
        })
    }

    /// 订阅变更：可挂载的子智能体 → 穿过挂载点，把子树 provider 报出的相对路径
    /// 补上挂载段再交给同一个 sink（子树内部只认自身相对路径）。
    async fn watch_at(
        &self,
        ctx: &VdfsContext,
        path: &str,
        sink: VdfsChangeSink,
    ) -> VdfsResult<()> {
        if let RelPath::File { id, rel } = parse_rel_path(path) {
            let id = id_of(id);
            if let Ok((p, sub, mount_rel)) = self.sub_vfs(ctx, &id).await {
                let wrapped: VdfsChangeSink =
                    Arc::new(move |c: VdfsChange| sink(c.map_paths(|x| mount_path(&mount_rel, x))));
                return p
                    .dispatch(&sub, rel, VdfsRequest::Watch { sink: wrapped })
                    .await?
                    .into_unit()
                    .ok_or_else(mismatch);
            }
        }
        watch_changes(PLUGIN_AGENT, path, sink).await
    }

    /// 取消订阅：与 `watch_at` 同一条判定——**成对**才配对得上计数
    /// （订阅记在子 provider 名下，漏了这一跳会留下永不释放的订阅）。
    async fn unwatch_at(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<()> {
        if let RelPath::File { id, rel } = parse_rel_path(path) {
            let id = id_of(id);
            if let Ok((p, sub, _mount_rel)) = self.sub_vfs(ctx, &id).await {
                return p
                    .dispatch(&sub, rel, VdfsRequest::Unwatch)
                    .await?
                    .into_unit()
                    .ok_or_else(mismatch);
            }
        }
        unwatch_changes(PLUGIN_AGENT, path).await
    }
}

/// 子 provider 响应形状不符时的统一错误（子树派发只应回对应形状）
fn mismatch() -> VdfsError {
    VdfsError::internal("响应类型不匹配")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `<id>` 之后是 Agent 目录内的任意相对路径（可多段）
    #[test]
    fn rel_path_splits_agent_and_inner_path() {
        assert!(matches!(parse_rel_path(""), RelPath::Root));
        assert!(matches!(parse_rel_path("/"), RelPath::Root));
        assert!(matches!(parse_rel_path("b1"), RelPath::Agent { id: "b1" }));
        assert!(matches!(
            parse_rel_path("b1.agent"),
            RelPath::Agent { id: "b1.agent" }
        ));
        match parse_rel_path("b1/skill/foo/SKILL.md") {
            RelPath::File { id, rel } => {
                assert_eq!(id, "b1");
                assert_eq!(rel, "skill/foo/SKILL.md");
            }
            other => panic!("期望 File，实际：{other:?}"),
        }
        // 第二段是记忆文件名 → 记忆，而不是普通文件
        match parse_rel_path("b1/AGENTS.md") {
            RelPath::Memory { id } => assert_eq!(id, "b1"),
            other => panic!("期望 Memory，实际：{other:?}"),
        }
        // 更深处的同名文件仍是普通文件（记忆只在 Agent 根这一层）
        assert!(matches!(
            parse_rel_path("b1/skill/AGENTS.md"),
            RelPath::File { .. }
        ));
    }

    /// 挂载根下的 `AGENTS.md` 是**本应用自身的指令**，不是名为它的 agent 目录
    ///
    /// 两者不可能相撞：agent id 的首字符必须是小写字母或数字（§5.1），
    /// 而保留名以大写 `A` 开头。
    #[test]
    fn root_agents_md_is_the_host_instruction_not_an_agent_dir() {
        assert!(matches!(parse_rel_path(AGENTS_FILE), RelPath::Instruction));
        assert!(matches!(parse_rel_path("/AGENTS.md"), RelPath::Instruction));
        // 带子路径时不再命中保留名（那是一条指向不存在条目的普通 agent 目录路径）
        assert!(matches!(
            parse_rel_path("AGENTS.md/x"),
            RelPath::File { .. }
        ));
    }

    /// 路径末段 → Agent id（去掉 `.agent` 呈现扩展名）
    #[test]
    fn id_of_strips_presentation_extension() {
        assert_eq!(id_of("demo"), "demo");
        assert_eq!(id_of("demo.agent"), "demo");
    }
}
