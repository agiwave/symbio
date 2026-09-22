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
 * 启动即「先停旧订阅再挂新的」（进程内天然单订阅，无需额外守卫标记）；
 * **合并策略**（此处防抖重拉、转写处逐路径串行链）则各自保留，因为那是业务差异而非重复。
 *
 * ## 清单同步的双模式
 *
 * - **后端消息模式**（本订阅）：后端增删改节点 → `notify_change`（唯一的 `vdfs`
 *   频道）→ 此处收敛（跨窗口一致的唯一事实源）。载荷是粗粒度的（只有 path +
 *   change，不带快照），所以：`deleted` 本地即时移除、`created` 防抖重拉、
 *   `updated` 交给 sink 用**载荷**就地收敛。
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
  VDFS_CHANGE_UPDATED,
  vdfsBase,
  type VdfsChange,
} from '@/schemas/vdfs'
import { logger } from '@/utils/logger'

/** 变更的落地目标（由外壳注入真实 store；本模块不认识 Pinia） */
export interface SessionNodeSink {
  /** 整表重拉（created / renamed 等粗粒度变更的收敛口） */
  refreshList(): void | Promise<void>
  /** 本地即时移除（deleted） */
  removeSessionLocal(id: string): void
  /** 带载荷的状态迁移就地收敛（updated，零回读） */
  applySessionNode(id: string, change: VdfsChange): void
}

let _unsubscribe: (() => void) | null = null
let _listRefreshTimer: ReturnType<typeof setTimeout> | null = null

/** 防抖重拉：created / renamed 等只给地址的变更，用于收敛排序与完整字段 */
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
      if (change.change === VDFS_CHANGE_UPDATED) {
        sink.applySessionNode(id, change)
        return
      }
      // created / renamed 等：本地乐观插入已覆盖同窗口场景；
      // 此处防抖重拉，收敛排序与完整字段。
      scheduleListRefresh(sink)
    },
    // 重同步：后端通道曾满，本端可能漏了会话节点的状态迁移（如 `working → active`）。
    // 漏掉终态会让侧栏永远停在「运行中」——而清单是幂等全量视图，整表重拉即权威收敛。
    () => scheduleListRefresh(sink),
  )
}
