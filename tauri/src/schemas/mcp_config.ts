// Corresponding Backend: symbio/src/symbio_core/schemas/mcp_config.rs
//
// 与后端 `McpServerConfig` / `McpConfig` 严格对齐
//
// **字段命名**：后端 Rust serde 标签为 snake_case（如 `include_tools`），
// 通过 HTTP/JSON 序列化后**保持 snake_case**。因此前端必须用 snake_case
// 访问，与后端 wire 格式一致。
//
// 字段约定：
// - `type`            : transport 类型（stdio | http | sse）—— 后端用 `#[serde(rename = "type")]`
// - `command`         : stdio transport 启动命令（仅 stdio 使用）
// - `args`            : stdio transport 命令参数
// - `env`             : stdio transport 环境变量
// - `url`             : http/sse transport URL（仅 http/sse 使用）
// - `include_tools`   : 工具白名单（undefined = 全部）
// - `exclude_tools`   : 工具黑名单（优先级高于 include_tools）
// - `enabled`         : 是否启用

export type McpTransportType = 'stdio' | 'http' | 'sse'

export interface McpServerConfig {
  /** transport 类型（默认 'stdio'） */
  type?: McpTransportType
  /** stdio transport 启动命令 */
  command?: string
  /** stdio transport 命令参数 */
  args?: string[]
  /** stdio transport 环境变量 */
  env?: Record<string, string>
  /** http / sse transport URL */
  url?: string
  /** BUG-MR28：http / sse transport 自定义请求头（如 `Authorization: Bearer ...`） */
  headers?: Record<string, string>
  /** 工具白名单（snake_case，与后端一致） */
  include_tools?: string[]
  /** 工具黑名单（snake_case，与后端一致） */
  exclude_tools?: string[]
  /** BUG-MR31：http transport 请求超时（秒），None = 使用默认值 30s */
  timeout_secs?: number
  /** 是否启用 */
  enabled?: boolean
}

export interface McpConfig {
  servers: Record<string, McpServerConfig>
}
