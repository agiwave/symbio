//! 组合根登记的编排执行者声明。
//!
//! `drive` 族是插件树的回退发现口；编排逻辑离开插件树后，装配方直接登记执行者。
//! `make` 是构造器而不是值，使宿主每次取回独立实例，也让测试能按场景决定是否登记。

use std::sync::Arc;

use symbio_plugin_api::drive::DriveRunner;

/// 一份由组合根交来的 `DriveRunner`。
pub struct DriveRunnerDecl {
    pub make: fn() -> Option<Arc<dyn DriveRunner>>,
}

inventory::collect!(DriveRunnerDecl);

/// 取组合根登记的编排执行者；未登记时由宿主回落到 `drive` 族。
pub fn declared_drive_runner() -> Option<Arc<dyn DriveRunner>> {
    inventory::iter::<DriveRunnerDecl>().find_map(|decl| (decl.make)())
}

#[cfg(test)]
#[path = "declared_drive_runner.test.rs"]
mod tests;
