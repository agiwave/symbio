/**
 * 统一配置服务
 *
 * 提供统一的配置管理接口，遵循 API_DESIGN.md 规范
 * 所有插件配置通过 {plugin}/config path 访问
 */

import { callPlugin } from './plugin'

// 配置路径常量
export const CONFIG_PATHS = {
  HOME: 'config',           // Home 插件全局配置
  AGENT: 'agent/config',    // Agent 插件配置（聚合）
  OPENAI: 'agent/openai/config',
  SESSION: 'agent/session/config',
  MEMORY: 'agent/memory/config',
  TOOLS: 'agent/tools/config',
  WORK: 'work/config',
  SETTING: 'setting/config',
} as const

// 配置类型定义
export interface OpenAiConfig {
  api_base: string
  api_key_set: boolean
  model: string
  temperature: number
  max_tokens?: number
  max_context_tokens: number
  system_prompt?: string
}

export interface SessionConfig {
  storage_dir: string
  max_messages: number
  auto_compress: boolean
  compress_threshold: number
}

export interface MemoryConfig {
  storage_dir: string
  max_entries: number
  categories: string[]
}

export interface ToolsConfig {
  shell_enabled: boolean
  file_enabled: boolean
  web_enabled: boolean
  allowed_paths: string[]
  blocked_commands: string[]
  shell_timeout: number
  web_timeout: number
}

export interface WorkConfig {
  workspace_path: string
  recent_workspaces: string[]
}

export interface GlobalConfig {
  plugins: {
    work?: WorkConfig
    agent?: {
      openai?: OpenAiConfig
      session?: SessionConfig
      memory?: MemoryConfig
      tools?: ToolsConfig
    }
    setting?: Record<string, unknown>
  }
}

// 配置 Schema 类型
export interface ConfigFieldSchema {
  type: string
  title: string
  description?: string
  default?: unknown
  enum?: string[]
  minimum?: number
  maximum?: number
  items?: { type: string }
  secret?: boolean
}

export type ConfigSchema = Record<string, ConfigFieldSchema>

/**
 * 获取插件配置
 */
export async function getConfig<T = Record<string, unknown>>(path: string): Promise<T> {
  return callPlugin<T>(path, { action: 'get' })
}

/**
 * 设置插件配置
 */
export async function setConfig(path: string, config: Record<string, unknown>): Promise<{ success: boolean }> {
  return callPlugin<{ success: boolean }>(path, {
    action: 'set',
    config
  })
}

/**
 * 获取配置 Schema
 */
export async function getConfigSchema(path: string): Promise<ConfigSchema> {
  const result = await callPlugin<{ success: boolean; schema: ConfigSchema }>(path, { action: 'schema' })
  return result.schema
}

/**
 * 获取全局配置（所有插件配置）
 */
export async function getGlobalConfig(): Promise<GlobalConfig> {
  return callPlugin<GlobalConfig>(CONFIG_PATHS.HOME, { action: 'get' })
}

/**
 * 保存全局配置到文件
 */
export async function saveGlobalConfig(): Promise<{ success: boolean; message?: string }> {
  return callPlugin<{ success: boolean; message?: string }>(CONFIG_PATHS.HOME, { action: 'save' })
}

/**
 * 从文件加载全局配置
 */
export async function loadGlobalConfig(): Promise<{ success: boolean; config?: GlobalConfig }> {
  return callPlugin<{ success: boolean; config?: GlobalConfig }>(CONFIG_PATHS.HOME, { action: 'load' })
}

/**
 * 收集所有插件配置
 */
export async function collectConfigs(): Promise<{ success: boolean; config: Record<string, unknown> }> {
  return callPlugin<{ success: boolean; config: Record<string, unknown> }>(CONFIG_PATHS.HOME, { action: 'collect' })
}

// ============ 便捷方法 ============

/**
 * 获取 OpenAI 配置
 */
export async function getOpenAiConfig(): Promise<OpenAiConfig> {
  return getConfig<OpenAiConfig>(CONFIG_PATHS.OPENAI)
}

/**
 * 设置 OpenAI 配置
 */
export async function setOpenAiConfig(config: Partial<OpenAiConfig>): Promise<{ success: boolean }> {
  return setConfig(CONFIG_PATHS.OPENAI, config)
}

/**
 * 获取 Session 配置
 */
export async function getSessionConfig(): Promise<SessionConfig> {
  return getConfig<SessionConfig>(CONFIG_PATHS.SESSION)
}

/**
 * 设置 Session 配置
 */
export async function setSessionConfig(config: Partial<SessionConfig>): Promise<{ success: boolean }> {
  return setConfig(CONFIG_PATHS.SESSION, config)
}

/**
 * 获取 Memory 配置
 */
export async function getMemoryConfig(): Promise<MemoryConfig> {
  return getConfig<MemoryConfig>(CONFIG_PATHS.MEMORY)
}

/**
 * 设置 Memory 配置
 */
export async function setMemoryConfig(config: Partial<MemoryConfig>): Promise<{ success: boolean }> {
  return setConfig(CONFIG_PATHS.MEMORY, config)
}

/**
 * 获取 Tools 配置
 */
export async function getToolsConfig(): Promise<ToolsConfig> {
  return getConfig<ToolsConfig>(CONFIG_PATHS.TOOLS)
}

/**
 * 设置 Tools 配置
 */
export async function setToolsConfig(config: Partial<ToolsConfig>): Promise<{ success: boolean }> {
  return setConfig(CONFIG_PATHS.TOOLS, config)
}

/**
 * 获取 Work 配置
 */
export async function getWorkConfig(): Promise<WorkConfig> {
  return getConfig<WorkConfig>(CONFIG_PATHS.WORK)
}

/**
 * 设置 Work 配置
 */
export async function setWorkConfig(config: Partial<WorkConfig>): Promise<{ success: boolean }> {
  return setConfig(CONFIG_PATHS.WORK, config)
}

// ============ 工作区路径接口 ============

/**
 * 获取工作区路径
 */
export async function getWorkspacePath(): Promise<{ workspace_path: string; expanded_path: string }> {
  return callPlugin<{ workspace_path: string; expanded_path: string }>('work', { 
    action: 'workspace_path' 
  })
}

/**
 * 设置工作区路径
 */
export async function setWorkspacePath(path: string): Promise<{ success: boolean; workspace_path: string }> {
  return callPlugin<{ success: boolean; workspace_path: string }>('work', { 
    action: 'set_workspace', 
    path 
  })
}
