use crate::symbio_core::HookOutput;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookConfigEntry {
    pub name: String,
    pub hook_type: HookType,
    pub command: Option<String>,
    pub url: Option<String>,
    pub timeout_ms: Option<u64>,
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

    /// 停用时清空注册态（[T-185](../../../../docs/009-task/185-插件停用收口批.md "T-185")）：
    /// 注册只活在内存里，插件被停用后不应再有一份「看起来还在」的 hooks 名单。
    pub async fn clear(&mut self) {
        self.hooks.clear();
    }
}

impl Default for HookRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "registry.test.rs"]
mod tests;
