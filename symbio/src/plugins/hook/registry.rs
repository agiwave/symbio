use crate::symbio_core::schemas::HookOutput;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str) -> HookConfigEntry {
        HookConfigEntry {
            name: name.to_string(),
            hook_type: HookType::Command,
            command: Some(format!("echo {name}")),
            url: None,
            timeout_ms: None,
            matcher: None,
        }
    }

    fn reg(event: &str, names: &[&str]) -> HookRegistration {
        HookRegistration {
            event: event.to_string(),
            hooks: names.iter().map(|n| entry(n)).collect(),
        }
    }

    #[tokio::test]
    async fn register_then_query_by_event() {
        let mut r = HookRegistry::new();
        r.register(&reg("session_start", &["a", "b"])).await;
        r.register(&reg("tool_call", &["c"])).await;

        assert_eq!(r.get_hooks("session_start").await.len(), 2);
        assert_eq!(r.get_hooks("tool_call").await.len(), 1);
        // 未注册的事件返回空表，不报错
        assert!(r.get_hooks("nope").await.is_empty());
    }

    /// 同一事件重复注册是**整体替换**而不是追加：配置每次全量写入，
    /// 旧条目必须随之消失，否则钩子会越攒越多、同一事件被反复触发。
    #[tokio::test]
    async fn re_registering_an_event_replaces_its_hooks() {
        let mut r = HookRegistry::new();
        r.register(&reg("e", &["a", "b"])).await;
        r.register(&reg("e", &["c"])).await;
        let got = r.get_hooks("e").await;
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].name, "c");
    }

    #[tokio::test]
    async fn list_hooks_covers_all_events() {
        let mut r = HookRegistry::new();
        r.register(&reg("e1", &["a"])).await;
        r.register(&reg("e2", &["b", "c"])).await;
        let all = r.list_hooks().await;
        assert_eq!(all.len(), 2);
        assert_eq!(all["e2"].len(), 2);
    }

    #[test]
    fn hook_config_entry_roundtrips_json() {
        let v = serde_json::to_value(entry("x")).unwrap();
        let back: HookConfigEntry = serde_json::from_value(v).unwrap();
        assert_eq!(back.name, "x");
        assert!(matches!(back.hook_type, HookType::Command));
        assert_eq!(back.command.as_deref(), Some("echo x"));
    }
}
