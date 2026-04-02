/**
 * 插件调用服务
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

export async function callPlugin<T = unknown>(
  path: string,
  input: Record<string, unknown>
): Promise<T> {
  console.log('[plugin] callPlugin:', path, input)
  
  try {
    const result = await invoke<StreamChunk[]>('invoke', { path, input })
    console.log('[plugin] invoke result:', result)

    if (Array.isArray(result) && result.length > 0) {
      const chunk = result[0]
      if (chunk.error) throw new Error(chunk.error)
      const data = chunk.data as PluginResponse
      if (data.error) throw new Error(data.error)
      if (data.data !== undefined) return data.data as T
      return data as T
    }

    throw new Error('插件调用返回空结果')
  } catch (err) {
    console.error('[plugin] callPlugin error:', err)
    throw err
  }
}

export async function getPluginMeta(path: string) {
  return invoke<{ path: string; meta: unknown }>('meta', { path })
}
