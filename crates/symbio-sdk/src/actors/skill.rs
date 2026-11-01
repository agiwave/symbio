//! 技能词汇（[DE-041](../../../../docs/003-design/041-技能编译与自我改进.md "DE-041-技能编译与自我改进")）：`memory.encoded` 载荷里的技能标签。
//!
//! 编译与路由是单消费方策略，归 owner 插件（`crates/symbio-plugin-skill`，D-189 / D-262）；
//! 本域只留这个**共享词汇**——写方在技能能力包、读数口在 session（D-177 / D-182）。

/// 技能的认知内容标签（`memory.encoded` 载荷里的 `tag`）。
///
/// 名字是数据、单点定义（D-043）：写方（`crates/symbio-plugin-skill` 的轮末贡献者）、读侧路由
/// 与读数口（`crates/symbio-plugin-session/src/stats.rs`）都 `use` 这一处。两处各写一个字面量时，改一边
/// 不改另一边 ⇒ 技能被当成普通记忆召回却不进路由，全程没有任何报错。
pub const SKILL_TAG: &str = "skill";
