//! 装配策略 —— 「一棵标准插件树挂哪些插件」「哪些插件不许被停用」。
//!
//! ## 为什么不在 `symbio_core`
//!
//! 这里的常量**不是**上下文键（不走 `SymbioKey`、不进任何 `ctx`），**也不是**
//! 路由地址——它们描述的是**装配方的事实**：一棵子树默认挂哪些插件、哪些插件属于
//! 「界面底座」因此不许停用、每个插件的出厂 id 叫什么。
//!
//! 按依赖方判据（[D-023](../../docs/003-design/001-核心架构.md "DE-001-核心架构")）它们
//! 本来「够格」留在 core（各有多个消费方）。但那条判据回答的是「两个互不相认的插件
//! 要不要共享一处**机制**」；这里要共享的不是机制，而是**宿主决定接哪些插件**。
//! 装配是宿主的事：插件清单、启停状态、出厂 id 与装配结果归装配方持有，容器只经
//! [`crate::symbio_core::REQUIRED_PLUGINS`] 接收结果（见 `plugins/composite` 的 README）。
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

/// 子 Agent 子树挂载的「默认插件」清单 —— 与父（系统）Agent **同构**的机制级常量，
/// 也是全仓插件清单的**唯一字面量**。
///
/// 子 Agent 是一棵 composite 插件树（与系统 Agent 同构，agent-directory-spec §1.1），
/// 构造时经 ctx 键 [`crate::symbio_core::REQUIRED_PLUGINS`] 告知容器「必须挂哪些插件」。
///
/// ## 与系统树清单的关系：**当前逐项相同**
///
/// 系统树那份（`plugins/home` 的 `SYSTEM_PLUGINS`）现在直接别名到本常量，
/// 因此不存在需要手工同步的第二份字面量。差异**不在清单里**，而在收集期的**作用域**：
///
/// - `vdfs`（VDFS 根）是**单槽**注册，归系统 Agent 独占。子树**会构造**自己的
///   `vdfs` 实例（故本清单在列），但它的**注册**经
///   `plugins/agent/host/scope.rs::SubAgentVisitor` 在每一层丢弃
///   （见其模块文档）——单槽归系统 Agent，子树重复注册不会生效。
/// - `model` **在列**：子智能体有自己的模型服务——子树会话收集能力时以**子容器**
///   为 parent（`collect_capabilities(sub_composite, …)`），子树 `model` 实例
///   注册进**该次收集自己的**管理器，因此子会话用子智能体自己解析的模型。
///   父（系统）会话收集期，子树的 `model` 注册才经 `SubAgentVisitor` **丢弃**
///   （单槽，防子树模型劫持父会话——见 scope 模块文档）。
/// - 其余插件（含 `agent` 自身、`plugin_manager`、`work`、`memory`）都在列：子树因此与
///   父树**结构相同**，前端看到的资源入口（含设置入口）与父 Agent 对齐。
///   `memory` 尤其必须分形：子智能体自己的 `AGENTS.md` 就躺在它的目录里，由子树里
///   那个 `memory` 实例读写与注入——系统树那份管的是 `{homedir}/AGENTS.md`，
///   两者是**同一个插件的两个实例**，不是两份实现。
///
/// 若将来两侧确需分叉，**加只属于某一侧的字面量并写清理由**——不要恢复
/// 「两张各写一遍、靠人同步」的形态：两份各自演化的清单会静默漂移（可以变成同一集合，
/// 而各处注释仍在描述差异，没有任何测试会因此变红）。
///
/// ## 分形：任意层级复用同一常量
///
/// 本常量被 `plugins/agent` 的 `sub_agent` 用于构造**每一棵**子树。若某天子 Agent
/// 也能在其目录内再挂子 Agent（`<id>/agent/<sub-id>` 递归），同一常量 + 同一套构造
/// 逻辑自动套用——不存在「支持子 Agent 却不支持子 Agent 的子 Agent」的特例：任何一层
/// 都走同一条机制，且都同样只跳过 `vdfs` 单槽（单槽归系统 Agent，由
/// `SubAgentVisitor` 在每一层丢弃）。
///
/// ## 在列 ≠ 不可卸载：`classify` / `compose` 为什么在列
///
/// 对话面拆成两个插件（`classify` 判决 · `compose` 措辞），两者的**卸载平凡值**是
/// 「全部输入直接进工具循环」「没有对话面文本，worker 正文照旧」——即卸载前的行为。
///
/// 它们在**本清单**（⇒ 目录自动补出、默认可达）而不是在
/// [`ASSEMBLY_UNDISABLABLE_PLUGINS`]：本清单管的是「一棵标准树默认挂什么」，
/// 与「能不能停用」是两件事。停用是**装配期**动作（不构造 ⇒ 路由 `NotFound` ⇒
/// 消费方按「缺插件」处理），因此它给出的正是「没有这个插件」的状态；
/// 而「可卸载」的判据要的正是后者，不是运行期读一个 `enabled` 字段。
///
/// 放进本清单是「有这个插件就拥有这个能力」的默认态：一个智能体开箱即能与人对话。
/// 若不在列，新装 / 迁移来的智能体默认没有对话面——那会让「默认」与「目标架构」分叉。
pub const ASSEMBLY_SUB_AGENT_PLUGINS: &[&str] = &[
    "plugin_manager", // 插件管理与配置入口（子 Agent 页同样需要）
    "event_bus",      // 事件总线
    "session",        // 会话
    "loop",           // 驱动器：一轮的判决 / 工具循环（经地址面被会话触发，随每棵子树分形）
    "heartbeat",      // 心跳调度：会话空闲后自主开一轮（经地址面写会话收件箱，随每棵子树分形）
    "classify",       // 对话面·判决（无它 ⇒ 全部输入直接进工具循环）
    "compose",        // 对话面·措辞（无它 ⇒ 无对话面文本，worker 正文照旧）
    "model",          // 模型服务（子智能体自己的模型；子树会话自行解析）
    "local",          // 本地文件
    "web",            // 网络访问
    "mcp",            // 工具
    "telegram",       // 消息渠道
    "hook",           // 钩子
    "agent",          // 智能体（含子子 Agent —— 分形）
    "skill",          // 技能
    "gateway",        // 外部 API 网关
    "vdfs",           // VDFS 根（单槽归系统 Agent）
    "memory", // 记忆：智能体自身（分形）+ 工作区（WORKDIR 继承父会话），两作用域同属一个所有者
    "setting", // 智能体自身的信息设置（分形：系统树与每棵子树各有一份）
];

/// **不可停用的插件** —— 它们是**界面自身的底座**，不是普通功能。
///
/// 停用一个普通插件（如 `telegram`）少的是一个功能；停用这里的任何一个，少的是
/// **整个界面**：前端所有资源页都经 `vdfs` 取数，插件清单与启停按钮都长在
/// `plugin_manager` 的页面上。于是用户会看到一个再也点不到「启用」的界面，
/// 只能去磁盘上改 `PLUGIN.yml`——那不是权限设计，是自断其路。
///
/// 因此它是一条**装配方**的判据（与 [`ASSEMBLY_SUB_AGENT_PLUGINS`] 同处）：装配方（容器）
/// 在执行停用前查它，而不是让每个插件自己声明「我不能被关」。插件的启停状态是
/// 装配方的事（见 [`crate::symbio_core::PLUGIN_KEY_ENABLED`]），这条规则也该住在同一处。
pub const ASSEMBLY_UNDISABLABLE_PLUGINS: &[&str] = &[
    PLUGIN_ID_MANAGER, // 插件管理入口：停用它就再也点不到「启用」
    PLUGIN_ID_VDFS,    // 资源访问层：停用它整棵资源树都取不到
];

// ============ 插件工厂 id（部署身份） ============

/// 插件工厂 id —— 全局对象注册表里的**插件身份**。
///
/// 所有经 `submit_object_creator!` 注册的插件工厂，其 id 字符串在此统一定义为
/// `&'static str`。收益：
///
/// - **单一真相源**：注册侧（宏的第一参）、装配侧（`PluginMeta::new(id, …)`）与
///   使用侧（`creator_create_object(id, …)`、测试期望）共享同一常量；
/// - **避免拼写漂移**：漏改 / 错改一处即被 `cargo check` 拦下；
/// - **IDE 友好**：跳转即可看到完整 id 列表。
///
/// ## 命名：`PLUGIN_ID_<工厂名>`
///
/// **值**与**标识符**分开：值（`"home"`）是跨进程 / 跨文件的字面量——插件目录名、
/// `PLUGIN.yml` 的 `plugin_provider`、前端路由前缀都取它，**不能改**；标识符只服务
/// 编译期，必须让调用点自证归属，故加 `PLUGIN_ID_` 前缀。
///
/// ## 为什么住装配方而不是 core
///
/// 「一个插件叫什么」是**部署事实**，与「一棵树装哪些插件」同源：两者都由装配方决定，
/// 内核不认识任何具体插件名。插件自己的 `PLUGIN.yml` 与目录名是运行期同一答案；
/// 本表是编译期的镜像，由 `plugin-entry-audit` 的 E-001 守着三者一致。
///
/// ## 边界
///
/// - **LLM 工具名不在这里**：工具名是 `CapabilityMeta.name` 短名，属各插件自己的
///   实现细节（`agent_run` 在 `plugins/agent/host/subagent.rs`，`vdfs_*` 由
///   `plugins/vdfs/protocol.rs::VDFS_OPS` 派生），不经全局注册表。
/// - **model 协议 id 不在这里**：协议 id 是插件内部实现细节，定义在
///   `plugins/model/protocols/mod.rs`；core 既不暴露 `MODEL_PROTOCOL_*` 常量，
///   也不定义 `ModelProtocol` trait。
/// - **嵌入服务 id 不在这里**：它描述的是嵌入服务而非插件，归 `embedding` 域
///   （`embedding::ids`）。
/// - **路由地址不在这里**：`<插件目录名>/<子路径>` 是调用路径而非对象身份，
///   见 `plugin::route`。
///
/// Home 插件工厂
pub const PLUGIN_ID_HOME: &str = "home";
/// Model 插件工厂
pub const PLUGIN_ID_MODEL: &str = "model";
/// Agent 插件工厂
pub const PLUGIN_ID_AGENT: &str = "agent";
/// Composite 插件工厂
pub const PLUGIN_ID_COMPOSITE: &str = "composite";
/// Web 插件工厂
pub const PLUGIN_ID_WEB: &str = "web";
/// Telegram 插件工厂
pub const PLUGIN_ID_TELEGRAM: &str = "telegram";
/// Skill 插件工厂
pub const PLUGIN_ID_SKILL: &str = "skill";
/// 插件管理插件工厂（智能体全部插件的管理与配置入口）
pub const PLUGIN_ID_MANAGER: &str = "plugin_manager";
/// Gateway 插件工厂（HTTP / WebSocket 入站网关）
pub const PLUGIN_ID_GATEWAY: &str = "gateway";
/// Session 插件工厂
pub const PLUGIN_ID_SESSION: &str = "session";
/// Heartbeat 插件工厂（会话空闲心跳：调度循环 + 设置入口）
pub const PLUGIN_ID_HEARTBEAT: &str = "heartbeat";
/// Loop 插件工厂（驱动器：一轮的判决 / 工具循环 / 落库视图，经地址面被会话触发）
pub const PLUGIN_ID_LOOP: &str = "loop";
/// Classify 插件工厂（对话面：判决这一轮该直接答、还是派给工具循环）
pub const PLUGIN_ID_CLASSIFY: &str = "classify";
/// Compose 插件工厂（对话面：措辞——首响 / 答话 / 汇报）
pub const PLUGIN_ID_COMPOSE: &str = "compose";
/// MCP 插件工厂
pub const PLUGIN_ID_MCP: &str = "mcp";
/// Local 插件工厂
pub const PLUGIN_ID_LOCAL: &str = "local";
/// Memory 插件工厂（记忆：智能体自身 + 工作区，两个作用域）
pub const PLUGIN_ID_MEMORY: &str = "memory";
/// Setting 插件工厂（智能体自身的信息设置）
pub const PLUGIN_ID_SETTING: &str = "setting";
/// Hook 插件工厂
pub const PLUGIN_ID_HOOK: &str = "hook";
/// VDFS 插件工厂（虚拟文件系统宿主）
pub const PLUGIN_ID_VDFS: &str = "vdfs";
/// Event Bus 插件工厂（统一事件总线）
pub const PLUGIN_ID_EVENT_BUS: &str = "event_bus";

// 注：`SYSTEM_LEVEL_PROVIDERS`（「系统级插件不参与容器扫描」）不在这里——它只被
// `plugins/composite` 一个模块消费，按 D-023 的依赖方判据已下沉到该模块。
