//! 默认可配置声明收集器 —— [`ConfigurableVisitor`] 的内存实现
//!
//! ## 为什么在 `providers/` 而不是 `symbio_core`
//!
//! 本类型的**写入者是全体插件**（在 `traverse` 里经 `ctx` 的 `CONFIGURABLE_VISITOR` 键
//! 声明自己的配置文档），安装它的宿主（composite 容器的 `children_of`）只是其中之一。
//! 它不是任何插件的内部物——完整判据见 [`crate::providers::collectors`] 的模块文档。

use crate::symbio_core::{ConfigurableVisitor, VdfsItem};
use async_trait::async_trait;
use indexmap::IndexMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// 默认可配置声明收集器：内存 IndexMap 实现，一次收集一个实例。
///
/// 语义与 [`crate::symbio_core::OptionVisitor`] 一致：按条目名去重、后者覆盖
/// （保留先注册的槽位），列表按注册顺序。
pub struct DefaultConfigurableVisitor {
    items: Arc<RwLock<IndexMap<String, VdfsItem>>>,
}

impl DefaultConfigurableVisitor {
    pub fn new() -> Self {
        Self {
            items: Arc::new(RwLock::new(IndexMap::new())),
        }
    }
}

impl Default for DefaultConfigurableVisitor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ConfigurableVisitor for DefaultConfigurableVisitor {
    async fn register_configurable(&self, item: VdfsItem) {
        let key = item.node.name.clone();
        self.items.write().await.insert(key, item);
    }

    async fn list_configurables(&self) -> Vec<VdfsItem> {
        self.items.read().await.values().cloned().collect()
    }
}

#[cfg(test)]
#[path = "configurable_visitor.test.rs"]
mod tests;
