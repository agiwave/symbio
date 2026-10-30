//! Task 插件 —— **任务能力包**（DE-025；名册第 26 行「独立验证与返工」、DE-033）。
//!
//! 它拥有的是一条完整的能力四元组，以及这条能力在三个时点上的表现：
//!
//! | 时点 | 形态 | 扩展面 |
//! |---|---|---|
//! | 工具执行之后 | 成功的 `todo_write` 声明 → `task.*` 事实草稿（含返工节点） | 效果端口（[`task::TaskTable`]） |
//! | 请求组装之前 | 就绪集渲染成一段提示 | 上下文 provider（[`context::ReadySetProvider`]） |
//! | 本轮收束之后 | 就绪集 → `task.ready` / `task.assigned` | 收束贡献（[`task::ScheduleContributor`]，D-250） |
//!
//! 三者都只**交草稿**：写口仍是宿主的唯一写入口（I1），判断与成格住本插件。
//! `core` / `kernel` 不认识「任务」这个词：格位、角色与名字都在 [`decl`]，
//! 读侧按 [`decl::TASK_VOCAB`] 认名字，跨插件消费经 `PLUGIN.yml` 的 `shared_events`
//! 投影——本插件不依赖也不认识任何兄弟插件。
//!
//! ## 域内分工
//!
//! | 文件 | 职责 |
//! |---|---|
//! | [`decl`] | 任务族的事件声明、角色（`CountsRework`）与词汇表 |
//! | [`task`] | 声明 → 事实的折叠（开 / 推进 / 终态 / 返工）、调度判据、调度段排版、收束贡献 |
//! | [`context`] | 把调度段交给请求视图（段身份 `READYSET`，prio 70） |
//! | [`plugin`] | 插件本体：无工具、无路由，只是装配挂载点 |
//!
//! 内部依赖是单向的：`context` / `task` 只读 [`decl`] 的名字；`plugin` 不认识其余模块。

mod context;
mod decl;
mod plugin;
mod task;

pub use plugin::TaskPlugin;
