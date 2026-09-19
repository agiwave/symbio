//! 插件核心 Trait（上下文注入版）

use crate::symbio_core::SymbioKey;
use crate::symbio_core::{lock_read, lock_write, InvokeResponse, PluginPayload};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::any::Any;
use std::collections::HashMap;
use std::sync::{Arc, RwLock, Weak};

/// 插件元数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginMeta {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub version: Option<String>,
    pub author: Option<String>,
}

impl PluginMeta {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            description: None,
            version: None,
            author: None,
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    pub fn with_author(mut self, author: impl Into<String>) -> Self {
        self.author = Some(author.into());
        self
    }
}

// InvokeRequest Trait & SimpleRequest

/// 插件上下文接口 - Symbio 架构的“血液”
///
/// 采用类型擦除模式，支持跨层级的能力注入与透传
pub trait InvokeRequest: Send + Sync {
    /// 内部原始提取器
    fn get_raw(&self, key: &str) -> Option<Arc<dyn Any + Send + Sync>>;

    /// 内部原始设置器，用于支持上下文变异
    fn set_raw(&self, key: &str, value: Arc<dyn Any + Send + Sync>);

    /// 克隆一个独立的上下文副本 (用于转发请求而不影响当前链路)
    fn fork(&self) -> Arc<dyn InvokeRequest>;

    /// 支持向下转型 (用于在分形层级中获取具体实现)
    fn as_any(&self) -> &dyn Any;
}

/// 插件上下文扩展 - 为开发者提供优雅的类型安全 API
pub trait InvokeRequestExt: InvokeRequest {
    /// 获取特定键的值或接口
    fn get<K: SymbioKey>(&self, key: K) -> Option<K::Value> {
        self.get_raw(key.name())
            .and_then(|any| any.downcast::<K::Value>().ok())
            .map(|arc| (*arc).clone())
    }

    /// 设置特定键的值或接口
    fn set<K: SymbioKey>(&self, key: K, value: K::Value) {
        self.set_raw(key.name(), Arc::new(value));
    }

    /// 获取父插件引用 (Weak)
    fn parent(&self) -> Option<std::sync::Weak<dyn crate::symbio_core::Plugin>> {
        self.get(crate::symbio_core::PARENT).flatten()
    }

    /// 获取配置信息 (Value)
    fn config(&self) -> Option<serde_json::Value> {
        self.get(crate::symbio_core::CONFIG)
    }

    /// 直接将上下文中的 PAYLOAD 解析为指定的强类型 T（进程内零拷贝）
    ///
    /// 优先尝试原生类型转换，失败时自动回退到 JSON 反序列化
    #[allow(deprecated)]
    fn payload<T: serde::de::DeserializeOwned + Clone + Send + Sync + 'static>(
        &self,
    ) -> Result<T, crate::symbio_core::PluginError> {
        let key = "payload";
        let any = self.get_raw(key).ok_or_else(|| {
            crate::symbio_core::PluginError::ValidationError(
                "Missing payload in context".to_string(),
            )
        })?;

        // 优先尝试直接类型转换（零拷贝）
        if let Ok(val) = any.clone().downcast::<T>() {
            return Ok((*val).clone());
        }

        // 尝试作为 JSON Value 反序列化
        if let Ok(json_val) = any.downcast::<serde_json::Value>() {
            return serde_json::from_value((*json_val).clone()).map_err(|e| {
                crate::symbio_core::PluginError::ValidationError(format!(
                    "参数反序列化失败，请核对契约类型或 Schema: {e}"
                ))
            });
        }

        Err(crate::symbio_core::PluginError::ValidationError(
            "Payload type mismatch".to_string(),
        ))
    }

    /// 设置上下文中的载荷数据（进程内零拷贝）
    ///
    /// 只保留**进程内零拷贝**这一种写入口（`Arc` 直存扩展桶），刻意不再提供
    /// "先 `serde_json::to_value` 再存"的旧版本：写入侧强加 `Serialize` 会挡掉那些
    /// 本就不该被序列化的进程内对象，并让每次转发白付一次序列化开销。序列化路径
    /// 放在读取侧——[`InvokeRequestExt::payload`] 自带"原生 downcast → JSON 反序列化"
    /// 的两段回退，需要它的地方才付这个代价。
    fn set_payload<T: Clone + Send + Sync + 'static>(
        &self,
        value: T,
    ) -> Result<(), crate::symbio_core::PluginError> {
        let key = "payload";
        self.set_raw(key, Arc::new(value));
        Ok(())
    }
}

impl<T: InvokeRequest + ?Sized> InvokeRequestExt for T {}

/// 标准上下文实现
pub struct SimpleRequest {
    /// 环境变量桶
    pub envs: Arc<RwLock<HashMap<String, String>>>,
    /// 万能扩展桶 (用于进程内透传复杂 Rust 对象，如 Payload, Metadata, PARENT, CONFIG 等)
    pub extensions: Arc<RwLock<HashMap<String, Arc<dyn Any + Send + Sync>>>>,
}

impl SimpleRequest {
    pub fn new(parent: Option<Weak<dyn Plugin>>, config: Option<Value>) -> Self {
        let mut extensions = HashMap::new();
        if let Some(p) = parent {
            extensions.insert(
                crate::symbio_core::PARENT.name().to_string(),
                Arc::new(Some(p)) as Arc<dyn Any + Send + Sync>,
            );
        }
        if let Some(c) = config {
            extensions.insert(
                crate::symbio_core::CONFIG.name().to_string(),
                Arc::new(c) as Arc<dyn Any + Send + Sync>,
            );
        }
        Self {
            envs: Arc::new(RwLock::new(HashMap::new())),
            extensions: Arc::new(RwLock::new(extensions)),
        }
    }

    /// 派生一个子上下文：`parent` 是子上下文的父引用，环境变量从 `from` 快照一份。
    ///
    /// 收口三处装配点的重复写法（`composite` 挂子插件、`home` 造 worker、`agent`
    /// 造子 Agent）。注意这两个参数**常常不是同一个对象**：容器挂子插件时父引用是
    /// 容器自己，而环境变量要顺着**外层请求上下文**继续往下传。
    ///
    /// 子上下文拿到的是环境的**副本**而非共享句柄——各子插件此后互不影响。
    /// `from` 若并非标准上下文实现，则无环境可继承（子上下文 envs 为空）。
    pub fn child_of(from: &Arc<dyn InvokeRequest>, parent: Option<Weak<dyn Plugin>>) -> Self {
        let child = Self::new(parent, None);
        if let Some(src) = from.as_any().downcast_ref::<SimpleRequest>() {
            *lock_write(&child.envs) = lock_read(&src.envs).clone();
        }
        child
    }
}

impl InvokeRequest for SimpleRequest {
    fn get_raw(&self, key: &str) -> Option<Arc<dyn Any + Send + Sync>> {
        // 1. 优先从扩展桶获取 (包括存储在此的 Payload、Metadata 等所有临时/高级对象)
        if let Ok(exts) = self.extensions.read() {
            if let Some(val) = exts.get(key) {
                return Some(Arc::clone(val));
            }
        }

        // 2. 兜底从环境变量获取 (Envs)
        if let Ok(envs) = self.envs.read() {
            if let Some(val) = envs.get(key) {
                return Some(Arc::new(val.clone()));
            }
        }

        None
    }

    fn set_raw(&self, key: &str, value: Arc<dyn Any + Send + Sync>) {
        if let Ok(mut exts) = self.extensions.write() {
            exts.insert(key.to_string(), value);
        }
    }

    fn fork(&self) -> Arc<dyn InvokeRequest> {
        Arc::new(Self {
            envs: Arc::clone(&self.envs),
            extensions: Arc::new(RwLock::new(lock_read(&self.extensions).clone())),
        })
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[async_trait]
pub trait Plugin: Send + Sync + 'static {
    /// 插件元信息（身份声明）。
    ///
    /// ⚠️ 目前**没有任何运行期消费方**：全仓 `.meta()` 的调用点命中的都是
    /// `Capability::meta`（能力，而非插件）；插件的身份实际来自各自的 `PLUGIN.yml`
    /// （见 `plugin_dir`），配置面板读的也是那一份。
    ///
    /// 之所以仍留在 trait 上，是**刻意的决定而非遗忘**：它是插件契约的一部分
    /// （每个 impl 都应能自报身份），删掉会波及全部插件实现，并从公开 trait 上
    /// 摘掉一项能力。若将来确认要删，请连同各 impl 一并清理，不要只删这一行声明。
    fn meta(&self) -> PluginMeta;

    /// 分形路由入口：接收一个抽象的上下文对象，按需提取参数
    async fn route(self: Arc<Self>, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload>;

    /// 分形遍历接口
    async fn traverse(
        self: Arc<Self>,
        path: String,
        ctx: Arc<dyn InvokeRequest>,
    ) -> InvokeResponse<PluginPayload>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_env(pairs: &[(&str, &str)]) -> Arc<SimpleRequest> {
        let ctx = SimpleRequest::new(None, None);
        for (k, v) in pairs {
            lock_write(&ctx.envs).insert((*k).to_string(), (*v).to_string());
        }
        Arc::new(ctx)
    }

    fn env_of(ctx: &dyn InvokeRequest, key: &str) -> Option<String> {
        ctx.get_raw(key)
            .and_then(|v| v.downcast::<String>().ok())
            .map(|s| (*s).clone())
    }

    /// `child_of` 继承父环境，且拿到的是**快照**而非共享句柄——改子不影响父。
    /// 这是三处装配点（composite / home / agent）收口后的语义契约。
    #[test]
    fn child_of_snapshots_envs_without_sharing() {
        let parent = with_env(&[("A", "1")]);
        let from: Arc<dyn InvokeRequest> = Arc::clone(&parent) as Arc<dyn InvokeRequest>;

        let child = SimpleRequest::child_of(&from, None);
        assert_eq!(env_of(&child, "A").as_deref(), Some("1"), "应继承父环境");

        lock_write(&child.envs).insert("B".into(), "2".into());
        assert_eq!(env_of(&child, "B").as_deref(), Some("2"));
        assert_eq!(
            env_of(parent.as_ref(), "B"),
            None,
            "子上下文的写入不得回流到父"
        );
    }

    /// 来源不是标准上下文时无环境可继承，且不得 panic。
    #[test]
    fn child_of_tolerates_non_standard_source() {
        struct Opaque;
        impl InvokeRequest for Opaque {
            fn get_raw(&self, _key: &str) -> Option<Arc<dyn Any + Send + Sync>> {
                None
            }
            fn set_raw(&self, _key: &str, _value: Arc<dyn Any + Send + Sync>) {}
            fn fork(&self) -> Arc<dyn InvokeRequest> {
                Arc::new(Opaque)
            }
            fn as_any(&self) -> &dyn Any {
                self
            }
        }

        let from: Arc<dyn InvokeRequest> = Arc::new(Opaque);
        let child = SimpleRequest::child_of(&from, None);
        assert!(lock_read(&child.envs).is_empty());
    }

    /// `fork` 复制扩展桶：改副本不影响原上下文（转发请求不改写当前链路）。
    #[test]
    fn fork_copies_extensions_without_sharing() {
        let ctx = SimpleRequest::new(None, None);
        ctx.set_raw("k", Arc::new("v".to_string()));

        let forked = ctx.fork();
        assert_eq!(env_of(forked.as_ref(), "k").as_deref(), Some("v"));

        ctx.set_raw("k", Arc::new("changed".to_string()));
        assert_eq!(
            env_of(forked.as_ref(), "k").as_deref(),
            Some("v"),
            "fork 之后原上下文的变化不应影响副本"
        );
    }
}
