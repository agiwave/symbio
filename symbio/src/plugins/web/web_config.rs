use serde::{Deserialize, Serialize};

/// Web 工具配置。字段与配置文档（`PLUGIN.yml` 的表单定义）一一对应。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebConfig {
    /// 单次网络请求超时（秒）——`web_fetch` 与 `http_request` **都读这一个值**。
    ///
    /// 早先这里有个 `web_timeout` 字段：面板上能改，实际从没被任何工具读过，
    /// 真正生效的是两个工具各自写死的 30s 常量。删掉配置项并不等于问题解决，
    /// 只是把「配置面板在骗人」换成「超时无法调」——后者连入口都没有了。
    /// 故接回单一真源：两个工具的构造都从这里取值。
    #[serde(default = "default_web_timeout")]
    pub web_timeout: u64,
    /// Tavily API Key
    #[serde(default)]
    pub tavily_api_key: Option<String>,
    /// Serper API Key (Google Search)
    #[serde(default)]
    pub serper_api_key: Option<String>,
}

fn default_web_timeout() -> u64 {
    30
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            web_timeout: default_web_timeout(),
            tavily_api_key: None,
            serper_api_key: None,
        }
    }
}
