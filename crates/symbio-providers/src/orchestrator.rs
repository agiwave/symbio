//! 编排 provider：把纯机制环作为装配层持有的服务交出去。
//!
//! `ring` / `plugin_rows` / `drive` 的实现保持原样迁移；本模块只提供组合根可登记的
//! `DriveRunner`。插件树里的 `loop` 暂留作回退，宿主优先使用这里登记的执行者。

use std::sync::Arc;

use symbio_plugin_api::drive::DriveRunner;

/// 造一份无状态的编排执行者。
pub fn runner() -> Arc<dyn DriveRunner> {
    Arc::new(crate::drive::LoopDrive)
}
