use serde::{Deserialize, Serialize};

/// Web 工具配置。字段与配置文档（`PLUGIN.yml` 的表单定义）一一对应。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebConfig {
    /// 单次网络请求超时（秒）——`web_fetch` / `web_search` / `http_request`
    /// **都读这一个值**，且都在每次调用时现读（保存后立即生效，无需重载插件）。
    ///
    /// 只让面板能改不等于让配置生效：工具若把超时写死成常量，这个字段就是摆设；
    /// 故三个工具共享同一个配置锁，不各自留一份拷贝。
    #[serde(default = "default_web_timeout")]
    pub web_timeout: u64,
    /// Tavily API Key
    #[serde(default)]
    pub tavily_api_key: Option<String>,
    /// Serper API Key (Google Search)
    #[serde(default)]
    pub serper_api_key: Option<String>,
    /// `http_request` 的出站域名白名单（ ④：此前在工具构造里写死 `*`，
    /// 恒真守卫——现在接进配置）。`*` = 不限制；`*.example.com` = 裸域名或子域。
    #[serde(default = "default_http_allowed_domains")]
    pub http_allowed_domains: Vec<String>,
    /// 搜索上游基址覆盖（ ⑤：三个上游地址此前全部写死，无法指向本地靶子）。
    /// **空 = 各供应商官方端点**；填了就替换三家的 scheme+host、保留各自路径
    /// （Tavily/Serper → `{base}/search`，DuckDuckGo → `{base}/html/`）——
    /// e2e 与自建代理用，改路径形状等于换供应商契约。
    #[serde(default)]
    pub search_api_base: String,
}

fn default_web_timeout() -> u64 {
    30
}

fn default_http_allowed_domains() -> Vec<String> {
    vec!["*".to_string()]
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            web_timeout: default_web_timeout(),
            tavily_api_key: None,
            serper_api_key: None,
            http_allowed_domains: default_http_allowed_domains(),
            search_api_base: String::new(),
        }
    }
}
