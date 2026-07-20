//! 文件系统实现的实体存储
//!
//! `FileEntityStore` 实现了 `crate::symbio_core::providers::storage::EntityStore`。
//!
//! 按 `<base>/<category>/<id>/<manifest_file>` 组织。
//!
//! ## 特性
//!
//! - 原子写入：临时文件 + rename
//! - 异步 I/O
//! - 自动创建父目录
//! - 删除时递归清理子目录

use crate::providers::storage_service::path_resolver::safe_id;
use crate::symbio_core::providers::{EntityStore, EntityStoreError};
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use tokio::fs;

/// 文件系统实现的实体存储
pub struct FileEntityStore {
    /// 根目录（如 `~/.symbio/`）
    base: PathBuf,
}

impl FileEntityStore {
    pub fn new(base: PathBuf) -> Self {
        Self { base }
    }

    /// 获取基础目录引用
    pub fn base(&self) -> &Path {
        &self.base
    }
}

#[async_trait]
impl EntityStore for FileEntityStore {
    async fn list_entities(&self, category: &str) -> Result<Vec<String>, EntityStoreError> {
        let cat_dir = self.category_dir(category);
        if !cat_dir.exists() {
            return Ok(Vec::new());
        }
        let mut entries = fs::read_dir(&cat_dir)
            .await
            .map_err(|e| EntityStoreError::Io(e.to_string()))?;
        let mut ids = Vec::new();
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| EntityStoreError::Io(e.to_string()))?
        {
            let file_type = entry
                .file_type()
                .await
                .map_err(|e| EntityStoreError::Io(e.to_string()))?;
            if file_type.is_dir() {
                if let Some(name) = entry.file_name().to_str() {
                    ids.push(name.to_string());
                }
            }
        }
        ids.sort();
        Ok(ids)
    }

    async fn read_entity(
        &self,
        category: &str,
        id: &str,
        manifest_file: &str,
    ) -> Result<String, EntityStoreError> {
        let path = self.entity_file(category, id, manifest_file);
        if !path.exists() {
            return Err(EntityStoreError::NotFound {
                category: box_str(category),
                id: id.to_string(),
            });
        }
        fs::read_to_string(&path)
            .await
            .map_err(|e| EntityStoreError::Io(e.to_string()))
    }

    async fn write_entity(
        &self,
        category: &str,
        id: &str,
        manifest_file: &str,
        content: &str,
    ) -> Result<(), EntityStoreError> {
        let dir = self.entity_dir(category, id);
        fs::create_dir_all(&dir)
            .await
            .map_err(|e| EntityStoreError::Io(e.to_string()))?;

        let final_path = self.entity_file(category, id, manifest_file);

        // 临时文件名：<name>.tmp
        let mut tmp_name = final_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("manifest")
            .to_string();
        tmp_name.push_str(".tmp");
        let tmp_path = final_path.with_file_name(tmp_name);

        // 1. 写入临时文件
        fs::write(&tmp_path, content)
            .await
            .map_err(|e| EntityStoreError::Io(e.to_string()))?;

        // 2. 原子重命名
        if let Err(e) = fs::rename(&tmp_path, &final_path).await {
            // 清理临时文件
            let _ = fs::remove_file(&tmp_path).await;
            return Err(EntityStoreError::Io(e.to_string()));
        }
        Ok(())
    }

    async fn delete_entity(&self, category: &str, id: &str) -> Result<(), EntityStoreError> {
        let dir = self.entity_dir(category, id);
        if !dir.exists() {
            return Err(EntityStoreError::NotFound {
                category: box_str(category),
                id: id.to_string(),
            });
        }
        fs::remove_dir_all(&dir)
            .await
            .map_err(|e| EntityStoreError::Io(e.to_string()))
    }

    async fn entity_exists(&self, category: &str, id: &str) -> Result<bool, EntityStoreError> {
        let dir = self.entity_dir(category, id);
        Ok(dir.exists() && dir.is_dir())
    }

    fn entity_dir(&self, category: &str, id: &str) -> PathBuf {
        self.category_dir(category).join(safe_id(id))
    }

    fn entity_file(&self, category: &str, id: &str, manifest_file: &str) -> PathBuf {
        self.entity_dir(category, id).join(manifest_file)
    }

    fn category_dir(&self, category: &str) -> PathBuf {
        // 所有插件实体都存放在 ~/.symbio/plugins/<category>/<id>/ 之下，
        // 便于通过遍历 ~/.symbio/plugins/ 即可知道加载了哪些插件。
        self.base.join("plugins").join(category)
    }
}

/// 把 &str 转换为 'static 借用，仅用于错误信息
fn box_str(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}
