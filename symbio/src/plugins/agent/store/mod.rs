//! Agent 存储层
//!
//! 所有 store 通过统一的注册机制创建，每个 store 负责自己的内部组装。
//!
//! ## 存储层级
//!
//! ```text
//! build_store(config, agent_dir)  ← 单一异步入口
//!   └─ 通过 config.storage_backend 查找注册表，调用对应工厂
//!
//! Mindscape（默认）
//!   └─ 内部创建 EmbeddingStore
//!       └─ 内部创建基础 store（Dir/File/Memory/Sqlite）
//! ```
//!
//! ## 自注册机制
//!
//! 每个 store 通过 `submit_store_backend!` 宏自注册，装饰器 store 内部自行创建子 store。
//! 工厂函数返回 `BoxFuture` 以支持异步初始化（如 MindscapeScaffold 需要读取 store）。
//!
//! ```ignore
//! impl DirStorage {
//!     pub fn create(config: &AgentConfig, agent_dir: &Path) -> BoxFuture<'static, Arc<dyn AgentStore>> { ... }
//! }
//! submit_store_backend!(StorageBackendType::Dir, DirStorage::create);
//! ```

// 子模块声明（私有，不暴露内部类型）
mod dir;
pub(crate) mod mindscape;
mod sqlite;

use crate::plugins::agent::core::{AgentConfig, StorageBackendType, StorageFormat, StoreError};
use std::collections::HashMap;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};

pub use crate::plugins::agent::core::AgentStore;

/// Boxed future for async store factory functions
pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

// ═══════════════════════════════════════════════════════════════════════════
// StorageFormat 自动检测
// ═══════════════════════════════════════════════════════════════════════════

/// 根据 agent_dir 自动检测存储格式
pub(crate) fn detect_format(agent_dir: &Path) -> StorageFormat {
    if agent_dir.is_file() {
        return format_from_ext(agent_dir);
    }
    for entry in std::fs::read_dir(agent_dir).into_iter().flatten() {
        let Ok(entry) = entry else { continue };
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("identity.") {
            return format_from_ext(&entry.path());
        }
    }
    StorageFormat::Yaml
}

fn format_from_ext(path: &Path) -> StorageFormat {
    match path.extension().and_then(|s| s.to_str()).unwrap_or("yaml") {
        "json" => StorageFormat::Json,
        _ => StorageFormat::Yaml,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// StoreBackend 注册机制
// ═══════════════════════════════════════════════════════════════════════════

/// 存储后端工厂函数类型
// P-13: factory 返回 Result，使错误能向上传播而非 panic。
pub(crate) type StoreFactory =
    fn(&AgentConfig, &Path) -> BoxFuture<Result<Arc<dyn AgentStore>, StoreError>>;

/// 存储后端注册条目
pub(crate) struct StoreBackendEntry {
    pub(crate) backend_type: StorageBackendType,
    pub(crate) factory: StoreFactory,
}

inventory::collect!(StoreBackendEntry);

/// 自注册宏：每个 store 文件末尾调用即可完成注册
#[macro_export]
macro_rules! submit_store_backend {
    ($backend_type:expr, $factory_fn:path) => {
        $crate::symbio_core::inventory::submit! {
            $crate::plugins::agent::store::StoreBackendEntry {
                backend_type: $backend_type,
                factory: $factory_fn,
            }
        }
    };
}

/// 存储后端注册表
struct StoreBackendRegistry {
    backends: HashMap<StorageBackendType, StoreFactory>,
}

static BACKEND_REGISTRY: OnceLock<StoreBackendRegistry> = OnceLock::new();

fn get_backend_registry() -> &'static StoreBackendRegistry {
    BACKEND_REGISTRY.get_or_init(|| {
        let mut backends = HashMap::new();
        for entry in inventory::iter::<StoreBackendEntry> {
            backends.insert(entry.backend_type, entry.factory);
        }
        StoreBackendRegistry { backends }
    })
}

/// 根据 StorageBackendType 查找并调用工厂函数
///
/// 修复 (P-13 防御性): 之前误配置/缺失的 backend 会通过 `unwrap_or_else(|| panic!(...))`
/// 进程级 panic。改为 `Result<...>`，由调用方决定如何降级（返回错误给上层 / 用默认 backend 兜底）。
/// 对应的 `build_store` / `mindscape::build` 同步传播错误。
pub(crate) async fn create_store(
    backend: StorageBackendType,
    config: &AgentConfig,
    agent_dir: &Path,
) -> Result<Arc<dyn AgentStore>, StoreError> {
    let registry = get_backend_registry();
    let factory = registry.backends.get(&backend).ok_or_else(|| {
        StoreError::Backend(format!(
            "未注册的存储后端类型: {:?}，请确认对应的 store 模块已通过 submit_store_backend! 注册",
            backend
        ))
    })?;
    (factory)(config, agent_dir).await
}

// ═══════════════════════════════════════════════════════════════════════════
// 构建入口
// ═══════════════════════════════════════════════════════════════════════════

/// 统一构建入口：根据 config.storage_backend 查找注册表，创建对应的 store
///
/// 所有 store（包括装饰器 store）通过注册表统一创建，
/// 装饰器 store（EmbeddingStore、MindscapeScaffold）内部自行创建子 store。
pub async fn build_store(
    config: &AgentConfig,
    agent_dir: &Path,
) -> Result<Arc<dyn AgentStore>, StoreError> {
    create_store(config.storage_backend, config, agent_dir).await
}

/// 构造一个临时 store（**测试专用**）
///
/// 使用 Sqlite + tempfile 后端（替代已删除的 MemoryStorage）。
/// 返回的 Arc 持有 tempfile::TempDir 的 ownership，drop 时自动清理。
#[cfg(test)]
pub(crate) fn build_in_memory_store() -> Arc<dyn AgentStore> {
    let dir = tempfile::tempdir().expect("failed to create tempdir");
    let storage = sqlite::SqliteStorage::new(dir.path());
    // 将 dir 泄漏为 'static，避免 drop 时删除 db 文件（测试结束自然清理）
    let dir = Box::leak(Box::new(dir));
    let _ = dir; // 保持 alive
    Arc::new(storage)
}

/// 构造完整 MindscapeScaffold 测试栈（NoopEmbedding，无 LLM 依赖）
#[cfg(test)]
pub(crate) async fn build_test_scaffold() -> Arc<dyn AgentStore> {
    use crate::plugins::agent::core::CognitionContext;

    let inner = build_in_memory_store();
    // embedding 已集成到 store 内部，无需装饰器
    let ctx = CognitionContext::new(Arc::new(AgentConfig::default()), std::path::PathBuf::new());
    Arc::new(mindscape::scaffold::MindscapeScaffold::new_with_inner(inner, &ctx).await)
}
