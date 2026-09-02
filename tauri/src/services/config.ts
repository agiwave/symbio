/**
 * 统一配置服务
 *
 * 提供统一的配置管理接口，遵循 API_DESIGN.md 规范
 *
 * ## 范围
 *
 * 本服务**仅**包含**通用 config 工具**（`getConfig` / `setConfig` / `getConfigSchema`）
 * 和**各业务插件**的 config 路由（model / session / agent / local / web / mcp / setting）。
 *
 * **home 插件的专属路由**（homedir / workdir）已迁至 `services/home.ts`，
 * 因为它们由 home 插件直接处理，不属于"通用 config"。
 */

import { callPlugin } from './plugin'
import * as Common from '../schemas/common'
import { ModelConfig } from '../schemas/model_config'
import { SessionConfig } from '../schemas/session_config'
import { AgentConfig } from '../schemas/agent_config'

import { LocalConfig } from '../schemas/local_config'
import { WebConfig } from '../schemas/web_config'
import { McpConfig, McpServerConfig } from '../schemas/mcp_config'

// 配置路径常量
export const CONFIG_PATHS = {
  SESSION: 'session/config',
  AGENT: 'agent/config',
  LOCAL: 'local/config',
  WEB: 'web/config'
}

/**
 * 获取插件配置
 */
export async function getConfig<T = Record<string, unknown>>(basePath: string): Promise<T> {
  return callPlugin<T>(`${basePath}/get`, {})
}

/**
 * 设置插件配置
 */
export async function setConfig(basePath: string, config: Record<string, unknown>): Promise<Common.SuccessResponse> {
  return callPlugin<Common.SuccessResponse>(`${basePath}/set`, config)
}

export interface ConfigSchema {
  type: string
  properties: Record<string, any>
  required?: string[]
}

/**
 * 获取配置 Schema
 */
export async function getConfigSchema(basePath: string): Promise<ConfigSchema> {
  const result = await callPlugin<Common.SchemaResponse>(`${basePath}/schema`, {})
  return result.schema
}

// ============ 便捷方法 ============

/**
 * 获取 Session 配置
 */
export async function getSessionConfig(): Promise<SessionConfig> {
  return callPlugin<SessionConfig>(`${CONFIG_PATHS.SESSION}/get`, {})
}

/**
 * 设置 Session 配置
 */
export async function setSessionConfig(config: Partial<SessionConfig>): Promise<Common.SuccessResponse> {
  return callPlugin<Common.SuccessResponse>(`${CONFIG_PATHS.SESSION}/set`, config)
}

/**
 * 获取 Agent 配置
 */
export async function getAgentConfig(): Promise<AgentConfig> {
  return callPlugin<AgentConfig>(`${CONFIG_PATHS.AGENT}/get`, {})
}

/**
 * 设置 Agent 配置
 */
export async function setAgentConfig(config: Partial<AgentConfig>): Promise<Common.SuccessResponse> {
  return callPlugin<Common.SuccessResponse>(`${CONFIG_PATHS.AGENT}/set`, config)
}

export async function getLocalConfig(): Promise<LocalConfig> {
  return callPlugin<LocalConfig>(`${CONFIG_PATHS.LOCAL}/get`, {})
}

/**
 * 设置 Local 配置
 */
export async function setLocalConfig(config: Partial<LocalConfig>): Promise<Common.SuccessResponse> {
  return callPlugin<Common.SuccessResponse>(`${CONFIG_PATHS.LOCAL}/set`, config)
}

/**
 * 获取 Web 配置
 */
export async function getWebConfig(): Promise<WebConfig> {
  return callPlugin<WebConfig>(`${CONFIG_PATHS.WEB}/get`, {})
}

/**
 * 设置 Web 配置
 */
export async function setWebConfig(config: Partial<WebConfig>): Promise<Common.SuccessResponse> {
  return callPlugin<Common.SuccessResponse>(`${CONFIG_PATHS.WEB}/set`, config)
}

// 注意：`getWorkspacePath` / `setWorkspacePath` 已迁至 `services/home.ts`
// （它们调用的是 home 插件的 `work/*` 路由，属于 home 服务范畴）
// MCP 配置已迁移到独立 Tab (`/mcp`) 及 `services/mcpServers.ts` 的单服务器 CRUD 模式。

export type { ModelConfig, SessionConfig, AgentConfig, LocalConfig, WebConfig, McpConfig, McpServerConfig }
