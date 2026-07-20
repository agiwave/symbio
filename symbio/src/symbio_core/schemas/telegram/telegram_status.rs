use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub configured: bool,
    pub has_chat_id: bool,
    pub streaming_enabled: bool,
    pub poll_enabled: bool,
    pub listener_running: bool,
    pub update_offset: i64,
}
