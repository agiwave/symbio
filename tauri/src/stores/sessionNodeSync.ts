/**
 * sessionNodeSync — 会话节点的 VDFS 变更 → 会话清单收敛
 *
 * ## 它原先长在哪儿
 *
 * 这段订阅原先写在 `stores/sessions.ts` 工厂函数的**尾部**：store 一被创建就订阅，
 * 既不能停、也不显式。后果有两个：
 *
 * 1. 订阅是**隐式副作用**——读代码的人看不到「清单为什么会变」，只能翻到文件末尾；
 * 2. HMR / 测试里每次建 store 都叠一层监听器（同一次变更被处理 N 次）。
 *
 * 现在它与 `sessionTranscriptSync` 同构：由**应用外壳**（`MainLayout`）显式启动，
 * 落地目标（sink）注入进来。于是 store 只有状态与动作，订阅是可启停的独立接线。
 * 启动即「先停旧订阅再挂新的」（进程内天然单订阅，无需额外守卫标记）。
 *
 * ## 它负责什么，不负责什么
 *
 * 地址形状是 `<根>/session/<sid>`——**恰一段**，即会话节点自身。它下面的东西
 * （`<sid>/<集合段>/<项 id>`）一概不进本模块：订阅作用域 `directChildren = true`
 * 就把它们挡在门外了（会话是容器，其下是若干并列的集合：消息 / 子会话 / 记忆 /
 * 工作目录，后续还会有任务列表、请求队列——各自有各自的消费端）。
 *
 * | 会话节点上变的东西 | 收敛方式 |
 * |---|---|
 * | **运行态**（`working` / 终态 / 结局 / 警告 / 错误） | 随 `data` 的全量节点视图 → `applySessionState` **零回读**就地落定 |
 * | **资源**（创建 / 删除 / 改名 / 标题 / metadata） | 无载荷 ⇒ 回读 `stat` **只分辨删除**（`NotFound` 即时移除）；其余防抖重拉清单 |
 *
 * ## 载荷怎么用：视图帧就地落定，无载荷回读分辨
 *
 * 信封没有操作枚举，分派只看载荷形状。运行态帧随 `data` 带节点视图
 * （后端与 `stat` 同一构造点构造，不是缓存副本）⇒ 直接落地，一次状态迁移
 * **零 IPC**。资源信号是**无载荷**变更——它们不携带视图，回读 `stat` 分辨
 * 删除与否；运行态不会被一条迟到的改名信号打回旧值（信号上根本没有状态可打，
 * 回读的 `message_count` / `updated_at` 又取自落库的会话摘要，一轮进行中会落后）。
 *
 * **回读只服务于删除判定**：会话转写订阅（`sessionTranscriptSync`）在同一个
 * `<sid>` 上另发一次 `stat` 做同一件事是多余的——同一路径同一时刻的两次 IPC。
 * 会话节点归本模块，转写模块只认集合项。
 *
 * ## 清单同步的双模式
 *
 * - **后端消息模式**（本订阅）：后端增删改会话节点 → `notify_change`（唯一的
 *   `vdfs` 频道）→ 此处收敛（跨窗口一致的唯一事实源）。对会话节点而言变更只有
 *   `path` + 载荷两个有意义的字段，所以有视图就落定、没视图就回读 + 防抖重拉。
 * - **前端模式**（乐观更新）：store 的 `createSession` / `deleteSession` 已直接改
 *   本地 list，并经 `publishVdfsChangedLocal` 以同构载荷即时通知其他页面，
 *   不等事件往返；后端事件随后幂等收敛。
 */

import { subscribeVdfsChanged } from '@/services/eventBus'
import { ensureSessionMountDir } from '@/services/vdfsScheme'
import { vdfsBase, type VdfsNode } from '@/schemas/vdfs'
import { statVdfs } from '@/services/vdfs'
import { READBACK_REASON } from '@/services/readback'
import { logger } from '@/utils/logger'

/** 变更的落地目标（由外壳注入真实 store；本模块不认识 Pinia） */
export interface SessionNodeSink {
  /** 整表重拉（资源信号的收敛口） */
  refreshList(): void | Promise<void>
  /** 本地即时移除（回读 `NotFound` 的落地口） */
  removeSessionLocal(id: string): void
  /** 会话节点状态迁移（运行态帧随 `data` 带全量节点视图，零回读） */
  applySessionState(id: string, node: VdfsNode): void
}

/**
 * 已登记的会话挂载目录。
 *
 * **不只一份**：子智能体空间（`agent/<id>/…`）是一棵完整子树，内部有自己的
 * `session` 挂载。只订根那一份会让子空间的会话节点变更**根本收不到**——
 * 它的地址是 `<根>/agent/<id>/session/<sid>`，匹配不上根挂载的前缀。
 * 「会话住在哪个空间」是地址的一部分，订阅因此按挂载目录逐份登记。
 */
const _mounts = new Set<string>()
let _sink: SessionNodeSink | null = null
let _unsubs: Array<() => void> = []
let _listRefreshTimer: ReturnType<typeof setTimeout> | null = null

/** 防抖重拉：只给地址的无载荷资源变更，用于收敛排序与完整字段 */
function scheduleListRefresh(sink: SessionNodeSink): void {
  if (_listRefreshTimer) clearTimeout(_listRefreshTimer)
  _listRefreshTimer = setTimeout(() => {
    _listRefreshTimer = null
    void Promise.resolve(sink.refreshList()).catch((err: unknown) =>
      logger.warn('[session-node-sync]', '资源变更触发清单刷新失败', err),
    )
  }, 800)
}

/**
 * 登记一个会话挂载目录（幂等）。
 *
 * 进入子智能体空间、或选中该空间里的会话时调用——那棵子树的 `session` 挂载
 * 到此才进入视野。已登记的目录重复调用是空操作。
 */
export async function registerSessionMount(mountDir: string): Promise<boolean> {
  if (!mountDir || _mounts.has(mountDir)) return false
  _mounts.add(mountDir)
  if (_sink) _unsubs.push(subscribeMount(mountDir, _sink))
  logger.info('[session-node-sync]', '登记会话挂载目录', mountDir)
  return true
}

/** 停止会话节点同步（重复启动 / HMR / 测试时调用；无订阅则空操作） */
export function stopSessionNodeSync(): void {
  for (const u of _unsubs) u()
  _unsubs = []
  if (_listRefreshTimer) {
    clearTimeout(_listRefreshTimer)
    _listRefreshTimer = null
  }
  _sink = null
}

/** 测试 / HMR 用：连已登记的挂载目录一起清空 */
export function resetSessionNodeSyncMounts(): void {
  stopSessionNodeSync()
  _mounts.clear()
}

/** 为一个挂载目录挂订阅（每个目录一份：前缀不同，登记也不同） */
function subscribeMount(mountDir: string, sink: SessionNodeSink): () => void {
  return subscribeVdfsChanged(
    { prefix: mountDir, directChildren: true },
    (change) => {
      const id = vdfsBase(change.path)
      if (!id) return
      // 信封没有操作枚举，语义按载荷形状分派：
      //
      // ① `data` = 全量节点视图（与 `stat` 同源构造）⇒ **零回读**就地落定
      //    状态 / 标题 / 计数——运行态是最需要即时的路径，一次状态迁移一次 IPC
      //    恰恰是最不该省的那一步。删除不在这条分支上：已删的会话取不到视图。
      // ② `data` 缺失（资源信号 / 删除）⇒ 回读 `stat` 只分辨**删除与否**：
      //    `NotFound` ⇒ 本地即时移除；有节点则**什么也不落**——运行态只有一个
      //    来源（随载荷的节点视图），一条迟到的改名信号不该把状态打回它那一刻
      //    的旧值（`stat` 的 `message_count` / `updated_at` 取自落库的会话摘要，
      //    一轮进行中会落后于本地）。其余收敛交给防抖重拉清单。
      const view = change.data
      if (view != null && typeof view === 'object') {
        sink.applySessionState(id, view as VdfsNode)
        return
      }
      void statVdfs(READBACK_REASON.RESOURCE_SIGNAL, change.path)
        .then((node) => {
          if (!node) sink.removeSessionLocal(id)
        })
        .catch((err: unknown) =>
          logger.warn('[session-node-sync]', `回读会话节点失败：${change.path}`, err),
        )
      scheduleListRefresh(sink)
    },
    // 重同步：后端通道曾满 / 连接曾断开，本端可能漏了会话节点的资源变更（漏掉删除
    // 信号会让侧栏留下一个已经不存在的会话）。清单是幂等全量视图，整表重拉即权威收敛。
    () => scheduleListRefresh(sink),
  )
}

/**
 * 启动会话节点同步（先停旧订阅再挂新的 → 进程内天然单订阅）。
 *
 * 与 `startTranscriptSync` 同构：重复启动不叠加监听器，而是替换。
 */
export async function startSessionNodeSync(sink: SessionNodeSink): Promise<void> {
  stopSessionNodeSync()
  _sink = sink

  // 挂载目录是运行期数据（按「可新建 ext=session 的挂载点」认出来），不是常量
  let mountDir: string
  try {
    mountDir = await ensureSessionMountDir()
  } catch (err) {
    // 解析不到就不订阅：宁可没有订阅，也不要订到一个拼错的 prefix 上
    // （那样侧栏会静默不更新，比报错难查）。
    logger.error('[session-node-sync]', '会话挂载目录解析失败，订阅未启动', err)
    return
  }
  if (!_mounts.has(mountDir)) _mounts.add(mountDir)
  _unsubs = [..._mounts].map((m) => subscribeMount(m, sink))
}
