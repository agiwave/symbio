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
 * 流式调用控制句柄
 */
export interface StreamController {
  /** 中止流式请求 */
  abort: () => void
  /** 请求 Promise */
  promise: Promise<void>
}

/**
 * 流式调用插件（支持中止）
 *
 * 使用 Tauri 事件系统实时接收每个 chunk
 * 返回包含 abort 方法的控制器
 */
export function streamPluginWithAbort(
  path: string,
  input: Record<string, unknown>,
  onChunk: (chunk: StreamChunk) => void
): StreamController {
  console.log('[plugin] streamPluginWithAbort:', path, input)

  // 生成唯一事件 ID
  const eventId = `stream-${Date.now()}-${Math.random().toString(36).slice(2, 9)}`

  // 中止标志
  let aborted = false
  let unlistenFn: (() => void) | null = null

  const promise = (async () => {
    // 监听流式事件
    unlistenFn = await listen<StreamChunk>(eventId, (event) => {
      if (aborted) return // 已中止则忽略
      console.log('[plugin] stream chunk:', event.payload)
      onChunk(event.payload)
    })

    // 如果在监听设置前就已中止
    if (aborted) {
      unlistenFn()
      return
    }

    try {
      // 调用 stream 命令，后端会实时推送 chunk
      await invoke('stream', { path, input, eventId })
    } catch (err) {
      if (aborted) {
        console.log('[plugin] stream aborted')
        return
      }
      console.error('[plugin] streamPlugin error:', err)
      throw err
    } finally {
      // 清理监听器
      unlistenFn?.()
    }
  })()

  return {
    abort: () => {
      aborted = true
      unlistenFn?.()
      console.log('[plugin] stream aborted by user')
    },
    promise
  }
}

/**
 * 流式调用插件（旧版，保持向后兼容）
 *
 * 使用 Tauri 事件系统实时接收每个 chunk
 * 类似 OpenAI/MCP 的 SSE 流式传输
 */
export async function streamPlugin(
  path: string,
  input: Record<string, unknown>,
  onChunk: (chunk: StreamChunk) => void
): Promise<void> {
  console.log('[plugin] streamPlugin:', path, input)

  // 生成唯一事件 ID
  const eventId = `stream-${Date.now()}-${Math.random().toString(36).slice(2, 9)}`

  // 监听流式事件
  const unlisten = await listen<StreamChunk>(eventId, (event) => {
    console.log('[plugin] stream chunk:', event.payload)
    onChunk(event.payload)
  })

  try {
    // 调用 stream 命令，后端会实时推送 chunk
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

// ==================== Connect 相关 API ====================

export interface Connection {
  connectionId: string
  unlisten: () => void
}

export interface ConnectEvent {
  type: string
  data: unknown
}

/**
 * 建立持久连接
 */
export async function connectPlugin(
  path: string,
  input: Record<string, unknown>,
  onEvent: (event: ConnectEvent) => void
): Promise<Connection> {
  console.log('[plugin] connectPlugin:', path, input)

  // 调用 connect 命令获取 connection_id
  const connectionId = await invoke<string>('connect', { path, input })
  console.log('[plugin] connect returned, connectionId:', connectionId)

  // 监听连接事件
  const eventName = `connect/${connectionId}`
  console.log('[plugin] listening for events:', eventName)
  
  const unlisten = await listen<ConnectEvent>(eventName, (event) => {
    console.log('[plugin] connect event raw:', event.payload, typeof event.payload)
    // 处理可能的 JSON 字符串
    let payload = event.payload
    if (typeof payload === 'string') {
      try {
        payload = JSON.parse(payload)
      } catch (e) {
        console.error('[plugin] Failed to parse event payload:', e)
      }
    }
    onEvent(payload as ConnectEvent)
  })

  console.log('[plugin] listener setup complete for:', eventName)
  return { connectionId, unlisten }
}

/**
 * 通过连接发送消息
 */
export async function sendToConnection(
  connectionId: string,
  message: Record<string, unknown>
): Promise<void> {
  await invoke('connect_send', { connectionId, message })
}

/**
 * 关闭连接
 */
export async function closeConnection(connectionId: string): Promise<void> {
  await invoke('connect_close', { connectionId })
}

/**
 * 查询连接状态
 */
export async function getConnectionStatus(connectionId: string): Promise<{ alive: boolean }> {
  return invoke('connect_status', { connectionId })
}

