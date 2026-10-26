//! VDFS 的**纯接口面**：`VdfsProvider` trait 与域类型（词表 / 访问位 / 节点 / 信封）。
//!
//! 宿主侧的桥（上下文注入、错误翻译、变更广播）与当前父地址机制留在
//! `symbio_core::vdfs::{host, address}`；本 crate 不依赖宿主。

mod access;
mod change;
mod content;
mod context;
mod error;
mod node;
mod path;
mod provider;
mod request;
mod words;

// ---- 纯接口：词表常量 ----
pub use words::{
    VDFS_ACTION_ABORT, VDFS_ACTION_CLEAR, VDFS_ACTION_DISABLE, VDFS_ACTION_ENABLE,
    VDFS_ACTION_EXPAND_SKELETON, VDFS_ACTION_EXPORT, VDFS_ACTION_IMPORT, VDFS_ACTION_PLUGINS,
    VDFS_ACTION_SWITCH_BRANCH, VDFS_ACTION_TEST, VDFS_ACTION_TRUNCATE, VDFS_EXT_FORM, VDFS_EXT_ZIP,
    VDFS_KIND_DIR, VDFS_PARAM_BEFORE, VDFS_PARAM_LIMIT, VDFS_PARAM_WORKDIR, VDFS_PLUGINS_FIELD,
    VDFS_PLUGIN_NAME_FIELD, VDFS_PLUGIN_PROVIDER_FIELD, VDFS_STATUS_ACTIVE, VDFS_STATUS_DISABLED,
    VDFS_STATUS_FAILED, VDFS_STATUS_NONE, VDFS_STATUS_UNKNOWN, VDFS_STATUS_WORKING,
};
// ---- 纯接口：域类型 ----
pub use access::VdfsAccess;
pub use change::{VdfsChange, VdfsChangeSink};
pub use content::VdfsContent;
pub use context::{VdfsContext, VdfsParams};
pub use error::{VdfsError, VdfsFieldError, VdfsResult, VdfsValidationError};
pub use node::{vdfs_derive_ext, VdfsItem, VdfsNewType, VdfsNode};
pub use path::{vdfs_has_parent_segment, vdfs_path_within};
pub use provider::{DynVdfsProvider, VdfsProvider};
pub use request::{VdfsActionResult, VdfsRequest, VdfsResponse, VdfsWriteResponse};

#[cfg(test)]
mod tests;
