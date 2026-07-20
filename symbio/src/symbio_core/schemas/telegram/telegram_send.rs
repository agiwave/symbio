use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Request {
    pub text: String,
    pub chat_id: Option<String>,
    pub parse_mode: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub sent: i32,
    pub message: String,
}
