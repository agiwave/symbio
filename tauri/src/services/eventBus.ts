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
export const KIND_ENTITY = 'entity'

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

/**
 * 内部状态（globalThis 单例）。
 *
 * 为什么挂到 globalThis：Vite HMR / 模块热更新会重新执行本模块，
 * 模块级变量被重置后旧连接成为"孤儿"（后端订阅仍在、前端监听器仍在），
 * 再次订阅时会建立第二条连接 → 同一事件送达两次（流式消息叠字的温床）。
 * 把连接相关状态收敛到 globalThis 单例，模块重载后复用同一份状态，
 * 保证任何场景下前端与后端的事件总线**只有一条有效连接**。
 */
interface EventBusState {
  connection: Connection | null
  connectionPromise: Promise<Connection> | null
  subscribers: Set<BusSubscription>
  nextSubId: number
  reconnectTimer: ReturnType<typeof setTimeout> | null
  reconnectDelay: number
  /** 切换会话时的"防乱序"缓冲（语义见下方 replay buffer 注释） */
  replayBuffer: Map<string, BusEvent[]>
  /** 前端模式：页面间本地通知注册表 */
  localEntityHandlers: Map<string, Set<(e: EntityChangedEvent) => void>>
}

const _G = globalThis as typeof globalThis & { __symEventBusState?: EventBusState }
const S: EventBusState = _G.__symEventBusState ?? (_G.__symEventBusState = {
  connection: null,
  connectionPromise: null,
  subscribers: new Set(),
  nextSubId: 1,
  reconnectTimer: null,
  reconnectDelay: 1000,
  replayBuffer: new Map(),
  localEntityHandlers: new Map()
})
const _maxReconnectDelay = 30000

/**
 * 切换会话时的"防乱序"缓冲。
 *
 * 背景：当用户从 B 切回 A 时，A 期间累积的事件分两部分到达：
 * 1. **回放事件**（fetchPendingSnapshot 返回的历史）
 * 2. **实时事件**（总线连接上正在推送的新事件）
 *
 * 旧实现：先 `S.subscribers.add(sub)`，再异步拉 snapshot。
 *   副作用：在 snapshot 拉回前的几百毫秒内，实时事件先到 handler；
 *   snapshot 中的"更早的事件"反而晚到，造成 Status / Abort 顺序错乱。
 *
 * 新实现：
 *   1. 订阅时立即拉 snapshot
 *   2. snapshot 拉回前，到达该 sessionId 的实时事件先缓存到 `S.replayBuffer`
 *   3. snapshot 处理完后再**有序**派发（先 snapshot，后缓存的实时事件）
 *   4. 派发完后从 `S.replayBuffer` 删除该 sessionId
 */

/**
 * 启动（幂等）事件总线连接
 */
export async function connectEventBus(): Promise<Connection> {
  if (S.connection && S.connection.isConnected) {
    return S.connection
  }
  if (S.connectionPromise) {
    return S.connectionPromise
  }

  S.connectionPromise = (async () => {
    // 单连接保证：新连接建立前先关闭旧连接（连接超时误判失活 / HMR 残留等场景）。
    // 旧连接若不回收：前端 route/{id} 监听器与后端订阅都在 → 同一事件送达两次。
    if (S.connection) {
      const stale = S.connection
      S.connection = null
      try {
        await stale.close()
      } catch (e) {
        logger.warn('[event-bus]', 'Close stale connection failed:', e)
      }
    }
    const conn = await connectPlugin('event_bus/subscribe', {}, handleConnectionEvent, {})
    S.connection = conn
    S.reconnectDelay = 1000
    logger.info('[event-bus]', 'Event bus connected', conn.connectionId)

    // 监听 disconnect 事件以便触发重连
    // （connectPlugin 会在 onEvent('disconnected') 时调用）
    S.connectionPromise = null
    return conn
  })()

  try {
    return await S.connectionPromise
  } catch (e) {
    S.connectionPromise = null
    logger.error('[event-bus]', 'Failed to connect:', e)
    scheduleReconnect()
    throw e
  }
}

/**
 * 主动断开事件总线
 */
export async function disconnectEventBus(): Promise<void> {
  if (S.reconnectTimer) {
    clearTimeout(S.reconnectTimer)
    S.reconnectTimer = null
  }
  if (S.connection) {
    try {
      await S.connection.close()
    } catch (e) {
      logger.warn('[event-bus]', 'Disconnect error:', e)
    }
    S.connection = null
  }
  S.connectionPromise = null
  S.subscribers.clear()
}

/**
 * 当前连接状态
 */
export function isEventBusConnected(): boolean {
  return !!(S.connection && S.connection.isConnected)
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
 *   `S.replayBuffer[sessionId]`**，等 snapshot 处理完后按"先 snapshot → 后实时"顺序派发。
 *   这一步保证 Status / Abort / Error 等状态类事件不会因为 race 而乱序。
 *
 * @returns 取消订阅函数
 */
export function subscribe(
  filter: BusSubscriptionFilter,
  handler: (event: BusEvent) => void
): () => void {
  const sub: BusSubscription = {
    id: S.nextSubId++,
    filter: {
      kind: filter.kind,
      sessionId: filter.sessionId ?? null
    },
    handler
  }
  S.subscribers.add(sub)

  // 第一次订阅时自动建立总线连接（fire-and-forget；连接失败时由 scheduleReconnect 处理）
  if (S.subscribers.size === 1) {
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
    if (!S.replayBuffer.has(sid)) {
      S.replayBuffer.set(sid, [])
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
      const buffered = S.replayBuffer.get(sid)
      if (buffered && buffered.length > 0) {
        S.replayBuffer.delete(sid) // 删除 key = 退出"回放中"状态
        for (const evt of buffered) {
          try {
            sub.handler(evt)
          } catch (e) {
            logger.error('[event-bus]', `buffered handler ${sub.id} threw:`, e)
          }
        }
      } else {
        S.replayBuffer.delete(sid)
      }
    }).catch(e => {
      // snapshot 拉取失败：直接清空 buffer 走实时事件
      logger.warn('[event-bus]', `fetchPendingSnapshot(${sid}) failed, draining buffer:`, e)
      const buffered = S.replayBuffer.get(sid)
      S.replayBuffer.delete(sid) // 删除 key = 退出"回放中"状态
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
    S.subscribers.delete(sub)
    // 取消订阅时清掉该 sessionId 的 buffer（避免内存泄漏）
    if (sub.filter.sessionId) {
      // 仅当没有其他订阅者使用此 sid 时才清 buffer
      let stillUsed = false
      for (const other of S.subscribers) {
        if (other.filter.sessionId === sub.filter.sessionId) {
          stillUsed = true
          break
        }
      }
      if (!stillUsed) {
        S.replayBuffer.delete(sub.filter.sessionId)
      }
    }
  }
}

/**
 * 实体实时状态事件（与后端 `publish_entity_status` 的载荷对齐）
 */
export interface EntityStatusEvent {
  entity_type: string
  id: string
  status: string
  status_detail?: string | null
}

/**
 * 订阅指定实体类型的实时状态变化（entity kind）
 *
 * 返回取消订阅函数。事件仅当 `entity_type` 匹配时回调，
 * 用于实体列表/详情即时刷新状态角标（初始态由 `entities/list` 兜底）。
 */
export function subscribeEntityStatus(
  entityType: string,
  handler: (e: EntityStatusEvent) => void
): () => void {
  return subscribe({ kind: KIND_ENTITY }, (busEvent) => {
    const d = busEvent.data?.data as EntityStatusEvent | undefined
    if (!d || d.entity_type !== entityType) return
    // 过滤生命周期载荷（{change:'created'|'updated'|'deleted'}）：两者共用
    // KIND_ENTITY 频道，status 处理器只认运行时状态事件，避免把列表项的
    // status 抹成 undefined（lifecycle 载荷无 status 字段）
    if ('change' in d) return
    handler(d)
  })
}

/**
 * 实体生命周期变更事件（与后端 `publish_entity_changed` 的载荷对齐）
 *
 * - `created`：实体新建 → 前端可乐观插入清单（或防抖重拉）
 * - `updated`：实体元数据更新 → 防抖重拉清单
 * - `deleted`：实体删除 → 前端直接从清单移除（并清理选中态）
 */
export interface EntityChangedEvent {
  entity_type: string
  id: string
  change: 'created' | 'updated' | 'deleted'
  /** 实体展示名（后端尽力提供；created 乐观插入时可直接用作标题） */
  title?: string | null
}

/**
 * 订阅指定实体类型的生命周期变更（entity kind）
 *
 * 返回取消订阅函数。事件仅当 `entity_type` 匹配时回调。
 * 与 `subscribeEntityStatus`（运行时状态角标）互补：本事件驱动
 * 清单的增/删/改同步，保证详情页操作实时反映到列表。
 */
export interface EntityChangedEvent {
  entity_type: string
  id: string
  change: 'created' | 'updated' | 'deleted'
  title?: string | null
  /** 容器归属（如子会话的父会话 id）；顶层实体为 null。订阅端据此过滤作用域 */
  parent_id?: string | null
}

/** 订阅作用域：声明只接收哪个容器归属下的实体事件 */
export interface EntityChangedScope {
  /** null = 仅顶层实体（顶层清单用，子会话事件不会误入）；指定 id = 仅该容器下的子实体；缺省 = 不过滤 */
  parentId?: string | null
}

export function subscribeEntityChanged(
  entityType: string,
  handler: (e: EntityChangedEvent) => void,
  scope?: EntityChangedScope
): () => void {
  // 作用域过滤收敛在 dispatch 一处：后端消息通道与前端本地通道共用同一判定，
  // 保证「通道唯一、作用域在订阅端声明」的机制约定。
  const dispatch = (e: EntityChangedEvent) => {
    if (scope && scope.parentId !== undefined && (e.parent_id ?? null) !== scope.parentId) return
    handler(e)
  }

  // 前端模式通道：注册进本地注册表（publishEntityChangedLocal 的投递目标）
  let localHandlers = S.localEntityHandlers.get(entityType)
  if (!localHandlers) {
    localHandlers = new Set()
    S.localEntityHandlers.set(entityType, localHandlers)
  }
  localHandlers.add(dispatch)

  // 后端消息通道：订阅事件总线
  const unsub = subscribe({ kind: KIND_ENTITY }, (busEvent) => {
    const d = busEvent.data?.data as EntityChangedEvent | undefined
    if (!d || d.entity_type !== entityType) return
    if (d.change !== 'created' && d.change !== 'updated' && d.change !== 'deleted') return
    dispatch(d)
  })

  return () => {
    unsub()
    localHandlers?.delete(dispatch)
    if (localHandlers && localHandlers.size === 0) S.localEntityHandlers.delete(entityType)
  }
}

// ===== 前端模式：页面间本地通知（与后端事件同构） =====
//
// 实体列表同步有两种模式，列表同步器经同一订阅入口（subscribeEntityChanged）收敛：
// - 后端消息模式（主通道）：后端增删改实体 → publish_entity_changed → 事件总线，
//   跨窗口一致的唯一事实源；
// - 前端模式（乐观更新）：操作发起方已本地变更数据（如 store 内直接改 list），
//   经 publishEntityChangedLocal 以**同构载荷**即时通知其他页面，不等事件往返；
//   后端事件随后到达，同步器幂等处理（防抖重拉收敛到服务端真相）。

/** 前端模式通知：本地发布实体生命周期变更（载荷与后端 publish_entity_changed 同构） */
export function publishEntityChangedLocal(
  entityType: string,
  change: EntityChangedEvent['change'],
  id: string,
  title?: string | null
): void {
  const event: EntityChangedEvent = { entity_type: entityType, id, change, title: title ?? null }
  S.localEntityHandlers.get(entityType)?.forEach((h) => {
    try {
      h(event)
    } catch (err) {
      logger.error('[eventBus]', 'local entity-changed handler 执行失败', err)
    }
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
    S.connection = null
    scheduleReconnect()
    return
  }

  // 2. 业务事件：派发给订阅者
  if (event.type === 'bus_event') {
    const busEvent = event as unknown as BusEvent
    if (!busEvent.data || typeof busEvent.data !== 'object') return

    const { kind, session_id } = busEvent.data
    if (!kind) return

    for (const sub of S.subscribers) {
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
          S.replayBuffer.has(sub.filter.sessionId)
        ) {
          let buf = S.replayBuffer.get(sub.filter.sessionId)
          if (!buf) {
            buf = []
            S.replayBuffer.set(sub.filter.sessionId, buf)
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
  if (S.reconnectTimer) return
  const delay = S.reconnectDelay
  logger.info('[event-bus]', `Reconnecting in ${delay}ms...`)
  S.reconnectTimer = setTimeout(async () => {
    S.reconnectTimer = null
    S.reconnectDelay = Math.min(S.reconnectDelay * 2, _maxReconnectDelay)
    try {
      await connectEventBus()
    } catch {
      // scheduleReconnect 已在 connectEventBus 中处理
    }
  }, delay)
}
