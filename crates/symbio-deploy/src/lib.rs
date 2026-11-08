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

/// 上下文压缩：超水位时把本轮之前的上下文压成一份快照（落 session.consolidated）
pub const PLUGIN_ID_COMPACT: &str = "compact";

/// 对话面·措辞（无它 ⇒ 无对话面文本，worker 正文照旧）
pub const PLUGIN_ID_COMPOSE: &str = "compose";

/// 执行容器
pub const PLUGIN_ID_COMPOSITE: &str = "composite";

/// 目标自生成：把「欲」的表达折成意图视图，并把升格做成自申报的债
pub const PLUGIN_ID_CONATION: &str = "conation";

/// 成本台账：把事件自带的实测耗时按主体折成一本书（熔断与出口共用的那一个口径）
pub const PLUGIN_ID_COST: &str = "cost";

/// 事件总线
pub const PLUGIN_ID_EVENT_BUS: &str = "event_bus";

/// 未来预测：把任务事实折成「还剩多少没完成」的读数视图（名册 #34）
pub const PLUGIN_ID_FORECAST: &str = "forecast";

/// 外部 API 网关
pub const PLUGIN_ID_GATEWAY: &str = "gateway";

/// 项目术语表：一个工具、一条领域事实、一条效果行为（含重复定义的前置判断）、一个上下文段
pub const PLUGIN_ID_GLOSSARY: &str = "glossary";

/// 心跳调度：会话空闲后自主开一轮（经地址面写会话收件箱，随每棵子树分形）
pub const PLUGIN_ID_HEARTBEAT: &str = "heartbeat";

/// 系统根
pub const PLUGIN_ID_HOME: &str = "home";

/// 钩子
pub const PLUGIN_ID_HOOK: &str = "hook";

/// 本地文件
pub const PLUGIN_ID_LOCAL: &str = "local";

/// 纯机制环：选债、派发、等待、配对、记账、预算与租约、重放
pub const PLUGIN_ID_LOOP: &str = "loop";

/// 工具
pub const PLUGIN_ID_MCP: &str = "mcp";

/// 记忆：智能体自身（分形）+ 工作区（WORKDIR 继承父会话），两作用域同属一个所有者
pub const PLUGIN_ID_MEMORY: &str = "memory";

/// 模型服务（子智能体自己的模型；子树会话自行解析）
pub const PLUGIN_ID_MODEL: &str = "model";

/// 人格：一处定义「我是谁」，两条输出面（轮内正文 / 对话措辞）读同一段
pub const PLUGIN_ID_PERSONA: &str = "persona";

/// 插件管理与配置入口
pub const PLUGIN_ID_MANAGER: &str = "plugin_manager";

/// 验收探针：一个世界端口 + 一笔债 + 一个执行体（债按会话前缀惰性）
pub const PLUGIN_ID_PROBE: &str = "probe";

/// 声誉台账：把承诺三格折成各主体的守约与违约账（淘汰裁决与出口共用的那一个口径）
pub const PLUGIN_ID_REPUTATION: &str = "reputation";

/// 会话
pub const PLUGIN_ID_SESSION: &str = "session";

/// 智能体自身的信息设置（分形：系统树与每棵子树各有一份）
pub const PLUGIN_ID_SETTING: &str = "setting";

/// 技能
pub const PLUGIN_ID_SKILL: &str = "skill";

/// 时延分层：按派发档折叠时延 / 兜底 / 跨度三列（单片读数 + 跨会话合并）
pub const PLUGIN_ID_SLO: &str = "slo";

/// 任务表：清单声明成事实、独立验证与返工、就绪集调度与调度段
pub const PLUGIN_ID_TASK: &str = "task";

/// 消息渠道
pub const PLUGIN_ID_TELEGRAM: &str = "telegram";

/// 工具世界（执行 / 路由 / 结果提取）
pub const PLUGIN_ID_TOOL: &str = "tool";

/// VDFS 根（单槽归系统 Agent）
pub const PLUGIN_ID_VDFS: &str = "vdfs";

/// 网络访问
pub const PLUGIN_ID_WEB: &str = "web";

/// 子 Agent 子树默认挂载的插件清单（按 `plugin_order` 排序）。
#[rustfmt::skip]
pub const ASSEMBLY_SUB_AGENT_PLUGINS: &[&str] = &[
    "plugin_manager", // 插件管理与配置入口
    "event_bus", // 事件总线
    "session", // 会话
    "loop", // 纯机制环：选债、派发、等待、配对、记账、预算与租约、重放
    "task", // 任务表：清单声明成事实、独立验证与返工、就绪集调度与调度段
    "heartbeat", // 心跳调度：会话空闲后自主开一轮（经地址面写会话收件箱，随每棵子树分形）
    "persona", // 人格：一处定义「我是谁」，两条输出面（轮内正文 / 对话措辞）读同一段
    "conation", // 目标自生成：把「欲」的表达折成意图视图，并把升格做成自申报的债
    "slo", // 时延分层：按派发档折叠时延 / 兜底 / 跨度三列（单片读数 + 跨会话合并）
    "cost", // 成本台账：把事件自带的实测耗时按主体折成一本书（熔断与出口共用的那一个口径）
    "classify", // 对话面·判决（无它 ⇒ 全部输入直接进工具循环）
    "reputation", // 声誉台账：把承诺三格折成各主体的守约与违约账（淘汰裁决与出口共用的那一个口径）
    "forecast", // 未来预测：把任务事实折成「还剩多少没完成」的读数视图（名册 #34）
    "compose", // 对话面·措辞（无它 ⇒ 无对话面文本，worker 正文照旧）
    "model", // 模型服务（子智能体自己的模型；子树会话自行解析）
    "compact", // 上下文压缩：超水位时把本轮之前的上下文压成一份快照（落 session.consolidated）
    "local", // 本地文件
    "tool", // 工具世界（执行 / 路由 / 结果提取）
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
    "probe", // 验收探针：一个世界端口 + 一笔债 + 一个执行体（债按会话前缀惰性）
    "glossary", // 项目术语表：一个工具、一条领域事实、一条效果行为（含重复定义的前置判断）、一个上下文段
];

/// **不可停用的插件**——界面自身的底座（停用其中一个，少的是整个界面）。
#[rustfmt::skip]
pub const ASSEMBLY_UNDISABLABLE_PLUGINS: &[&str] = &[
    "plugin_manager", // 插件管理与配置入口
    "vdfs", // VDFS 根（单槽归系统 Agent）
];
