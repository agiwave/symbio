//! 插件核心 Trait

use crate::core::types::{InvokeStream, PluginError, PluginMeta, PluginResult};
use serde_json::Value;
use std::sync::{Arc, Weak};

/// 插件能力标识
/// 
/// 用于能力路由，插件声明自己支持的能力
pub const CAPABILITY_LLM: &str = "llm";
pub const CAPABILITY_SESSION: &str = "session";
pub const CAPABILITY_MEMORY: &str = "memory";
pub const CAPABILITY_TOOLS: &str = "tools";
pub const CAPABILITY_TELEGRAM: &str = "telegram";
pub const CAPABILITY_DOCKER: &str = "docker";

/// 插件接口定义
/// 
/// 每个插件都是一个完整的主体，通过 path 参数支持分形嵌套。
/// - 空路径 (""): 操作插件自身
/// - 非空路径 ("child/grandchild"): 逐级查找并操作子插件
/// - 能力路由 ("@capability"): 通过能力标识查找并调用插件
#[async_trait::async_trait]
pub trait Plugin: Send + Sync {
    /// 获取插件元数据
    fn meta(&self, path: &str) -> PluginResult<PluginMeta> {
        let _ = path;
        Err(PluginError::NotImplemented)
    }

    /// 调用插件
    fn invoke(&self, path: &str, input: Value) -> PluginResult<InvokeStream> {
        let _ = (path, input);
        Err(PluginError::NotImplemented)
    }

    /// 获取插件能力列表
    /// 
    /// 返回插件支持的能力标识数组，用于能力路由
    fn capabilities(&self) -> Vec<&'static str> {
        Vec::new()
    }
}

/// 插件工厂接口
#[async_trait::async_trait]
pub trait PluginFactory: Send + Sync {
    /// 获取工厂元数据
    fn meta(&self) -> PluginMeta;

    /// 创建插件实例
    ///
    /// parent: 父插件弱引用，用于插件间协作（避免循环引用）
    /// config: 可选的配置参数
    fn create(&self, parent: Option<Weak<dyn Plugin>>, config: Option<&Value>) -> Arc<dyn Plugin>;
}

/// 插件父引用辅助结构
/// 
/// 用于子插件存储父插件的弱引用
pub struct ParentRef {
    parent: Weak<dyn Plugin>,
}

impl ParentRef {
    pub fn new(parent: &Arc<dyn Plugin>) -> Self {
        Self {
            parent: Arc::downgrade(parent),
        }
    }

    /// 获取父插件引用
    /// 
    /// 如果父插件已销毁则返回 None
    pub fn get(&self) -> Option<Arc<dyn Plugin>> {
        self.parent.upgrade()
    }
}