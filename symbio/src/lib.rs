//! Symbio - Fractal Plugin Architecture Library
//!
//! Symbio 是一个基于分形插件架构的核心库,提供:
//! - 统一的插件接口和工厂模式
//! - 全局工厂注册表
//! - 流式调用支持
//! - 能力路由系统

pub mod symbio_core;
pub mod plugins;
pub mod init;

// 重新导出核心类型和 trait
pub use symbio_core::{Plugin, PluginFactory, PluginFactoryRegistry};
pub use symbio_core::types::{PluginMeta, PluginError, PluginResult, StreamChunk, InvokeStream, BoxStream};
pub use symbio_core::traits::CAPABILITY_LLM;
pub use symbio_core::event::{EventSender, OptionalEventSender};

// 重新导出初始化函数
pub use init::create_root_plugin;

// 重新导出所有插件工厂
pub use plugins::{
    HomeFactory, WorkFactory, NoteFactory, SettingFactory,
    AgentFactory, ChatFactory, ToolsFactory, MemoryFactory,
    SessionFactory, TelegramFactory, OpenAiFactory,
    EchoFactory, DockerFactory, CompositeFactory,
    ExplorerFactory,
};
