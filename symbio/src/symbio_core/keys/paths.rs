//! 全项目调用路径（route path）统一常量
//!
//! # 地址规则
//!
//! > 本模块文档是地址规则的**权威表述**；展开版（含审计结果与守卫边界）见
//! > [`docs/design/plugin-route-address.md`](../../../docs/design/plugin-route-address.md)。
//!
//! ## 一、地址只有两种形态
//!
//! | 形态 | 形状 | 谁写它 | 谁读它 |
//! |---|---|---|---|
//! | **绝对地址** | `<插件目录名>/<子路径>` | 调用方（`ctx.set(PATH, …)` 后过 `route`） | 容器剥首段 → 目标插件的 `match` |
//! | **相对臂** | `<子路径>` | 目标插件自己的 `route` 体内 `match` | 容器剥完首段后剩下的那截 |
//!
//! 两者的关系是**一次剥离**：容器按首段找到子插件，把**余下部分**塞回 `PATH` 再转发。
//! 所以插件里的 `match` 永远只写相对臂，跨插件调用永远写绝对地址——**不要**在插件内
//! 匹配绝对地址，也**不要**在调用侧写相对臂。
//!
//! ## 二、前缀是**插件目录名**，不是 `PluginMeta`
//!
//! 容器（`composite`）扫描插件根下的**一层目录**建实例表，键就是目录名
//! （`composite.rs` 注释：「目录名 = 实例名」），`route` 也按它分发。因此目录名
//! 才是真正的路由前缀。`PluginMeta::new` 的首参（`id`）**不参与路由**——路由按目录名
//! 分发，`PluginMeta` 唯一运行期消费方是 `composite/vdfs.rs`（读 `order`/`root_access`/
//! `name`/`description`/`hidden` 聚合组合视图），拿 id 当前缀会产出**不存在的路由**——
//! `hook` 插件写成 `"hooks"` 就曾让 `docs/CURRENT.md` 与两处文档照抄出 `hooks/fire`。
//! 该不一致由 `scripts/plugin-entry-audit.mjs` 的 E-001 守住。
//!
//! ## 三、过路由才设 `PATH`；直连方法**不设**
//!
//! `PATH` 是给**容器**看的。调用方如果拿到的是插件实例并直连方法
//! （`self.handle_xxx(…)`），`PATH` 不会被任何人读——设了就是死赋值。
//! 判据很简单：**下一跳是 `route()` 还是方法调用**。
//!
//! ## 四、`traverse` 的 `PATH` 只有两个合法值
//!
//! [`TRAVERSE_AVAILABLE_TOOLS`](crate::symbio_core::TRAVERSE_AVAILABLE_TOOLS) 与
//! [`TRAVERSE_AVAILABLE_OPTIONS`](crate::symbio_core::TRAVERSE_AVAILABLE_OPTIONS)。
//! 它们是**协议端点**，不是插件路径，因此不进本模块。
//!
//! # 本模块的职责与边界
//!
//! 本模块收**有真实调用方的绝对地址常量**（`&'static str`）。收益：
//! - **单一真相源**：注册侧（插件的 `match` 臂）与调用侧（`ctx.set(PATH, …)`）一致；
//! - **编译期检查**：拼写漂移 / 漏改 / 错改立刻被 `cargo check` 拦截；
//! - **IDE 友好**：跳转即可看到所有可用路径。
//!
//! **不为「将来可能用到」的路由预置常量**——那正是本模块 2026-09-18 清掉的那类腐烂
//! （见 [`HOOK_FIRE`] 上方关于 `AGENT_CHAT` 的说明）。当前无调用方的路由由审计脚本
//! 报告，由人决定去留，而不是先给它们一个体面的常量名。
//!
//! 与 [`ids`](crate::symbio_core::keys::ids) 的差别：
//! - `ids` 描述「注册到注册表的对象 id」（插件工厂、capability、协议）；
//! - 本模块描述「运行期跨插件调用的路由路径」（`<插件目录名>/<子路径>`）。
//!
//! 命名约定：`<PLUGIN>_<OPERATION>` 形式，全部大写下划线。

// ============ Session 插件 ============
/// session/chat/send — 发起一轮对话（**统一编排入口**）
///
/// 这是全仓**唯一**的会话发言入口。子智能体派生（`agent/host/subagent.rs`）、
/// 心跳（`session/heartbeat.rs`，直连）、Telegram 通道都汇到这里。
///
/// 注意不是 `session/chat`：那个路径**不存在**（session 的 `route` 只认
/// `chat/send` 与 `chat/abort` 两条相对臂）。Telegram 曾用它，见下方 [`HOOK_FIRE`]
/// 同类的记录。
pub const SESSION_CHAT_SEND: &str = "session/chat/send";

/// session/chat/abort — 中止进行中的一轮
///
/// **唯一调用方不在 Rust 侧**：前端 `tauri/src/constants/pluginPaths.ts::CHAT_ABORT`
/// （由 `${SESSION_PATH}/chat/abort` 拼出，前缀收敛为 `worker/`；`worker` 可省略，
/// 故与本常量是同一路由的两种合法写法）。Rust 侧因此没有任何代码引用它，但路径
/// **真实存在**（session 的 `route` 认 `chat/abort` 臂）——与 `AGENT_CHAT` 那类
/// 「描述了一条不存在的路由」的幽灵常量不同，故保留。
///
/// 保留的代价为零（一个 `&'static str`），收益是「前端认识的后端路由」在后端也有
/// 一条可检索的登记。这条理由由 `#[allow(dead_code)]` 同行注明，供
/// `scripts/dead-code-audit.mjs` 识别为**刻意保留**而非漏删。
#[allow(dead_code)] // dead-code-allow R-001: 唯一调用方在前端 pluginPaths.ts::CHAT_ABORT，路由真实存在
pub const SESSION_CHAT_ABORT: &str = "session/chat/abort";

// ============ VDFS 插件 ============
/// vdfs/root — **进入地址空间**：取根地址，调用方不给地址。
///
/// 调用方：Rust 侧 `agent/host/subagent.rs`（拼 Run 的 VDFS 根地址）、前端
/// `tauri/src/schemas/vdfs.ts::VDFS_ROOT`（启动期取根当运行期数据，
/// `services/vdfsScheme.ts` 据此拼会话地址）。保留登记的理由与 [`SESSION_CHAT_ABORT`] 相同
/// ——「前端认识的后端路由」在后端也应有一条可检索的常量；且**根名只归 vdfs 插件**
/// （`plugins/vdfs/fs.rs::VDFS_ADDR_ROOT`，仓级守卫 S-010 禁止它在别处出现），
/// 故消费方一律取运行期值、不写字面量。
#[allow(dead_code)] // dead-code-allow R-001: 调用方在前端 schemas/vdfs.ts + services/vdfsScheme.ts，路由真实存在
pub const VDFS_ROOT: &str = "vdfs/root";

/// vdfs/watch — 订阅一棵地址子树的变更。
///
/// 调用方：`agent/host/subagent.rs`（Run 转播登记）与前端
/// `tauri/src/schemas/vdfs.ts::VDFS_WATCH` + `services/vdfs.ts`（本轮渲染）。
/// 后端只向**登记过路径**的订阅者投递变更（`core/vdfs/host::ChangeSubscriptions`），
/// 因此这是「能收到 VDFS 变更」的前置条件：只订阅全局总线而不登记 watch，
/// 等于在一条没人开闸的频道上等事件（一条也收不到）。
#[allow(dead_code)] // dead-code-allow R-001: 调用方在前端 schemas/vdfs.ts + services/vdfs.ts，路由真实存在
pub const VDFS_WATCH: &str = "vdfs/watch";

/// vdfs/unwatch — 取消订阅（与 [`VDFS_WATCH`] 严格配对）。
///
/// 引用计数归零才真正摘除，多余一次 `unwatch` 是安全的空操作；
/// 但**漏掉**它会留下幽灵订阅（后端持续投递、消费者早已不在）。
#[allow(dead_code)] // dead-code-allow R-001: 调用方在前端 schemas/vdfs.ts + services/vdfs.ts，路由真实存在
pub const VDFS_UNWATCH: &str = "vdfs/unwatch";

// ============ Event Bus 插件 ============
/// event_bus/subscribe — 建立进程内帧订阅连接
///
/// 调用方有两处：Rust 侧 `cli/src/client.rs`（订阅 `vdfs` 频道），以及前端
/// `tauri/src/constants/pluginPaths.ts::EVENT_BUS_SUBSCRIBE` + `services/eventBus.ts`
/// ——两边都拿 `PluginFrame::Data` 收会话帧。
pub const EVENT_BUS_SUBSCRIBE: &str = "event_bus/subscribe";

// ============ Hook 插件 ============
/// hook/fire — 触发命名 hook
///
/// 前缀是**目录名 `hook`**（不是 `PluginMeta` 曾写的 `"hooks"`）。
/// 唯一调用方 `session/tool_executor.rs::fire_hook`（PreToolUse / Stop 等生命周期点）。
///
/// ---
///
/// **被本模块删掉的两个常量（2026-09-18）**：`AGENT_CHAT = "agent/chat"` 与
/// `AGENT_CREATE = "agent/create"`。它们全仓**零调用方**，而且描述的路径**根本不存在**
/// ——`agent` 插件的 `route` 恒返回 `NotFound`（agent 目录一律经 VDFS 访问），
/// 子智能体派生走的是 [`SESSION_CHAT_SEND`]。
///
/// 留着它们比没有更糟：`AGENT_CHAT` 的文档曾写着「子智能体会话执行入口
/// （仅 agent_run 能力内部调用）」，而**没有任何代码那样调用**——这正是本模块
/// 存在的理由（防止路径漂移）被反过来利用的样子。`paths.rs` 只收有真实调用方的路径。
pub const HOOK_FIRE: &str = "hook/fire";
