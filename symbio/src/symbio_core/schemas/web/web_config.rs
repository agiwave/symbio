use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebConfig {
    #[serde(default = "default_true")]
    pub web_enabled: bool,
    #[serde(default = "default_web_timeout")]
    pub web_timeout: u64,
    /// Tavily API Key
    #[serde(default)]
    pub tavily_api_key: Option<String>,
    /// Serper API Key (Google Search)
    #[serde(default)]
    pub serper_api_key: Option<String>,
}

fn default_true() -> bool {
    true
}
fn default_web_timeout() -> u64 {
    300
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            web_enabled: true,
            web_timeout: 300,
            tavily_api_key: None,
            serper_api_key: None,
        }
    }
}
