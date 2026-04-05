//! Symbio 初始化模块
//!
//! 提供简化的 API 来创建和配置整个插件系统
//! Tauri 层只需调用一个函数即可获得完整的 root plugin

use crate::symbio_core::{PluginFactoryRegistry, PluginFactory, Plugin};
use crate::symbio_core::event::OptionalEventSender;
use crate::plugins::{
    HomeFactory, WorkFactory, NoteFactory, SettingFactory,
    AgentFactory, ChatFactory, ToolsFactory, MemoryFactory,
    SessionFactory, TelegramFactory, OpenAiFactory,
    EchoFactory, DockerFactory, CompositeFactory,
    ExplorerFactory,
};
use std::sync::Arc;

/// 创建完整的 root plugin
///
/// 这个函数会：
/// 1. 初始化全局工厂注册表
/// 2. 注册所有内置插件工厂
/// 3. 创建并返回 root plugin (Home)
///
/// # 参数
/// - `event_sender`: 可选的事件发送器，用于插件向宿主发送事件
///                   Explorer 插件会使用此发送器来发送文件变化事件
///
/// # 返回
/// 返回配置完整的 root plugin，可以直接用于 Tauri 应用
///
/// # 示例
/// ```rust
/// use symbio::create_root_plugin;
/// use symbio::OptionalEventSender;
///
/// let root = create_root_plugin(OptionalEventSender::new(None));
/// ```
pub fn create_root_plugin(event_sender: OptionalEventSender) -> Arc<dyn Plugin> {
    // 初始化全局工厂注册表
    PluginFactoryRegistry::init();
    let registry = PluginFactoryRegistry::global();

    // 注册所有不依赖外部资源的工厂
    registry.register(Arc::new(WorkFactory::new()));
    registry.register(Arc::new(NoteFactory::new()));
    registry.register(Arc::new(SettingFactory::new()));
    registry.register(Arc::new(AgentFactory::new()));
    registry.register(Arc::new(ChatFactory::new()));
    registry.register(Arc::new(ToolsFactory::new()));
    registry.register(Arc::new(MemoryFactory::new()));
    registry.register(Arc::new(SessionFactory::new()));
    registry.register(Arc::new(TelegramFactory::new()));
    registry.register(Arc::new(OpenAiFactory::new()));
    registry.register(Arc::new(EchoFactory::new()));
    registry.register(Arc::new(DockerFactory::new()));
    registry.register(Arc::new(CompositeFactory::with_defaults()));
    
    // 注册 ExplorerFactory（使用传入的事件发送器）
    registry.register(Arc::new(ExplorerFactory::new(event_sender)));

    // 创建并返回 root plugin
    HomeFactory::new().create(None, None)
}
