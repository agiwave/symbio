//! `chat` —— 会话线格式的**类型面**（节点与主体）。

pub mod chat_node;
pub mod principal;
// 工具主体的命名空间集合（生成物）：加一个带工具的插件 = 在自己目录里写一行。
#[path = "tool_namespaces.gen.rs"]
mod tool_namespaces_gen;

pub use chat_node::*;
pub use principal::*;
pub use tool_namespaces_gen::TOOL_NAMESPACES;
