// @vitest-environment happy-dom
/**
 * sessions store —— VDFS 变更驱动的会话清单收敛
 *
 * 覆盖两条约定：
 *
 * 1. **只有一条变更频道**：会话清单的同步来自 `kind = 'vdfs'` 的
 *    `VdfsChangeEvent`，作用域按**展示地址前缀**分流（`.vdfs/session` 的直接子项
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

/** store 在 setup 时订阅一次 vdfs 变更频道；此处捕获作用域与回调以便直接投递事件 */
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

const sessionApi = vi.hoisted(() => ({
  listSessions: vi.fn(async () => []),
  deleteMessage: vi.fn(),
}))
vi.mock('@/services/session', () => ({
  listSessions: sessionApi.listSessions,
  deleteSession: vi.fn(),
  createSessionId: () => 'generated-id',
  updateSession: vi.fn(),
  clearMessages: vi.fn(),
  deleteMessage: sessionApi.deleteMessage,
  updateMessage: vi.fn(),
}))

const vdfsApi = vi.hoisted(() => ({
  readVdfs: vi.fn(),
  statVdfs: vi.fn(),
  writeVdfs: vi.fn(),
  watchVdfs: vi.fn(async () => {}),
  unwatchVdfs: vi.fn(async () => {}),
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

import { useSessionsStore } from '../sessions'
import { VDFS_SESSION_DIR, VDFS_ROOT, vdfsJoin } from '@/schemas/vdfs'

/** 投递一条变更（路径就是展示地址） */
function emit(change: Record<string, unknown>) {
  captured.handlers[0](change)
}

function sessionNode(over: Record<string, unknown> = {}) {
  return {
    path: `${VDFS_ROOT}/${VDFS_SESSION_DIR}/s1`,
    name: 's1',
    title: '解释一下 VDFS 的地址模型',
    kind: 'session',
    status: 'active',
    access: 'rw',
    ...over,
  }
}

describe('sessions store — VDFS 变更的清单收敛', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    captured.scopes.length = 0
    captured.handlers.length = 0
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
    expect(path).toBe(vdfsJoin(VDFS_ROOT, VDFS_SESSION_DIR))
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

  it('store 在 setup 时恰好订阅一次，作用域 = 会话叶子的直接子项', () => {
    useSessionsStore()
    expect(captured.handlers).toHaveLength(1)
    expect(captured.scopes[0]).toEqual({
      prefix: vdfsJoin(VDFS_ROOT, VDFS_SESSION_DIR),
      directChildren: true,
    })
  })

  it('updated 带节点视图 → 就地落 status / 标题，零回读、零整表重拉', async () => {
    const store = useSessionsStore()
    store.titles['s1'] = '新对话'
    store.list.push({ id: 's1', message_count: 0, updated_at: 0, status: 'active', metadata: { title: '新对话' } } as never)

    emit({
      path: '.vdfs/session/s1',
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

    emit({ path: '.vdfs/session/s1', change: 'updated' })
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
      path: '.vdfs/session/s1',
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

    emit({ path: '.vdfs/session/s1', change: 'updated', node: sessionNode({ status: 'working' }) })

    expect(store.getSessionError('s1')).toBeNull()
    expect(store.isSessionFailed('s1')).toBe(false)
  })

  it('提示音只在 working → 非 working 的**迁移**上响，音色取 outcome', async () => {
    const store = useSessionsStore()
    store.list.push({ id: 's1', message_count: 0, updated_at: 0, status: 'active', metadata: {} } as never)

    // ① 空闲 → 空闲（标题更新等）：不是迁移，不响
    emit({ path: '.vdfs/session/s1', change: 'updated', node: sessionNode({ status: 'active' }) })
    expect(chime.playCompletionChime).not.toHaveBeenCalled()

    // ② 空闲 → 运行中：是迁移，但不是"结束"，不响
    emit({ path: '.vdfs/session/s1', change: 'updated', node: sessionNode({ status: 'working' }) })
    expect(chime.playCompletionChime).not.toHaveBeenCalled()

    // ③ 运行中 → 空闲（正常结束）：响 completed
    emit({
      path: '.vdfs/session/s1',
      change: 'updated',
      node: sessionNode({ status: 'active', outcome: 'completed' }),
    })
    expect(chime.playCompletionChime).toHaveBeenCalledWith('completed', 's1')

    // ④ 中止与失败各取自己的音色（结局是状态，不靠"谁先到"区分）
    emit({ path: '.vdfs/session/s1', change: 'updated', node: sessionNode({ status: 'working' }) })
    emit({
      path: '.vdfs/session/s1',
      change: 'updated',
      node: sessionNode({ status: 'active', outcome: 'aborted' }),
    })
    expect(chime.playCompletionChime).toHaveBeenLastCalledWith('aborted', 's1')

    emit({ path: '.vdfs/session/s1', change: 'updated', node: sessionNode({ status: 'working' }) })
    emit({
      path: '.vdfs/session/s1',
      change: 'updated',
      node: sessionNode({ status: 'failed', outcome: 'failed', error: 'boom' }),
    })
    expect(chime.playCompletionChime).toHaveBeenLastCalledWith('failed', 's1')
    expect(chime.playCompletionChime).toHaveBeenCalledTimes(3)
  })

  it('标题更新（状态未变）不覆盖消息节点派生的活动文字', async () => {
    const store = useSessionsStore()
    store.list.push({ id: 's1', message_count: 0, updated_at: 0, status: 'active', metadata: {} } as never)
    emit({ path: '.vdfs/session/s1', change: 'updated', node: sessionNode({ status: 'working' }) })
    store.putStatus('s1', { activity: '正在思考…' })

    emit({ path: '.vdfs/session/s1', change: 'updated', node: sessionNode({ status: 'working' }) })

    expect(store.getSessionStatus('s1').activity).toBe('正在思考…')
  })

  it('created → 防抖重拉清单（收敛排序与完整字段）', async () => {
    vi.useFakeTimers()
    try {
      const store = useSessionsStore()
      emit({ path: '.vdfs/session/newsid', change: 'created' })
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

    emit({ path: '.vdfs/session/s1', change: 'deleted' })

    expect(store.list).toHaveLength(0)
    expect(sessionApi.listSessions).not.toHaveBeenCalled()
  })

  it('appended（转写增量）与会话清单无关：既不重读也不重拉', async () => {
    const store = useSessionsStore()
    store.list.push({ id: 's1', message_count: 0, updated_at: 0, metadata: {} } as never)

    emit({ path: '.vdfs/session/s1/消息/m1', change: 'appended', delta: '半句' })
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
