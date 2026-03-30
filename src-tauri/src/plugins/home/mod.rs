//! Home 插件模块 - 根插件，持有 work/agent/setting 子插件

mod plugin;
mod factory;

pub use plugin::HomePlugin;
pub use factory::HomeFactory;
