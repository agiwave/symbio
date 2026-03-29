//! 插件工厂注册表
//! 
//! 全局单例，管理所有插件工厂

use crate::core::traits::PluginFactory;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

/// 插件工厂注册表（全局单例）
pub struct PluginFactoryRegistry {
    factories: RwLock<HashMap<String, Arc<dyn PluginFactory>>>,
}

impl PluginFactoryRegistry {
    /// 创建新的注册表（私有，仅由 init 调用）
    fn new() -> Self {
        PluginFactoryRegistry {
            factories: RwLock::new(HashMap::new()),
        }
    }
    
    /// 初始化全局注册表（仅调用一次）
    pub fn init() {
        let _ = GLOBAL.set(PluginFactoryRegistry::new());
    }
    
    /// 获取全局注册表实例
    pub fn global() -> &'static PluginFactoryRegistry {
        GLOBAL.get().expect("PluginFactoryRegistry not initialized, call init() first")
    }
    
    /// 注册工厂
    pub fn register(&self, factory: Arc<dyn PluginFactory>) {
        let name = factory.meta().name.clone();
        self.factories.write().unwrap().insert(name, factory);
    }
    
    /// 获取所有工厂实例
    pub fn list(&self) -> Vec<Arc<dyn PluginFactory>> {
        self.factories.read().unwrap().values().cloned().collect()
    }
}

/// 全局单例
static GLOBAL: OnceLock<PluginFactoryRegistry> = OnceLock::new();
