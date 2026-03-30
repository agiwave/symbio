//! Composite 插件 - 通用组合插件
//!
//! 支持动态管理多个子插件，通过配置创建子插件实例，
//! 提供子插件的列表/添加/删除功能，自动路由到子插件。

pub mod plugin;
pub mod factory;

pub use factory::CompositeFactory;
