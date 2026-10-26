// 编译期装配清单 —— **由 `scripts/facts/assembly.mjs` 生成，请勿手改**。
//
// 事实源：`crates/symbio-plugin-<id>/PLUGIN.yml`。加插件 = 建目录 + 写声明 + 代码，
// 本文件与 `assembly.rs` 都不需要人改；漂移由 `gen-facts --check` 判红。
// dead-code-allow R-003: 插件 id / 装配清单是部署契约面，未用常量仍须存在（owner: scripts/facts/assembly.mjs 生成，每个插件目录的 PLUGIN.yml 是唯一声明处）

// ============ 插件工厂 id（部署身份） ============

/// 智能体（含子 Agent —— 分形）
pub const PLUGIN_ID_AGENT: &str = "agent";

/// 对话面·判决（无它 ⇒ 全部输入直接进工具循环）
pub const PLUGIN_ID_CLASSIFY: &str = "classify";

/// 对话面·措辞（无它 ⇒ 无对话面文本，worker 正文照旧）
pub const PLUGIN_ID_COMPOSE: &str = "compose";

/// 执行容器
pub const PLUGIN_ID_COMPOSITE: &str = "composite";

/// 事件总线
pub const PLUGIN_ID_EVENT_BUS: &str = "event_bus";

/// 外部 API 网关
pub const PLUGIN_ID_GATEWAY: &str = "gateway";

/// 心跳调度：会话空闲后自主开一轮（经地址面写会话收件箱，随每棵子树分形）
pub const PLUGIN_ID_HEARTBEAT: &str = "heartbeat";

/// 系统根
pub const PLUGIN_ID_HOME: &str = "home";

/// 钩子
pub const PLUGIN_ID_HOOK: &str = "hook";

/// 本地文件
pub const PLUGIN_ID_LOCAL: &str = "local";

/// 驱动器：一轮的判决 / 工具循环（经地址面被会话触发，随每棵子树分形）
pub const PLUGIN_ID_LOOP: &str = "loop";

/// 工具
pub const PLUGIN_ID_MCP: &str = "mcp";

/// 记忆：智能体自身（分形）+ 工作区（WORKDIR 继承父会话），两作用域同属一个所有者
pub const PLUGIN_ID_MEMORY: &str = "memory";

/// 模型服务（子智能体自己的模型；子树会话自行解析）
pub const PLUGIN_ID_MODEL: &str = "model";

/// 插件管理与配置入口
pub const PLUGIN_ID_MANAGER: &str = "plugin_manager";

/// 会话
pub const PLUGIN_ID_SESSION: &str = "session";

/// 智能体自身的信息设置（分形：系统树与每棵子树各有一份）
pub const PLUGIN_ID_SETTING: &str = "setting";

/// 技能
pub const PLUGIN_ID_SKILL: &str = "skill";

/// 消息渠道
pub const PLUGIN_ID_TELEGRAM: &str = "telegram";

/// VDFS 根（单槽归系统 Agent）
pub const PLUGIN_ID_VDFS: &str = "vdfs";

/// 网络访问
pub const PLUGIN_ID_WEB: &str = "web";

/// 子 Agent 子树默认挂载的插件清单（按 `plugin_order` 排序）。
pub const ASSEMBLY_SUB_AGENT_PLUGINS: &[&str] = &[
    "plugin_manager", // 插件管理与配置入口
    "event_bus", // 事件总线
    "session", // 会话
    "loop", // 驱动器：一轮的判决 / 工具循环（经地址面被会话触发，随每棵子树分形）
    "heartbeat", // 心跳调度：会话空闲后自主开一轮（经地址面写会话收件箱，随每棵子树分形）
    "classify", // 对话面·判决（无它 ⇒ 全部输入直接进工具循环）
    "compose", // 对话面·措辞（无它 ⇒ 无对话面文本，worker 正文照旧）
    "model", // 模型服务（子智能体自己的模型；子树会话自行解析）
    "local", // 本地文件
    "web", // 网络访问
    "mcp", // 工具
    "telegram", // 消息渠道
    "hook", // 钩子
    "agent", // 智能体（含子 Agent —— 分形）
    "skill", // 技能
    "gateway", // 外部 API 网关
    "vdfs", // VDFS 根（单槽归系统 Agent）
    "memory", // 记忆：智能体自身（分形）+ 工作区（WORKDIR 继承父会话），两作用域同属一个所有者
    "setting", // 智能体自身的信息设置（分形：系统树与每棵子树各有一份）
];

/// **不可停用的插件**——界面自身的底座（停用其中一个，少的是整个界面）。
pub const ASSEMBLY_UNDISABLABLE_PLUGINS: &[&str] = &[
    "plugin_manager", // 插件管理与配置入口
    "vdfs", // VDFS 根（单槽归系统 Agent）
];
