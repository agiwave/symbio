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
 * - **不假设顺序**：状态由节点承载（幂等全量视图），因此本模块**没有**
 *   「回放缓冲 / 快照补帧」那类补丁——见 `subscribe` 的说明。
 */

import { connectPlugin, type Connection, type ConnectEvent } from './plugin'
import { watchVdfs, unwatchVdfs } from './vdfs'
import { logger } from '@/utils/logger'
import { VDFS_BUS_RESYNC, VDFS_EVENT_KIND, type VdfsChange } from '@/schemas/vdfs'
import { vdfsRoot } from '@/schemas/vdfsRoot'
import { EVENT_BUS_SUBSCRIBE } from '@/constants/pluginPaths'

/**
 * 从后端 `event_bus` 收到的统一事件结构
 */
export interface BusEvent {
  type: 'bus_event'
  data: {
    // 闭集：'vdfs'（资源变更，业务身份在载荷的地址里）| 'system'（总线自身握手）
    kind: string
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
  /** 前端模式：页面间本地通知注册表（资源变更，与后端事件同构） */
  localVdfsHandlers: Set<(change: VdfsChange) => void>
  /**
   * 后端明示「你可能漏了变更」时（通道曾满）要执行的重读动作。
   *
   * 为什么需要它：本频道**没有序号**，丢帧本身不可检测。后端在通道满时会丢帧但
   * 保留订阅，并补送一条 resync 指令（`VDFS_BUS_RESYNC`）——这是消费端唯一能知道
   * 「我刚才漏了东西」的途径。登记了本动作的作用域会整份重读，未登记者靠
   * 下一次变更或用户导航自愈（幂等全量视图下这是安全的，只是慢一点）。
   */
  resyncHandlers: Set<() => void>
  /**
   * 已向后端登记的路径 → 引用计数。
   *
   * 后端 `vdfs/watch` 是按路径引用计数的（`ChangeSubscriptions`），前端必须
   * 一一对应地登记与摘除，否则要么重复投递（叠字）、要么提前摘掉别人的订阅。
   */
  vdfsWatchCounts: Map<string, number>
  /**
   * 每个路径的在途 watch / unwatch 链。
   *
   * 登记与摘除都是异步调用，**乱序到达即永久错位**：快速切换会话时，
   * `unwatch(A)` 若晚于 `watch(A)` 发出并先到后端，计数就被清零，随后
   * `watch(A)` 又建一条——此后该路径的订阅再也摘不掉（幽灵订阅）。
   * 链上每一环等前一环结束，保证后端的计数变化严格按本端的发起顺序应用。
   */
  vdfsWatchChain: Map<string, Promise<void>>
}

const _G = globalThis as typeof globalThis & { __symEventBusState?: EventBusState }
const S: EventBusState = _G.__symEventBusState ?? (_G.__symEventBusState = {
  connection: null,
  connectionPromise: null,
  subscribers: new Set(),
  nextSubId: 1,
  reconnectTimer: null,
  reconnectDelay: 1000,
  localVdfsHandlers: new Set(),
  resyncHandlers: new Set(),
  vdfsWatchCounts: new Map(),
  vdfsWatchChain: new Map()
})
const _maxReconnectDelay = 30000

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
    const conn = await connectPlugin(EVENT_BUS_SUBSCRIBE, {}, handleConnectionEvent, {})
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
 * 订阅事件总线
 *
 * - `filter.kind` 必填（如 `VDFS_EVENT_KIND`）
 * - `filter.sessionId` 可选（null/undefined 接收所有该 kind 的事件）
 * - 首次订阅时会自动触发总线连接；连接是全局单例
 *
 * ## 这里**没有**回放缓冲（S20 删除）
 *
 * 曾经有一段「切换会话防乱序」的机制：订阅具体 `sessionId` 时先拉
 * `event_bus/pending/snapshot`，在快照派发完之前把实时事件缓存起来，再按
 * 「先快照 → 后实时」的顺序派发——因为 `Status` / `Abort` 这类事件的**顺序**
 * 决定 UI 对错。
 *
 * 那段机制存在的唯一理由，就是「事件是有顺序的」这个前提。会话显示现在不再
 * 消费任何事件：运行态是**会话节点的属性**，变更携带全量节点视图，因此幂等、
 * 可交换、丢一次也不影响正确性（见
 * `symbio/src/plugins/session/docs/node-state-streaming.md` §4）。
 * 前提没了，补丁也就不需要了。
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

  return () => {
    S.subscribers.delete(sub)
  }
}

// ===== 资源变更：唯一的变更频道 + 按路径前缀分流 =====
//
// VDFS 是唯一的资源协议，后端只发 `kind = VDFS_EVENT_KIND` 一条频道，载荷是
// `VdfsChange { path, data? }`（`schemas/vdfs.ts`）。
// 「这条变更属于哪一类资源 / 哪一个会话」由**展示地址前缀**表达，不再靠第二条频道。
//
// ## 信封没有操作枚举（S27）：`{path, data?}`，语义全在 `data` 的字段上
//
// `path` **恒为被变更节点自身的地址**：会话是容器，其下是若干并列的集合（消息 /
// 子会话 / 记忆 / 工作目录，后续还会有任务列表、请求队列……），集合项的形状统一为
// `<sid>/<集合段>/<项 id>`——消息的落点是 `<sid>/message/<mid>` 这个节点，**身份
// 就是地址末段**（与 `data.id` 同一事实，以地址为准）。`data` 是业务载荷：消息帧上
// 是 `ChatMessage`（`delta` 追加 / `content` 替换 / `status = removed` 移除）、
// 会话节点上是 `VdfsNode` 全量视图；缺失 = 无载荷，回读收敛（资源删除回读
// `NotFound` 即删除）。这里曾有过 `created` / `updated` / `deleted` 操作枚举与
// 一张「按变更频率分配载荷宽度」的表，全部**没有生产性生产者**（或与载荷语义
// 重复），S27 整体退役——判据见 `schemas/vdfs.VdfsChange`。
//
// 顺序敏感的实时面（会话运行态 / 消息正文）**也在这条通道上**（ADR-025）：
// 顺序是**节点属性**（`ChatMessage.seq` 决定显示顺序），不靠到达顺序——消费端
// 的协调规则见 `stores/sessionTranscriptSync.ts`。

/** 订阅作用域：按展示地址前缀分流（哪一类资源、哪个会话） */
export interface VdfsChangeScope {
  /** 资源前缀（如 `<根>/session`）；前缀本身与其子树内的变更都算命中 */
  prefix: string
  /**
   * 只接收前缀的**直接子项**（`<根>/session/<id>` 命中，
   * `<根>/session/<id>/message/<mid>` 不命中）。
   * 缺省 = 整个子树。
   */
  directChildren?: boolean
}

/** 前缀规整：去尾部斜杠 */
function normPrefix(prefix: string): string {
  return prefix.replace(/\/+$/, '')
}

/** 变更路径是否落在订阅作用域内（导出以便单测直接验证判定本身） */
export function vdfsChangeInScope(scope: VdfsChangeScope, path: string): boolean {
  const prefix = normPrefix(scope.prefix)
  if (!prefix) return true
  if (scope.directChildren) {
    // 只认「前缀 / 一段」；前缀自身不是自己的子项（会话清单目录的变更
    // 不属于任何一个会话叶子，落进来会凭空造出一个 id）
    if (!path.startsWith(`${prefix}/`)) return false
    return !path.slice(prefix.length + 1).includes('/')
  }
  return path === prefix || path.startsWith(`${prefix}/`)
}

/**
 * 订阅一类资源的变更（后端总线 + 前端本地通道共用同一作用域判定）。
 *
 * 本函数只做「频道 + 前缀」两件事，**不解释变更语义**——哪些变更重拉、
 * 哪些变更就地应用，归消费者。
 *
 * ## 向后端登记 watch（本函数不可省的一半）
 *
 * 后端只向**登记过路径**的订阅者投递变更（`vdfs/watch` → `ChangeSubscriptions`）。
 * 只 `subscribe` 总线而不登记，等于在一条没人开闸的频道上等事件——前端模式
 * （`publishVdfsChangedLocal`）照常工作、单测照常通过，接上真实后端后**一条
 * 变更都收不到**。因此这里按作用域前缀登记，并在最后一个订阅者撤走时摘除。
 *
 * 登记是异步的且**不能等**：订阅必须在同步语义下立即生效（否则订阅发生在
 * await 之前、期间的变更全部丢失），所以放进顺序链里 fire-and-forget。
 *
 * @returns 取消订阅函数
 */
export function subscribeVdfsChanged(
  scope: VdfsChangeScope,
  handler: (change: VdfsChange) => void,
  onResync?: () => void
): () => void {
  const dispatch = (change: VdfsChange) => {
    if (!change || typeof change.path !== 'string') return
    if (!vdfsChangeInScope(scope, change.path)) return
    handler(change)
  }

  // 前端模式通道：注册进本地注册表（publishVdfsChangedLocal 的投递目标）
  S.localVdfsHandlers.add(dispatch)
  // 后端「你漏了变更」指令的落地：由调用方决定怎么重读自己的作用域
  if (onResync) S.resyncHandlers.add(onResync)

  // 后端消息通道：订阅事件总线的 vdfs 频道
  const unsub = subscribe({ kind: VDFS_EVENT_KIND }, (busEvent) => {
    dispatch(busEvent.data?.data as VdfsChange)
  })

  // 向后端登记作用域前缀（同一前缀的多个订阅者共享一次登记）。
  // `<根>` 根本身不在任何 provider 身上、无实时能力，登记它会换来后端报错——
  // 订根的人只能靠各自拉取，这里与文件浏览器的处理保持一致。
  const watchPath = normPrefix(scope.prefix)
  if (watchPath && watchPath !== vdfsRoot()) {
    setVdfsWatch(watchPath, 1)
  }

  let released = false
  return () => {
    if (released) return
    released = true
    unsub()
    S.localVdfsHandlers.delete(dispatch)
    if (onResync) S.resyncHandlers.delete(onResync)
    if (watchPath && watchPath !== vdfsRoot()) setVdfsWatch(watchPath, -1)
  }
}

/**
 * 登记 / 摘除一条后端订阅（引用计数 + 串行链）。
 *
 * `+1` 表示新增一个消费者，`-1` 表示释放一个；计数归零才真正 `vdfs/unwatch`。
 * 同一路径上的调用严格按发起顺序执行，避免 watch/unwatch 乱序导致的永久错位。
 */
function setVdfsWatch(path: string, delta: 1 | -1): void {
  const prev = S.vdfsWatchChain.get(path) ?? Promise.resolve()
  const next = prev.then(async () => {
    const cur = S.vdfsWatchCounts.get(path) ?? 0
    const after = cur + delta
    if (after <= 0) {
      if (cur > 0) await unwatchVdfs(path)
      S.vdfsWatchCounts.delete(path)
      return
    }
    // watchVdfs / unwatchVdfs 自身吞错（无实时能力的 provider 由后端 no-op），
    // 因此这里不区分成败：计数照常推进，链路永不断。
    if (cur === 0) await watchVdfs(path)
    S.vdfsWatchCounts.set(path, after)
  })
  // 兜底：链上任何意外都不得阻断后续环节（否则该路径的登记会永久卡死）
  S.vdfsWatchChain.set(
    path,
    next.catch((err: unknown) => {
      logger.warn('[event-bus]', `VDFS watch 登记异常 path=${path} delta=${delta}`, err)
    })
  )
}

// ===== 前端模式：页面间本地通知（与后端事件同构） =====
//
// 清单同步有两种模式，同步器经同一订阅入口（subscribeVdfsChanged）收敛：
// - 后端消息模式（主通道）：后端增删改节点 → `notify_change` → 事件总线，
//   跨窗口一致的唯一事实源；
// - 前端模式（乐观更新）：操作发起方已本地变更数据（如 store 内直接改 list），
//   经 publishVdfsChangedLocal 以**同构载荷**即时通知其他页面，不等事件往返；
//   后端事件随后到达，同步器幂等处理（防抖重拉收敛到服务端真相）。

/** 前端模式通知：本地发布一条资源变更（载荷与后端 `VdfsChange` 同构，无载荷信封） */
export function publishVdfsChangedLocal(change: VdfsChange): void {
  for (const h of S.localVdfsHandlers) {
    try {
      h(change)
    } catch (err) {
      logger.error('[eventBus]', 'local vdfs-changed handler 执行失败', err)
    }
  }
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

    // 重同步指令先于订阅者派发处理掉。
    //
    // 它**不是一条变更**（没有 `path`），若放它进派发循环，消费者会把它当成
    // 「一条形状不对的变更」——`subscribeVdfsChanged` 的 `dispatch` 恰好会忽略它，
    // 但裸 `subscribe` 的消费者不会。因此在这里一次性截断，语义边界才清晰。
    const payload = busEvent.data.data as { type?: string } | null | undefined
    if (payload?.type === VDFS_BUS_RESYNC) {
      logger.warn('[event-bus]', '收到 vdfs 重同步指令（后端通道曾满），重读全部已登记作用域')
      for (const cb of S.resyncHandlers) {
        try {
          cb()
        } catch (e) {
          logger.error('[event-bus]', 'resync handler 执行失败:', e)
        }
      }
      return
    }

    for (const sub of S.subscribers) {
      if (sub.filter.kind !== kind) continue
      // sessionId 过滤：null 接收所有；否则精确匹配
      if (sub.filter.sessionId != null && sub.filter.sessionId !== session_id) continue
      try {
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
