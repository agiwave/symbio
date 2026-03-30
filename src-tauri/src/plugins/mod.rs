//! 插件实现模块

pub mod agent;
pub mod echo;
pub mod calculator;
pub mod formatter;
pub mod docker;
pub mod work;
pub mod setting;
pub mod home;

pub use echo::EchoFactory;
pub use calculator::CalculatorFactory;
pub use formatter::FormatterFactory;
pub use docker::DockerFactory;
pub use home::HomeFactory;
