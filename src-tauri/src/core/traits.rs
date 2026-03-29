//! 插件核心 Trait

use crate::core::types::{InvokeStream, PluginError, PluginMeta, PluginResult};
use serde_json::Value;

/// 插件接口定义
/// 
/// 每个插件都是一个完整的主体，通过 path 参数支持分形嵌套。
/// - 空路径 (""): 操作插件自身
/// - 非空路径 ("child/grandchild"): 逐级查找并操作子插件
#[async_trait::async_trait]
pub trait Plugin: Send + Sync {
    /// 获取插件元数据
    /// 
    /// - path 为空: 返回自身元数据
    /// - path 非空: 返回子插件元数据
    fn meta(&self, path: &str) -> PluginResult<PluginMeta> {
        let _ = path;
        Err(PluginError::NotImplemented)
    }

    /// 调用插件
    /// 
    /// - path 为空: 调用自身
    /// - path 非空: 调用子插件
    /// 
    /// 返回 `InvokeStream`:
    /// - 同步场景: 返回 `InvokeStream::Single`
    /// - 流式场景: 返回 `InvokeStream::Stream`
    fn invoke(&self, path: &str, input: Value) -> PluginResult<InvokeStream> {
        let _ = (path, input);
        Err(PluginError::NotImplemented)
    }
}

/// 插件工厂接口
#[async_trait::async_trait]
pub trait PluginFactory: Send + Sync {
    /// 获取工厂元数据
    fn meta(&self) -> PluginMeta;

    /// 创建插件实例
    fn create(&self, parent: Option<&dyn Plugin>, config: Option<&Value>) -> std::sync::Arc<dyn Plugin>;
}