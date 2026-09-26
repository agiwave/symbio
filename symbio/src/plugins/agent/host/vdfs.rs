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
//! ## 呈现：整棵目录树，不分类
//!
//! Agent 目录**整棵呈现**：`<条目 id>/<任意相对路径>`。给它分「提示词 / 技能 / MCP」
//! 三类容器等于宿主替能力目录解释语义（每类一套路径白名单、新建模板与默认正文），
//! 改一处要改三处，且与宿主的技能系统 / MCP 客户端天然不同步。
//!
//! 唯一的例外是根下的 `AGENTS.md`（§6 人格与记忆），它走内核的
//! `MemoryFile::node`（带容量闸门），与工作区记忆同一口径。
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
use super::store::{AgentDirRecord, AgentDirStore};
use crate::providers::vdfs_service;
use crate::symbio_core::{descend_addr, host_ctx, notify_change, unwatch_changes, watch_changes};
use crate::symbio_core::{
    VdfsAccess, VdfsActionResult, VdfsChange, VdfsChangeSink, VdfsContent, VdfsContext, VdfsError,
    VdfsItem, VdfsNewType, VdfsNode, VdfsProvider, VdfsRequest, VdfsResponse, VdfsResult,
    VdfsWriteResponse, VDFS_ACTION_EXPORT, VDFS_ACTION_IMPORT, VDFS_EXT_FORM,
};
use crate::symbio_core::{MEMORY_AGENTS_FILE, PLUGIN_FILE, PLUGIN_ID_AGENT};
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
    if p == MEMORY_AGENTS_FILE {
        return RelPath::Instruction;
    }
    match p.split_once('/') {
        None => RelPath::Agent { id: p },
        // 第二段是记忆文件名 → 记忆，而不是「名为 AGENTS.md 的普通文件」
        Some((id, rest)) if rest == MEMORY_AGENTS_FILE => RelPath::Memory { id },
        Some((id, rest)) => RelPath::File { id, rel: rest },
    }
}

/// 路径末段 → 条目 id（去掉 `.agent` 呈现扩展名）
fn id_of(path: &str) -> String {
    vdfs_service::entry::id_of(path, PLUGIN_ID_AGENT)
}

/// 把子 composite 返回的**子树相对路径**提升为本插件空间内的路径。
///
/// 与 `composite::child_path` 同义（这里不复用，避免在插件间引入依赖）。子 composite
/// 内部只认自身子树内的相对路径（`work/AGENTS.md`），因此委托方必须补上挂载段。
///
/// ⚠️ `mount_rel` 是**树内相对**前缀（本插件空间里的首段，见 `sub_vfs`），不是
/// 地址空间里的绝对地址——带上根名会拼出 `<根>/<根>/…`（展示地址只在 `UnifiedFs`
/// 出口翻译一次）。
///
/// ⚠️ **只用于变更帧路径（[`AgentPlugin::watch_at`]），不得用于条目地址。**
/// 两者对「本层补的前缀」要求相反：变更帧的路径会被**每一层容器**继续
/// `child_path` 续接（父 composite 补 `agent`、`UnifiedFs` 补根名），这里给的
/// 相对前缀正是它要的；而**条目地址不会被容器续接**（见
/// `composite/vdfs.rs::container_leaves_item_addresses_untouched`），填进去就是一个
/// 少了外层挂载段的假地址——访问层见非空即不再回填。踩点见 [`AgentPlugin::list_at`]。
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
    // 「装了哪些能力」由**目录**回答（§4.1：目录里有就表示已安装）；宿主按类别
    // 点数会与真实的能力来源形成两份真相。
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
    n.kind = PLUGIN_ID_AGENT.to_string();
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

impl AgentPlugin {
    /// Agent 目录底座（每次请求独立）
    ///
    /// 根 = **本插件自己持有的目录**（构造时由父插件经 `PLUGIN_DIR` 告知，落在
    /// `config_file` 上）——与 [`AgentPlugin::sub_agent`] 取的是**同一份**。
    ///
    /// ⚠️ **不得改回「从请求上下文取」**（`dir_from_ctx(&**host, PLUGIN_ID_AGENT)`）。
    /// `PLUGIN_DIR` 只在**装配期**给出：`composite::build` 构造子插件时、
    /// `AgentPlugin::sub_agent` 造子树时。请求上下文里没有它，而 core 已**不再**
    /// 提供任何全局回退（全局 agent 根那套已随 homedir 下沉到 `home` 删除）——
    /// 拿请求 ctx 取只会 panic 或错位：子智能体空间里列出的「智能体」会变成顶层
    /// 清单（回归钉：`host::tests::sub_agent_agent_list_is_scoped_to_its_own_space`）。
    /// 同理，`RelPath::Agent` / `File` / `Memory` 各域在子空间里也都会读到全局
    /// 的智能体包。
    fn store(&self) -> AgentDirStore {
        AgentDirStore::new(self.config_file().dir().dir())
    }

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
    /// 返回 `Err` = 该 `<id>` 不在挂载根清单里（目录缺失 / manifest 缺失 / 未过 §10
    /// 门槛，判据唯一出处见 [`super::store::AgentDirStore::load_record`]）——调用方
    /// 一律 `NotFound`：不存在「列不出来却能浏览」的目录。
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
    /// 唯一入口：**先按 `path` 定位资源域，再按 `req` 执行操作**。
    ///
    /// 配置文档（`PLUGIN.yml`）按真实文件名可达——先判路径再分流操作；其余全部经
    /// [`parse_rel_path`] 按 path 形状定域，各域逻辑收敛在下方私有方法里
    /// （派发面与实现面分离：本方法只做路由，域内怎么落盘是各方法自己的事）。
    ///
    /// ## 根的自述走 `Stat` 空路径
    ///
    /// 根下只有一种新建类型：智能体——它由 [`Self::stat_at`] 的 `Root` 臂给出，
    /// 而**不是** trait 上的另一个方法。根与更深层的节点因此走同一条通道
    /// （「描述这个节点」），使用方不必先知道某个节点是不是根（见 ADR-030）。
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
    ///
    /// 返回的是**条目**（地址 + 节点）：穿过挂载点时子 composite 给的地址是它自己
    /// 树内的相对地址，必须补上挂载前缀才能交给上层——这正是「地址属于列表」的
    /// 一处实例（同一个节点在父树与子树里地址不同，节点本身没有地址）。
    async fn list_at(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<Vec<VdfsItem>> {
        let store = self.store();
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
                .map(|r| VdfsItem::new(agent_dir_node(&r, &store)))
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
                //
                // ⚠️ **条目地址原样透出，本层不填**（与容器同一契约，见
                // `composite/vdfs.rs::container_leaves_item_addresses_untouched`）：
                // 条目地址要么由**拥有者**填成树内绝对地址，要么**留空**由访问层
                // （`plugins/vdfs/host.rs::item_addr`）按请求地址回填——而请求地址
                // 是完整地址，含本挂载点被挂在哪（`agent/`）。
                //
                // 本层**填不出**这个地址：`mount_rel` 只是本插件空间内的首段
                // （不含 `agent/`），而 `mount_path` 对空 `rel` 返回的正是它——于是
                // 访问层见到非空地址，认为「拥有者已填好」而**不再回填**，子空间里
                // 列出的**每一条**都顶着子空间根地址（会话节点因此变成
                // `<根>/<agent-id>`——那是个目录，前端点开会话即「读取会话转写失败」）。
                // 变更帧不受影响：那条路每层容器都会续接（见 `mount_path`）。
                let (p, sub, _mount_rel) = self.sub_vfs(ctx, &id).await?;
                p.dispatch(
                    &sub,
                    "",
                    VdfsRequest::List {
                        limit: None,
                        before: None,
                    },
                )
                .await?
                .into_list()
                .ok_or_else(mismatch)
            }
            // 记忆是叶子节点
            RelPath::Memory { .. } => Err(VdfsError::not_found(format!(
                "智能体记忆是叶子节点，没有子项：{path}"
            ))),
            RelPath::File { id, rel } => {
                let id = id_of(id);
                // 可挂载的子智能体 → 穿过挂载点，列出子 composite 内对应子树。
                // 条目地址的处理与上面 `RelPath::Agent` 臂**同一条**（原样透出，
                // 本层不填）——理由与那次真实事故都写在那里，改一处必须改两处。
                let (p, sub, _mount_rel) = self.sub_vfs(ctx, &id).await?;
                p.dispatch(
                    &sub,
                    rel,
                    VdfsRequest::List {
                        limit: None,
                        before: None,
                    },
                )
                .await?
                .into_list()
                .ok_or_else(mismatch)
            }
        }
    }

    /// `path` 域的节点元数据（配置文档已在 [`Self::dispatch`] 按路径先行分流）
    ///
    /// ## 根的自述（`RelPath::Root`）里带着「可新建类型」
    ///
    /// 草稿页与落成后的条目是**同一张**详情（`node_ext = form` + 概览定义），
    /// 因此「点添加」与「选中一项」在交互上没有第二种形态——差别只在草稿没有内容。
    ///
    /// 草稿上唯一可做的是**导入整包**，而它是详情页的一条动作
    /// （[`VDFS_ACTION_IMPORT`]，与「导出」「删除」同级），**不是类型里的字段**：
    /// 类型只说「这类东西落成后长什么样」，创建语义归 provider（见 ADR-029）。
    ///
    /// 这一项不能进同步的 `PluginMeta`（`schema` 是 `serde_json::Value`，装配期
    /// 就有，但别的 provider 可能要运行期汇流），因此它随**根节点自述**一起给出：
    /// 容器合成挂载点节点时向本 provider 发一次 `Stat("")`，取走 `new_type`。
    async fn stat_at(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<VdfsNode> {
        let store = self.store();
        match parse_rel_path(path) {
            // 自身根：**名字留空**——provider 不知道自己的挂载名，由使用方回填
            RelPath::Root => Ok(
                VdfsNode::dir("", LABEL, VdfsAccess::LIST_TRAVERSE).with_new_type(Some(
                    VdfsNewType::new(PLUGIN_ID_AGENT, LABEL)
                        .with_description(format!("新建{LABEL}——在详情页里导入整包（.zip）"))
                        .with_node_ext(VDFS_EXT_FORM)
                        .with_schema_opt(
                            serde_json::to_value(super::detail::agent_detail_definition()).ok(),
                        ),
                )),
            ),
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
                // 穿过挂载点：stat 的是**子 composite 的视图**，不是裸 agent 目录里的
                // 同名物理文件。不在清单里 → `sub_vfs` 直接 `NotFound`。
                let (p, sub, _mount_rel) = self.sub_vfs(ctx, &id).await?;
                p.dispatch(&sub, rel, VdfsRequest::Stat)
                    .await?
                    .into_stat()
                    .ok_or_else(mismatch)
            }
        }
    }

    /// `path` 域的内容读取
    async fn read_at(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<VdfsContent> {
        let store = self.store();
        match parse_rel_path(path) {
            // 本应用自身的指令（`{homedir}/AGENTS.md`）
            RelPath::Instruction => {
                let text = self
                    .instruction_store()
                    .await
                    .read()
                    .map_err(|e| VdfsError::not_found(format!("读取系统指令失败：{e}")))?;
                Ok(VdfsContent::text(text))
            }
            // Agent 目录内的文件 / 子路径
            RelPath::File { id, rel } => {
                let id = id_of(id);
                // 穿过挂载点：读的是**子 composite 的视图**
                // （`<挂载点名>/<条目 id>/work/AGENTS.md` 读的是那个子树里 work 的记忆），
                // 而不是裸 agent 目录里的同名物理文件。判定按**路径前缀**统一发生，
                // 不按操作逐个枚举——漏一个操作就会出现「列得出、读不到」。
                let (p, sub, _mount_rel) = self.sub_vfs(ctx, &id).await?;
                p.dispatch(&sub, rel, VdfsRequest::Read)
                    .await?
                    .into_read()
                    .ok_or_else(mismatch)
            }
            // 智能体记忆：Agent 目录下的 `AGENTS.md`
            RelPath::Memory { id } => {
                let id = id_of(id);
                let text = self
                    .memory_store(&store, &id)
                    .await
                    .read()
                    .map_err(|e| VdfsError::not_found(format!("读取智能体记忆失败：{e}")))?;
                Ok(VdfsContent::text(text))
            }
            // Agent 条目本身：读的是**概览**（详情表单 `binding: info` 的输入）
            RelPath::Agent { id } => {
                let id = id_of(id);
                let r = store
                    .get(&id)
                    .ok_or_else(|| VdfsError::not_found(format!("未找到{LABEL}「{id}」")))?;
                let text = serde_json::to_string_pretty(&agent_dir_info(&r, &store))
                    .map_err(|e| VdfsError::internal(format!("概览序列化失败：{e}")))?;
                Ok(VdfsContent::text(text).with_mime("application/json"))
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
        let store = self.store();
        // 系统智能体自身的指令写回（容量闸门在内核里，本插件不重复实现）
        if matches!(parse_rel_path(path), RelPath::Instruction) {
            if content.binary {
                return Err(VdfsError::invalid("AGENTS.md 是文本文件，不接受二进制内容"));
            }
            let instr = self.instruction_store().await;
            let existed = instr.exists();
            let text = content.text.as_deref().unwrap_or_default();
            instr.write(text).map_err(VdfsError::invalid)?;
            notify_change(PLUGIN_ID_AGENT, path);
            return Ok(VdfsWriteResponse {
                name: None,
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
            notify_change(PLUGIN_ID_AGENT, path);
            return Ok(VdfsWriteResponse {
                name: None,
                created: !existed,
                etag: None,
            });
        }
        // 二进制载荷**不再是创建通道**：整包导入已改走详情页动作
        // （[`VDFS_ACTION_IMPORT`]，见 [`Self::action_at`]）。这里显式拒绝，而不是
        // 让它落到下面的文本分支——那会报「不是合法 JSON」，让人以为是内容格式
        // 问题，而真正的原因是**用错了通道**。
        if content.binary {
            return Err(VdfsError::invalid(format!(
                "{LABEL}不接受二进制写入：整包导入请走 `{VDFS_ACTION_IMPORT}` 动作"
            )));
        }
        // Agent 目录内的文件 / 子路径写入
        if let RelPath::File { id, rel } = parse_rel_path(path) {
            let id = id_of(id);
            // 穿过挂载点：写进**子 composite 的视图**。绕过挂载点会让写入既污染
            // 智能体包、又让子 composite 的 provider 完全没参与。
            let (p, sub, _mount_rel) = self.sub_vfs(ctx, &id).await?;
            return p
                .dispatch(
                    &sub,
                    rel,
                    VdfsRequest::Write {
                        content: content.clone(),
                    },
                )
                .await?
                .into_write()
                .ok_or_else(mismatch);
        }
        // Agent 条目本身不可直接写入（无「先建空壳」形态）：新建 = 导入整包动作
        Err(VdfsError::Forbidden(format!(
            "{LABEL}条目不可直接写入：新建请走 `{VDFS_ACTION_IMPORT}` 动作：{path}"
        )))
    }

    /// 删除：系统指令与智能体记忆不可删（要清空就写入空内容）
    async fn delete_at(&self, ctx: &VdfsContext, path: &str, recursive: bool) -> VdfsResult<()> {
        if path.is_empty() {
            return Err(VdfsError::Forbidden(format!("不可删除挂载点：{path}")));
        }
        let store = self.store();
        // 系统指令不可删除（与各层记忆同一口径）：要清空就写入空内容
        if matches!(parse_rel_path(path), RelPath::Instruction) {
            return Err(VdfsError::Forbidden(format!(
                "系统指令不可删除（删除即丢失全部指令）。\
                 如需清空，请向 `{MEMORY_AGENTS_FILE}` 写入空内容。"
            )));
        }
        // 智能体记忆不可删除（与工作区记忆同一口径）：要清空就写入空内容
        if matches!(parse_rel_path(path), RelPath::Memory { .. }) {
            return Err(VdfsError::Forbidden(format!(
                "智能体记忆不可删除（删除即丢失全部长期记忆）。\
                 如需清空，请向 `{MEMORY_AGENTS_FILE}` 写入空内容。"
            )));
        }
        // Agent 目录内的文件 / 子目录
        if let RelPath::File { id, rel } = parse_rel_path(path) {
            let id = id_of(id);
            // 穿过挂载点：删除子 composite 内对应条目。
            let (p, sub, _mount_rel) = self.sub_vfs(ctx, &id).await?;
            return p
                .dispatch(&sub, rel, VdfsRequest::Delete { recursive })
                .await?
                .into_unit()
                .ok_or_else(mismatch);
        }
        let id = id_of(path);
        store
            .delete(&id)
            .map_err(|e| VdfsError::invalid(format!("删除{LABEL}失败：{e}")))?;
        notify_change(PLUGIN_ID_AGENT, &id);
        Ok(())
    }

    /// 新建目录：**只在子智能体内部生效**。
    ///
    /// 挂载根与条目本身不接受 `Mkdir`——新建智能体只有「导入整包」一条通道
    /// （[`VDFS_ACTION_IMPORT`]，见 [`Self::action_at`]）。子树则可以：怎么落由那个
    /// 子树的 provider 决定。
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

    /// 节点动作：「导入」用整包在本目录建出一份资源、「导出」把 agent 目录打成
    /// zip 随 `data` 回传——两者互为逆向（见 [`VDFS_ACTION_IMPORT`] /
    /// [`VDFS_ACTION_EXPORT`]）。
    ///
    /// 「导入」落在**挂载根**上：草稿详情页上那条动作就打在根上，条目 id 取自
    /// 包内 manifest——使用方给不了名字，也就没有「导到哪个名字下」这回事。
    /// 「导出」落在**条目**上（要导出就得先有东西可导）。
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
        if action == VDFS_ACTION_IMPORT {
            if !matches!(parse_rel_path(path), RelPath::Root) {
                return Err(VdfsError::invalid(format!(
                    "「导入」只对{LABEL}挂载根可用：{path}"
                )));
            }
            let pack = vdfs_service::VdfsUnpack::from_payload(payload)
                .map_err(|e| VdfsError::invalid(e.0))?;
            let bytes = pack.bytes().map_err(|e| VdfsError::invalid(e.0))?;
            let store = self.store();
            let r = store
                .import(&bytes, true)
                .map_err(|e| VdfsError::invalid(format!("导入失败：{e}")))?;
            notify_change(PLUGIN_ID_AGENT, &r.id);
            return Ok(VdfsActionResult {
                action: VDFS_ACTION_IMPORT.to_string(),
                ok: true,
                message: format!("已导入{LABEL}「{}」", r.id),
                data: None,
            });
        }
        if action != VDFS_ACTION_EXPORT {
            return Err(VdfsError::NotImplemented);
        }
        if !matches!(parse_rel_path(path), RelPath::Agent { .. }) {
            return Err(VdfsError::invalid(format!(
                "「导出」只对{LABEL}条目可用：{path}"
            )));
        }
        let store = self.store();
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
        watch_changes(PLUGIN_ID_AGENT, path, sink).await
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
        unwatch_changes(PLUGIN_ID_AGENT, path).await
    }
}

/// 子 provider 响应形状不符时的统一错误（子树派发只应回对应形状）
fn mismatch() -> VdfsError {
    VdfsError::internal("响应类型不匹配")
}

#[cfg(test)]
#[path = "vdfs.test.rs"]
mod tests;
