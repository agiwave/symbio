/**
 * sessionInboxSync — 收件箱（**排队中**的用户消息）的实时面消费端
 *
 * ## 它补的是哪一个窗口
 *
 * 用户消息一旦被消费，后端就发权威帧（落库回包，见
 * `../../../symbio/src/plugins/session/docs/109-vdfs-session-messages.md` §3.4），
 * 转写消费端（`sessionTranscriptSync`）照实显示——**无论发送方是谁**
 * （前端自己 / CLI / telegram / 心跳 / 别的进程）。
 *
 * 唯一看不见的窗口是**排队中**：消息已入队、空间还在忙，要等当前轮跑完才被消费。
 * 前端对这一段的呈现曾经只有 `working` 乐观置位（空白流 + 一条等待骨架），
 * 而「不是我发的消息」连这个都没有。本模块把那段时间补上：队列里有几条、
 * 各是什么，逐条显示为「待发送」气泡。
 *
 * ## 与另外两个消费端的分工（同一棵树上的三个地址段）
 *
 * 会话是容器，其下是若干并列集合，地址形状统一 `<挂载目录>/<sid>/<集合段>/<项 id>`：
 *
 * | 地址 | 归属 |
 * |---|---|
 * | `<挂载目录>/<sid>` | 会话节点自身（运行态 / 资源）→ `sessionNodeSync` |
 * | `<挂载目录>/<sid>/<转写段>/<mid>` | 一条**已落库**消息 → `sessionTranscriptSync` |
 * | `<挂载目录>/<sid>/<收件箱段>/<iid>` | 一条**待消费**消息 → **本模块** |
 *
 * 三者互为前提、互不重叠：转写段与收件箱段是同一个会话下的两个并列集合，
 * 段名都是运行期数据（`ensureSessionScheme` 按 `kind` 认，不写死）。
 *
 * ## 帧语义：载荷给身份与预览，无载荷回读只分辨删除
 *
 * 信封没有操作枚举，语义按**载荷有无**分派（与另两个消费端同一套规矩）：
 *
 * - **带载荷** = 入队通知（`enqueue_inbox` 发的是 `inbox_item_node` 的节点视图）
 *   ⇒ 就地加一条。正文只到**预览**（`description` = `待消费 · <前 60 字>`）——
 *   排队气泡不需要全文，全文由它落库后的转写帧给，同一个 id，所以是「换位置」
 *   而不是「多一条」。
 * - **无载荷** = 出队 / 取消 / 清空（`drain_inbox_once` / `cancel_inbox_item` /
 *   `clear_inbox` 发的都是 bare 变更）⇒ 回读 `stat` 只分辨**删除与否**：
 *   `NotFound` ⇒ 就地移除。不回读正文：那时条目已经不在队列里。
 *
 * ## 重同步
 *
 * 总线曾满 / 连接曾断开时本端可能漏帧（漏一条入队 = 少一个气泡；漏一条出队 =
 * 一个永不消失的幽灵气泡）。resync  ⇒ 按**见过收件箱变更的会话**整份重列，
 * 权威清单直接替换本地。
 *
 * ## 已知代价（写清楚，别当成免费）
 *
 * **冷启动不补列**：应用起来之前就排在队里的消息，本端看不到它的气泡（列表里
 * 也没有「待发送」标记）。它不影响任何正确性——出队时照样收到 bare 变更、
 * 照样 `NotFound` 空操作移除；只是那一小段没有可视化。调用方（`ChatMainPanel`
 * 打开会话时）补一次 `refreshInbox` 即闭合，代价是每次打开会话多一次
 * `vdfs/list`。
 */

import { subscribeVdfsChanged } from '@/services/eventBus'
import { statVdfs } from '@/services/vdfs'
import { READBACK_REASON } from '@/services/readback'
import { ensureSessionScheme } from '@/services/vdfsScheme'
import { type VdfsChange, type VdfsNode } from '@/schemas/vdfs'
import { logger } from '@/utils/logger'

/** 一条排队中的用户消息（气泡的全部所需：地址末段即身份 + 展示文案） */
export interface InboxEntry {
  /** 条目 id = 地址末段，也是它落库后那条消息节点的 id（**同一个 id**） */
  id: string
  /** 展示文案：后端给的 `description`（`待消费 · <预览>`） */
  label: string
}

/**
 * 收件箱的落地目标（**依赖倒置**：本模块不反向依赖 Pinia store）。
 *
 * 生产由应用外壳传 `useSessionsStore()`，测试传普通对象即可。
 */
export interface SessionInboxSink {
  /** 入队通知（带节点视图）⇒ 加一条 / 更新一条 */
  applyInboxItem(sessionId: string, entry: InboxEntry): void
  /** 出队通知（无载荷，回读确认已不在）⇒ 移除一条 */
  removeInboxItem(sessionId: string, itemId: string): void
  /** 重同步 / 打开会话时整份重列（权威清单替换本地） */
  reloadInbox(sessionId: string): void | Promise<void>
}

interface InboxSyncState {
  sink: SessionInboxSink | null
  unsubscribes: Array<() => void>
  /** 已登记的挂载目录 → 收件箱段名（按挂载目录缓存，与另两个消费端同由） */
  mounts: Map<string, string>
  /** 见过收件箱变更的会话（resync 时按它重列） */
  seenSessions: Set<string>
}

/**
 * 内部状态（globalThis 单例）。
 *
 * 理由与 `eventBus` / `sessionTranscriptSync` 相同：Vite HMR 会重新执行本模块，
 * 模块级变量重置后旧订阅成为孤儿，再次启动即叠一层监听器——同一次变更被处理两遍。
 */
const _G = globalThis as typeof globalThis & { __symInboxSyncState?: InboxSyncState }
const S: InboxSyncState = _G.__symInboxSyncState ?? (_G.__symInboxSyncState = {
  sink: null,
  unsubscribes: [],
  mounts: new Map(),
  seenSessions: new Set(),
})
S.mounts ??= new Map()
S.seenSessions ??= new Set()
S.unsubscribes ??= []

/**
 * 变更地址落在哪个已登记的挂载目录下（没有就是不属于本消费端）。
 *
 * 取**最长**匹配：挂载目录互为前缀时（`<根>/session` 与 `<根>/session/…` 不会，
 * 但 `<根>/agent` 之下可能），最长匹配才是「属于谁」的正确答案。
 */
function mountOf(path: string): string | null {
  let best: string | null = null
  for (const m of S.mounts.keys()) {
    if (!path.startsWith(`${m}/`)) continue
    if (best === null || m.length > best.length) best = m
  }
  return best
}

/**
 * 处理一条变更（订阅回调与单测共用同一入口）。
 *
 * 分派只按**地址形状**（`<sid>/<收件箱段>/<iid>` 恰三段才是条目）与**载荷有无**，
 * 没有需要推断的操作枚举。
 */
export function handleVdfsChange(change: VdfsChange): void {
  const sink = S.sink
  if (!sink) return
  const path = change.path
  if (!path) return
  // 「属于哪个空间」由**地址**回答：登记过的挂载目录里取最长匹配
  const mountDir = mountOf(path)
  if (!mountDir) return
  const inboxSeg = S.mounts.get(mountDir)
  if (!inboxSeg) return

  // 地址形状（与转写消费端同一套判据）：
  // - 一段 = 会话节点自身 → 归 `sessionNodeSync`；
  // - 两段 = 集合目录（`<sid>/<收件箱段>`）→ 后端不发这种变更（入队 / 出队 /
  //   取消 / 清空逐条落在条目自己的地址上），收到也不该由本模块解释；
  // - 三段且中段是收件箱段 ⇒ **本模块的条目**。
  const segs = path.slice(mountDir.length + 1).split('/')
  if (segs.length !== 3 || segs[1] !== inboxSeg) return
  const sid = segs[0]
  const iid = segs[2]
  if (!sid || !iid) return
  S.seenSessions.add(sid)

  const view = change.data
  if (view != null && typeof view === 'object') {
    // 带载荷 = 入队。身份就是地址末段，载荷里有没有 `id` 不影响分派；两者不一致
    // 是生产端的 bug，喊出来但不丢帧。
    const node = view as VdfsNode
    if (node.name !== undefined && node.name !== iid) {
      logger.warn('[inbox-sync]', `帧身份与地址不一致（按地址落地）：${path}`)
    }
    sink.applyInboxItem(sid, { id: iid, label: inboxLabelOf(node) })
    return
  }

  // 无载荷 = 出队 / 取消 / 清空。回读只分辨**删除与否**——条目已不在队列里，
  // 读不到正文也不该读（IPC 换不来任何要显示的东西）。
  void statVdfs(READBACK_REASON.RESOURCE_SIGNAL, path)
    .then((node) => {
      if (!node) sink.removeInboxItem(sid, iid)
    })
    .catch((err: unknown) =>
      logger.warn('[inbox-sync]', `回读收件箱条目失败：${path}`, err),
    )
}

/**
 * 气泡文案：后端 `inbox_item_node` 给的 `description`（`待消费 · <预览>`）。
 *
 * 用**预览**而不是回读全文：排队气泡是状态提示，全文由它落库后的转写帧给
 * （同一个 id，落地即「换位置」）。为空（纯附件消息 / 空正文）时退回段名，
 * 至少让气泡有东西可显示。
 */
function inboxLabelOf(node: VdfsNode): string {
  const d = typeof node.description === 'string' ? node.description.trim() : ''
  return d.length > 0 ? d : '待发送'
}

/** 为一个挂载目录挂订阅（每个目录一份：前缀不同，登记也不同） */
function subscribeMount(mountDir: string, sink: SessionInboxSink): () => void {
  return subscribeVdfsChanged(
    { prefix: mountDir },
    handleVdfsChange,
    () => {
      // 重同步：漏帧会让本地多一个幽灵气泡或少一个气泡——按见过的会话整份重列，
      // 权威清单直接替换本地（与转写消费端的 resync 同构）。
      for (const sid of [...S.seenSessions]) {
        void Promise.resolve(sink.reloadInbox(sid)).catch((err: unknown) =>
          logger.warn('[inbox-sync]', `重列收件箱失败：${sid}`, err),
        )
      }
    },
  )
}

/**
 * 登记一个会话挂载目录（幂等）。
 *
 * 进入子智能体空间、或选中该空间里的会话时调用——那棵子树的 `session` 挂载
 * 到此才进入视野，其下收件箱的地址前缀与根那份**不同**。
 *
 * 与 `registerTranscriptMount` / `registerSessionMount` 是**三个消费端各自的登记**
 * （同一个目录要被三份订阅各认一次），名字因此分开。
 *
 * 解析失败（该挂载下还没有任何会话 ⇒ 推导不出集合段）返回 `false`，调用方
 * 在新建会话 / 清单刷新后补一次即可。
 */
export async function registerInboxMount(mountDir: string): Promise<boolean> {
  if (!mountDir || S.mounts.has(mountDir)) return false
  let inboxSeg: string
  try {
    inboxSeg = (await ensureSessionScheme(mountDir)).inboxSeg
  } catch (err) {
    logger.warn('[inbox-sync]', `会话挂载目录 ${mountDir} 的收件箱段未解析`, err)
    return false
  }
  // 解析期间可能已被另一次调用登记（同一个目录并发进入）——后到者不重复挂
  if (S.mounts.has(mountDir)) return false
  S.mounts.set(mountDir, inboxSeg)
  if (S.sink) S.unsubscribes.push(subscribeMount(mountDir, S.sink))
  logger.info('[inbox-sync]', '登记会话挂载目录', mountDir)
  return true
}

/**
 * 启动收件箱同步（先停旧订阅再挂新的 → 进程内天然单订阅）。
 *
 * @param sink 落地目标。**必须显式注入**（生产由应用外壳传 `useSessionsStore()`，
 *   测试可传普通对象）——本模块不认识 Pinia。
 */
export async function startSessionInboxSync(sink: SessionInboxSink): Promise<void> {
  stopSessionInboxSync()
  S.sink = sink

  // 默认挂载目录（根空间那份）。段名是运行期数据，解析失败就不订阅：
  // 宁可没有订阅，也不要订到一个拼错的 prefix 上（那样侧栏的待发送气泡
  // 永远不出现，而它与「本来就没有排队消息」在界面上完全一样）。
  let scheme: Awaited<ReturnType<typeof ensureSessionScheme>>
  try {
    scheme = await ensureSessionScheme()
  } catch (err) {
    logger.error('[inbox-sync]', '会话挂载目录解析失败，订阅未启动', err)
    return
  }
  if (!S.mounts.has(scheme.mountDir)) {
    S.mounts.set(scheme.mountDir, scheme.inboxSeg)
  }
  S.unsubscribes = [...S.mounts.keys()].map((m) => subscribeMount(m, sink))
  logger.info('[inbox-sync]', '收件箱同步已启动', [...S.mounts.keys()])
}

/** 停止收件箱同步（重复启动 / HMR / 测试时调用） */
export function stopSessionInboxSync(): void {
  for (const u of S.unsubscribes) u()
  S.unsubscribes = []
  S.sink = null
  S.seenSessions.clear()
  // S.mounts 保留：地址方案与订阅无关，重启动省一次解析
  // （`resetSessionInboxSyncForTest` 才清）
}

/** 测试辅助：清空全部内部状态（globalThis 单例跨用例残留时调用） */
export function resetSessionInboxSyncForTest(): void {
  stopSessionInboxSync()
  S.mounts.clear()
}

/** 测试辅助：某个挂载目录当前的收件箱段名（断言登记结果用） */
export function inboxSegOf(mountDir: string): string | undefined {
  return S.mounts.get(mountDir)
}
