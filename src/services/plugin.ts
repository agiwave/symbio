/**
 * 插件调用服务
 */

import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'

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

/**
 * 流式调用插件
 * 
 * @param path 插件路径
 * @param input 输入参数
 * @param onChunk 每个 chunk 的回调
 * @returns Promise<void> 流结束时 resolve
 */
export async function streamPlugin(
  path: string,
  input: Record<string, unknown>,
  onChunk: (chunk: StreamChunk) => void
): Promise<void> {
  console.log('[plugin] streamPlugin:', path, input)

  const eventId = `stream-${Date.now()}-${Math.random()}`

  // 监听事件
  const unlisten = await listen<StreamChunk>(eventId, (event) => {
    console.log('[plugin] stream chunk:', event.payload)
    onChunk(event.payload)
  })

  try {
    // 调用 stream 命令
    await invoke('stream', { path, input, eventId })
  } catch (err) {
    console.error('[plugin] streamPlugin error:', err)
    throw err
  } finally {
    // 清理监听器
    unlisten()
  }
}

export async function getPluginMeta(path: string) {
  return invoke<{ path: string; meta: unknown }>('meta', { path })
}

