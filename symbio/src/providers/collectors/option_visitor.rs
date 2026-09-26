//! 默认选项收集器 —— [`OptionVisitor`] 的内存实现
//!
//! ## 为什么在 `providers/` 而不是 `symbio_core`
//!
//! 本类型的**写入者是全体插件**（在 `traverse` 里经 `ctx` 的 `OPTION_VISITOR` 键贡献
//! 选项字段），安装它的宿主（session 的 `collect_options`）只是其中之一。
//! 它不是任何插件的内部物——完整判据见 [`crate::providers::collectors`] 的模块文档。
//!
//! ## `order` 只影响收集层排序
//!
//! 注册时带的 `order` 号用于**收集层**排序（`order` 相同者保持注册顺序），
//! 不出现在产物里——`list_option_fields` 只回字段本身。

use crate::symbio_core::schemas::detail::DetailField;
use crate::symbio_core::OptionVisitor;
use async_trait::async_trait;
use indexmap::IndexMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// 默认选项收集器：内存 IndexMap 实现，一次收集一个实例。
pub struct DefaultOptionVisitor {
    fields: Arc<RwLock<IndexMap<String, (i32, DetailField)>>>,
}

impl DefaultOptionVisitor {
    pub fn new() -> Self {
        Self {
            fields: Arc::new(RwLock::new(IndexMap::new())),
        }
    }
}

impl Default for DefaultOptionVisitor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl OptionVisitor for DefaultOptionVisitor {
    async fn register_option_field(&self, order: i32, field: DetailField) {
        let key = field.key.clone();
        let mut fields = self.fields.write().await;
        fields.insert(key, (order, field));
    }

    async fn list_option_fields(&self) -> Vec<DetailField> {
        let fields = self.fields.read().await;
        let mut out: Vec<(i32, DetailField)> = fields.values().cloned().collect();
        // 稳定排序：order 相同者保持注册顺序（IndexMap 保序）
        out.sort_by_key(|(order, _)| *order);
        out.into_iter().map(|(_, field)| field).collect()
    }
}

#[cfg(test)]
#[path = "option_visitor.test.rs"]
mod tests;
