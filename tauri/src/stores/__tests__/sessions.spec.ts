// @vitest-environment happy-dom
/**
 * sessions store —— VDFS 变更驱动的会话清单收敛
 *
 * 覆盖两条约定：
 *
 * 1. **只有一条变更频道**：会话清单的同步来自 `kind = 'vdfs'` 的
 *    `VdfsChangeEvent`，作用域按**展示地址前缀**分流（`@vfs/session` 的直接子项
 *    = 会话叶子；转写列表项 / 子会话的变更不进侧栏）。
 * 2. **状态类变更必带节点视图**（`node-state-streaming.md` §8.2）：
 *    - `updated`（含 busy/idle/failed 运行态与标题变更）→ 用**载荷**就地收敛，
 *      **零回读**、零整表重拉；
 *    - `created` → 防抖重拉收敛排序与完整字段；
 *    - `deleted` → 本地即时移除。
 */
import { describe, it, expect, beforeEach, vi } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import { flushPromises } from '@vue/test-utils'

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
  updateSession: vi.fn(),
  readSessionTranscript: sessionApi.readSessionTranscript,
  clearMessages: vi.fn(),
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
// 地址方案：注入夹具，避免真去列目录
vi.mock('@/services/vdfsScheme', () => ({
  ensureSessionMountDir: vi.fn(async () => SCHEME.mountDir),
  ensureVdfsSessionScheme: vi.fn(async () => SCHEME),
  vdfsSessionScheme: vi.fn(() => SCHEME),
}))

import { useSessionsStore } from '../sessions'
import { startSessionNodeSync, stopSessionNodeSync } from '../sessionNodeSync'
import { VDFS_STATUS_WORKING } from '@/schemas/vdfs'

/**
 * 协议夹具：会话挂载目录与转写段是**运行期数据**（列目录认出来）。
 * 单测不去列目录，直接注入——断言仍把地址钉成字面量。
 *
 * `vi.hoisted`：`vi.mock` 工厂先于 import 执行，直接引用顶层 const 会撞 TDZ。
 */
const { SCHEME } = vi.hoisted(() => ({
  SCHEME: { mountDir: '@vfs/session', messagesSeg: '消息' },
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
    vdfsApi.writeVdfs.mockReset()
    vdfsApi.readVdfs.mockResolvedValue({ text: '{"messages":[]}' })
  })

  it('新建会话 = 一次 vdfs/write 到会话挂载根；id 由后端生成，前端不编造', async () => {
    // 后端返回的是**树内相对路径**（容器把挂载目录名补在前面），前端只取末段
    vdfsApi.writeVdfs.mockResolvedValue({ path: 'session/s9', created: true })
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

  it('后端没给出新地址时报错，而不是插一条 id 为空的会话', async () => {
    vdfsApi.writeVdfs.mockResolvedValue({ path: '', created: true })
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
    vdfsApi.writeVdfs.mockResolvedValue({ path: 'session/s9', created: true })
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
    vdfsApi.writeVdfs.mockResolvedValue({ path: 'session/s9', created: true })
    const store = useSessionsStore()
    await store.createSessionWithFirstMessage(undefined, { text: 'hi' })

    expect(store.consumePendingFirstMessage('s10')).toBeNull()
    expect(store.consumePendingFirstMessage('s9')?.text).toBe('hi')
  })

  it('创建失败不留半截队列项，也不污染邮箱里原有的那条', async () => {
    vdfsApi.writeVdfs.mockResolvedValue({ path: 'session/s9', created: true })
    const store = useSessionsStore()
    await store.createSessionWithFirstMessage(undefined, { text: '第一条' })

    // 第二次创建失败（后端没回地址）
    vdfsApi.writeVdfs.mockResolvedValue({ path: '', created: true })
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

  it('updated 带节点视图 → 就地落 status / 标题，零回读、零整表重拉', async () => {
    const store = useSessionsStore()
    store.titles['s1'] = '新对话'
    store.list.push({ id: 's1', message_count: 0, updated_at: 0, status: 'active', metadata: { title: '新对话' } } as never)

    emit({
      path: '@vfs/session/s1',
      change: 'updated',
      node: sessionNode({ status: 'working', message_count: 7 }),
    })
    await flushPromises()

    // 状态是**节点属性**，随载荷送达 ⇒ 不必也不能回读
    expect(vdfsApi.statVdfs, '状态类变更带载荷：不得回读').not.toHaveBeenCalled()
    expect(store.list[0].status).toBe('working')
    expect(store.list[0].message_count).toBe(7)
    expect(store.isSessionWorking('s1')).toBe(true)
    // 标题也随节点自述就地更新，不因为一次变更把整张清单重拉一遍
    expect(store.titles['s1']).toBe('解释一下 VDFS 的地址模型')
    expect(store.list[0].metadata?.title).toBe('解释一下 VDFS 的地址模型')
    expect(sessionApi.listSessions).not.toHaveBeenCalled()
  })

  it('updated 未带节点视图 → 忽略并留痕（不做静默回读兜底）', async () => {
    const store = useSessionsStore()
    store.list.push({ id: 's1', message_count: 0, updated_at: 0, status: 'working', metadata: { title: 'T' } } as never)

    emit({ path: '@vfs/session/s1', change: 'updated' })
    await flushPromises()

    // 载荷缺失意味着后端违反了「状态变更必带节点视图」的不变量；
    // 回读兜底会把它掩盖成"看起来能用"，所以这里既不改状态也不回读。
    expect(vdfsApi.statVdfs).not.toHaveBeenCalled()
    expect(store.list[0].status).toBe('working')
    expect(sessionApi.listSessions).not.toHaveBeenCalled()
  })

  it('failed 是独立的会话状态，不是 active + 布尔标志', async () => {
    const store = useSessionsStore()
    store.list.push({ id: 's1', message_count: 0, updated_at: 0, status: 'working', metadata: {} } as never)

    emit({
      path: '@vfs/session/s1',
      change: 'updated',
      node: sessionNode({ status: 'failed', outcome: 'failed', error: '上游 429' }),
    })

    expect(store.getSessionStatus('s1').status).toBe('failed')
    expect(store.isSessionFailed('s1')).toBe(true)
    expect(store.isSessionWorking('s1')).toBe(false)
    // 会话级错误是**节点属性**（覆盖"错误发生在任何消息节点创建之前"的场景）
    expect(store.getSessionError('s1')).toBe('上游 429')
  })

  it('新一轮开始（working）时节点不带 error ⇒ 上一轮错误自动清空', async () => {
    const store = useSessionsStore()
    store.list.push({ id: 's1', message_count: 0, updated_at: 0, status: 'failed', metadata: {} } as never)
    store.setSessionError('s1', '上一轮的错误')

    emit({ path: '@vfs/session/s1', change: 'updated', node: sessionNode({ status: 'working' }) })

    expect(store.getSessionError('s1')).toBeNull()
    expect(store.isSessionFailed('s1')).toBe(false)
  })

  it('提示音只在 working → 非 working 的**迁移**上响，音色取 outcome', async () => {
    const store = useSessionsStore()
    store.list.push({ id: 's1', message_count: 0, updated_at: 0, status: 'active', metadata: {} } as never)

    // ① 空闲 → 空闲（标题更新等）：不是迁移，不响
    emit({ path: '@vfs/session/s1', change: 'updated', node: sessionNode({ status: 'active' }) })
    expect(chime.playCompletionChime).not.toHaveBeenCalled()

    // ② 空闲 → 运行中：是迁移，但不是"结束"，不响
    emit({ path: '@vfs/session/s1', change: 'updated', node: sessionNode({ status: 'working' }) })
    expect(chime.playCompletionChime).not.toHaveBeenCalled()

    // ③ 运行中 → 空闲（正常结束）：响 completed
    emit({
      path: '@vfs/session/s1',
      change: 'updated',
      node: sessionNode({ status: 'active', outcome: 'completed' }),
    })
    expect(chime.playCompletionChime).toHaveBeenCalledWith('completed', 's1')

    // ④ 中止与失败各取自己的音色（结局是状态，不靠"谁先到"区分）
    emit({ path: '@vfs/session/s1', change: 'updated', node: sessionNode({ status: 'working' }) })
    emit({
      path: '@vfs/session/s1',
      change: 'updated',
      node: sessionNode({ status: 'active', outcome: 'aborted' }),
    })
    expect(chime.playCompletionChime).toHaveBeenLastCalledWith('aborted', 's1')

    emit({ path: '@vfs/session/s1', change: 'updated', node: sessionNode({ status: 'working' }) })
    emit({
      path: '@vfs/session/s1',
      change: 'updated',
      node: sessionNode({ status: 'failed', outcome: 'failed', error: 'boom' }),
    })
    expect(chime.playCompletionChime).toHaveBeenLastCalledWith('failed', 's1')
    expect(chime.playCompletionChime).toHaveBeenCalledTimes(3)
  })

  it('标题更新（状态未变）不覆盖消息节点派生的活动文字', async () => {
    const store = useSessionsStore()
    store.list.push({ id: 's1', message_count: 0, updated_at: 0, status: 'active', metadata: {} } as never)
    emit({ path: '@vfs/session/s1', change: 'updated', node: sessionNode({ status: 'working' }) })
    store.putStatus('s1', { activity: '正在思考…' })

    emit({ path: '@vfs/session/s1', change: 'updated', node: sessionNode({ status: 'working' }) })

    expect(store.getSessionStatus('s1').activity).toBe('正在思考…')
  })

  it('created → 防抖重拉清单（收敛排序与完整字段）', async () => {
    vi.useFakeTimers()
    try {
      const store = useSessionsStore()
      emit({ path: '@vfs/session/newsid', change: 'created' })
      expect(sessionApi.listSessions).not.toHaveBeenCalled()
      await vi.advanceTimersByTimeAsync(800)
      expect(sessionApi.listSessions).toHaveBeenCalledTimes(1)
      expect(store.list).toEqual([])
    } finally {
      vi.useRealTimers()
    }
  })

  it('deleted → 本地即时移除，不等重拉', () => {
    const store = useSessionsStore()
    store.list.push({ id: 's1', message_count: 0, updated_at: 0, metadata: {} } as never)

    emit({ path: '@vfs/session/s1', change: 'deleted' })

    expect(store.list).toHaveLength(0)
    expect(sessionApi.listSessions).not.toHaveBeenCalled()
  })

  it('appended（转写增量）与会话清单无关：既不重读也不重拉', async () => {
    const store = useSessionsStore()
    store.list.push({ id: 's1', message_count: 0, updated_at: 0, metadata: {} } as never)

    emit({ path: '@vfs/session/s1/消息/m1', change: 'appended', delta: '半句' })
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
      { id: 'u1', role: 'user', type: 'user_prompt', seq: 1, content: '问题一' },
      { id: 't1', type: 'turn', seq: 2 },
      { id: 'a1', role: 'assistant', type: 'text', seq: 3, content: '回答一' },
      { id: 'u2', role: 'user', type: 'user_prompt', seq: 4, content: '问题二' },
      { id: 't2', type: 'turn', seq: 5 },
      { id: 'a2', role: 'assistant', type: 'text', seq: 6, content: '回答二' },
    ] as never)
  }

  const ids = (store: ReturnType<typeof useSessionsStore>) =>
    store.getSessionMessages(SID).map((m) => m.id)

  beforeEach(() => {
    setActivePinia(createPinia())
    sessionApi.deleteMessage.mockReset()
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

  it('从 Turn 起截断：该 Turn 及其全部子节点一并消失', () => {
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
    expect(sessionApi.deleteMessage).toHaveBeenCalledWith(SID, 'u2')
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
 * `hydrateFromHistory` 的**合并语义**（不是整表替换）。
 *
 * 它接收的是「一次 IPC 往返之前」的快照，取回期间流式补丁仍在写入 store。
 * 整表替换会把这些**更新的**补丁覆盖掉（经典 lost update），而它们不会被重发
 * ——表现是消息内容倒退若干 token，或整条在途节点凭空消失，也就是
 * 「Turn 运行中切走再切回，正在跑的那一轮不见了」。
 *
 * 规则：快照权威；快照里没有的本地节点**仅当仍在飞行中**才保留。
 */
describe('sessions store — 历史水合是合并，不是整表替换', () => {
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

  it('快照里没有的在途节点必须保留（正在跑的那一轮不能凭空消失）', async () => {
    const store = useSessionsStore()
    store.hydrateFromHistory(SID, [
      { id: 'u1', role: 'user', type: 'user_prompt', seq: 1, content: '问题' },
    ] as never)
    store.putMessage(SID, {
      id: 'turn-live',
      type: 'turn',
      status: 'streaming',
      content: '半截',
    } as never)

    snapshot([{ id: 'u1', role: 'user', type: 'user_prompt', seq: 1, content: '问题' }])
    await store.loadMessages(SID)

    const msgs = store.getSessionMessages(SID)
    expect(msgs.map((m) => m.id)).toEqual(['u1', 'turn-live'])
    expect(msgs[1].status).toBe('streaming')
    expect(msgs[1].content).toBe('半截')
  })

  it('保留的在途节点重排到历史之后（本地 seq 游标可能小于快照最大值）', async () => {
    const store = useSessionsStore()
    // 本地游标从 0 起 ⇒ 这条在途节点拿到 seq = 1，远小于快照里的 5/6
    store.putMessage(SID, { id: 'live', type: 'text', status: 'streaming' } as never)

    snapshot([
      { id: 'h1', type: 'text', seq: 5 },
      { id: 'h2', type: 'text', seq: 6 },
    ])
    await store.loadMessages(SID)

    expect(ids(store), '沿用旧 seq 会让在途节点排到历史之前').toEqual(['h1', 'h2', 'live'])
  })

  it('终态且不在快照里的本地节点被丢弃（被删除的陈旧副本不得复活）', async () => {
    const store = useSessionsStore()
    store.putMessage(SID, { id: 'ghost', type: 'text', status: 'completed' } as never)

    snapshot([{ id: 'h1', type: 'text', seq: 1 }])
    await store.loadMessages(SID)

    expect(ids(store)).toEqual(['h1'])
  })

  it('快照里的节点整条替换：内容与状态都以快照为准', async () => {
    const store = useSessionsStore()
    store.putMessage(SID, {
      id: 'a1',
      type: 'text',
      status: 'streaming',
      content: '半截',
    } as never)

    snapshot([{ id: 'a1', type: 'text', seq: 3, status: 'completed', content: '完整' }])
    await store.loadMessages(SID)

    const [a1] = store.getSessionMessages(SID)
    expect(a1.content).toBe('完整')
    expect(a1.status).toBe('completed')
  })
})

/**
 * 转写对账：**终态补丁丢失后的自愈**。
 *
 * 消息状态只经 `kind = "vdfs"` 一条通道下发，而这条通道**不重放**
 * （订阅表为空时 `ChangeSubscriptions::notify` 直接返回，而 watch 登记是
 * fire-and-forget 的异步动作；消费循环也会在 `is_working` 翻转时丢掉手上那一帧）。
 * 丢一次终态，前端就永久停在「运行中」——它只做增量收敛，从不整表重拉。
 *
 * 判据**不是启发式**：节点补丁恒先于会话状态下发（正常收尾「清在途 → 复位
 * `is_working` → `emit_session_state`」，中止收尾「`converge_inflight` 广播节点终态
 * → `emit_session_state(aborted)`」），因此**会话报"不忙"时服务端不可能还有节点
 * 停在非终态**。据此回读收敛，而不是就地猜成 `completed`（猜错就是把"半截"
 * 谎报成"正常结束"并抹掉重试入口）。
 */
describe('sessions store — 终态丢失后的转写对账', () => {
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

  it('会话转空闲但本地仍有 streaming 节点 → 回读权威终态收敛', async () => {
    const store = useSessionsStore()
    store.putMessage(SID, {
      id: 'tc1',
      type: 'tool_call',
      status: 'streaming',
      content: '{}',
    } as never)
    // 服务端权威：这一条其实被定稿成 aborted（用户中止）。
    // 就地猜 completed 会把"半截"谎报成"正常结束"，并抹掉重试入口。
    vdfsApi.readVdfs.mockResolvedValue({
      text: JSON.stringify({
        messages: [{ id: 'tc1', type: 'tool_call', seq: 1, status: 'aborted', content: '{}' }],
      }),
    })

    emit({
      path: `@vfs/session/${SID}`,
      change: 'updated',
      node: sessionNode({ name: SID, status: 'active' }),
    })
    await flushPromises()

    expect(vdfsApi.readVdfs, '必须回读权威状态，而不是就地猜').toHaveBeenCalled()
    const [tc1] = store.getSessionMessages(SID)
    expect(tc1.status).toBe('aborted')
    expect(tc1.seq, 'seq 以快照为准，不沿用本地游标').toBe(1)
  })

  it('会话转空闲且本地没有非终态节点 → 一次回读都不发（正常路径零成本）', async () => {
    const store = useSessionsStore()
    store.putMessage(SID, {
      id: 'a1',
      type: 'text',
      status: 'completed',
      content: '好了',
    } as never)

    emit({
      path: `@vfs/session/${SID}`,
      change: 'updated',
      node: sessionNode({ name: SID, status: 'active' }),
    })
    await flushPromises()

    expect(vdfsApi.readVdfs).not.toHaveBeenCalled()
  })

  it('会话仍在运行时不触发对账（在途节点是合法的，不得被当成陈旧副本）', async () => {
    const store = useSessionsStore()
    store.putMessage(SID, {
      id: 'tc1',
      type: 'tool_call',
      status: 'streaming',
    } as never)

    emit({
      path: `@vfs/session/${SID}`,
      change: 'updated',
      node: sessionNode({ name: SID, status: 'working' }),
    })
    await flushPromises()

    expect(vdfsApi.readVdfs).not.toHaveBeenCalled()
    expect(store.getSessionMessages(SID)[0].status).toBe('streaming')
  })

  it('回读失败 → 保持现状（陈旧副本好过清空转写）', async () => {
    const store = useSessionsStore()
    store.putMessage(SID, {
      id: 'tc1',
      type: 'tool_call',
      status: 'streaming',
    } as never)
    vdfsApi.readVdfs.mockRejectedValue(new Error('IPC 断了'))

    emit({
      path: `@vfs/session/${SID}`,
      change: 'updated',
      node: sessionNode({ name: SID, status: 'active' }),
    })
    await flushPromises()

    expect(store.getSessionMessages(SID).map((m) => m.id)).toEqual(['tc1'])
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

    store.putMessage(SID, {
      id: 'a1',
      role: 'assistant',
      type: 'text',
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
