//! 插件测试夹具——只在 `cfg(test)` 或显式 `test-fixtures` feature 下编译。
//!
//! 独立 crate 曾解决 `#[cfg(test)]` 对下游不可见的问题；feature 提供同一能力，
//! 且默认生产构建不看见测试出口。

mod stubs;

pub use stubs::trivial_actor_spec;

pub use crate::EVENT_BUS_RESYNC_MARKER_TYPE;
pub use symbio_platform::kernel::decl::Builder;
pub use symbio_platform::kernel::event::EVENT_RETRY;
pub use symbio_platform::kernel::store::{EventStore, Store};
pub use symbio_platform::kernel::vdfs::VDFS_KIND_DIR;
pub use symbio_platform::kernel::Seq;
pub use symbio_platform::sdk::capability::{capability_entry_of, capability_option_fields};
pub use symbio_platform::sdk::declarations::sealed_declarations;
pub use symbio_platform::sdk::invariants::check_all;
pub use symbio_platform::sdk::keys::PRINCIPAL_MAIN;
pub use symbio_platform::sdk::plugin::fact_source;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
