/**
 * 代码执行服务
 *
 * 提供代码块的执行能力
 */

import { invoke } from '@tauri-apps/api/core'

export interface ExecutionResult {
  exit_code: number
  stdout: string
  stderr: string
  duration_ms: number
  timed_out: boolean
}

export interface ExecutionConfig {
  cpu_limit: number
  memory_limit: number
  time_limit: number
  network_disabled: boolean
  read_only_paths: string[]
  writable_paths: string[]
  workdir: string
  image: string
}

/**
 * 检查 Docker 是否可用
 */
export async function isDockerAvailable(): Promise<boolean> {
  return invoke<boolean>('docker_available')
}

/**
 * 检查镜像是否存在
 */
export async function isImageExists(tag: string): Promise<boolean> {
  return invoke<boolean>('docker_image_exists', { tag })
}

/**
 * 构建执行环境镜像
 */
export async function buildImage(dockerfilePath: string, tag: string): Promise<void> {
  return invoke('docker_build_image', { 
    dockerfile_path: dockerfilePath, 
    tag 
  })
}

/**
 * 执行命令
 */
export async function executeCommand(
  command: string, 
  config?: ExecutionConfig
): Promise<ExecutionResult> {
  return invoke<ExecutionResult>('docker_execute', { 
    command, 
    config 
  })
}

/**
 * 执行脚本
 */
export async function executeScript(
  scriptPath: string,
  language: string,
  config?: ExecutionConfig
): Promise<ExecutionResult> {
  return invoke<ExecutionResult>('docker_execute_script', { 
    script_path: scriptPath, 
    language,
    config 
  })
}

/**
 * 执行代码块
 * 
 * 将代码写入临时文件，然后执行
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
      command = `python3 -c "${code.replace(/"/g, '\\"')}"`
      break
    case 'r':
      command = `Rscript -e "${code.replace(/"/g, '\\"')}"`
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
  read_only_paths: [],
  writable_paths: ['/workspace'],
  workdir: '/workspace',
  image: 'symbio-executor:latest',
}
