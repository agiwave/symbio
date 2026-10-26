//! 装配策略 —— 「一棵标准插件树挂哪些插件」「哪些插件不许被停用」。
//!
//! ## 为什么不在 `symbio_core`
//!
//! 这里的常量**不是**上下文键（不走 `SymbioKey`、不进任何 `ctx`），**也不是**
//! 路由地址——它们描述的是**装配方的事实**：一棵子树默认挂哪些插件、哪些插件属于
//! 「界面底座」因此不许停用、每个插件的出厂 id 叫什么。
//!
//! 按依赖方判据（[D-023](../../../docs/003-design/001-核心架构.md "DE-001-核心架构")）它们
//! 本来「够格」留在 core（各有多个消费方）。但那条判据回答的是「两个互不相认的插件
//! 要不要共享一处**机制**」；这里要共享的不是机制，而是**宿主决定接哪些插件**。
//! 装配是宿主的事：插件清单、启停状态、出厂 id 与装配结果归装配方持有，容器只经
//! [`symbio_core::REQUIRED_PLUGINS`] 接收结果（见 `plugins/composite` 的 README）。
//! 因此加一个插件不再需要改 `symbio_core`——这是「加插件不改内核」的判据。
//!
//! ## 消费方
//!
//! | 常量 | 消费方 |
//! |---|---|
//! | [`ASSEMBLY_SUB_AGENT_PLUGINS`] | `plugins/home`（系统树）· `plugins/agent`（子 Agent 树） |
//! | [`ASSEMBLY_UNDISABLABLE_PLUGINS`] | `plugins/composite`（执行停用前查）· `plugins/plugin_manager`（界面置灰） |
//! | [`PLUGIN_ID_*`](PLUGIN_ID_HOME) | 每个插件自己的工厂注册与 `PluginMeta`；跨插件只经对象注册表按名字查找 |
//!
//! 只被**一个**模块依赖的两条已按同判据下沉：系统树的插件清单并入 `plugins/home` 的
//! `SYSTEM_PLUGINS`（原本只是本清单的纯别名，无第二份字面量），
//! `SYSTEM_LEVEL_PROVIDERS` → `plugins/composite`。
//!
//! ## 与 `plugin::ids` 的分工（该域已整体并入本模块）
//!
//! - 本模块：**部署事实**——插件叫什么（`PLUGIN_ID_*`）、一棵树装哪些、哪些不能关；
//! - `plugin` 域：**协议面**——`Plugin` trait、信封、`PluginDir`、清单键。
//!   两者都以插件名为字面量，但一个是「这批插件怎么装」，一个是「插件对象长什么样」——
//!   问题不同，owner 也不同。
//!
//! ## 清单是生成的（DE-023 D-181）
//!
//! 下面三组常量由 `scripts/facts/assembly.mjs` 从各插件目录的 `PLUGIN.yml` 生成到
//! `assembly.generated.rs`，本文件只 `include!` 它。要加一个插件：建目录、写
//! `PLUGIN.yml`、加代码——`assembly.rs` 与 `symbio_core` 都不需要改；手改生成区会被
//! `gen-facts --check` 判红。
//!
//! ## 子 Agent 子树清单的事实与边界
//!
//! 子 Agent 是一棵 composite 插件树（与系统 Agent 同构，agent-directory-spec §1.1），
//! 构造时经 ctx 键 [`symbio_core::REQUIRED_PLUGINS`] 告知容器「必须挂哪些插件」。
//! 系统树那份（`plugins/home` 的 `SYSTEM_PLUGINS`）直接别名到本清单，因此不存在需要
//! 手工同步的第二份字面量。差异**不在清单里**，而在收集期的**作用域**：
//!
//! - `vdfs`（VDFS 根）是**单槽**注册，归系统 Agent 独占。子树**会构造**自己的
//!   `vdfs` 实例（故本清单在列），但它的**注册**经
//!   `plugins/agent/host/scope.rs::SubAgentVisitor` 在每一层丢弃。
//! - `model` **在列**：子智能体有自己的模型服务——子树会话收集能力时以**子容器**
//!   为 parent，子树 `model` 注册进该次收集自己的管理器；父会话收集期，子树的
//!   `model` 注册才经 `SubAgentVisitor` 丢弃（单槽，防子树模型劫持父会话）。
//! - 其余插件（含 `agent` 自身、`plugin_manager`、`memory`）都在列：子树因此与父树
//!   **结构相同**。`memory` 尤其必须分形：子智能体自己的 `AGENTS.md` 由子树里那个
//!   `memory` 实例读写与注入——系统树那份管的是 `{homedir}/AGENTS.md`，两者是**同一个
//!   插件的两个实例**，不是两份实现。
//! - 任意层级复用同一清单：若子 Agent 自己再挂子 Agent，同一清单 + 同一套构造逻辑
//!   自动套用，不存在「支持子 Agent 却不支持子 Agent 的子 Agent」的特例。
//!
//! 若将来两侧确需分叉，**加只属于某一侧的字面量并写清理由**——不要恢复「两张各写
//! 一遍、靠人同步」的形态：两份各自演化的清单会静默漂移。
//!
//! ## 在列 ≠ 不可卸载：`classify` / `compose` 为什么在列
//!
//! 对话面拆成两个插件（`classify` 判决 · `compose` 措辞），两者的**卸载平凡值**是
//! 「全部输入直接进工具循环」「没有对话面文本，worker 正文照旧」——即卸载前的行为。
//! 它们在**本清单**（⇒ 目录自动补出、默认可达）而不是在
//! [`ASSEMBLY_UNDISABLABLE_PLUGINS`]：本清单管的是「一棵标准树默认挂什么」，与
//! 「能不能停用」是两件事。停用是**装配期**动作（不构造 ⇒ 路由 `NotFound` ⇒
//! 消费方按「缺插件」处理）；把可卸载项放进本清单是「有这个插件就拥有这个能力」的
//! 默认态——一个智能体开箱即能与人对话。
//!
//! ## 不可停用的判据
//!
//! 停用一个普通插件（如 `telegram`）少的是一个功能；停用 [`ASSEMBLY_UNDISABLABLE_PLUGINS`]
//! 里的任何一个，少的是**整个界面**：前端所有资源页都经 `vdfs` 取数，插件清单与启停
//! 按钮都长在 `plugin_manager` 的页面上。于是用户会看到一个再也点不到「启用」的界面，
//! 只能去磁盘上改 `PLUGIN.yml`——那不是权限设计，是自断其路。因此它是一条**装配方**的
//! 判据（与清单同处）：装配方（容器）在执行停用前查它，而不是让每个插件自己声明
//! 「我不能被关」。
//!
//! ## 插件工厂 id 的事实与边界
//!
//! **值**与**标识符**分开：值（`"home"`）是跨进程 / 跨文件的字面量——插件目录名、
//! `PLUGIN.yml` 的 `plugin_provider`、前端路由前缀都取它，**不能改**；标识符只服务
//! 编译期，必须让调用点自证归属，故加 `PLUGIN_ID_` 前缀。插件自己的 `PLUGIN.yml` 与
//! 目录名是运行期同一答案；本表是编译期的镜像，由 `plugin-entry-audit` 的 E-001 守着
//! 三者一致。
//!
//! - **LLM 工具名不在这里**：工具名是 `CapabilityMeta.name` 短名，属各插件自己的
//!   实现细节（`agent_run` 在 `plugins/agent/host/subagent.rs`，`vdfs_*` 由
//!   `plugins/vdfs/protocol.rs::VDFS_OPS` 派生），不经全局注册表。
//! - **model 协议 id 不在这里**：协议 id 是插件内部实现细节，定义在
//!   `plugins/model/protocols/mod.rs`；core 既不暴露 `MODEL_PROTOCOL_*` 常量，
//!   也不定义 `ModelProtocol` trait。
//! - **嵌入服务 id 不在这里**：它描述的是嵌入服务而非插件，归 `embedding` 域
//!   （`embedding::ids`）。
//! - **路由地址不在这里**：`<插件目录名>/<子路径>` 是调用路径而非对象身份，
//!   见 `crate::routes`。

pub use symbio_deploy::*;

// 注：`SYSTEM_LEVEL_PROVIDERS`（「系统级插件不参与容器扫描」）不在这里——它只被
// `plugins/composite` 一个模块消费，按 D-023 的依赖方判据已下沉到该模块。
