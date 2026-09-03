/**
 * 统一事件总线 - 前端服务层
 *
 * 与后端 `core/event_bus` 插件通讯，建立单一长连接，
 * 接收后端推送的 `bus_event` 帧（已带 `kind` / `session_id`）。
 *
 * 设计：
 * - **单连接**：app 启动时只调用一次 `connectEventBus()`
 * - **类型安全**：`BusEvent.kind` 是字面量联合，可直接分发
 * - **订阅式**：`subscribe({ kind, sessionId }, handler)` 注册回调
 * - **自动重连**：连接断开时按指数退避重连
 */

import { connectPlugin, callPlugin, type Connection, type ConnectEvent } from './plugin'
import { logger } from '@/utils/logger'

// 与后端 `KIND_*` 常量保持一致
export const KIND_SESSION = 'session'
export const KIND_EXPLORER = 'explorer'
export const KIND_SYSTEM = 'system'
export const KIND_RESOURCE = 'resource'

/**
 * 从后端 `event_bus` 收到的统一事件结构
 */
export interface BusEvent {
  type: 'bus_event'
  data: {
    kind: string              // 'session' | 'explorer' | 'system' | ...
    session_id: string | null // 关联到具体会话/工作区
    data: unknown             // 原始业务数据（消费者按 kind 窄化类型）
  }
}

/**
 * 订阅过滤器：kind 必须匹配；session_id 模糊匹配（null 表示不限定）
 */
export interface BusSubscriptionFilter {
  kind: string
  /** null/undefined 表示不限定具体会话 */
  sessionId?: string | null
}

/**
 * 订阅句柄
 */
export interface BusSubscription {
  id: number
  filter: BusSubscriptionFilter
  handler: (event: BusEvent) => void
}

/** 内部状态 */
let _connection: Connection | null = null
let _connectionPromise: Promise<Connection> | null = null
const _subscribers = new Set<BusSubscription>()
let _nextSubId = 1
let _reconnectTimer: ReturnType<typeof setTimeout> | null = null
let _reconnectDelay = 1000
const _maxReconnectDelay = 30000

/**
 * 切换会话时的"防乱序"缓冲。
 *
 * 背景：当用户从 B 切回 A 时，A 期间累积的事件分两部分到达：
 * 1. **回放事件**（fetchPendingSnapshot 返回的历史）
 * 2. **实时事件**（总线连接上正在推送的新事件）
 *
 * 旧实现：先 `_subscribers.add(sub)`，再异步拉 snapshot。
 *   副作用：在 snapshot 拉回前的几百毫秒内，实时事件先到 handler；
 *   snapshot 中的"更早的事件"反而晚到，造成 Status / Abort 顺序错乱。
 *
 * 新实现：
 *   1. 订阅时立即拉 snapshot
 *   2. snapshot 拉回前，到达该 sessionId 的实时事件先缓存到 `_replayBuffer`
 *   3. snapshot 处理完后再**有序**派发（先 snapshot，后缓存的实时事件）
 *   4. 派发完后从 `_replayBuffer` 删除该 sessionId
 */
const _replayBuffer: Map<string, BusEvent[]> = new Map()

/**
 * 启动（幂等）事件总线连接
 */
export async function connectEventBus(): Promise<Connection> {
  if (_connection && _connection.isConnected) {
    return _connection
  }
  if (_connectionPromise) {
    return _connectionPromise
  }

  _connectionPromise = (async () => {
    const conn = await connectPlugin('event_bus/subscribe', {}, handleConnectionEvent, {})
    _connection = conn
    _reconnectDelay = 1000
    logger.info('[event-bus]', 'Event bus connected', conn.connectionId)

    // 监听 disconnect 事件以便触发重连
    // （connectPlugin 会在 onEvent('disconnected') 时调用）
    _connectionPromise = null
    return conn
  })()

  try {
    return await _connectionPromise
  } catch (e) {
    _connectionPromise = null
    logger.error('[event-bus]', 'Failed to connect:', e)
    scheduleReconnect()
    throw e
  }
}

/**
 * 主动断开事件总线
 */
export async function disconnectEventBus(): Promise<void> {
  if (_reconnectTimer) {
    clearTimeout(_reconnectTimer)
    _reconnectTimer = null
  }
  if (_connection) {
    try {
      await _connection.close()
    } catch (e) {
      logger.warn('[event-bus]', 'Disconnect error:', e)
    }
    _connection = null
  }
  _connectionPromise = null
  _subscribers.clear()
}

/**
 * 当前连接状态
 */
export function isEventBusConnected(): boolean {
  return !!(_connection && _connection.isConnected)
}

/**
 * 拉取并清空指定 session 的回放缓冲
 *
 * 后端会缓存最近 64 帧（按 session 维度），
 * 切换会话时调用此 RPC 一次性拉回所有"上次订阅时漏掉的事件"。
 *
 * 注意：拉回的事件**已经过 bus 过滤**（kind=session, session_id 匹配），
 * 直接派发给 handler 即可。
 */
export async function fetchPendingSnapshot(sessionId: string): Promise<BusEvent[]> {
  try {
    const resp: unknown = await callPlugin('event_bus/pending/snapshot', { session_id: sessionId })
    const events: unknown[] = Array.isArray((resp as { events?: unknown[] })?.events)
      ? ((resp as { events: unknown[] }).events)
      : []
    return events.map((e) => ({ type: 'bus_event', data: e as BusEvent['data'] }))
  } catch (e) {
    logger.warn('[event-bus]', `fetchPendingSnapshot(${sessionId}) failed:`, e)
    return []
  }
}

/**
 * 订阅事件总线
 *
 * - `filter.kind` 必填（如 'session'）
 * - `filter.sessionId` 可选（null/undefined 接收所有该 kind 的事件）
 * - 首次订阅时会自动触发总线连接；连接是全局单例
 * - **当 `filter.sessionId` 不为 null 时**，会自动调用 `fetchPendingSnapshot(sessionId)`
 *   拉回上次切换走时漏掉的中间事件，**不重复派发**（handler 用幂等合并即可）
 * - **切换防乱序**：在 snapshot 拉回并派发完之前，**该 sessionId 的实时事件先缓存到
 *   `_replayBuffer[sessionId]`**，等 snapshot 处理完后按"先 snapshot → 后实时"顺序派发。
 *   这一步保证 Status / Abort / Error 等状态类事件不会因为 race 而乱序。
 *
 * @returns 取消订阅函数
 */
export function subscribe(
  filter: BusSubscriptionFilter,
  handler: (event: BusEvent) => void
): () => void {
  const sub: BusSubscription = {
    id: _nextSubId++,
    filter: {
      kind: filter.kind,
      sessionId: filter.sessionId ?? null
    },
    handler
  }
  _subscribers.add(sub)

  // 第一次订阅时自动建立总线连接（fire-and-forget；连接失败时由 scheduleReconnect 处理）
  if (_subscribers.size === 1) {
    connectEventBus().catch(e => {
      logger.error('[event-bus]', 'Auto-connect on first subscribe failed:', e)
    })
  }

  // **事件回放**：如果订阅了具体 sessionId，立刻拉回漏掉的中间事件。
  // 修复（CHAT_FLOW_ANALYSIS P2-24）：在 snapshot 派发完之前缓存实时事件，
  // 避免"先到的实时事件"覆盖"还在路上的回放事件"导致顺序错乱。
  if (sub.filter.sessionId) {
    const sid = sub.filter.sessionId
    // **关键**：先在 buffer 里占个位（空数组），让 handleConnectionEvent 知道
    // 当前 sid 处于"回放中"状态，从而把实时事件先缓存起来
    if (!_replayBuffer.has(sid)) {
      _replayBuffer.set(sid, [])
    }
    fetchPendingSnapshot(sid).then((events) => {
      // 1. 先按到达顺序派发 snapshot
      for (const evt of events) {
        try {
          sub.handler(evt)
        } catch (e) {
          logger.error('[event-bus]', `replay handler ${sub.id} threw:`, e)
        }
      }
      // 2. 再派发缓存的实时事件（按到达顺序）
      const buffered = _replayBuffer.get(sid)
      if (buffered && buffered.length > 0) {
        _replayBuffer.delete(sid) // 删除 key = 退出"回放中"状态
        for (const evt of buffered) {
          try {
            sub.handler(evt)
          } catch (e) {
            logger.error('[event-bus]', `buffered handler ${sub.id} threw:`, e)
          }
        }
      } else {
        _replayBuffer.delete(sid)
      }
    }).catch(e => {
      // snapshot 拉取失败：直接清空 buffer 走实时事件
      logger.warn('[event-bus]', `fetchPendingSnapshot(${sid}) failed, draining buffer:`, e)
      const buffered = _replayBuffer.get(sid)
      _replayBuffer.delete(sid) // 删除 key = 退出"回放中"状态
      if (buffered && buffered.length > 0) {
        for (const evt of buffered) {
          try {
            sub.handler(evt)
          } catch (e2) {
            logger.error('[event-bus]', `buffered handler ${sub.id} threw:`, e2)
          }
        }
      }
    })
  }

  return () => {
    _subscribers.delete(sub)
    // 取消订阅时清掉该 sessionId 的 buffer（避免内存泄漏）
    if (sub.filter.sessionId) {
      // 仅当没有其他订阅者使用此 sid 时才清 buffer
      let stillUsed = false
      for (const other of _subscribers) {
        if (other.filter.sessionId === sub.filter.sessionId) {
          stillUsed = true
          break
        }
      }
      if (!stillUsed) {
        _replayBuffer.delete(sub.filter.sessionId)
      }
    }
  }
}

/**
 * 资源实时状态事件（与后端 `publish_resource_status` 的载荷对齐）
 */
export interface ResourceStatusEvent {
  resource_type: string
  id: string
  status: string
  status_detail?: string | null
}

/**
 * 订阅指定资源类型的实时状态变化（resource kind）
 *
 * 返回取消订阅函数。事件仅当 `resource_type` 匹配时回调，
 * 用于资源列表/详情即时刷新状态角标（初始态由 `resources/list` 兜底）。
 */
export function subscribeResourceStatus(
  resourceType: string,
  handler: (e: ResourceStatusEvent) => void
): () => void {
  return subscribe({ kind: KIND_RESOURCE }, (busEvent) => {
    const d = busEvent.data?.data as ResourceStatusEvent | undefined
    if (!d || d.resource_type !== resourceType) return
    handler(d)
  })
}

// ===== 内部 =====

function handleConnectionEvent(event: ConnectEvent): void {
  // 1. 断开/错误：清理连接并触发重连
  if (event.type === 'disconnected' || event.type === 'error') {
    if (event.type === 'error') {
      logger.error('[event-bus]', 'Connection error:', event.data)
    } else {
      logger.warn('[event-bus]', 'Disconnected:', event.data)
    }
    _connection = null
    scheduleReconnect()
    return
  }

  // 2. 业务事件：派发给订阅者
  if (event.type === 'bus_event') {
    const busEvent = event as unknown as BusEvent
    if (!busEvent.data || typeof busEvent.data !== 'object') return

    const { kind, session_id } = busEvent.data
    if (!kind) return

    for (const sub of _subscribers) {
      if (sub.filter.kind !== kind) continue
      // sessionId 过滤：null 接收所有；否则精确匹配
      if (sub.filter.sessionId != null && sub.filter.sessionId !== session_id) continue
      try {
        // 修复（CHAT_FLOW_ANALYSIS P2-24）：
        // 如果该 sessionId 正在回放（snapshot 拉取中），先把事件塞到 buffer
        // 等 snapshot 派发完再统一派发，避免乱序
        if (
          sub.filter.sessionId != null &&
          sub.filter.sessionId === session_id &&
          _replayBuffer.has(sub.filter.sessionId)
        ) {
          let buf = _replayBuffer.get(sub.filter.sessionId)
          if (!buf) {
            buf = []
            _replayBuffer.set(sub.filter.sessionId, buf)
          }
          buf.push(busEvent)
          continue
        }
        sub.handler(busEvent)
      } catch (e) {
        logger.error('[event-bus]', `Subscriber ${sub.id} threw:`, e)
      }
    }
  }
}

function scheduleReconnect(): void {
  if (_reconnectTimer) return
  const delay = _reconnectDelay
  logger.info('[event-bus]', `Reconnecting in ${delay}ms...`)
  _reconnectTimer = setTimeout(async () => {
    _reconnectTimer = null
    _reconnectDelay = Math.min(_reconnectDelay * 2, _maxReconnectDelay)
    try {
      await connectEventBus()
    } catch {
      // scheduleReconnect 已在 connectEventBus 中处理
    }
  }, delay)
}
