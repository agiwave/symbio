//! 插件实现模块

pub mod agent;
pub mod echo;
pub mod docker;
pub mod work;
pub mod setting;
pub mod home;
pub mod composite;

pub use echo::EchoFactory;
pub use docker::DockerFactory;
pub use home::HomeFactory;
pub use composite::CompositeFactory;
pub use agent::ChatFactory;