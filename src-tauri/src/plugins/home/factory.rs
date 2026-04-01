//! Home 插件工厂 - 创建根插件，并组装子插件
//!
//! 简洁设计：
//! - 创建时从配置文件读取配置，传入各工厂构造函数
//! - 用户修改时调用 save_config 保存
//! - 不在初始化时触发任何 save_config

use crate::core::traits::{Plugin, PluginFactory};
use crate::core::types::PluginMeta;
use crate::core::PluginFactoryRegistry;
use super::HomePlugin;
use super::plugin::GlobalConfig;
use crate::plugins::agent::Agent;
use std::sync::{Arc, Weak};
use serde_json::Value;
use std::path::PathBuf;

pub struct HomeFactory;

impl HomeFactory {
    pub fn new() -> Self {
        HomeFactory
    }
    
    /// 配置文件路径
    fn config_path() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".symbio")
            .join("config.yaml")
    }
    
    /// 读取配置文件
    fn load_config_file() -> GlobalConfig {
        let path = Self::config_path();
        if !path.exists() {
            return GlobalConfig::default();
        }
        
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|content| serde_yaml::from_str(&content).ok())
            .unwrap_or_default()
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
        
        // 读取配置文件
        let global_config = Self::load_config_file();
        eprintln!("[home] loaded config from file");
        
        // 使用 Factory 创建 work，传入配置
        let work_config = global_config.plugins.get("work");
        let work = registry.get("work")
            .map(|f| f.create(None, work_config))
            .unwrap_or_else(|| Arc::new(crate::plugins::work::WorkPlugin::new(None)));
        
        let setting_config = global_config.plugins.get("setting");
        let setting = registry.get("setting")
            .map(|f| f.create(None, setting_config))
            .unwrap_or_else(|| Arc::new(crate::plugins::setting::SettingPlugin::new()));
        
        // 使用 Arc::new_cyclic 创建 HomePlugin，同时传递自引用给 Agent
        let home = Arc::new_cyclic(|home_weak| {
            let home_weak_dyn: Weak<dyn Plugin> = home_weak.clone() as Weak<dyn Plugin>;
            
            // 创建 Agent，传入配置
            let agent_config = global_config.plugins.get("agent");
            let agent = create_agent_with_factory(home_weak_dyn, agent_config);
            
            HomePlugin::new_with_config(work.clone(), agent, setting.clone(), global_config.clone())
        });
        
        home as Arc<dyn Plugin>
    }
}

/// 使用 Factory 机制创建 Agent 及其子插件
fn create_agent_with_factory(home_weak: Weak<dyn Plugin>, agent_config: Option<&Value>) -> Arc<dyn Plugin> {
    let registry = PluginFactoryRegistry::global();
    
    // 创建 Agent
    let agent = Arc::new(Agent::new());
    let agent_ref: Arc<dyn Plugin> = agent.clone();
    
    // 设置父引用（指向 Home）
    agent.set_parent(home_weak);
    
    // 注册内置管理插件
    agent.add_instance("add".to_string(), Arc::new(crate::plugins::agent::AddPlugin::new()) as Arc<dyn Plugin>);
    agent.add_instance("remove".to_string(), Arc::new(crate::plugins::agent::RemovePlugin::new()) as Arc<dyn Plugin>);
    
    // 从配置中提取各子插件配置
    let configs = agent_config
        .and_then(|c| c.get("config"))
        .and_then(|c| c.as_object());
    
    // 创建各子插件，传入配置
    eprintln!("[home] creating plugins with config...");
    
    if let Some(factory) = registry.get("chat") {
        let cfg = configs.and_then(|c| c.get("chat"));
        agent.add_instance("chat".to_string(), factory.create(Some(agent_ref.clone()), cfg));
    }
    
    if let Some(factory) = registry.get("tools") {
        let cfg = configs.and_then(|c| c.get("tools"));
        agent.add_instance("tools".to_string(), factory.create(Some(agent_ref.clone()), cfg));
    }
    
    if let Some(factory) = registry.get("memory") {
        let cfg = configs.and_then(|c| c.get("memory"));
        agent.add_instance("memory".to_string(), factory.create(Some(agent_ref.clone()), cfg));
    }
    
    if let Some(factory) = registry.get("session") {
        let cfg = configs.and_then(|c| c.get("session"));
        agent.add_instance("session".to_string(), factory.create(Some(agent_ref.clone()), cfg));
    }
    
    if let Some(factory) = registry.get("openai") {
        let cfg = configs.and_then(|c| c.get("openai"));
        agent.add_instance("openai".to_string(), factory.create(Some(agent_ref.clone()), cfg));
    }
    
    if let Some(factory) = registry.get("telegram") {
        let cfg = configs.and_then(|c| c.get("telegram"));
        agent.add_instance("telegram".to_string(), factory.create(Some(agent_ref.clone()), cfg));
    }
    
    if let Some(factory) = registry.get("docker") {
        let cfg = configs.and_then(|c| c.get("docker"));
        agent.add_instance("docker".to_string(), factory.create(Some(agent_ref.clone()), cfg));
    }
    
    eprintln!("[home] all plugins created");
    agent_ref
}