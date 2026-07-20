#![allow(clippy::module_inception)]

// 所有子模块私有——实现细节不对外暴露
pub(crate) mod create_agent;
mod engine_pool;
mod index;
mod loader;
mod manager;
mod model;
mod path;
mod registry;
mod tracker;

// 唯一对外接口——跨模块契约
pub use manager::AgentManager;
pub use model::AgentProfile;
pub use path::{resolve_workspace_dir, validate_workspace_root};
pub use registry::AgentRegistry;
// ProfileLoader 暂不对外暴露，保留供内部使用
// pub use loader::ProfileLoader;
