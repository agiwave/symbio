use crate::symbio_core::schemas::system::hook::HookOutput;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookConfigEntry {
    pub name: String,
    pub hook_type: HookType,
    pub command: Option<String>,
    pub url: Option<String>,
    pub timeout_ms: Option<u64>,
    pub matcher: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HookType {
    Command,
    Http,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookRegistration {
    pub event: String,
    pub hooks: Vec<HookConfigEntry>,
}

#[derive(Debug, Clone)]
pub struct HookExecutionResult {
    pub success: bool,
    pub output: HookOutput,
    pub error: Option<String>,
}

pub struct HookRegistry {
    hooks: HashMap<String, Vec<HookConfigEntry>>,
}

impl HookRegistry {
    pub fn new() -> Self {
        Self {
            hooks: HashMap::new(),
        }
    }

    pub async fn register(&mut self, reg: &HookRegistration) {
        self.hooks.insert(reg.event.clone(), reg.hooks.clone());
    }

    pub async fn get_hooks(&self, event: &str) -> Vec<HookConfigEntry> {
        self.hooks.get(event).cloned().unwrap_or_default()
    }

    pub async fn list_hooks(&self) -> HashMap<String, Vec<HookConfigEntry>> {
        self.hooks.clone()
    }
}

impl Default for HookRegistry {
    fn default() -> Self {
        Self::new()
    }
}
