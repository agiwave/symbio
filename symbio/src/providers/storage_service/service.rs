//! 通用存储服务实现（symbio 的具体实现层）
//!
//! 注册 `StorageService` 工厂，crate 内插件通过工厂获取实例。
//!
//! ## 设计原则
//!
//! - trait 抽象在 [`crate::symbio_core::providers::storage`]
//! - 本模块只放具体实现
//! - 通过 `submit_object_creator!` 工厂注册 `dyn StorageService`
//! - 业务模块**不**直接 `use` 本模块的类型
//!
//! ## 系统目录 (homedir)
//!
//! 存储服务的基础目录来自 [`HomedirRegistry::get()`]（运行时可配置）。
//! homedir 切换后，存储服务实例需重新创建（通过 home/reload 触发）。

use crate::providers::storage_service::file_entity_store::FileEntityStore;
use crate::symbio_core::providers::StorageService;
use crate::symbio_core::HomedirRegistry;
use std::path::PathBuf;
use std::sync::Arc;

crate::submit_object_creator!("storage_service", build_storage_service, dyn StorageService);

/// `StorageService` 工厂
///
/// **homedir 来源**：从 [`HomedirRegistry::get()`] 读取。
/// homedir 切换后需要重新调用此工厂以拿到新基址的实例。
pub fn build_storage_service(
    _ctx: Arc<dyn crate::symbio_core::InvokeRequest>,
) -> Arc<dyn StorageService> {
    let base = HomedirRegistry::get();
    Arc::new(FileStorageService::new(base))
}

/// `StorageService` 的文件系统实现
pub struct FileStorageService {
    store: FileEntityStore,
}

impl FileStorageService {
    pub fn new(base: PathBuf) -> Self {
        Self {
            store: FileEntityStore::new(base),
        }
    }
}

impl StorageService for FileStorageService {
    fn entity_store(&self) -> &dyn crate::symbio_core::providers::EntityStore {
        &self.store
    }

    fn base(&self) -> &std::path::Path {
        self.store.base()
    }
}
