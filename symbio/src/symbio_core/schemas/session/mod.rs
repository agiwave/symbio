pub mod chat_node;
pub mod facts;
pub mod node_build;
pub mod principal;
pub mod session_chat;

// `chat_node` 用 `super::Principal` 引同目录的 `principal`——这里做一次短路径 re-export
// 让兄弟模块不必写完整链；外部一律走 `symbio_core::principal::Principal`（叶模块形态）。
#[allow(unused_imports)]
pub use principal::Principal;
