//! 插件核心 Trait (V3.0 Final - 上下文注入版)

use crate::symbio_core::SymbioKey;
use crate::symbio_core::{InvokeResponse, PluginPayload};
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
    fn set_payload<T: Clone + Send + Sync + 'static>(
        &self,
        value: T,
    ) -> Result<(), crate::symbio_core::PluginError> {
        let key = "payload";
        self.set_raw(key, Arc::new(value));
        Ok(())
    }

    // /// 设置上下文中的载荷数据 (强类型序列化)
    // #[allow(deprecated)]
    // fn set_payload<T: serde::Serialize>(
    //     &self,
    //     value: T,
    // ) -> Result<(), crate::symbio_core::PluginError> {
    //     let val = serde_json::to_value(value).map_err(|e| {
    //         crate::symbio_core::PluginError::InternalError(format!("Payload 序列化失败: {e}"))
    //     })?;
    //     self.set(crate::symbio_core::PAYLOAD, val);
    //     Ok(())
    // }
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
            extensions: Arc::new(RwLock::new(self.extensions.read().unwrap().clone())),
        })
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[async_trait]
pub trait Plugin: Send + Sync + 'static {
    /// 获取插件元信息
    fn meta(&self) -> PluginMeta;

    /// [重构] 分形路由入口 (V3.0)
    /// 接收一个抽象的上下文对象，按需提取参数
    async fn route(self: Arc<Self>, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload>;

    /// [重构] 分形遍历接口 (V3.0)
    async fn traverse(
        self: Arc<Self>,
        path: String,
        ctx: Arc<dyn InvokeRequest>,
    ) -> InvokeResponse<PluginPayload>;
}
