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
