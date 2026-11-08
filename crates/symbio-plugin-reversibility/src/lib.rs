//! 可逆性设计插件 —— **动作的可逆等级 + 读数**（名册 #40，DE-026 §2.3）。
//!
//! 本 crate 与 `symbio-plugin-causal` / `symbio-plugin-world_model` 用**相同的**
//! 扩展面组合，验证的是「一个新的判断维度能整包长出来」：
//!
//! | 面 | 用在哪 | 谁读 |
//! |---|---|---|
//! | 工具（`Capability`） | `reversibility_annotate` | 模型（经工具面调用） |
//! | 行为（`ToolBehavior`） | 一次成功标注 ⇒ 一条 `reversibility.assessed` 草稿 | 宿主唯一的写口（I-1） |
//! | 事实（`EventDecl` / `FactDecl` / 归宿行） | `reversibility.assessed` 落在 `domain × asserted` | 写闸按声明校验名字与格位 |
//! | 视图（`ViewDecl` + `shared_views`） | `reversibility.register` 折出当前等级分布 | 任何按名字读的插件（D-221） |
//!
//! 没有新路由、没有配置、没有新端口——`kernel` / `sdk` / `loop` / `session` 一行都不动。
//!
//! ## 这条能力为什么值得独立成包
//!
//! 可逆性是**动作的属性**，不是某个工具的私有知识：同一个删除动作，在缓存目录里是
//! 可逆的，在客户数据上是不可逆的。把标注做成领域事实以后，任何读方（审批闸、风险
//! 面板、后续计划）都能按动作名读到当前判断，而不必理解是哪个工具执行了它。
//!
//! ## 依赖面
//!
//! `[dependencies]` 只有 `symbio-plugin-api`（加 `serde_json` / `tokio` / `async-trait`
//! 这些通用库），HS-001 判的 `kernel` / `sdk` / `deploy` 一个都不列。

mod behavior;
mod decl;
mod plugin;
mod tool;
