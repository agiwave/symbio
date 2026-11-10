//! `llm` —— 模型往返的**数据词汇**与消息词汇（不含端口）。
//!
//! 端口形态住在 model 插件里（`crates/symbio-plugin-model/src/provider.rs`），
//! 理由见 [`model`] 的模块文档与 `D-437`。

pub mod model;
pub mod prompt_message;
pub mod round;

pub use model::*;
pub use prompt_message::*;
pub use round::*;
