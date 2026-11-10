//! 五类扩展的**局部性夹具**（[DE-023](../../../docs/003-design/023-内核边界与扩展局部性.md "DE-023-内核边界与扩展局部性") A1）。
//!
//! 每类扩展各一个用例：**事实 / 视图 / 策略 / 插件 / 共享类型**。夹具全部住
//! 本 crate、只用内核的公开扩展口（`PluginDeclarations` + `Builder` 的声明类型、
//! `Projection`、`CapabilityFamily` / `CapabilityRow`），因此「加一样东西要不要动
//! 内核」有了一条可执行的答案：这些用例编译并通过，而内核文件一个都没改。
//!
//! 它们是**测试夹具**，不是生产声明——`#[cfg(test)]` 保证不进产物；生产声明仍由
//! `crates/symbio-plugin-loop/src/control_decl.rs` 等 owner 文件交出。

/// 测试替身：[`trivial_actor_spec`]。
///
/// 测试替身住**测试设施**：公开面只装「被两处以上生产代码看见」的
/// 东西，[`trivial_actor_spec`] 只有测试面消费，故归这里。
pub mod stubs;

pub use stubs::trivial_actor_spec;

// ---- 测试面的宿主机制出口（D-443） ----
//
// 门面（`symbio-plugin-api`）只承**生产**依赖面（HS-003 的消费方 = 插件生产代码，
// `.test.rs` 不再为条目续命）之后，插件测试夹具需要的宿主机制名经本 crate 转出：
// E-009 ⑥ 认定的**测试设施公共出口**——它不含任何插件实现、不携带会话状态，
// `pub use symbio_plugin_*`（插件类型）仍被 E-009 ⑥ 判红，这里转出的只有
// `symbio-kernel` / `symbio-sdk` 的宿主机制名。每一条都对应至少一个插件测试夹具。
pub use symbio_kernel::decl::Builder;
pub use symbio_kernel::event::EVENT_RETRY;
pub use symbio_kernel::store::{EventStore, Store};
pub use symbio_kernel::vdfs::VDFS_KIND_DIR;
pub use symbio_kernel::Seq;
pub use symbio_sdk::capability::{capability_entry_of, capability_option_fields};
pub use symbio_sdk::declarations::sealed_declarations;
pub use symbio_sdk::event_bus::EVENT_BUS_RESYNC_MARKER_TYPE;
pub use symbio_sdk::invariants::check_all;
pub use symbio_sdk::keys::PRINCIPAL_MAIN;
pub use symbio_sdk::plugin::fact_source;

#[cfg(test)]
#[path = "lib.test.rs"]
mod tests;
