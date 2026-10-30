//! 能力表的**宿主侧**：收集通道、审批 / 可配置 / 模型 / 提示词 / 挂载族的自由函数。
//!
//! 契约本体（`Capability` / `Registry` trait、元数据、行与族词汇、`failure_kind`）已在
//! `symbio_kernel`（D-188 C2b）。

mod collect;
mod configurable;
mod error;
mod grants;
mod model;
mod mount;
mod option;
mod prompt;
mod tool_name;

pub use symbio_kernel::capability::*;

// 三条收集通道键与错误桶键住本域（键与值同住，D-095 根因一）。**只导实例、
// 不导 `…Key` 类型**——实例是 `ctx` 上的读写凭据（对外协议面），类型名没有任何
// 域外消费方（C-003），`pub struct` 留在子模块里由本域自用。`CapabilityError`
// 同理不出本域：读侧经 `capability_take_errors` 的返回类型推断拿到它，
// 无需实名（字段 `plugin` / `message` 直接可读）。
pub use collect::{tool_metas, CAPABILITY_VISITOR, CONFIGURABLE_VISITOR, OPTION_VISITOR};
pub use error::{
    capability_init_error_bucket, capability_report_error, capability_take_errors,
    CAPABILITY_ERRORS,
};

// 域内子模块私有，公开面在此显式重导出
//
// 注：全表共用的内存收集器（`DefaultToolVisitor`）**不在本域**——
// 它是契约的默认实现，写入者是全体插件，故住 `symbio_kernel::capability::providers::collectors`。
pub use configurable::{
    capability_announce_configurable, capability_configurables, capability_entry_of,
};
pub use grants::ApprovalGrants;
pub use model::{capability_model_active, capability_model_resolve};
pub use mount::capability_vdfs_root;
pub use option::capability_option_fields;
pub use prompt::capability_prompts;
pub use tool_name::{capability_resolve, capability_to_wire};
