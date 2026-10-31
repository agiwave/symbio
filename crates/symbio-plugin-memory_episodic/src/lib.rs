//! 情景记忆能力包（DE-026 D-263；名册第 2 行「情景记忆」）。
//!
//! 它拥有的是**编码**这一段：一轮收束时把「用户说了什么、智能体答了什么」编成
//! 至多两条 `memory.encoded`（经验 / 判断）。召回、巩固与遗忘仍在
//! `crates/symbio-plugin-loop`（名册 #6 检索段是下一批）——两边只认清单生成物里
//! 的事件名，不互相链接。
//!
//! | 文件 | 职责 |
//! |---|---|
//! | [`decl`] | 事实形状（`memory.encoded`）与归宿行 |
//! | [`contributor`] | 收束贡献：两条编码与同文去重 |
//! | [`plugin`] | 插件本体：无工具、无路由，只是装配挂载点 |

mod contributor;
mod decl;
mod plugin;

pub use plugin::MemoryEpisodicPlugin;
