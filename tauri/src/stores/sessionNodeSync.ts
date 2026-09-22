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
 * 现在它与 `transcriptStream` 同构：由**应用外壳**（`MainLayout`）显式启动，
 * 落地目标（sink）注入进来。于是 store 只有状态与动作，订阅是可启停的独立接线。
 * 启动即「先停旧订阅再挂新的」（进程内天然单订阅，无需额外守卫标记）。
 *
 * ## 它负责什么，不负责什么
 *
 * | 会话叶子上变的东西 | 通道 | 收敛方式 |
 * |---|---|---|
 * | **运行态**（`working` / 终态 / 结局 / 警告 / 错误） | 转写流 `transcript_session` | `sessions.applySessionState`（零回读） |
 * | **资源**（创建 / 删除 / 改名 / 标题 / metadata） | 本模块的 VDFS 订阅 | `deleted` 即时移除，其余防抖重拉清单 |
 *
 * ## 为什么这里的变更**不带节点快照**
 *
 * 会话叶子的节点快照只有两个来源，都**有序或幂等**：转写流的运行态帧（与消息共用
 * `seq` 空间 ⇒ 保序）与 `list` / `stat` 回读。VDFS 变更是一条**独立的无序通道**——
 * 在它上面捎带快照，一次迟到的改名就会把运行态**回退**成它自己那一刻的旧值
 * （自动命名发生在轮次中，快照说 `working`，而转写流早已报 `finished`）。
 * 所以运行态不在这条链路上，这里只做"资源变了"的收敛：重拉清单。
 *
 * ## 清单同步的双模式
 *
 * - **后端消息模式**（本订阅）：后端增删改会话叶子 → `notify_change`（唯一的 `vdfs`
 *   频道）→ 此处收敛（跨窗口一致的唯一事实源）。载荷是**粗粒度**的（只有 path +
 *   change），所以 `deleted` 本地即时移除、其余防抖重拉。
 * - **前端模式**（乐观更新）：store 的 `createSession` / `deleteSession` 已直接改
 *   本地 list，并经 `publishVdfsChangedLocal` 以同构载荷即时通知其他页面，
 *   不等事件往返；后端事件随后幂等收敛。
 *
 * 作用域 `directChildren = true`：只看会话叶子（`<根>/session/<id>`）——
 * 子会话与转写列表项的变更不进侧栏清单。
 */

import { subscribeVdfsChanged } from '@/services/eventBus'
import { ensureSessionMountDir } from '@/services/vdfsScheme'
import {
  VDFS_CHANGE_APPENDED,
  VDFS_CHANGE_DELETED,
  vdfsBase,
} from '@/schemas/vdfs'
import { logger } from '@/utils/logger'

/** 变更的落地目标（由外壳注入真实 store；本模块不认识 Pinia） */
export interface SessionNodeSink {
  /** 整表重拉（created / updated / renamed 等粗粒度变更的收敛口） */
  refreshList(): void | Promise<void>
  /** 本地即时移除（deleted） */
  removeSessionLocal(id: string): void
}

let _unsubscribe: (() => void) | null = null
let _listRefreshTimer: ReturnType<typeof setTimeout> | null = null

/** 防抖重拉：只给地址的粗粒度变更（created / updated / renamed），用于收敛排序与完整字段 */
function scheduleListRefresh(sink: SessionNodeSink): void {
  if (_listRefreshTimer) clearTimeout(_listRefreshTimer)
  _listRefreshTimer = setTimeout(() => {
    _listRefreshTimer = null
    void Promise.resolve(sink.refreshList()).catch((err: unknown) =>
      logger.warn('[session-node-sync]', '资源变更触发清单刷新失败', err),
    )
  }, 800)
}

/** 停止会话节点同步（重复启动 / HMR / 测试时调用；无订阅则空操作） */
export function stopSessionNodeSync(): void {
  if (_unsubscribe) {
    _unsubscribe()
    _unsubscribe = null
  }
  if (_listRefreshTimer) {
    clearTimeout(_listRefreshTimer)
    _listRefreshTimer = null
  }
}

/**
 * 启动会话节点同步（先停旧订阅再挂新的 → 进程内天然单订阅）。
 *
 * 与 `startTranscriptSync` 同构：重复启动不叠加监听器，而是替换。
 */
export async function startSessionNodeSync(sink: SessionNodeSink): Promise<void> {
  stopSessionNodeSync()

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

  _unsubscribe = subscribeVdfsChanged(
    { prefix: mountDir, directChildren: true },
    (change) => {
      // 追加型变更只发生在转写列表项上（由 transcriptStream 就地应用 delta），
      // 与会话清单无关——绝不能让流式的每一帧触发一次重拉。
      if (change.change === VDFS_CHANGE_APPENDED) return
      const id = vdfsBase(change.path)
      if (!id) return
      if (change.change === VDFS_CHANGE_DELETED) {
        sink.removeSessionLocal(id)
        return
      }
      // created / updated / renamed：本地乐观更新已覆盖同窗口场景；
      // 此处防抖重拉，收敛排序、标题与完整字段（**不看载荷**——它不带节点快照，
      // 见模块文档「为什么这里的变更不带节点快照」）。
      scheduleListRefresh(sink)
    },
    // 重同步：后端通道曾满，本端可能漏了会话叶子的资源变更（漏掉 `deleted` 会让
    // 侧栏留下一个已经不存在的会话）。清单是幂等全量视图，整表重拉即权威收敛。
    () => scheduleListRefresh(sink),
  )
}
