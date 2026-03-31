//! Home 插件工厂 - 创建根插件，并组装子插件
//!
//! 创建自引用链条：
//! Home → Agent (parent=Home) → openai (parent=Agent)
//! 这样 openai 可以通过 parent 链条调用 Home 的 save_config

use crate::core::traits::{Plugin, PluginFactory};
use crate::core::types::PluginMeta;
use super::HomePlugin;
use crate::plugins::work::WorkFactory;
use crate::plugins::agent::Agent;
use crate::plugins::setting::SettingFactory;
use std::sync::{Arc, Weak};
use serde_json::Value;

pub struct HomeFactory;

impl HomeFactory {
    pub fn new() -> Self {
        HomeFactory
    }
}

#[async_trait::async_trait]
impl PluginFactory for HomeFactory {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "home".to_string(),
            description: "Symbio 主插件".to_string(),
            version: "0.1.0".to_string(),
            input: None,
            output: None,
            author: Some("Symbio Team".to_string()),
        }
    }

    fn create(&self, _parent: Option<Arc<dyn Plugin>>, _config: Option<&Value>) -> Arc<dyn Plugin> {
        // 先创建一个临时的 HomePlugin（不包含 agent）
        let work = WorkFactory::new().create(None, None);
        let setting = SettingFactory::new().create(None, None);
        
        // 使用 Arc::new_cyclic 创建 HomePlugin，同时传递自引用给 Agent
        Arc::new_cyclic(|home_weak| {
            // home_weak 是 &Weak<HomePlugin>
            // 我们需要创建一个 Weak<dyn Plugin> 来传递给 Agent
            let home_weak_dyn: Weak<dyn Plugin> = home_weak.clone() as Weak<dyn Plugin>;
            
            // 创建 Agent，parent 指向 Home
            let agent = Arc::new(Agent::with_parent(home_weak_dyn.upgrade()));
            
            HomePlugin::new(work.clone(), agent, setting.clone())
        })
    }
}