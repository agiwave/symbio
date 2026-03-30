//! 核心模块

pub mod traits;
pub mod types;
pub mod registry;

pub use traits::{Plugin, PluginFactory};
pub use registry::PluginFactoryRegistry;
