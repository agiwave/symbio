//! 主部署驱动入口的**装配登记**——状态与推进逻辑住在 `symbio_host::drive`。
//!
//! 这里只剩三件事：
//! 1. 把「主部署的驱动入口」登记进 `DriveEntryDecl`（`inventory`，链接期生效）；
//! 2. 按当前部署根解析出那一份实例（`HostDriveBinding` 按根缓存，见宿主）；
//! 3. 给 CLI 暴露 `settled`（等，不是推）。
//!
//! ## 为什么登记留在这里
//!
//! 「主部署用哪个入口」是**装配选择**：`symbio` 是唯一知道根是怎么发布的地方，
//! 换一个入口（比如测试装配）就是换一行登记。而 `HostDrive` / `KickState` /
//! `BOUND` / `IN_DRIVE` 是**运行时状态**（按会话累积、跨轮存活），归 `symbio-host`
//! ——宿主不再被拆成「状态在装配 crate、对象在宿主 crate」两半（P1 批一）。
//!
//! ## 为什么按根构造，不按入口构造
//!
//! 每个 `HostDrive` 绑一份部署根：能力收集从这份根出发，会话的 `session_id` 只用来
//! 取上下文身份。实例按根缓存，`MainDriveEntry` 与插件 `drive_entry` 拿到的是同一个
//! 节点——`settled` 等的那份与 `kick` 写的那份必须是同一个（D-359 的 t29 根因）。

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

pub use symbio_host::drive::{drive_on_new_fact, open_fact_source, FactWorld};

use symbio_plugin_api::{DriveEntry, DriveTarget};

/// 主部署的那一份实例。
///
/// 每次调用按当前部署根解析：`HostDriveBinding` 按根缓存，因此这里拿到的与插件
/// `drive_entry` 绑的是同一个实例；reload 重建根后自然换到新根。
fn main_drive() -> Option<Arc<dyn DriveEntry>> {
    let root = symbio_plugin_api::deployment_root()?;
    Some(symbio_plugin_api::declared_drive_binding()?.bind(root))
}

/// 主部署实例的静态入口（`inventory` 登记；未装配时 `None`）。
struct MainDriveEntry;

static MAIN_DRIVE_ENTRY: MainDriveEntry = MainDriveEntry;

inventory::submit! {
    symbio_plugin_api::DriveEntryDecl { entry: &MAIN_DRIVE_ENTRY }
}

impl DriveEntry for MainDriveEntry {
    fn kick(
        &self,
        target: DriveTarget,
    ) -> Option<Pin<Box<dyn Future<Output = ()> + Send + 'static>>> {
        main_drive()?.kick(target)
    }

    fn settled(&self, session_id: &str) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        let session_id = session_id.to_string();
        Box::pin(async move {
            if let Some(drive) = main_drive() {
                drive.settled(&session_id).await;
            }
        })
    }
}

/// 等本会话驱动空闲（**等，不是推**）。
///
/// CLI 在 `session/consume` 之后用它等写口踢出的那次驱动落地，再投影转写。
pub async fn settled(session_id: &str) {
    if let Some(entry) = symbio_plugin_api::declared_drive_entry() {
        entry.settled(session_id).await;
    }
}

#[cfg(test)]
#[path = "drive.test.rs"]
mod tests;
