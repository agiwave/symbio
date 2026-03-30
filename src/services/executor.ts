/**
 * Docker 执行服务
 *
 * 通过标准插件接口调用 Docker 执行能力
 */

import { invoke } from '@tauri-apps/api/core'

export interface ExecutionResult {
  success: boolean
  exit_code: number
  stdout: string
  stderr: string
  duration_ms: number
  timed_out: boolean
}

export interface ExecutionConfig {
  cpu_limit?: number
  memory_limit?: number
  time_limit?: number
  network_disabled?: boolean
  image?: string
}

/**
 * 调用 Docker 插件
 */
async function callDockerPlugin(action: string, params: Record<string, unknown>): Promise<unknown> {
  const result = await invoke<unknown[]>('invoke', {
    path: 'docker',
    input: { action, ...params }
  })
  
  // invoke 返回 StreamChunk 数组，取第一个的 data
  if (Array.isArray(result) && result.length > 0) {
    return result[0].data
  }
  return result
}

/**
 * 检查 Docker 是否可用
 */
export async function isDockerAvailable(): Promise<boolean> {
  const result = await callDockerPlugin('available', {}) as { success: boolean; available: boolean }
  return result.available
}

/**
 * 执行命令
 */
export async function executeCommand(
  command: string,
  config?: ExecutionConfig
): Promise<ExecutionResult> {
  return callDockerPlugin('execute', { command, config }) as Promise<ExecutionResult>
}

/**
 * 执行脚本
 */
export async function executeScript(
  scriptPath: string,
  language: string,
  config?: ExecutionConfig
): Promise<ExecutionResult> {
  return callDockerPlugin('execute_script', { 
    script_path: scriptPath, 
    language,
    config 
  }) as Promise<ExecutionResult>
}

/**
 * 执行代码块
 * 
 * 将代码作为命令执行
 */
export async function executeCodeBlock(
  code: string,
  language: string,
  config?: ExecutionConfig
): Promise<ExecutionResult> {
  // 根据语言构建命令
  let command: string
  
  switch (language.toLowerCase()) {
    case 'python':
    case 'python3':
    case 'py':
      command = `python3 -c "${code.replace(/"/g, '\\"').replace(/\n/g, ' ')}"`
      break
    case 'r':
      command = `Rscript -e "${code.replace(/"/g, '\\"').replace(/\n/g, ' ')}"`
      break
    case 'bash':
    case 'sh':
    case 'shell':
      command = code
      break
    default:
      throw new Error(`Unsupported language: ${language}`)
  }
  
  return executeCommand(command, config)
}

/**
 * 默认执行配置
 */
export const defaultExecutionConfig: ExecutionConfig = {
  cpu_limit: 2.0,
  memory_limit: 4096,
  time_limit: 3600,
  network_disabled: true,
  image: 'symbio-executor:latest',
}