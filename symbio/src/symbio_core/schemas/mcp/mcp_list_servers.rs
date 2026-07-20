// Corresponding Frontend: tauri/src/schemas/mcp_list_servers.ts
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerInfo {
    pub name: String,
    pub transport: String,
    pub status: String,
}

/// List of MCP servers
pub type Response = Vec<McpServerInfo>;
