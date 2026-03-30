//! Home 插件 - 根插件，持有 work/agent/setting 子插件实例

use crate::core::traits::Plugin;
use crate::core::types::{PluginMeta, PluginResult, PluginError, InvokeStream};
use serde_json::{Value, json};
use std::sync::Arc;

pub struct HomePlugin {
    meta: PluginMeta,
    work: Arc<dyn Plugin>,
    agent: Arc<dyn Plugin>,
    setting: Arc<dyn Plugin>,
}

impl HomePlugin {
    pub fn new(
        work: Arc<dyn Plugin>,
        agent: Arc<dyn Plugin>,
        setting: Arc<dyn Plugin>,
    ) -> Self {
        HomePlugin {
            meta: PluginMeta {
                name: "home".to_string(),
                description: "Symbio 主插件".to_string(),
                version: "0.1.0".to_string(),
                input: Some(json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "子插件路径，如 work/agent/setting"
                        }
                    }
                })),
                output: None,
                author: Some("Symbio Team".to_string()),
            },
            work,
            agent,
            setting,
        }
    }

    fn route(&self, path: &str) -> Result<(Arc<dyn Plugin>, String), PluginError> {
        let parts: Vec<&str> = path.splitn(2, '/').collect();
        let plugin_name = parts[0];
        let sub_path = parts.get(1).map(|s| s.to_string()).unwrap_or_default();

        match plugin_name {
            "work" => Ok((Arc::clone(&self.work), sub_path)),
            "agent" => Ok((Arc::clone(&self.agent), sub_path)),
            "setting" => Ok((Arc::clone(&self.setting), sub_path)),
            _ => Err(PluginError::NotFound(format!("未知的插件路径: {}", plugin_name))),
        }
    }
}

#[async_trait::async_trait]
impl Plugin for HomePlugin {
    fn meta(&self, path: &str) -> PluginResult<PluginMeta> {
        if path.is_empty() {
            return Ok(self.meta.clone());
        }
        let (plugin, sub_path) = self.route(path)?;
        plugin.meta(&sub_path)
    }

    fn invoke(&self, path: &str, input: Value) -> PluginResult<InvokeStream> {
        if path.is_empty() {
            // 返回 home 的基本信息
            return Ok(InvokeStream::single(json!({
                "success": true,
                "data": {
                    "plugins": ["work", "agent", "setting"]
                }
            })));
        }
        let (plugin, sub_path) = self.route(path)?;
        plugin.invoke(&sub_path, input)
    }
}
