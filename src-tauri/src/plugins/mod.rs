//! 插件实现模块

pub mod agent;
pub mod echo;
pub mod docker;
pub mod work;
pub mod setting;
pub mod home;
pub mod composite;
pub mod explorer;

// 导出所有工厂
pub use echo::EchoFactory;
pub use docker::DockerFactory;
pub use home::HomeFactory;
pub use work::WorkFactory;
pub use setting::SettingFactory;
pub use composite::CompositeFactory;
pub use explorer::ExplorerFactory;

// 导出 Agent 子插件工厂
pub use agent::{ChatFactory, ToolsFactory, MemoryFactory, SessionFactory, TelegramFactory, OpenAiFactory, AgentFactory};