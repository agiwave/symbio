//! 全项目调用路径（route path）统一常量
//!
//! 设计原则：所有通过 `ctx.set(PATH, "<plugin>/<cap>")` 注入的路由路径，
//! 或在 plugin 内部 `match path.as_str() { "<plugin>/<cap>" => ... }` 中使用的字符串，
//! 必须在本模块中以 `&'static str` 常量定义并复用。
//!
//! 与 [`ids`] 模块的差别：
//! - `ids` 描述"注册到注册表的对象 id"（插件工厂、capability、协议）
//! - 本模块描述"运行期调用的路由路径"（`plugin/operation`）
//!
//! 收益：
//! - **单一真相源**：注册侧（plugin handler 的 `match` 分支）和调用侧（`ctx.set(PATH, ...)`）一致
//! - **编译期检查**：拼写漂移 / 漏改 / 错改立刻被 `cargo check` 拦截
//! - **IDE 友好**：跳转即可看到所有可用路径
//!
//! 命名约定：`<PLUGIN>_<OPERATION>` 形式，全部大写下划线

// ============ Config 插件 ============
/// config/get — 读取全局配置项
pub const CONFIG_GET: &str = "config/get";
/// config/set — 写入全局配置项
pub const CONFIG_SET: &str = "config/set";

// ============ Session 插件 ============
/// session/open — 创建/打开会话
pub const SESSION_OPEN: &str = "session/open";
/// session/chat — 会话对话（流式）
pub const SESSION_CHAT: &str = "session/chat";
/// session/compress — 压缩会话历史
pub const SESSION_COMPRESS: &str = "session/compress";

// ============ Agent 插件 ============
/// agent/chat — **子智能体会话执行入口**（仅 agent_run 能力内部调用）
///
/// 重构说明：顶层会话已不再经过此路径。前端会话统一走
/// `SESSION_CHAT_SEND`（session 插件编排），agent 插件只通过
/// `traverse(available_tools)` 向会话贡献工具；只有当上级会话需要派生一个
/// 子智能体会话时，才会由此路径进入 agent 插件内部执行。
pub const AGENT_CHAT: &str = "agent/chat";
/// agent/create — 创建新 Agent
pub const AGENT_CREATE: &str = "agent/create";
/// agent/get — 查询 Agent 详情
pub const AGENT_GET: &str = "agent/get";
/// agent/list — 列出所有 Agent
pub const AGENT_LIST: &str = "agent/list";
/// agent/delete — 删除 Agent
pub const AGENT_DELETE: &str = "agent/delete";

// ============ Model 插件 ============
/// model/chat — 直接调 Model 协议
pub const MODEL_CHAT: &str = "model/chat";

// ============ Hook 插件 ============
/// hook/fire — 触发命名 hook
pub const HOOK_FIRE: &str = "hook/fire";
