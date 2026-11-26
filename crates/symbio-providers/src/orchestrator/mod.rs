//! 编排 provider：把纯机制环作为装配层持有的服务交出去。
//!
//! `ring` / `plugin_rows` / `drive` 的实现原样迁入本模块；对外只暴露组合根可登记的
//! `DriveRunner`，不把环的内部类型交给宿主或插件。

use std::sync::Arc;

use symbio_plugin_api::drive::DriveRunner;

pub(crate) mod drive;
pub(crate) mod plugin_rows;
pub(crate) mod ring;

/// 造一份无状态的编排执行者。
pub fn runner() -> Arc<dyn DriveRunner> {
    Arc::new(drive::LoopDrive)
}
