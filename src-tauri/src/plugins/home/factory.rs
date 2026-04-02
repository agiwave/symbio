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

    fn create(&self, _parent: Option<Weak<dyn Plugin>>, _config: Option<&Value>) -> Arc<dyn Plugin> {
        let registry = PluginFactoryRegistry::global();

        // 读取配置文件
        let global_config = Self::load_config_file();
        eprintln!("[home] loaded config from file");

        // 使用 Arc::new_cyclic 创建 HomePlugin，同时传递自引用给子插件
        let home: Arc<dyn Plugin> = Arc::new_cyclic(|home_weak| {
            let home_weak_dyn: Weak<dyn Plugin> = home_weak.clone() as Weak<dyn Plugin>;

            // 使用 Factory 创建各子插件，统一传入 home_weak_dyn 作为 parent
            let work_config = global_config.plugins.get("work");
            let work = registry.get("work")
                .map(|f| f.create(Some(home_weak_dyn.clone()), work_config))
                .unwrap_or_else(|| Arc::new(crate::plugins::work::WorkPlugin::new(Some(home_weak_dyn.clone()))));

            let doc_config = global_config.plugins.get("doc");
            let doc = registry.get("doc")
                .map(|f| f.create(Some(home_weak_dyn.clone()), doc_config))
                .unwrap_or_else(|| Arc::new(crate::plugins::doc::DocPlugin::new(Some(home_weak_dyn.clone()))));

            let setting_config = global_config.plugins.get("setting");
            let setting = registry.get("setting")
                .map(|f| f.create(Some(home_weak_dyn.clone()), setting_config))
                .unwrap_or_else(|| Arc::new(crate::plugins::setting::SettingPlugin::new(Some(home_weak_dyn.clone()))));

            let explorer_config = global_config.plugins.get("explorer");
            let explorer = registry.get("explorer")
                .map(|f| f.create(Some(home_weak_dyn.clone()), explorer_config))
                .unwrap_or_else(|| Arc::new(crate::plugins::explorer::ExplorerPlugin::new(Some(home_weak_dyn.clone()))));

            // 创建 Agent，传入配置
            let agent_config = global_config.plugins.get("agent");
            let agent = create_agent_with_factory(home_weak_dyn, agent_config);

            HomePlugin::new_with_config(work.clone(), doc.clone(), agent, setting.clone(), explorer.clone(), global_config.clone())
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
    agent.set_parent(home_weak.clone());

    // 注册内置管理插件
    agent.add_instance("add".to_string(), Arc::new(crate::plugins::agent::AddPlugin::new()) as Arc<dyn Plugin>);
    agent.add_instance("remove".to_string(), Arc::new(crate::plugins::agent::RemovePlugin::new()) as Arc<dyn Plugin>);

    // agent_config 直接包含各子插件配置（memory, openai, session 等）
    let configs = agent_config.and_then(|c| c.as_object());

    // 创建各子插件，传入 agent_ref 的 Weak（与 home 创建 agent 的方式一致）
    let agent_weak = Arc::downgrade(&agent_ref);
    eprintln!("[home] creating plugins with config...");

    if let Some(factory) = registry.get("chat") {
        let cfg = configs.and_then(|c| c.get("chat"));
        eprintln!("[home] chat config: {:?}", cfg);
        agent.add_instance("chat".to_string(), factory.create(Some(agent_weak.clone()), cfg));
    }

    if let Some(factory) = registry.get("tools") {
        let cfg = configs.and_then(|c| c.get("tools"));
        eprintln!("[home] tools config: {:?}", cfg);
        agent.add_instance("tools".to_string(), factory.create(Some(agent_weak.clone()), cfg));
    }

    if let Some(factory) = registry.get("memory") {
        let cfg = configs.and_then(|c| c.get("memory"));
        eprintln!("[home] memory config: {:?}", cfg);
        agent.add_instance("memory".to_string(), factory.create(Some(agent_weak.clone()), cfg));
    }

    if let Some(factory) = registry.get("session") {
        let cfg = configs.and_then(|c| c.get("session"));
        eprintln!("[home] session config: {:?}", cfg);
        agent.add_instance("session".to_string(), factory.create(Some(agent_weak.clone()), cfg));
    }

    if let Some(factory) = registry.get("openai") {
        let cfg = configs.and_then(|c| c.get("openai"));
        eprintln!("[home] openai config: {:?}", cfg);
        agent.add_instance("openai".to_string(), factory.create(Some(agent_weak.clone()), cfg));
    }

    if let Some(factory) = registry.get("telegram") {
        let cfg = configs.and_then(|c| c.get("telegram"));
        agent.add_instance("telegram".to_string(), factory.create(Some(agent_weak.clone()), cfg));
    }

    if let Some(factory) = registry.get("docker") {
        let cfg = configs.and_then(|c| c.get("docker"));
        agent.add_instance("docker".to_string(), factory.create(Some(agent_weak.clone()), cfg));
    }

    eprintln!("[home] all plugins created");
    agent_ref
}
