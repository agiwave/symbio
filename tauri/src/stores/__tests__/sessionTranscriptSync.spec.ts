/**
 * `sessionTranscriptSync` 的单元测试。
 *
 * 断言的是**新信封形状**（S27：`path` = 消息目录 / 会话叶子，`data` = 业务载荷，
 * 没有操作枚举）：消息帧的身份在 `data.id`，语义在字段上（`delta` 追加 /
 * `content` 替换 / `status = removed` 移除）；会话叶子随 `data` 带全量节点视图。
 */
import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest'

const mocks = vi.hoisted(() => ({
  statVdfs: vi.fn(),
  readVdfs: vi.fn(),
}))

vi.mock('@/services/vdfs', () => ({
  statVdfs: mocks.statVdfs,
  readVdfs: mocks.readVdfs,
}))
vi.mock('@/services/eventBus', () => ({
  subscribeVdfsChanged: vi.fn(),
}))
vi.mock('@/services/vdfsScheme', () => ({
  ensureVdfsSessionScheme: vi.fn(async () => ({ mountDir: '/root/session', messagesSeg: '消息' })),
}))
vi.mock('@/utils/logger', () => ({
  logger: { error: vi.fn(), warn: vi.fn(), debug: vi.fn(), info: vi.fn() },
}))

import {
  startSessionTranscriptSync,
  stopSessionTranscriptSync,
  resetSessionTranscriptSyncForTest,
  type TranscriptSyncSink,
} from '../sessionTranscriptSync'
import { subscribeVdfsChanged } from '@/services/eventBus'
import type { VdfsChange, VdfsNode } from '@/schemas/vdfs'

// 订阅接线打桩：捕获 handler 与 resync 回调，测试直接派发变更
let dispatch: ((c: VdfsChange) => void) | null = null
let fireResync: (() => void) | null = null

const MOUNT = '/root/session'
const MSG = `${MOUNT}/s1/消息`

/** 一条消息帧：信封的 `path` 是消息**目录**，载荷 `data` 就是那条 ChatMessage */
function frame(sid: string, data: Record<string, unknown> | null): VdfsChange {
  const c: Record<string, unknown> = { path: `${MOUNT}/${sid}/消息` }
  if (data !== null) c.data = data
  return c as unknown as VdfsChange
}

/** 会话叶子节点（stat 回读 / 随载荷视图） */
function sessionNode(status: string): VdfsNode {
  return { path: `${MOUNT}/s1`, name: 's1', title: '', kind: 'session', status, access: 'rw' }
}

/** 消息节点（stat 回读；attributes 平铺在顶层，与后端 `message_node` 同形） */
function messageNode(over: Partial<VdfsNode> & Record<string, unknown>): VdfsNode {
  return {
    path: `${MSG}/m1`, name: 'm1', title: '', kind: '', access: 'r', ext: 'message',
    status: 'streaming', updated_at: 1000,
    role: 'assistant', type: 'text', parent_id: 'T1', seq: 7,
    ...over,
  } as VdfsNode
}

/** 记录落地动作的假 sink */
function spySink() {
  const calls: string[] = []
  const reloaded: string[] = []
  const has: Record<string, boolean> = {}
  const sink: TranscriptSyncSink = {
    hasMessage: (sid, mid) => has[`${sid}/${mid}`] ?? false,
    applyTranscriptMessages: (sid, ms) => {
      for (const m of ms) calls.push(`message:${sid}:${m.id}:${m.delta ?? m.content ?? m.status ?? ''}`)
    },
    applySessionState: (sid, node) => calls.push(`session:${sid}:${node.status}`),
    reload: (sid) => {
      reloaded.push(sid)
    },
  }
  return { sink, calls, reloaded, has }
}

/** 异步微任务冲尽（等 `void readMessage()` 的 stat+read 完成且落地） */
async function settle(): Promise<void> {
  for (let i = 0; i < 6; i++) await Promise.resolve()
}

beforeEach(() => {
  vi.resetAllMocks()
  resetSessionTranscriptSyncForTest()
  dispatch = null
  fireResync = null
  vi.mocked(subscribeVdfsChanged).mockImplementation(((_scope: unknown, handler: (c: VdfsChange) => void, onResync?: () => void) => {
    dispatch = handler
    fireResync = onResync ?? null
    return () => {}
  }) as never)
})
afterEach(() => {
  stopSessionTranscriptSync()
  vi.useRealTimers()
})

describe('消息帧：语义全在 data 的字段上（信封没有操作枚举）', () => {
  it('delta + 身份已知 ⇒ 零回读：不发 stat/read，增量按到达顺序落地', async () => {
    vi.useFakeTimers()
    const spy = spySink()
    spy.has['s1/m1'] = true
    await startSessionTranscriptSync(spy.sink)

    dispatch!(frame('s1', { id: 'm1', delta: '第一段' }))
    dispatch!(frame('s1', { id: 'm1', delta: '，第二段' }))

    // 零回读：一次 I/O 都没有
    expect(mocks.statVdfs).not.toHaveBeenCalled()
    expect(mocks.readVdfs).not.toHaveBeenCalled()

    await vi.advanceTimersByTimeAsync(48)
    expect(spy.calls, '增量逐条、按到达顺序落地').toEqual([
      'message:s1:m1:第一段',
      'message:s1:m1:，第二段',
    ])
  })

  it('全量帧（无 delta）⇒ 回读身份与正文基线（stat + read）', async () => {
    vi.useFakeTimers()
    const spy = spySink()
    await startSessionTranscriptSync(spy.sink)
    mocks.statVdfs.mockResolvedValue(messageNode({ status: 'completed', seq: 3 }))
    mocks.readVdfs.mockResolvedValue({ path: `${MSG}/m1`, text: '正文', binary: false, size: 6 })

    dispatch!(frame('s1', { id: 'm1' }))
    await vi.advanceTimersByTimeAsync(0)

    expect(spy.calls).toEqual(['message:s1:m1:正文'])
    // stat 只打消息节点，不打会话叶子
    expect(mocks.statVdfs.mock.calls[0][0]).toBe(`${MSG}/m1`)
  })

  it('status = removed ⇒ 就地移除（信封无 deleted），队列里的增量一并丢弃', async () => {
    vi.useFakeTimers()
    const spy = spySink()
    spy.has['s1/m9'] = true
    await startSessionTranscriptSync(spy.sink)

    // 先来一条增量（进合帧队列），随后删除
    dispatch!(frame('s1', { id: 'm9', delta: 'x' }))
    dispatch!(frame('s1', { id: 'm9', status: 'removed' }))

    expect(spy.calls, '删除立即落地（不等合帧窗口）').toEqual(['message:s1:m9:removed'])
    await vi.advanceTimersByTimeAsync(48)
    expect(spy.calls, '队列里的增量已随删除丢弃').toEqual(['message:s1:m9:removed'])
  })

  it('容器节点（Turn / ToolCall）的正文是整条消息 JSON：按结构解析', async () => {
    vi.useFakeTimers()
    const spy = spySink()
    await startSessionTranscriptSync(spy.sink)
    const full = { id: 'tc1', role: 'assistant', type: 'tool_call', parent_id: 'T1', name: 'read', status: 'completed', content: '{"a":1}' }
    mocks.statVdfs.mockResolvedValue(messageNode({ name: 'tc1', type: 'tool_call', status: 'completed' }))
    mocks.readVdfs.mockResolvedValue({ path: `${MSG}/tc1`, text: JSON.stringify(full), binary: false, size: 10 })

    dispatch!(frame('s1', { id: 'tc1' }))
    await vi.advanceTimersByTimeAsync(0)

    expect(spy.calls).toEqual([`message:s1:tc1:{"a":1}`])
  })

  it('无载荷变更（`data` 缺失）不认：回读收敛不归本模块管', async () => {
    vi.useFakeTimers()
    const spy = spySink()
    await startSessionTranscriptSync(spy.sink)

    dispatch!(frame('s1', null))
    await vi.advanceTimersByTimeAsync(0)

    expect(spy.calls).toEqual([])
    expect(mocks.statVdfs).not.toHaveBeenCalled()
  })
})

describe('读与流的协调（appendGuard 的代际记法）', () => {
  it('回读期间**没有**增量落地 ⇒ 接受 content，并丢弃队列里该 id 的待落地增量', async () => {
    vi.useFakeTimers()
    const spy = spySink()
    // id 未知 ⇒ 先落增量（进队列）再回读；队列里的增量发生在读取发起**之前**
    spy.has['s1/m1'] = false
    await startSessionTranscriptSync(spy.sink)
    mocks.statVdfs.mockResolvedValue(messageNode({}))
    mocks.readVdfs.mockResolvedValue({ path: `${MSG}/m1`, text: '基线＋已发增量', binary: false, size: 10 })

    dispatch!(frame('s1', { id: 'm1', delta: '增量' }))
    await vi.advanceTimersByTimeAsync(0)

    // content 已含该增量（快照在读取发起之后），队列里的增量不得再应用
    expect(spy.calls).toEqual(['message:s1:m1:基线＋已发增量'])
  })

  it('回读期间**有**增量落地 ⇒ 丢弃回读的 content、只取身份（本地增量链是真值前缀）', async () => {
    vi.useFakeTimers()
    const spy = spySink()
    spy.has['s1/m1'] = false
    await startSessionTranscriptSync(spy.sink)

    // read 慢一拍：先记下代际，增量在读取期间落地
    let resolveStat: (n: VdfsNode) => void = () => {}
    mocks.statVdfs.mockReturnValue(new Promise((res) => (resolveStat = res)))
    mocks.readVdfs.mockResolvedValue({ path: `${MSG}/m1`, text: '可能落后的快照', binary: false, size: 10 })

    dispatch!(frame('s1', { id: 'm1' }))
    await settle()
    // 读取窗口内：身份已知的增量落地 ⇒ 代际 +1
    spy.has['s1/m1'] = true
    dispatch!(frame('s1', { id: 'm1', delta: '窗口内的增量' }))
    resolveStat(messageNode({}))
    await vi.advanceTimersByTimeAsync(48)

    // 回读的 content 被丢弃（只落地身份），窗口内落地的增量照常应用
    expect(spy.calls).toEqual([
      'message:s1:m1:streaming',
      'message:s1:m1:窗口内的增量',
    ])
  })

  it('重复回读的代际各记各的：增量只对**读取窗口内**的响应生效', async () => {
    vi.useFakeTimers()
    const spy = spySink()
    spy.has['s1/m1'] = false
    await startSessionTranscriptSync(spy.sink)

    mocks.statVdfs.mockResolvedValue(messageNode({ status: 'completed', seq: 5 }))
    mocks.readVdfs.mockResolvedValue({ path: `${MSG}/m1`, text: '全量', binary: false, size: 6 })

    // 第一次回读（无增量干扰）→ 接受 content
    dispatch!(frame('s1', { id: 'm1' }))
    await vi.advanceTimersByTimeAsync(0)
    expect(spy.calls).toEqual(['message:s1:m1:全量'])

    // id 现在已知：增量走零回读，不再有读取窗口
    spy.has['s1/m1'] = true
    dispatch!(frame('s1', { id: 'm1', delta: '后到的增量' }))
    await vi.advanceTimersByTimeAsync(48)
    expect(spy.calls).toEqual(['message:s1:m1:全量', 'message:s1:m1:后到的增量'])
  })
})

describe('会话叶子（运行态 / 资源 / 删除）', () => {
  it('`<sid>` 带节点视图 ⇒ 零回读：data 就是全量视图，直接 applySessionState', async () => {
    vi.useFakeTimers()
    const spy = spySink()
    await startSessionTranscriptSync(spy.sink)

    dispatch!({ path: `${MOUNT}/s1`, data: sessionNode('active') } as unknown as VdfsChange)
    await settle()

    expect(spy.calls).toEqual(['session:s1:active'])
    expect(mocks.statVdfs).not.toHaveBeenCalled()
  })

  it('`<sid>` 无载荷 ⇒ stat 回读兜底（资源信号不走视图）', async () => {
    vi.useFakeTimers()
    const spy = spySink()
    await startSessionTranscriptSync(spy.sink)
    mocks.statVdfs.mockResolvedValue(sessionNode('active'))

    dispatch!({ path: `${MOUNT}/s1` } as unknown as VdfsChange)
    await vi.advanceTimersByTimeAsync(0)

    expect(spy.calls).toEqual(['session:s1:active'])
  })

  it('`<sid>` 已删（stat 得 null）⇒ 静默无事：收敛归清单订阅', async () => {
    vi.useFakeTimers()
    const spy = spySink()
    await startSessionTranscriptSync(spy.sink)
    mocks.statVdfs.mockResolvedValue(null)

    dispatch!({ path: `${MOUNT}/s1` } as unknown as VdfsChange)
    await vi.advanceTimersByTimeAsync(0)

    expect(spy.calls).toEqual([])
  })
})

describe('作用域与 resync', () => {
  it('记忆段 / 更深层级 / 别的挂载 / 无载荷的消息目录不认', async () => {
    vi.useFakeTimers()
    const spy = spySink()
    await startSessionTranscriptSync(spy.sink)

    dispatch!({ path: `${MOUNT}/s1/AGENTS.md`, change: 'updated' } as unknown as VdfsChange)
    dispatch!({ path: `${MSG}/m1/深层`, change: 'updated' } as unknown as VdfsChange)
    dispatch!({ path: '/root/other/x', change: 'updated' } as unknown as VdfsChange)
    dispatch!(frame('s1', null))
    await vi.advanceTimersByTimeAsync(0)

    expect(spy.calls).toEqual([])
    expect(mocks.statVdfs).not.toHaveBeenCalled()
  })

  it('resync ⇒ 按见过的会话整份重读，并丢弃待落地增量', async () => {
    vi.useFakeTimers()
    const spy = spySink()
    await startSessionTranscriptSync(spy.sink)

    dispatch!(frame('a', { id: 'm1' }))
    dispatch!(frame('b', { id: 'm1' }))
    mocks.statVdfs.mockResolvedValue(messageNode({}))
    mocks.readVdfs.mockResolvedValue({ path: '', text: '', binary: false, size: 0 })
    await vi.advanceTimersByTimeAsync(0)

    fireResync!()
    await vi.advanceTimersByTimeAsync(0)
    expect([...spy.reloaded].sort()).toEqual(['a', 'b'])
  })
})
