//! 设置插件的请求/响应 schema（只被本插件消费，故定义在插件内部）

pub mod setting_get {
    use serde::{Deserialize, Serialize};

    /// 获取设置请求
    #[derive(Debug, Serialize, Deserialize, Clone)]
    pub struct Request {
        pub category: String,
    }

    /// 获取设置响应
    #[derive(Debug, Serialize, Deserialize, Clone)]
    pub struct Response {
        pub category: String,
        pub settings: serde_json::Value,
    }
}

pub mod setting_list {
    use serde::{Deserialize, Serialize};

    /// 设置分类项
    #[derive(Debug, Serialize, Deserialize, Clone)]
    pub struct SettingCategory {
        pub id: String,
        pub name: String,
        pub icon: String,
    }

    /// 列出设置分类响应
    #[derive(Debug, Serialize, Deserialize, Clone)]
    pub struct Response {
        pub categories: Vec<SettingCategory>,
    }
}
