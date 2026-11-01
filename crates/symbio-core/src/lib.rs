//! 核心模块
//!
//! ## 根上只出 core 自己定义的东西
//!
//! core 的根出口是一份**账本**，不是一条通道：每个名字都必须是 core 自己定义的
//! （DE-023 D-257 对内核符号立的规矩，D-272 把它推到 SDK 与两跳的内核符号上）。
//! 三条边界各归各位：
//!
//! | 层 | 谁能写 | 长什么样 |
//! |---|---|---|
//! | `symbio_kernel` | 任何 crate | 封闭语法（事件信封、`Entity × Verb`、声明容器、五个 trait） |
//! | `symbio_sdk` | 控制面与宿主壳（插件经门面） | 插件能用的宿主机制（配置 IO、事件总线、地址装配、日志） |
//! | `symbio_core`（本 crate） | 控制面与宿主壳 | 控制面运行时：主体策略、授权表、履约环、可见域 |
//!
//! 同一批符号在 `symbio_core::` 与 `symbio_sdk::` 两处都写得通，后果与 D-257 记的
//! 一字不差：消费方写哪个前缀取决于当初谁顺手写了哪个，而两份清单可以各自漂移
//! （SDK 加一个变体、core 根的清单不必动，编译器不会说话）。D-272 之前根上 324 条
//! 出口里 247 条不属于 core——其中 123 条连 SDK 的都不是，是经 SDK 的域模块两跳的
//! 内核符号。这条通道因此不是「一次搬家的遗留」，是**持续泄漏**。
//!
//! ## 域目录留着，根出口不留
//!
//! 子目录一律 `mod <域>;` 私有（C-001 校验，编译器兜底）：一个域不能因为「实现搬走了」
//! 就从 `core-naming-audit` N-004 的登记表里消失——搬的是住址，不是归属。留下的域模块
//! 只做同名转出，仓内既有 `crate::<域>::…` 路径因此不变；对外**不再**经根转发，
//! 调用点按符号的真实归属写 `symbio_kernel::<域>::…` / `symbio_sdk::<域>::…`。
//!
//! ## 为什么不再转发
//!
//! 控制面（`loop` / `session` / `plugin-api`）本来就允许直接依赖 kernel / sdk
//! （HS-001 只拦扩展层插件），所以「不转发」不需要任何新权限。C-002 禁的是**深引
//! core 的域目录**，而 sdk / kernel 的域路径是它们自己的公开面——两者不是一回事。

mod actors;
mod adapters;
// **Agent 包的权限声明与它的执行方**（DE-011 §5.3 / D-153）：实现已随 D-258 进
// `symbio-sdk`，本域只剩登记（N-004）。
mod agent;
// **本机授权表**（谁持有什么能力 + 可见域）——与 `governance` 的机制同住一棵模块树：
// 表与判定分家的唯一坏处是「表授予 A、闸门判 B」这种错位**没有任何东西会变红**。
// 表整块是架构元素，故保留 `authz` 命名空间、`pub(crate)` 开给全 crate（**不开对外
// 模块门**，故 C-001 不判、C-003 也不数它）：表里写的是部署事实，不是契约。
pub(crate) mod authz;
mod capability;
mod chat;
mod clock;
mod creator;
// 事实 / 视图声明层：字段清单与视图读法在装载期可查。
// 义务环的输入面——行表（`crates/symbio-plugin-loop/src/loop_rows.rs`）用 `Frozen::eval` 求视图值。
pub(crate) mod decl;
mod embedding;
mod event;
mod event_bus;
mod exec;
mod governance;
mod invariants;
mod keys;
mod llm;
mod logger;
pub(crate) mod plugin;
mod present;
mod projection;
mod store;
// 用例自己交的事件拼写与三张词汇表（DE-024 D-203 / D-205 / D-206）：只在 `cfg(test)`
// 下存在，不是 core 的词典——生产拼写由各 owner 的清单生成物提供，core 一行都不读。
// 实体随 `projection` 搬进 `symbio-sdk`（D-259），core 只用不拥有：这里是 `use` 不是
// `pub use`——用例写 `crate::test_fixtures::…` 照旧能解析（私有绑定对后代模块可见），
// 但 core 的根公共面不多出一条。
#[cfg(test)]
use symbio_sdk::test_fixtures;
mod text;
mod turn;
mod vdfs;
mod view;

// ==================== 主体（② actors） ====================
// 只收类型化输入、只产事件（DE-018 §3.1 ② 行）。
//
// ⚠️ `TurnRunner` / `TurnInput` / `TurnOutcome` / `TurnResume` / `RoundInjector`
// **不在本层**（住 `crates/symbio-plugin-loop/src/turn_runner.rs`）：它们是**执行引擎**
// （流循环 + 落格顺序），生产消费方只有 session 一个，按 README §4 四问第 1 问归插件；
// 且它们**不是** F1–F6 冻结锚点（F3 冻的是 `ActorSpec` 字段集 + 档位→令牌映射，不是运行器）。
//
// `ActorSpec`（F3 冻结锚点：`DE-001 §4` 的**主体规格五字段**）的数据面住 `symbio_kernel`
// 的 `actors`，读法（`trivial_actor_spec` / `pattern_for_principal` / `actor_can_*`）读的是
// 本机**部署表**，不是语法 ⇒ 留在这里（D-099 / D-188 C2b）。宿主 crate 不能给内核类型写
// 固有 impl（orphan），故以自由函数表达。
// Agent 权限声明（DE-011 §5.3 / D-153）**不出根**：三个消费方里 agent 插件与 web /
// vdfs 工具走门面的 SDK 臂（`symbio_plugin_api::AgentPermissions`），session 走
// `symbio_sdk::agent::…`——五个符号都已随 D-258 住进 `symbio-sdk`，再从 core 根出一份
// 就是 D-272 拦的重复出口。
pub use actors::{
    actor_can_assert_verification, actor_can_assign_work, actor_can_judge_intent,
    actor_can_land_artifact, actor_can_land_speech, pattern_for_principal,
};
pub use actors::{commitment_events, commitment_records};
pub use actors::{EliminationDecider, SKILL_TAG};

// ==================== 可见域 ====================
// 可见域判据与智能体身份前缀：两处共用**同一条**判定——写视图的 `window_by_turn`
// （core）与写消息过滤的 `context/view`（插件）。两个判定方 = 两套可见域与两种身份
// 形状，迟早漂移成「判的是 A、写的是 B」。
// `principal_namespace` 只被 `authz`（core 内）取用、core 外零消费方，故走
// `pub(crate)` 出口——`crate::principal_namespace` 这一写法不变，`authz` 无需改。
pub(crate) use view::principal_namespace;
pub use view::visible_to;

// ==================== 权限与可见性（⑥ governance） ====================
// 读写成对、fail-closed（DE-001 §7）。根出口只出**可见域**一个类型：
// 判定面（`crates/symbio-plugin-session` 的读侧闸）要指称它，构造面（本根 `authz`）的表行也带它。
// core 外消费方只有 `crates/symbio-plugin-session` 一个，但它**不能**下沉——core 内 `authz` /
// `governance` 自己也在用（core 不依赖插件），插件又够不着 `governance` 域目录的深引
// （C-002）⇒ 必须留根出口；C-003 的「≥2 消费方」由 core 内这两个模块成立。
//
// `PermissionMatrix` **不在根出口**（收窄）：core 外没有一处指称这个类型——
// `stats.rs` 的读侧闸只经 `authz::production_matrix()` 拿表（返回类型走类型推导，不点名）
// ⇒ 按 C-003「名字没跨出 core 的符号不许占根出口」收回 core 内。
//
// `GrantsOverlay`（淘汰回路的机制半边）：授权表是装配期常量，淘汰是
// 运行期事实 ⇒ 覆盖层住表旁边（表答「登记过什么」，它答「其中什么被收回了」）。
// 两个插件侧消费方——`derived_facts` 轮末步（判「有没有既有淘汰」）与工具执行层的
// 撤销闸（判「现在准不准委派」）⇒ 与 `VisScope` 同形态 `pub` 出根。
pub use governance::GrantsOverlay;
pub use governance::VisScope;
// 授权表的**外部面**：`authz` 收敛成域目录后，`symbio_core::authz::…`
// 就是一条深路径（C-002 判红），而插件侧有十几个符号在用。按 §1 的口径，根平铺是
// 唯一合法路径 ⇒ 表仍住 `authz/`，出口逐个列在这里。
pub use authz::{
    bind_inheriting, bind_root, content_visible, principal_of, production_matrix,
    request_principal, unbind_session, EffectiveMatrix, CAP_ASSERT_VERIFICATION, CAP_ASSIGN_WORK,
    CAP_DEFINE_WORK, CAP_EXTERNAL_EXECUTION, PRINCIPAL_AUTONOMOUS, PRINCIPAL_MAIN,
    PRINCIPAL_REFLEX, PRINCIPAL_SCHEDULER, PRINCIPAL_USER,
};
// 行名 / 属主。四个名字的消费方**不同类**，别当成一组：
// - `SESSION_OWNER`：**生产**消费方两处（`loop` 的 `memory_events`、
//   `session` 的 `stats`）；
// - `PRINCIPAL_ARBITER` / `PRINCIPAL_WORKER`：只有 core 外测试点名
//   （`derived_facts.test.rs` / `autonomous.test.rs`）；
// - `PRINCIPAL_PEER`：core 内自用。
pub use authz::{PRINCIPAL_ARBITER, PRINCIPAL_PEER, PRINCIPAL_WORKER, SESSION_OWNER};
