/**
 * 插件调用服务
 *
 * 提供统一的后端插件调用接口
 */

import { invoke } from '@tauri-apps/api/core'

export interface StreamChunk {
  data: unknown
  done: boolean
  error?: string
}

interface PluginResponse {
  success?: boolean
  data?: unknown
  message?: string
  error?: string
}

/**
 * 调用插件
 * @param path 插件路径，如 'work', 'agent/chat', 'setting'
 * @param input 输入参数
 */
export async function callPlugin<T = unknown>(
  path: string,
  input: Record<string, unknown>
): Promise<T> {
  const result = await invoke<StreamChunk[]>('invoke', {
    path,
    input
  })

  // invoke 返回 StreamChunk 数组，取第一个
  if (Array.isArray(result) && result.length > 0) {
    const chunk = result[0]
    
    // 检查错误
    if (chunk.error) {
      throw new Error(chunk.error)
    }
    
    // 解包响应
    const data = chunk.data as PluginResponse
    
    // 如果有 error 字段，抛出错误
    if (data.error) {
      throw new Error(data.error)
    }
    
    // 如果有 data 字段，返回 data 内容
    if (data.data !== undefined) {
      return data.data as T
    }
    
    // 否则直接返回整个 data
    return data as T
  }
  
  throw new Error('插件调用返回空结果')
}

/**
 * 获取插件元数据
 */
export async function getPluginMeta(path: string) {
  return invoke<{ path: string; meta: unknown }>('meta', { path })
}
