//! Home 插件工厂 - 创建根插件，并组装子插件
//!
//! 使用 Factory 机制创建所有子插件：
//! Home → Agent (parent=Home) → openai/session/memory/tools/chat (parent=Agent)

use crate::core::traits::{Plugin, PluginFactory};
use crate::core::types::PluginMeta;
use crate::core::PluginFactoryRegistry;
use super::HomePlugin;
use crate::plugins::agent::Agent;
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
        let registry = PluginFactoryRegistry::global();
        
        // 使用 Factory 创建 work 和 setting（不需要父引用）
        let work = registry.get("work")
            .map(|f| f.create(None, None))
            .unwrap_or_else(|| Arc::new(crate::plugins::work::WorkPlugin::new(None)));
        
        let setting = registry.get("setting")
            .map(|f| f.create(None, None))
            .unwrap_or_else(|| Arc::new(crate::plugins::setting::SettingPlugin::new()));
        
        // 使用 Arc::new_cyclic 创建 HomePlugin，同时传递自引用给 Agent
        let home = Arc::new_cyclic(|home_weak| {
            let home_weak_dyn: Weak<dyn Plugin> = home_weak.clone() as Weak<dyn Plugin>;
            
            // 使用 Factory 机制创建 Agent 及其子插件
            let agent = create_agent_with_factory(home_weak_dyn);
            
            HomePlugin::new(work.clone(), agent, setting.clone())
        });
        
        home as Arc<dyn Plugin>
    }
}

/// 使用 Factory 机制创建 Agent 及其子插件
fn create_agent_with_factory(home_weak: Weak<dyn Plugin>) -> Arc<dyn Plugin> {
    let registry = PluginFactoryRegistry::global();
    
    // 创建 Agent
    let agent = Arc::new(Agent::new());
    let agent_ref: Arc<dyn Plugin> = agent.clone();
    
    // 设置父引用（指向 Home）
    agent.set_parent(home_weak);
    
    // 注册内置管理插件（不需要父引用）
    agent.add_instance("add".to_string(), Arc::new(crate::plugins::agent::AddPlugin::new()) as Arc<dyn Plugin>);
    agent.add_instance("remove".to_string(), Arc::new(crate::plugins::agent::RemovePlugin::new()) as Arc<dyn Plugin>);
    
    // 创建 Chat 插件
    eprintln!("[home] creating chat plugin...");
    if let Some(factory) = registry.get("chat") {
        agent.add_instance("chat".to_string(), factory.create(Some(agent_ref.clone()), None));
        eprintln!("[home] chat plugin created");
    } else {
        eprintln!("[home] ERROR: chat factory not found!");
    }
    
    // 创建 Tools 插件
    eprintln!("[home] creating tools plugin...");
    if let Some(factory) = registry.get("tools") {
        agent.add_instance("tools".to_string(), factory.create(Some(agent_ref.clone()), None));
        eprintln!("[home] tools plugin created");
    } else {
        eprintln!("[home] ERROR: tools factory not found!");
    }
    
    // 创建 Memory 插件
    eprintln!("[home] creating memory plugin...");
    if let Some(factory) = registry.get("memory") {
        agent.add_instance("memory".to_string(), factory.create(Some(agent_ref.clone()), None));
        eprintln!("[home] memory plugin created");
    } else {
        eprintln!("[home] ERROR: memory factory not found!");
    }
    
    // 创建 Session 插件
    eprintln!("[home] creating session plugin...");
    if let Some(factory) = registry.get("session") {
        agent.add_instance("session".to_string(), factory.create(Some(agent_ref.clone()), None));
        eprintln!("[home] session plugin created");
    } else {
        eprintln!("[home] ERROR: session factory not found!");
    }
    
    // 创建 OpenAI 插件
    eprintln!("[home] creating openai plugin...");
    if let Some(factory) = registry.get("openai") {
        agent.add_instance("openai".to_string(), factory.create(Some(agent_ref.clone()), None));
        eprintln!("[home] openai plugin created");
    } else {
        eprintln!("[home] ERROR: openai factory not found!");
    }
    
    // 创建 Telegram 插件
    eprintln!("[home] creating telegram plugin...");
    if let Some(factory) = registry.get("telegram") {
        agent.add_instance("telegram".to_string(), factory.create(Some(agent_ref.clone()), None));
        eprintln!("[home] telegram plugin created");
    } else {
        eprintln!("[home] ERROR: telegram factory not found!");
    }
    
    // 创建 Docker 插件
    eprintln!("[home] creating docker plugin...");
    if let Some(factory) = registry.get("docker") {
        agent.add_instance("docker".to_string(), factory.create(Some(agent_ref.clone()), None));
        eprintln!("[home] docker plugin created");
    } else {
        eprintln!("[home] ERROR: docker factory not found!");
    }
    
    eprintln!("[home] agent creation complete");
    agent_ref
}