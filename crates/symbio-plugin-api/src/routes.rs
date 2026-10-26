//! 跨插件调用路径常量 —— **由 `scripts/facts/routes-rs.mjs` 生成，请勿手改**。
//!
//! 事实源：各插件自己目录里的 `PLUGIN.yml` 的 `plugin_routes:` 行（相对臂）。
//! 生成器按目录名补前缀、与该插件 `route()` 的真实臂（或 `VDFS_OPS`）核对后
//! 产出常量。加一条跨插件地址 = 改**自己目录**里的清单；手改本文件会被
//! `gen-facts --check` 判红。
//!
//! # 地址规则（展开见 `docs/003-design/006-插件路由与地址.md`）
//!
//! ## 一、地址只有两种形态
//!
//! | 形态 | 形状 | 谁写它 | 谁读它 |
//! |---|---|---|---|
//! | **绝对地址** | `<插件目录名>/<子路径>` | 调用方（`ctx.set(PATH, …)` 后过 `route`） | 容器剥首段 → 目标插件的 `match` |
//! | **相对臂** | `<子路径>` | 目标插件自己的 `route` 体内 `match` | 容器剥完首段后剩下的那截 |
//!
//! 插件里的 `match` 只写相对臂，跨插件调用只写绝对地址——不要在插件内匹配绝对
//! 地址，也不要在调用侧写相对臂。
//!
//! ## 二、前缀是**插件目录名**，不是 `PluginMeta`
//!
//! 容器按目录名建实例表并分发（「目录名 = 实例名」），所以目录名才是路由前缀；
//! `PluginMeta::new` 的首参不参与路由（E-001 钉住两者相等）。
//!
//! ## 三、过路由才设 `PATH`；直连方法**不设**
//!
//! `PATH` 是给**容器**看的。判据：下一跳是 `route()` 还是方法调用。
//!
//! ## 四、本模块只收**有真实 Rust 调用方**的地址
//!
//! 前端要用的地址是 `route()` 臂的生成物（`tauri/src/constants/routes.gen.ts`），
//! 不在本模块登记。插件只声明自己要导出给 Rust 调用方的那几条，不为「将来可能
//! 用到」预置常量。
//!
//! # 命名
//!
//! 标识符去掉 `ROUTE_` 前缀后，必须**逐字等于**值的大写形式（`/` → `_`）。
//! 这条与「值必须是真实分发目标」一起由 `scripts/plugin-entry-audit.mjs` 的
//! E-012 检查。

// ============ classify ============

/// classify/decide — 判决这一轮该直接回答、还是派给工具循环。
pub const ROUTE_CLASSIFY_DECIDE: &str = "classify/decide";

/// classify/parse — 把一次判决往返的**应答**解析成判决（D-122 决策 2 的另一半）。
pub const ROUTE_CLASSIFY_PARSE: &str = "classify/parse";

// ============ compose ============

/// compose/wording — 按上游判决组织一段面向用户的文本。
pub const ROUTE_COMPOSE_WORDING: &str = "compose/wording";

// ============ event_bus ============

/// event_bus/subscribe — 建立进程内帧订阅连接（前端实时面与 CLI 都走它收会话帧）。
pub const ROUTE_EVENT_BUS_SUBSCRIBE: &str = "event_bus/subscribe";

// ============ hook ============

/// hook/fire — 触发一次命名 hook（生命周期点调用，插件之间的通用挂钩）。
pub const ROUTE_HOOK_FIRE: &str = "hook/fire";

// ============ vdfs ============

/// vdfs/root — 经 `VDFS_OPS` 分发（无静态 `match` 臂）
pub const ROUTE_VDFS_ROOT: &str = "vdfs/root";

/// vdfs/unwatch — 经 `VDFS_OPS` 分发（无静态 `match` 臂）
pub const ROUTE_VDFS_UNWATCH: &str = "vdfs/unwatch";

/// vdfs/watch — 经 `VDFS_OPS` 分发（无静态 `match` 臂）
pub const ROUTE_VDFS_WATCH: &str = "vdfs/watch";
