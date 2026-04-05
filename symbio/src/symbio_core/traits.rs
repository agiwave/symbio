//! 插件核心 Trait

use crate::symbio_core::types::{Connection, InvokeStream, PluginError, PluginMeta, PluginResult};
use serde_json::Value;
use std::sync::{Arc, Weak};

/// 插件能力标识
///
/// 用于能力路由，插件声明自己支持的能力
pub const CAPABILITY_LLM: &str = "llm";

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

    /// 建立持久连接
    ///
    /// 用于双向通信场景。插件通过 `Connection` 句柄实现：
    /// - 向客户端发送消息：`conn.send(data)`
    /// - 接收客户端消息：`conn.on_message(handler)`
    /// - 连接状态管理：`conn.is_closed()`, `conn.close(reason)`
    /// - 连接级状态存储：`conn.state()`
    async fn connect(
        &self,
        path: &str,
        input: Value,
        conn: Connection,
    ) -> PluginResult<()> {
        let _ = (path, input, conn);
        Err(PluginError::NotImplemented)
    }

    /// 获取插件能力列表
    ///
    /// 返回插件支持的能力标识数组，用于能力路由
    fn capabilities(&self) -> Vec<&'static str> {
        Vec::new()
    }

    /// 获取可用工具列表（分形递归）
    ///
    /// 返回插件提供的所有可用工具的 PluginMeta。
    /// - 容器插件（如 Agent）：递归收集所有子插件的工具
    /// - 叶子插件（如 Tools）：返回自己提供的工具的 meta
    fn available_tools(&self) -> Vec<PluginMeta> {
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