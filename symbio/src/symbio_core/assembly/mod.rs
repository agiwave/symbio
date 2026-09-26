//! 装配策略 —— 「一棵标准插件树挂哪些插件」「哪些插件不许被停用」。
//!
//! ## 为什么独立成域
//!
//! 这里的常量**不是**上下文键（不走 `SymbioKey`、不进任何 `ctx`），**也不是**
//! 插件 id 或路由地址——它们描述的是**装配方的策略**：一个 composite 子树默认挂
//! 哪些插件、哪些插件属于「界面底座」因此不许停用。
//!
//! 它们此前停在 `keys` 域，只因当初没找到更合适的家。而 `keys` 的职责是「类型安全
//! 上下文键 + 插件 id + 路由地址」（README §2），收下它们会让该域的公开面读不出
//! 主题——「一域一目录，域内私有」的前提是**这个域讲得清自己是什么**。
//!
//! ## 判据：依赖方数量（ADR-023）
//!
//! 两个常量各有**两个**消费方，因此按判据留在 core：
//!
//! | 常量 | 消费方 |
//! |---|---|
//! | [`ASSEMBLY_SUB_AGENT_PLUGINS`] | `plugins/home`（系统树）· `plugins/agent`（子 Agent 树） |
//! | [`ASSEMBLY_UNDISABLABLE_PLUGINS`] | `plugins/composite`（执行停用前查）· `plugins/plugin_manager`（界面置灰） |
//!
//! 只被**一个**模块依赖的两条已按同判据下沉：系统树的插件清单并入 `plugins/home` 的
//! `SYSTEM_PLUGINS`（原本只是本清单的纯别名，无第二份字面量），
//! `SYSTEM_LEVEL_PROVIDERS` → `plugins/composite`。
//!
//! ## 与 `keys::ids` 的分工
//!
//! - `keys::ids`：**单个**对象的注册 id（插件工厂 / 服务）；
//! - 本域：**一组**插件的装配策略。两者都以插件名为字面量，但一个是「这个插件叫什么」，
//!   一个是「这批插件怎么装」——问题不同，owner 也不同。

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
/// - 其余插件（含 `agent` 自身、`plugin_manager`、`work`）都在列：子树因此与父树**结构相同**，
///   前端看到的资源入口（含设置入口）与父 Agent 对齐。
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
pub const ASSEMBLY_SUB_AGENT_PLUGINS: &[&str] = &[
    "plugin_manager", // 插件管理与配置入口（子 Agent 页同样需要）
    "event_bus",      // 事件总线
    "session",        // 会话
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
    "work",           // 工作区记忆（WORKDIR 继承父会话，不再双重注入）
];

/// **不可停用的插件** —— 它们是**界面自身的底座**，不是普通功能。
///
/// 停用一个普通插件（如 `telegram`）少的是一个功能；停用这里的任何一个，少的是
/// **整个界面**：前端所有资源页都经 `vdfs` 取数，插件清单与启停按钮都长在
/// `plugin_manager` 的页面上。于是用户会看到一个再也点不到「启用」的界面，
/// 只能去磁盘上改 `PLUGIN.yml`——那不是权限设计，是自断其路。
///
/// 因此它是一条**机制级**判据（与 [`ASSEMBLY_SUB_AGENT_PLUGINS`] 同处）：装配方（容器）
/// 在执行停用前查它，而不是让每个插件自己声明「我不能被关」。插件的启停状态是
/// 装配方的事（见 [`crate::symbio_core::KEY_ENABLED`]），这条规则也该住在同一处。
pub const ASSEMBLY_UNDISABLABLE_PLUGINS: &[&str] = &[
    crate::symbio_core::PLUGIN_MANAGER, // 插件管理入口：停用它就再也点不到「启用」
    crate::symbio_core::PLUGIN_VDFS,    // 资源访问层：停用它整棵资源树都取不到
];
