//! 插件核心 Trait

use crate::core::types::{StreamChunk, PluginError, PluginMeta, PluginResult};
use serde_json::Value;
use std::sync::Arc;

/// 插件接口定义
#[async_trait::async_trait]
pub trait Plugin: Send + Sync {
    /// 获取插件元数据
    fn meta(&self) -> PluginMeta;
        
    /// 获取子插件（分形模式）
    /// 
    /// path 规则：["name1", "name2", ...] 逐级查找子插件
    fn plugin(&self, path: &[String]) -> Option<Arc<dyn Plugin>>;

    /// 同步调用接口
    async fn invoke(&self, input: Value) -> PluginResult<Value>;

    /// 流式调用接口（可选）
    async fn sinvoke(&self, input: Value) -> PluginResult<Vec<StreamChunk>> {
        let _ = input;
        Err(PluginError::NotImplemented)
    }
}

/// 插件工厂接口
#[async_trait::async_trait]
pub trait PluginFactory: Send + Sync {
    /// 获取工厂元数据
    fn meta(&self) -> PluginMeta;
    
    /// 创建插件实例
    fn create(&self, parent: Option<&dyn Plugin>, config: Option<&Value>) -> Arc<dyn Plugin>;
}
