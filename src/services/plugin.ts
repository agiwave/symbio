/**
 * 插件调用服务
 *
 * 提供统一的后端插件调用接口
 */

import { invoke } from '@tauri-apps/api/core'

export interface StreamChunk {
  data: unknown
  done: boolean
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

  // invoke 返回 StreamChunk 数组，取第一个的 data
  if (Array.isArray(result) && result.length > 0) {
    return result[0].data as T
  }
  return result as T
}

/**
 * 获取插件元数据
 */
export async function getPluginMeta(path: string) {
  return invoke<{ path: string; meta: unknown }>('meta', { path })
}
