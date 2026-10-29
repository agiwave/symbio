// @vitest-environment happy-dom
/**
 * sessions store —— 清单收敛与运行态收敛
 *
 * 覆盖两条**互不重叠**的约定：
 *
 * 1. **清单只认无载荷的资源信号**：作用域按**展示地址前缀**分流（`@vfs/session`
 *    的直接子项 = 会话叶子；转写列表项 / 子会话的变更不进侧栏）。
 *    - 有地址、无载荷 → 防抖重拉清单（**不看载荷**：这条通道不带节点快照）；
 *    - 回读 `stat` 得 `NotFound` → 本地即时移除；
 *    - 且资源信号**不得**改运行态（快照只有有序来源，见 `sessionNodeSync` 文档）。
 * 2. **运行态只认带全量视图的节点帧**（`applySessionState`）：状态 / 结局 / 错误 /
 *    告警 / 提示音全在这里收敛，**零回读、零整表重拉**。
 */
import { describe, it, expect, beforeEach, vi } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import { flushPromises } from '@vue/test-utils'
import { statsOf, useReleasedSkeletonsStore } from '../skeletonLazy'

/**
 * 会话清单的订阅由 `startSessionNodeSync(store)` 建立（store 自己不挂监听器）；
 * 此处捕获作用域与回调以便直接投递事件。
 */
const captured = vi.hoisted(() => ({
  scopes: [] as Array<{ prefix: string; directChildren?: boolean }>,
  handlers: [] as Array<(e: unknown) => void>,
}))

vi.mock('@/services/eventBus', () => ({
  subscribeVdfsChanged: (
    scope: { prefix: string; directChildren?: boolean },
    handler: (e: unknown) => void
  ) => {
    captured.scopes.push(scope)
    captured.handlers.push(handler)
    return () => {}
  },
  publishVdfsChangedLocal: vi.fn(),
}))

const vdfsApi = vi.hoisted(() => ({
  readVdfs: vi.fn(),
  statVdfs: vi.fn(),
  writeVdfs: vi.fn(),
  watchVdfs: vi.fn(async () => {}),
  unwatchVdfs: vi.fn(async () => {}),
}))

const sessionApi = vi.hoisted(() => ({
  listSessions: vi.fn(async () => []),
  deleteMessage: vi.fn(),
  /** 会话 metadata 的后端写口（`services/session.updateSession`） */
  updateSession: vi.fn(async () => {}),
  /**
   * 读整份转写（门面 `services/session.readSessionTranscript`）。
   *
   * 这里复用 `readVdfs` 的桩实现同形状的解析——本文件的用例关心的是
   * **水合的合并语义**，不是文档解析；解析本身在 session service 那一层。
   */
  readSessionTranscript: vi.fn(async (): Promise<unknown[]> => {
    const c = await vdfsApi.readVdfs()
    const doc = JSON.parse(c?.text ?? '{}') as { messages?: unknown[] }
    return Array.isArray(doc.messages) ? doc.messages : []
  }),
}))
vi.mock('@/services/session', () => ({
  listSessions: sessionApi.listSessions,
  deleteSession: vi.fn(),
  updateSession: sessionApi.updateSession,
  readSessionTranscript: sessionApi.readSessionTranscript,
  deleteMessage: sessionApi.deleteMessage,
  updateMessage: vi.fn(),
}))
vi.mock('@/services/vdfs', () => ({
  readVdfs: vdfsApi.readVdfs,
  statVdfs: vdfsApi.statVdfs,
  writeVdfs: vdfsApi.writeVdfs,
  watchVdfs: vdfsApi.watchVdfs,
  unwatchVdfs: vdfsApi.unwatchVdfs,
}))
vi.mock('@/services/plugin', () => ({
  setLastWorkdir: vi.fn(),
  getLastWorkdir: () => '',
  callPlugin: vi.fn(),
}))

// 提示音是**副作用**（会碰 Web Audio）：此处只断言"在正确的状态迁移上被调用"
const chime = vi.hoisted(() => ({ playCompletionChime: vi.fn() }))
vi.mock('@/services/completionChime', () => ({
  playCompletionChime: chime.playCompletionChime,
}))

// 地址方案：注入夹具，避免真去列目录。
// `ensureSessionScheme(mountDir?)` 忠实于生产契约：返回的 `mountDir` 就是请求的
// 那个（省略 = 默认挂载目录那份）。
vi.mock('@/services/vdfsScheme', () => ({
  ensureSessionMountDir: vi.fn(async () => SCHEME.mountDir),
  ensureSessionScheme: vi.fn(async (mountDir?: string) => ({
    ...SCHEME,
    mountDir: mountDir ?? SCHEME.mountDir,
  })),
  vdfsSessionScheme: vi.fn(() => SCHEME),
}))

import { useSessionsStore } from '../sessions'
import { startSessionNodeSync, stopSessionNodeSync } from '../sessionNodeSync'
import { VDFS_STATUS_WORKING } from '@/schemas/vdfs'
// 回读理由是**词表**（独立模块，未被替身），断言按它取值——替身里不抄第二份
import { READBACK_REASON } from '@/services/readback'

/**
 * 协议夹具：会话挂载目录与两个集合段都是**运行期数据**（列目录认出来）。
 * 单测不去列目录，直接注入——断言仍把地址钉成字面量。
 *
 * `vi.hoisted`：`vi.mock` 工厂先于 import 执行，直接引用顶层 const 会撞 TDZ。
 */
const { SCHEME } = vi.hoisted(() => ({
  SCHEME: { mountDir: '@vfs/session', messagesSeg: 'message', inboxSeg: 'inbox' },
}))

/** 投递一条变更（路径就是展示地址） */
function emit(change: Record<string, unknown>) {
  captured.handlers[0](change)
}

function sessionNode(over: Record<string, unknown> = {}) {
  return {
    path: `${SCHEME.mountDir}/s1`,
    name: 's1',
    title: '解释一下 VDFS 的地址模型',
    kind: 'session',
    status: 'active',
    access: 'rw',
    ...over,
  }
}

describe('sessions store — VDFS 变更的清单收敛', () => {
  let store: ReturnType<typeof useSessionsStore>

  beforeEach(() => {
    setActivePinia(createPinia())
    captured.scopes.length = 0
    captured.handlers.length = 0
    // 订阅现在由外壳显式建立（生产侧是 MainLayout）
    stopSessionNodeSync()
    store = useSessionsStore()
    startSessionNodeSync(store)
    sessionApi.listSessions.mockClear()
    chime.playCompletionChime.mockClear()
    vdfsApi.statVdfs.mockReset()
    vdfsApi.statVdfs.mockResolvedValue(sessionNode())
    vdfsApi.writeVdfs.mockReset()
    vdfsApi.readVdfs.mockResolvedValue({ text: '{"messages":[]}' })
  })

  it('新建会话 = 一次 vdfs/write 到会话挂载根；id 由后端生成，前端不编造', async () => {
    // 匿名写的回执只给**名字**（`VdfsWriteResponse.name`）——地址由调用方拼
    vdfsApi.writeVdfs.mockResolvedValue({ name: 's9', created: true })
    const store = useSessionsStore()

    const id = await store.createSession({ workdir: 'D:/work' })

    expect(id).toBe('s9')
    expect(vdfsApi.writeVdfs).toHaveBeenCalledTimes(1)
    const [path, body, opts] = vdfsApi.writeVdfs.mock.calls[0] as [string, string, { create: boolean }]
    expect(path).toBe(SCHEME.mountDir)
    expect(opts).toEqual({ create: true })
    // 目标名不由前端给：会话名字来自首条消息（display_title），此处只带 metadata
    expect(JSON.parse(body)).toEqual({ metadata: expect.objectContaining({ workdir: 'D:/work' }) })
    expect(store.list[0]?.id).toBe('s9')
    expect(store.activeId).toBe('s9')
  })

  it('后端没给出新名字时报错，而不是插一条 id 为空的会话', async () => {
    vdfsApi.writeVdfs.mockResolvedValue({ created: true })
    const store = useSessionsStore()
    await expect(store.createSession()).rejects.toThrow()
    expect(store.list).toHaveLength(0)
  })

  // ── 懒创建握手：建会话 + 排队首条消息 ──────────────────────────────
  //
  // 「队列项的 id === 新建出来的 id」是这条链路的**唯一不变式**，且它必须由
  // store 保证：此前是调用方先拿 id 再赋值邮箱（两步），漏第二步就是静默丢首条
  // 消息。邮箱本体不对组件暴露，故这里只能经 `consumePendingFirstMessage` 观察
  // ——正因如此，下面这些断言同时也是「只有一扇门」的护栏。

  it('懒创建是原子操作：排队的首条消息归属于刚建出来的那个 id', async () => {
    vdfsApi.writeVdfs.mockResolvedValue({ name: 's9', created: true })
    const store = useSessionsStore()
    const img = { base64: 'AAA', mimeType: 'image/png', thumbnailUrl: 'blob:1' }

    const id = await store.createSessionWithFirstMessage(
      { workdir: 'D:/work' },
      { text: '你好', images: [img] }
    )

    expect(id).toBe('s9')
    expect(store.consumePendingFirstMessage('s9')).toEqual({
      id: 's9',
      text: '你好',
      images: [img],
    })
    // 取出即清除：邮箱不是可重复读的缓存，重复消费拿不到东西
    expect(store.consumePendingFirstMessage('s9')).toBeNull()
  })

  it('邮箱按 id 匹配，别的会话取不走属于本会话的首条消息', async () => {
    vdfsApi.writeVdfs.mockResolvedValue({ name: 's9', created: true })
    const store = useSessionsStore()
    await store.createSessionWithFirstMessage(undefined, { text: 'hi' })

    expect(store.consumePendingFirstMessage('s10')).toBeNull()
    expect(store.consumePendingFirstMessage('s9')?.text).toBe('hi')
  })

  it('创建失败不留半截队列项，也不污染邮箱里原有的那条', async () => {
    vdfsApi.writeVdfs.mockResolvedValue({ name: 's9', created: true })
    const store = useSessionsStore()
    await store.createSessionWithFirstMessage(undefined, { text: '第一条' })

    // 第二次创建失败（后端没回名字）
    vdfsApi.writeVdfs.mockResolvedValue({ created: true })
    await expect(
      store.createSessionWithFirstMessage(undefined, { text: '第二条' })
    ).rejects.toThrow()

    // 邮箱仍是第一次排的那条：失败的那次没写进任何东西
    expect(store.consumePendingFirstMessage('s9')?.text).toBe('第一条')
  })

  it('订阅恰好建立一次，作用域 = 会话叶子的直接子项', () => {
    expect(captured.handlers).toHaveLength(1)
    expect(captured.scopes[0]).toEqual({
      prefix: SCHEME.mountDir,
      directChildren: true,
    })
  })

  it('重复启动不再叠加订阅（幂等）', () => {
    startSessionNodeSync(store)
    startSessionNodeSync(store)
    expect(captured.handlers).toHaveLength(1)
  })

  it('未启动订阅时 store 是纯状态容器（建 store 不产生任何监听器）', () => {
    stopSessionNodeSync()
    captured.handlers.length = 0
    setActivePinia(createPinia())
    useSessionsStore()
    expect(captured.handlers).toHaveLength(0)
  })

  it('无载荷变更 ⇒ 回读 stat 分辨删除 + 防抖重拉清单', async () => {
    vi.useFakeTimers()
    try {
      const store = useSessionsStore()
      store.list.push({ id: 's1', message_count: 0, updated_at: 0, status: 'active', metadata: {} } as never)

      emit({ path: '@vfs/session/s1' })
      // 信封没有操作枚举：先回读 stat 分辨删除与否（NotFound ⇒ 本地即时移除）；
      // 理由随请求进 `metadata.origin`，路由留痕据此可判「这一次读是为什么」
      expect(vdfsApi.statVdfs).toHaveBeenCalledWith(
        READBACK_REASON.RESOURCE_SIGNAL,
        '@vfs/session/s1'
      )
      expect(sessionApi.listSessions).not.toHaveBeenCalled()

      await vi.advanceTimersByTimeAsync(800)
      expect(sessionApi.listSessions).toHaveBeenCalledTimes(1)
    } finally {
      vi.useRealTimers()
    }
  })

  it('资源信号（无载荷）**不**改运行态：状态只有一个来源（随载荷的节点视图）', async () => {
    const store = useSessionsStore()
    store.list.push({ id: 's1', message_count: 0, updated_at: 0, status: 'working', metadata: {} } as never)
    store.putStatus('s1', { status: 'working' })

    // 一次迟到的资源变更（如自动命名）是**无载荷**信号——信号上根本没有状态
    // 可打，运行态不会被它打回任何旧值（见 sessionNodeSync 模块文档）。
    emit({ path: '@vfs/session/s1' })
    await flushPromises()

    expect(store.getSessionStatus('s1').status).toBe('working')
    expect(store.list[0].status).toBe('working')
  })

  it('无载荷变更（新建）→ 防抖重拉清单（收敛排序与完整字段）', async () => {
    vi.useFakeTimers()
    try {
      const store = useSessionsStore()
      emit({ path: '@vfs/session/newsid' })
      expect(sessionApi.listSessions).not.toHaveBeenCalled()
      await vi.advanceTimersByTimeAsync(800)
      expect(sessionApi.listSessions).toHaveBeenCalledTimes(1)
      expect(store.list).toEqual([])
    } finally {
      vi.useRealTimers()
    }
  })

  it('删除（回读 NotFound）→ 本地即时移除，不等重拉', async () => {
    const store = useSessionsStore()
    store.list.push({ id: 's1', message_count: 0, updated_at: 0, metadata: {} } as never)
    vdfsApi.statVdfs.mockResolvedValue(null)

    emit({ path: '@vfs/session/s1' })
    await flushPromises()

    expect(store.list).toHaveLength(0)
    expect(sessionApi.listSessions).not.toHaveBeenCalled()
  })

  it('消息目录的变更帧在 directChildren 作用域之外：既不回读也不重拉', async () => {
    const store = useSessionsStore()
    store.list.push({ id: 's1', message_count: 0, updated_at: 0, metadata: {} } as never)

    emit({ path: '@vfs/session/s1/message', data: { id: 'm1', delta: '半句' } })
    await flushPromises()

    expect(vdfsApi.statVdfs).not.toHaveBeenCalled()
    expect(sessionApi.listSessions).not.toHaveBeenCalled()
    expect(store.list).toHaveLength(1)
  })
})

/**
 * 删除的级联：**前端自己算区间**，不等后端把被删的每一条逐个通知回来。
 *
 * 后端 `chat/delete_message` 的语义是「在已排序列表里删掉目标及其之后的所有消息」；
 * 这里锁定前端与之同源（判据 `seq`）的实现，以及三条容易踩错的边界：
 * 锚点不存在时**不截断**、写后端失败要**回滚**、后端返回的权威列表更宽时**补齐**。
 */
describe('sessions store — 删除消息的级联（目标 + 其后全部）', () => {
  const SID = 's1'

  /** 一段线性转写：u1 → t1 → a1 → u2 → t2 → a2（`seq` 即顺序，与后端同源） */
  function seed(store: ReturnType<typeof useSessionsStore>) {
    store.hydrateFromHistory(SID, [
      { id: 'u1', principal: { kind: 'person', id: 'person/local' }, type: 'body', seq: 1, content: '问题一' },
      { id: 't1', type: 'skeleton', seq: 2 },
      { id: 'a1', principal: { kind: 'agent', id: 'session/main' }, type: 'body', seq: 3, content: '回答一' },
      { id: 'u2', principal: { kind: 'person', id: 'person/local' }, type: 'body', seq: 4, content: '问题二' },
      { id: 't2', type: 'skeleton', seq: 5 },
      { id: 'a2', principal: { kind: 'agent', id: 'session/main' }, type: 'body', seq: 6, content: '回答二' },
    ] as never)
  }

  const ids = (store: ReturnType<typeof useSessionsStore>) =>
    store.getSessionMessages(SID).map((m) => m.id)

  beforeEach(() => {
    setActivePinia(createPinia())
    sessionApi.deleteMessage.mockReset()
    // 声明当前空间（生产侧由工作台按「当前目录能新建会话」声明）：叶子操作
    // 因此按**地址**寻址——`mountDirOf` 取到的就是这个挂载目录。
    useSessionsStore().setSessionSpace(SCHEME.mountDir)
  })

  it('从用户消息起截断：它及其后的全部消失，之前的一条不动', () => {
    const store = useSessionsStore()
    seed(store)

    const removed = store.removeFrom(SID, 'u2')

    expect(ids(store)).toEqual(['u1', 't1', 'a1'])
    expect(removed.map((m) => m.id), '返回被移除的消息，供失败回滚').toEqual([
      'u2',
      't2',
      'a2',
    ])
  })

  it('从根级骨架起截断：该骨架及其全部子节点一并消失', () => {
    const store = useSessionsStore()
    seed(store)

    store.removeFrom(SID, 't1')

    expect(ids(store)).toEqual(['u1'])
  })

  it('删最后一条：只掉它自己', () => {
    const store = useSessionsStore()
    seed(store)

    store.removeFrom(SID, 'a2')

    expect(ids(store)).toEqual(['u1', 't1', 'a1', 'u2', 't2'])
  })

  it('锚点不在本地：什么都不删，也不返回任何 id', () => {
    const store = useSessionsStore()
    seed(store)

    expect(store.removeFrom(SID, '不存在')).toEqual([])
    expect(ids(store)).toHaveLength(6)

    // 会话本身没加载（store 里连这张表都没有）同样不炸
    expect(store.removeFrom('别的会话', 'u1')).toEqual([])
  })

  it('缺 seq 的旧数据按排序位置截断（不依赖 seq 一定存在）', () => {
    const store = useSessionsStore()
    // hydrate 会给缺 seq 的消息补一个递增游标，两者都必须落在同一顺序上
    store.hydrateFromHistory(SID, [
      { id: 'x1', content: '一' },
      { id: 'x2', content: '二' },
      { id: 'x3', content: '三' },
    ] as never)

    store.removeFrom(SID, 'x2')

    expect(ids(store)).toEqual(['x1'])
  })

  it('deleteMessage：本地先级联（不等后端返回），再用权威 deleted_ids 幂等对齐', async () => {
    const store = useSessionsStore()
    seed(store)
    // 后端这次删得比本地推算更宽（例如它把某个本地尚未见到的在途节点也删了）
    sessionApi.deleteMessage.mockResolvedValue({ deleted: 4, deleted_ids: ['u2', 't2', 'a2', 'x9'] })

    await store.deleteMessage(SID, 'u2')

    expect(ids(store)).toEqual(['u1', 't1', 'a1'])
    // 叶子操作按**地址**寻址：会话 id 之外还要带上它所在的挂载目录
    expect(sessionApi.deleteMessage).toHaveBeenCalledWith(SID, 'u2', SCHEME.mountDir)
  })

  it('deleteMessage：写后端失败则回滚本地改动（没落库就不显示已删除）', async () => {
    const store = useSessionsStore()
    seed(store)
    sessionApi.deleteMessage.mockRejectedValue(new Error('IPC 断了'))

    await expect(store.deleteMessage(SID, 'u2')).rejects.toThrow('IPC 断了')

    expect(ids(store), '失败必须把消息放回去').toEqual(['u1', 't1', 'a1', 'u2', 't2', 'a2'])
  })
})

/**
 * `hydrateFromHistory` 的**快照直载语义**（快照即权威）。
 *
 * 转写的权威副本在存储：`loadMessages` 以此整表装载，增量由
 * `sessionTranscriptSync` 持续收敛——装载与增量消费是同一条数据链路的
 * 两个入口，不存在需要"合并"的两个真相来源。旧合并语义（保留本地在途节点）
 * 已删除：那是双数据源混写的补丁层。
 */
describe('sessions store — 历史水合是快照直载', () => {
  const SID = 's1'
  const snapshot = (messages: unknown[]) => {
    vdfsApi.readVdfs.mockResolvedValue({ text: JSON.stringify({ messages }) })
  }
  const ids = (store: ReturnType<typeof useSessionsStore>) =>
    store.getSessionMessages(SID).map((m) => m.id)

  beforeEach(() => {
    setActivePinia(createPinia())
    sessionApi.listSessions.mockResolvedValue([])
  })

  it('快照即权威：本地不在快照里的节点不保留（在途节点由增量通道继续收敛）', async () => {
    const store = useSessionsStore()
    store.hydrateFromHistory(SID, [
      { id: 'u1', principal: { kind: 'person', id: 'person/local' }, type: 'body', seq: 1, content: '问题' },
    ] as never)
    store.applyTranscriptMessage(SID, {
      id: 'skel-live',
      type: 'skeleton',
      status: 'streaming',
      content: '半截',
    } as never)

    snapshot([{ id: 'u1', principal: { kind: 'person', id: 'person/local' }, type: 'body', seq: 1, content: '问题' }])
    await store.loadMessages(SID)

    expect(ids(store)).toEqual(['u1'])
  })

  it('终态且不在快照里的本地节点被丢弃（被删除的陈旧副本不得复活）', async () => {
    const store = useSessionsStore()
    store.applyTranscriptMessage(SID, { id: 'ghost', type: 'body', status: 'completed' } as never)

    snapshot([{ id: 'h1', type: 'body', seq: 1 }])
    await store.loadMessages(SID)

    expect(ids(store)).toEqual(['h1'])
  })

  it('快照里的节点整条替换：内容与状态都以快照为准', async () => {
    const store = useSessionsStore()
    store.applyTranscriptMessage(SID, {
      id: 'a1',
      type: 'body',
      status: 'streaming',
      content: '半截',
    } as never)

    snapshot([{ id: 'a1', type: 'body', seq: 3, status: 'completed', content: '完整' }])
    await store.loadMessages(SID)

    const [a1] = store.getSessionMessages(SID)
    expect(a1.content).toBe('完整')
    expect(a1.status).toBe('completed')
  })
})

/**
 * 会话节点状态收敛：**零回读**。
 *
 * 旧机制里有「终态丢失自愈」（对账回读）——那是对「变更通道不重放」的补丁层，
 * 已整体删除。现在唯一的数据链路是：会话节点状态由变更载荷就地收敛；
 * 消息终态由后端保证落库，本地陈旧副本在下次快照直载（切换会话 / 重开）时纠正。
 * 因此任何状态下都不回读——状态迁移是幂等的，后端权威节点视图会持续覆盖。
 */
describe('sessions store — 会话节点状态收敛（零回读）', () => {
  const SID = 's1'

  beforeEach(() => {
    setActivePinia(createPinia())
    captured.scopes.length = 0
    captured.handlers.length = 0
    stopSessionNodeSync()
    vdfsApi.readVdfs.mockReset()
    vdfsApi.statVdfs.mockReset()
    startSessionNodeSync(useSessionsStore())
  })

  it('会话转空闲（含本地仍有 streaming 节点）→ 不回读，状态由载荷就地落定', async () => {
    const store = useSessionsStore()
    store.applyTranscriptMessage(SID, {
      id: 'tc1',
      type: 'skeleton',
      status: 'streaming',
      content: '{}',
    } as never)

    emit({ path: `@vfs/session/${SID}`, data: sessionNode({ name: SID, status: 'active' }) })
    await flushPromises()

    expect(vdfsApi.readVdfs, '状态收敛零回读：对账自愈已删除').not.toHaveBeenCalled()
  })

  it('会话仍在运行时同样零回读（在途节点是合法的）', async () => {
    const store = useSessionsStore()
    store.applyTranscriptMessage(SID, {
      id: 'tc1',
      type: 'skeleton',
      status: 'streaming',
    } as never)

    emit({ path: `@vfs/session/${SID}`, data: sessionNode({ name: SID, status: 'working' }) })
    await flushPromises()

    expect(vdfsApi.readVdfs).not.toHaveBeenCalled()
    expect(store.getSessionMessages(SID)[0].status).toBe('streaming')
  })
})

/**
 * 存储号与本地号的关系：**权威号就地落定，不回读**。
 *
 * `seq` 是唯一的顺序锚点，但它有两套来源：未落库的节点拿到的是**本地游标**发的号
 * （`nextSeq`），落库后拿到的是**存储**分配的号。存储回包携带的权威号直接覆盖本地号
 * （幂等更新），顺序随之自动恢复——旧的「分叉标记 + 转空闲回读」是对账补丁，已删除。
 */
describe('sessions store — 存储号就地落定，不回读', () => {
  const SID = 's1'

  beforeEach(() => {
    setActivePinia(createPinia())
    captured.scopes.length = 0
    captured.handlers.length = 0
    stopSessionNodeSync()
    vdfsApi.readVdfs.mockReset()
    vdfsApi.statVdfs.mockReset()
    startSessionNodeSync(useSessionsStore())
  })

  /** 把会话节点状态推成「不忙」 */
  async function goIdle() {
    emit({ path: `@vfs/session/${SID}`, data: sessionNode({ name: SID, status: 'active' }) })
    await flushPromises()
  }

  it('本地号被存储号替换 ⇒ 就地覆盖（顺序随权威号恢复），不回读', async () => {
    const store = useSessionsStore()
    // 上一轮的产物：存储号 5 / 6，本地游标因此被抬到 6
    store.hydrateFromHistory(SID, [
      { id: 'h1', type: 'body', seq: 5, status: 'completed', content: '旧的' },
      { id: 'h2', type: 'body', seq: 6, status: 'completed', content: '旧的' },
    ] as never)
    // 用户发送：本地游标发号 7（此刻存储还没落库）
    store.applyTranscriptMessage(SID, {
      id: 'u1',
      principal: { kind: 'person', id: 'person/local' },
      type: 'body',
      status: 'completed',
      content: '问题',
    } as never)
    expect(store.getSessionMessages(SID).map((m) => m.id), '此刻它排在末尾').toEqual([
      'h1',
      'h2',
      'u1',
    ])

    // 存储回包：权威号是 4（截断 / 压缩后存储重排过号）⇒ 就地覆盖本地号
    store.applyTranscriptMessage(SID, {
      id: 'u1',
      principal: { kind: 'person', id: 'person/local' },
      type: 'body',
      seq: 4,
      status: 'completed',
      content: '问题',
    } as never)

    expect(store.getSessionMessages(SID).map((m) => m.id), '以存储号为锚点重排').toEqual([
      'u1',
      'h1',
      'h2',
    ])

    // 转空闲：零回读（对账已删除）
    await goIdle()
    expect(vdfsApi.readVdfs).not.toHaveBeenCalled()
  })

  it('权威号与本地号一致 ⇒ 无变化（幂等）', async () => {
    const store = useSessionsStore()
    store.hydrateFromHistory(SID, [
      { id: 'h1', type: 'body', seq: 1, status: 'completed', content: '旧的' },
    ] as never)
    // 本地游标发号 2，随后存储也说是 2 —— 两套号没有分叉
    store.applyTranscriptMessage(SID, { id: 'u1', type: 'body', status: 'completed' } as never)
    store.applyTranscriptMessage(SID, {
      id: 'u1',
      type: 'body',
      seq: 2,
      status: 'completed',
    } as never)

    expect(store.getSessionMessages(SID).map((m) => m.id)).toEqual(['h1', 'u1'])

    await goIdle()
    expect(vdfsApi.readVdfs).not.toHaveBeenCalled()
  })
})

/**
 * 实时状态（`sessionStatuses`）的唯一变更通道。
 *
 * 收敛前有四处各自手写「展开 → 合并 → 整体替换」，其中最隐蔽的一处是**缺省值**：
 * 条目一旦存在，`getSessionStaleReason` 就会按 `last_event_at` 算「多久没消息了」，
 * 若某处把缺省值写成 0，刚建出来的条目会被立刻判成「状态已过期 N 分钟」——
 * 界面凭空报「连接已断开」，而没有任何测试会失败。
 * 这组用例锁的正是这些**不会报错的错**。
 */
describe('sessions store — 实时状态的唯一变更通道', () => {
  const SID = 's1'

  beforeEach(() => {
    setActivePinia(createPinia())
    sessionApi.listSessions.mockResolvedValue([])
  })

  it('putStatus：自动推进 last_event_at，并忽略调用方传入的时间戳', () => {
    const store = useSessionsStore()
    const t0 = Date.now()

    store.putStatus(SID, { last_event_at: 0, activity: '处理中…' })

    const st = store.getSessionStatus(SID)
    expect(st.activity).toBe('处理中…')
    expect(st.last_event_at, 'last_event_at 由通道统一维护').toBeGreaterThanOrEqual(t0)
  })

  it('消息落地写 last_preview 时同样推进时间戳（写入方不手动维护它）', () => {
    const store = useSessionsStore()
    const t0 = Date.now()

    store.applyTranscriptMessage(SID, {
      id: 'a1',
      principal: { kind: 'agent', id: 'session/main' },
      type: 'body',
      content: '回答',
    } as never)

    const st = store.getSessionStatus(SID)
    expect(st.last_preview).toBe('回答')
    expect(st.last_event_at).toBeGreaterThanOrEqual(t0)
  })

  it('水合建出的状态条目**不会**被立刻判成过期（初值必须是当前时刻，不是 0）', () => {
    const store = useSessionsStore()
    // 空历史：这条路径必须仍然建出条目（否则下面的断言会因「没有条目」而假通过）
    store.hydrateFromHistory(SID, [])

    expect(store.getSessionStatus(SID).last_event_at).toBeGreaterThan(0)
    expect(store.getSessionStaleReason(SID)).toBeNull()
  })

  it('摘掉状态条目（删除会话）后不再报过期，且消息一并清空', () => {
    const store = useSessionsStore()
    store.putStatus(SID, { status: VDFS_STATUS_WORKING })
    expect(store.getSessionStatus(SID).status).toBe(VDFS_STATUS_WORKING)

    store.dropSessionState(SID)

    // 条目没了 ⇒ 回到「从未有过事件」，而不是留下一条 last_event_at 很旧的陈旧状态
    expect(store.getSessionStaleReason(SID)).toBeNull()
    expect(store.getSessionMessages(SID)).toEqual([])
  })
})

/**
 * 会话运行态 —— 唯一来源是**会话节点的变更帧**
 * （落地口 `applySessionState`，全量节点视图 ⇒ 零回读）。
 *
 * 同一批变更还挂在 `sessionNodeSync` 上做防抖重拉清单；两条收敛路径幂等且同源。
 */
describe('sessions store — 会话运行态（节点视图帧）', () => {
  const SID = 's1'

  let store: ReturnType<typeof useSessionsStore>

  beforeEach(() => {
    setActivePinia(createPinia())
    captured.scopes.length = 0
    captured.handlers.length = 0
    stopSessionNodeSync()
    store = useSessionsStore()
    store.list.push({ id: SID, message_count: 0, updated_at: 0, status: 'active', metadata: {} } as never)
    chime.playCompletionChime.mockClear()
    // 本组断言「这一帧不得触发整表重拉」，故调用计数必须从零起算
    // （前面的用例声明「当前空间」时会正常触发一次清单刷新）
    sessionApi.listSessions.mockClear()
    vdfsApi.statVdfs.mockReset()
    vdfsApi.readVdfs.mockResolvedValue({ text: '{"messages":[]}' })
  })

  it('节点视图就地落 status / 标题 / 计数，零回读、零整表重拉', () => {
    store.titles[SID] = '新对话'
    store.list[0] = { ...store.list[0], metadata: { title: '新对话' } }

    store.applySessionState(SID, sessionNode({ status: 'working', message_count: 7 }) as never)

    // 状态是**节点属性**，随帧送达 ⇒ 不必也不能回读
    expect(vdfsApi.statVdfs, '运行态帧带全量视图：不得回读').not.toHaveBeenCalled()
    expect(store.list[0].status).toBe('working')
    expect(store.list[0].message_count).toBe(7)
    expect(store.isSessionWorking(SID)).toBe(true)
    // 标题也随节点自述就地更新，不因为一次状态迁移把整张清单重拉一遍
    expect(store.titles[SID]).toBe('解释一下 VDFS 的地址模型')
    expect(store.list[0].metadata?.title).toBe('解释一下 VDFS 的地址模型')
    expect(sessionApi.listSessions).not.toHaveBeenCalled()
  })

  it('failed 是独立的会话状态，不是 active + 布尔标志', () => {
    store.applySessionState(
      SID,
      sessionNode({ status: 'failed', outcome: 'failed', error: '上游 429' }) as never,
    )

    expect(store.getSessionStatus(SID).status).toBe('failed')
    expect(store.isSessionFailed(SID)).toBe(true)
    expect(store.isSessionWorking(SID)).toBe(false)
    // 会话级错误是**节点属性**（覆盖"错误发生在任何消息节点创建之前"的场景）
    expect(store.getSessionError(SID)).toBe('上游 429')
  })

  it('新一轮开始（working）时节点不带 error ⇒ 上一轮错误自动清空', () => {
    store.list[0] = { ...store.list[0], status: 'failed' }
    store.setSessionError(SID, '上一轮的错误')

    store.applySessionState(SID, sessionNode({ status: 'working' }) as never)

    expect(store.getSessionError(SID)).toBeNull()
    expect(store.isSessionFailed(SID)).toBe(false)
  })

  it('提示音只在 working → 非 working 的**迁移**上响，音色取 outcome', () => {
    // ① 空闲 → 空闲：不是迁移，不响
    store.applySessionState(SID, sessionNode({ status: 'active' }) as never)
    expect(chime.playCompletionChime).not.toHaveBeenCalled()

    // ② 空闲 → 运行中：是迁移，但不是"结束"，不响
    store.applySessionState(SID, sessionNode({ status: 'working' }) as never)
    expect(chime.playCompletionChime).not.toHaveBeenCalled()

    // ③ 运行中 → 空闲（正常结束）：响 completed
    store.applySessionState(SID, sessionNode({ status: 'active', outcome: 'completed' }) as never)
    expect(chime.playCompletionChime).toHaveBeenCalledWith('completed', SID)

    // ④ 中止与失败各取自己的音色（结局是状态，不靠"谁先到"区分）
    store.applySessionState(SID, sessionNode({ status: 'working' }) as never)
    store.applySessionState(SID, sessionNode({ status: 'active', outcome: 'aborted' }) as never)
    expect(chime.playCompletionChime).toHaveBeenLastCalledWith('aborted', SID)

    store.applySessionState(SID, sessionNode({ status: 'working' }) as never)
    store.applySessionState(
      SID,
      sessionNode({ status: 'failed', outcome: 'failed', error: 'boom' }) as never,
    )
    expect(chime.playCompletionChime).toHaveBeenLastCalledWith('failed', SID)
    expect(chime.playCompletionChime).toHaveBeenCalledTimes(3)
  })

  it('状态未变时不覆盖消息节点派生的活动文字', () => {
    store.applySessionState(SID, sessionNode({ status: 'working' }) as never)
    store.putStatus(SID, { activity: '正在思考…' })

    // 同状态重复到达（如一轮里的多条运行态帧）：activity 原样保留
    store.applySessionState(SID, sessionNode({ status: 'working' }) as never)

    expect(store.getSessionStatus(SID).activity).toBe('正在思考…')
  })
})

/**
 * 惰性重取账本的回放（[] 前端半，`hydrateFromHistory` 的那一段）。
 *
 * 覆盖「跨刷新保留」的**后半句**：全量快照会把本地已释放的骨架子树带回来
 * （后端从不知道前端释放过），账本说它折叠着就得再摘一次——否则刷新后
 * 子树全须重新长出来，折叠 chip 的计数与实际持有对不上。
 * 顺带钉两条清偿：骨架已不在 ⇒ 摘条目；会话删除 ⇒ 摘整段。
 */
describe('sessions store — 惰性重取账本回放', () => {
  const SID = 's1'
  const LEDGER_KEY = 'symbio.releasedSkeletons'
  const tree = [
    { id: 'skel', type: 'skeleton', status: 'completed', seq: 1 },
    { id: 'a', parent_id: 'skel', type: 'body', status: 'completed', seq: 2 },
    { id: 'b', parent_id: 'a', type: 'notice', status: 'completed', seq: 3 },
    { id: 'keep', type: 'body', status: 'completed', seq: 4 },
  ]
  const ids = (store: ReturnType<typeof useSessionsStore>) =>
    store.getSessionMessages(SID).map((m) => m.id)

  beforeEach(() => {
    setActivePinia(createPinia())
    localStorage.clear()
    sessionApi.listSessions.mockResolvedValue([])
  })

  it('账本说释放着 ⇒ 水合后再摘一次子树（root 与无关节点保留，条目不动）', () => {
    const store = useSessionsStore()
    const released = useReleasedSkeletonsStore()
    released.markReleased(SID, 'skel', statsOf(tree.slice(1) as never))

    store.hydrateFromHistory(SID, tree as never)

    expect(ids(store)).toEqual(['skel', 'keep'])
    // 条目**不摘**：下次水合（切走再切回）还要摘
    expect(released.isReleased(SID, 'skel')).toBe(true)
  })

  it('未释放的骨架不受回放影响（子树整棵留在 store）', () => {
    const store = useSessionsStore()
    store.hydrateFromHistory(SID, tree as never)
    expect(ids(store)).toEqual(['skel', 'a', 'b', 'keep'])
  })

  it('骨架已不在快照里 ⇒ 清偿条目（被截断的账不留在 localStorage）', () => {
    const store = useSessionsStore()
    const released = useReleasedSkeletonsStore()
    released.markReleased(SID, 'skel', statsOf(tree.slice(1) as never))
    released.markReleased(SID, 'still-here', statsOf([]))

    store.hydrateFromHistory(SID, [
      { id: 'still-here', type: 'skeleton', status: 'completed', seq: 1 },
      { id: 'x', parent_id: 'still-here', type: 'body', status: 'completed', seq: 2 },
    ] as never)

    expect(released.isReleased(SID, 'skel')).toBe(false)
    expect(released.isReleased(SID, 'still-here')).toBe(true)
    // 只清「骨架已不在」的那条；仍在的条目原样保留
    expect(Object.keys(JSON.parse(localStorage.getItem(LEDGER_KEY)!).s1)).toEqual(['still-here'])
  })

  it('dropSessionState 同批清偿账本（会话删除后条目永驻只会误判同 id 新会话）', () => {
    const store = useSessionsStore()
    const released = useReleasedSkeletonsStore()
    released.markReleased(SID, 'skel', statsOf(tree.slice(1) as never))

    store.dropSessionState(SID)

    expect(released.isReleased(SID, 'skel')).toBe(false)
    expect(JSON.parse(localStorage.getItem(LEDGER_KEY)!)).toEqual({})
  })
})

/**
 * 「本会话放行」——把会话的执行阈值抬一档。
 *
 * 这条动作**不新增任何授权事实**：`authz.granted` 仍由用户对那一次调用的批准写，
 * 它只改会话 metadata 里的一个字段（后端 `resolve_session_params` 把它落成
 * `ctx[RISK_LEVEL]`）。所以判据全落在「写了什么 / 什么时候不写 / 写失败怎么办」
 * 三件事上。
 */
describe('sessions store —「本会话放行」只抬阈值', () => {
  const SID = 's1'
  let store: ReturnType<typeof useSessionsStore>

  beforeEach(() => {
    setActivePinia(createPinia())
    sessionApi.updateSession.mockClear()
    store = useSessionsStore()
    store.list.push({
      id: SID,
      message_count: 0,
      updated_at: 0,
      status: 'active',
      metadata: { risk_level: 'medium' },
    } as never)
  })

  it('抬到本次调用的风险档：一次 metadata 写 + 本地镜射', async () => {
    expect(await store.raiseSessionRiskLevel(SID, 'high')).toBe('high')

    expect(sessionApi.updateSession).toHaveBeenCalledWith(SID, { risk_level: 'high' })
    // 镜射进 store 的两处：清单项的 metadata（后端视角）与回填后的选择 map（UI 视角）
    expect(store.list[0].metadata?.risk_level).toBe('high')
    expect(store.getSessionRiskLevel(SID)).toBe('high')
  })

  it('已经是该档 ⇒ 不写（一次无意义的写会广播一次资源变更、白重拉清单）', async () => {
    await store.raiseSessionRiskLevel(SID, 'high')
    sessionApi.updateSession.mockClear()

    expect(await store.raiseSessionRiskLevel(SID, 'high')).toBe('high')
    expect(sessionApi.updateSession).not.toHaveBeenCalled()
  })

  it('只抬不降：更严的一档不写，返回当前实际档位', async () => {
    await store.raiseSessionRiskLevel(SID, 'high')
    sessionApi.updateSession.mockClear()

    expect(await store.raiseSessionRiskLevel(SID, 'low')).toBe('high')
    expect(sessionApi.updateSession).not.toHaveBeenCalled()
    expect(store.getSessionRiskLevel(SID)).toBe('high')
  })

  it('写失败不镜射：不会留下「界面已经改了、后端没改」的半截状态', async () => {
    sessionApi.updateSession.mockRejectedValueOnce(new Error('boom'))

    await expect(store.raiseSessionRiskLevel(SID, 'high')).rejects.toThrow('boom')
    expect(store.getSessionRiskLevel(SID)).toBe('medium')
    expect(store.list[0].metadata?.risk_level).toBe('medium')
  })
})
