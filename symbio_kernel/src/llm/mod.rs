//! `llm` —— 模型契约与单轮产物的**类型面**。

pub mod model_provider;
pub mod turn;

pub use model_provider::*;
pub use turn::*;
