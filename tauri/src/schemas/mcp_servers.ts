// Corresponding Backend: symbio/src/symbio_core/schemas/mcp_servers.rs
//
// 与后端 `mcp_servers` schema 一一对应
// - servers/list  : 列出全部 MCP 服务器
// - servers/get   : 按 name 获取单个服务器
// - servers/set   : 按 name upsert（创建 / 更新）
// - servers/delete: 按 name 删除
// - servers/test  : 测试服务器连接（不修改配置/缓存）

import type { McpConfig, McpServerConfig } from './mcp_config'

/** servers/list - 列出全部 MCP 服务器 */
export namespace McpServersList {
  export interface Request {}
  export interface Response {
    config: McpConfig
  }
}

/** servers/get - 获取单个 MCP 服务器 */
export namespace McpServersGet {
  export interface Request {
    name: string
  }
  export interface Response {
    name: string
    server: McpServerConfig
  }
}

/** servers/set - 创建或更新一个 MCP 服务器（按 name upsert） */
export namespace McpServersSet {
  export interface Request {
    name: string
    server: McpServerConfig
  }
  export interface Response {
    config: McpConfig
  }
}

/** servers/delete - 删除一个 MCP 服务器 */
export namespace McpServersDelete {
  export interface Request {
    name: string
  }
  export interface Response {
    config: McpConfig
  }
}

/** servers/test - 测试一个 MCP 服务器的连接（不修改任何状态） */
export namespace McpServersTest {
  export interface Request {
    name: string
  }
  export interface Response {
    name: string
    ok: boolean
    tool_count: number
    protocol_version: string
    /** BUG-MR30：server 报告的名称（来自 initialize 响应） */
    server_name?: string | null
    /** BUG-MR30：server 报告的版本（来自 initialize 响应） */
    server_version?: string | null
    /** BUG-MR32：server 提供的使用说明 */
    instructions?: string | null
    error: string | null
    elapsed_ms: number
  }
}
