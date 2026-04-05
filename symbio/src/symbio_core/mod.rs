//! 核心模块

pub mod traits;
pub mod types;
pub mod registry;
pub mod event;

pub use traits::{Plugin, PluginFactory};
pub use registry::PluginFactoryRegistry;
pub use event::{EventSender, OptionalEventSender};
