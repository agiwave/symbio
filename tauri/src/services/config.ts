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
import { ModelProvidersConfig, providerToModelConfig } from '../schemas/model_providers'

import { LocalConfig } from '../schemas/local_config'
import { WebConfig } from '../schemas/web_config'
import { McpConfig, McpServerConfig } from '../schemas/mcp_config'

// 配置路径常量
export const CONFIG_PATHS = {
  MODEL: 'model/config',
  SESSION: 'session/config',
  AGENT: 'agent/config',
  LOCAL: 'local/config',
  WEB: 'web/config',
  MCP: 'mcp/config',
  SETTING: 'setting/config'
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
 * 获取 Model \u63d2\u4ef6配置（返回当前活动 Provider 的 ModelConfig 视图）
 */
export async function getModelConfig(): Promise<ModelConfig> {
  try {
    const providers = await callPlugin<ModelProvidersConfig>(`${CONFIG_PATHS.MODEL}/get`, {})
    const defaultId = providers.default_provider_id
    if (defaultId && providers.providers[defaultId]) {
      return providerToModelConfig(providers.providers[defaultId])
    }
    const firstEnabled = Object.values(providers.providers).find(p => p.enabled)
    if (firstEnabled) {
      return providerToModelConfig(firstEnabled)
    }
    return {
      provider: '',
      api_base: '',
      api_key: '',
      model: '',
      temperature: 0.7,
      max_tokens: 4096,
      api_protocol: '',
    }
  } catch {
    return {
      provider: '',
      api_base: '',
      api_key: '',
      model: '',
      temperature: 0.7,
      max_tokens: 4096,
      api_protocol: '',
    }
  }
}

export const getOpenModelConfig = getModelConfig

/**
 * 设置 Model \u63d2\u4ef6配置（已弃用：请使用 services/ModelProviders.ts）
 */
export async function setModelConfig(_config: Partial<ModelConfig>): Promise<Common.SuccessResponse> {
  throw new Error(
    'setModelConfig 已弃用：AI 配置请通过 services/ModelProviders.ts 的 setProvider 更新'
  )
}

export const setOpenModelConfig = setModelConfig

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

/**
 * 获取 MCP 配置
 *
 * 委托给 `mcp/servers/list` 路由（与 Model Providers / McpView 一致）。
 * 该函数保留以兼容旧调用方（如 agent / hook 等可能直接读旧路径）。
 */
export async function getMcpConfig(): Promise<McpConfig> {
  const resp = await callPlugin<McpConfig>('mcp/servers/list', {})
  return resp ?? { servers: {} }
}

/**
 * 设置 MCP 配置
 *
 * 说明：MCP 配置已迁移到独立 Tab (`/mcp`) 下的单服务器 CRUD 模式，
 * 不再支持"覆盖整个 McpConfig"。若需要更新整体配置，请使用
 * `services/mcpServers.ts` 中提供的 `setMcpServer` / `deleteMcpServer`。
 */
export async function setMcpConfig(_config: McpConfig): Promise<Common.SuccessResponse> {
  throw new Error(
    'setMcpConfig 已弃用：MCP 配置请通过 services/mcpServers.ts 的 setMcpServer/deleteMcpServer 更新'
  )
}

export type { ModelConfig, SessionConfig, AgentConfig, LocalConfig, WebConfig, McpConfig, McpServerConfig }
