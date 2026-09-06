//! 通用存储服务抽象（symbio_core 层）
//!
//! 所有插件通过 `dyn StorageService` 和 `dyn EntityStore` 访问存储，
//! **不**直接引用 `crate::providers::storage_service::FileEntityStore` 等具体实现。

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
/// 分类名由调用方定义，本 trait 不绑定具体业务分类。
#[async_trait]
pub trait EntityStore: Send + Sync {
    /// 列出指定分类下的全部实体 ID
    async fn list_entities(&self, category: &str) -> Result<Vec<String>, EntityStoreError>;

    /// 读取指定实体的 manifest 文件原始内容（字符串）
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
    async fn delete_entity(&self, category: &str, id: &str) -> Result<(), EntityStoreError>;

    /// 判断指定实体是否存在
    async fn entity_exists(&self, category: &str, id: &str) -> Result<bool, EntityStoreError>;

    /// 获取指定实体的子目录路径
    fn entity_dir(&self, category: &str, id: &str) -> std::path::PathBuf;

    /// 获取指定实体的 manifest 文件完整路径
    fn entity_file(&self, category: &str, id: &str, manifest_file: &str) -> std::path::PathBuf;

    /// 获取分类根目录
    fn category_dir(&self, category: &str) -> std::path::PathBuf;
}

/// 通用存储服务接口
///
/// 通过工厂模式创建：`create_object::<dyn StorageService>("storage_service", ctx)`。
/// 业务模块**只**通过 `dyn StorageService` 引用，**不**直接 `use` 具体实现类型。
pub trait StorageService: Send + Sync {
    /// 获取底层 EntityStore（按分类/ID 操作）
    fn entity_store(&self) -> &dyn EntityStore;

    /// 获取基础目录
    fn base(&self) -> &std::path::Path;
}

/// 类别分类常量
///
/// **目录名与插件名保持一致**：
/// - `model`（原 `ai`）— Model Provider 分类
/// - `mcp` — MCP Server 分类
/// - `skill` — Skill 分类
/// - `channel` — Channel（渠道）分类
/// - `agent` — Agent 分类
/// - `session` — Session 分类
///
/// 这样在 `~/.symbio/plugins/` 下，目录名直接对应插件名，
/// 遍历 `~/.symbio/plugins/` 即可知道加载了哪些插件。
///
/// **向后兼容**：旧代码中 `ai` 分类的数据会被自动迁移到 `model` 分类。
pub mod categories {
    /// Model Provider 分类（原 `ai`，重命名以贴合插件名）
    pub const MODEL: &str = "model";
    /// MCP Server 分类
    pub const MCP: &str = "mcp";
    /// Skill 分类
    pub const SKILL: &str = "skill";
    /// Channel（渠道）分类
    pub const CHANNEL: &str = "channel";
    /// Agent 分类
    pub const AGENT: &str = "agent";
    /// Session 分类
    pub const SESSION: &str = "session";
}

/// 各分类的 manifest 文件名常量
pub mod manifests {
    pub const PROVIDER: &str = "provider.json";
    pub const SERVER: &str = "server.json";
    pub const SKILL: &str = "SKILL.md";
    pub const CHANNEL: &str = "channel.json";
}
