//! Symbio - Fractal Plugin Architecture Library
//!
//! Symbio 是一个基于分形插件架构的核心库，提供:
//! - 统一的插件接口
//! - 通用对象创建注册表（基于 `inventory` 的静态注册 + `InvokeRequest` 驱动的运行时构造）
//! - 流式调用支持
//! - 能力路由系统
//!
//! ## 模块分层
//!
//! - `plugins/`：业务插件（私有，插件之间不直接相互引用）
//! - `symbio_core/`：核心抽象（Plugin trait、InvokeRequest、schemas、**服务 trait**）
//! - `providers/`：通用服务基础设施（私有，**不**对外暴露）
//!   - 各服务的**抽象**在 `symbio_core::providers`
//!   - 各服务的**实现**在 `src/providers/`
//!   - 各服务通过 `submit_object_creator!` 工厂注册
//!   - 业务模块通过 `create_object::<dyn XXXService>(...)` 获取实例
//!   - **不**通过 `pub use` 暴露给 crate 外部

pub mod init;
mod plugins;
pub(crate) mod providers;
pub mod symbio_core;
