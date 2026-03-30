//! OpenAI Compatible LLM 插件
//!
//! 支持: OpenAI, Azure OpenAI, 以及任何 OpenAI 兼容 API

mod types;
mod token;
mod plugin;
pub mod factory;

pub use types::*;
pub use token::*;
pub use plugin::OpenAiPlugin;
pub use factory::OpenAiFactory;
