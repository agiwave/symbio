//! 实体存储 trait
//!
//! 抽象层：每个"实体"对应磁盘上的一个子目录，子目录中有一个或多个文件。
//! 例如 Model Provider 是 `~/.symbio/plugins/model/<id>/provider.json`。
//!
//! ## 用法
//!
//! 插件不直接使用具体实现，而是通过 `StorageService`（工厂获取）拿到
//! `EntityStore` 实例，然后按 category 操作。

use async_trait::async_trait;
use thiserror::Error;

/// 实体存储错误
#[derive(Debug, Error)]
pub enum EntityStoreError {
    #[error("IO 错误: {0}")]
    Io(String),
    #[error("实体不存在: {category}/{id}")]
    NotFound { category: &'static str, id: String },
    #[error("实体已存在: {category}/{id}")]
    AlreadyExists { category: &'static str, id: String },
    #[error("序列化错误: {0}")]
    Serde(String),
    #[error("ID 非法: {0}")]
    InvalidId(String),
    #[error("其他错误: {0}")]
    Other(String),
}

/// 实体存储抽象
///
/// ## 模式约定
///
/// - 实体按 `<base>/<category>/<id>/<manifest_file>` 组织
/// - category 是分类标识（如 "model"、"mcp"、"channel"）
/// - id 是实体在分类内的唯一标识
/// - manifest_file 是该分类的主文件名（如 "provider.json"、"server.json"）
///
/// ## 分类常量
///
/// 分类常量统一在 `crate::symbio_core::providers::categories` 中定义。
#[async_trait]
pub trait EntityStore: Send + Sync {
    /// 列出指定分类下的全部实体 ID
    async fn list_entities(&self, category: &str) -> Result<Vec<String>, EntityStoreError>;

    /// 读取指定实体的 manifest 文件原始内容（字符串）
    ///
    /// 由调用方负责反序列化为对应类型
    async fn read_entity(
        &self,
        category: &str,
        id: &str,
        manifest_file: &str,
    ) -> Result<String, EntityStoreError>;

    /// 写入指定实体的 manifest 文件
    ///
    /// - 若 ID 对应的实体不存在：创建
    /// - 若已存在：覆盖
    /// - 写入采用临时文件 + 原子重命名
    async fn write_entity(
        &self,
        category: &str,
        id: &str,
        manifest_file: &str,
        content: &str,
    ) -> Result<(), EntityStoreError>;

    /// 删除指定实体（递归删除其子目录）
    async fn delete_entity(
        &self,
        category: &str,
        id: &str,
    ) -> Result<(), EntityStoreError>;

    /// 判断指定实体是否存在
    async fn entity_exists(
        &self,
        category: &str,
        id: &str,
    ) -> Result<bool, EntityStoreError>;

    /// 获取指定实体的子目录路径
    ///
    /// - 当 `ensure` 为 true 时，若不存在会自动创建
    fn entity_dir(&self, category: &str, id: &str) -> std::path::PathBuf;

    /// 获取指定实体的 manifest 文件完整路径
    fn entity_file(
        &self,
        category: &str,
        id: &str,
        manifest_file: &str,
    ) -> std::path::PathBuf;

    /// 获取分类根目录
    fn category_dir(&self, category: &str) -> std::path::PathBuf;
}
