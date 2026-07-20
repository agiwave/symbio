use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Request {
    pub key: String,
    pub content: String,
    pub category: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub key: String,
    pub message: String,
}
