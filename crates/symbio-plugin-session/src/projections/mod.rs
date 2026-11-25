//! 会话插件的纯折叠（DE-025 D-221 / DE-018 D-561）。
//!
//! 实现与测试跟随唯一生产消费方 session；共享词表仍由 SDK 提供。
//! 时延、兜底与跨度口径走能力包 `slo`，自主程度走 `conation.autonomy`，
//! 跨插件读取经 `views` 按名字进行，不在这里复制第二份实现。

pub(crate) mod branch;
pub(crate) mod changes;
pub(crate) mod checkpoint;
pub(crate) mod elision;
pub(crate) mod facts;
pub(crate) mod mutations;
pub(crate) mod node_build;
#[cfg(test)]
pub(crate) mod prompt_fidelity;
pub(crate) mod transcript;
