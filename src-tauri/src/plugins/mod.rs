//! 插件实现模块

pub mod agent;
pub mod echo;
pub mod calculator;
pub mod formatter;

pub use agent::AgentFactory;
pub use echo::EchoFactory;
pub use calculator::CalculatorFactory;
pub use formatter::FormatterFactory;
