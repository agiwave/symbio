use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct SearchResult {
    pub key: String,
    pub content: String,
    pub category: Option<String>,
    pub relevance: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Request {
    pub query: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub results: Vec<SearchResult>,
    pub query: String,
    pub count: usize,
}
