//! OpenAI Compatible LLM 插件
//!
//! 支持: OpenAI, Azure OpenAI, 以及任何 OpenAI 兼容 API

mod types;
mod token;
mod plugin;
pub mod factory;

pub use plugin::OpenAiPlugin;
