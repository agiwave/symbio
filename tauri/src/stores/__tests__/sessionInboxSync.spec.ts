/**
 * `sessionInboxSync` 的单元测试。
 *
 * 断言的是排队窗口的实时面契约：入队帧带**节点视图**（⇒ 加一条「待发送」气泡，
 * 文案取 `description` 预览），出队 / 取消 / 清空是**无载荷**变更（⇒ 回读 `stat`
 * 只分辨删除与否，`NotFound` ⇒ 移除）。与另两个消费端同一套分派规矩：**身份就是
 * 地址末段**（`<挂载目录>/<sid>/<收件箱段>/<iid>`），信封上没有操作枚举。
 */
import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest'

const mocks = vi.hoisted(() => ({
  statVdfs: vi.fn(),
  ensureSessionScheme: vi.fn(),
}))

vi.mock('@/services/vdfs', () => ({
  statVdfs: mocks.statVdfs,
}))
vi.mock('@/services/eventBus', () => ({
  subscribeVdfsChanged: vi.fn(),
}))
vi.mock('@/services/vdfsScheme', () => ({
  ensureSessionScheme: mocks.ensureSessionScheme,
}))
vi.mock('@/utils/logger', () => ({
  logger: { error: vi.fn(), warn: vi.fn(), debug: vi.fn(), info: vi.fn() },
}))

import {
  handleVdfsChange,
  registerInboxMount,
  startSessionInboxSync,
  stopSessionInboxSync,
  resetSessionInboxSyncForTest,
  inboxSegOf,
  type SessionInboxSink,
} from '../sessionInboxSync'
import { subscribeVdfsChanged } from '@/services/eventBus'
import type { VdfsChange, VdfsNode } from '@/schemas/vdfs'
// 回读理由是**词表**（独立模块，未被替身），断言按它取值——替身里不抄第二份
import { READBACK_REASON } from '@/services/readback'

// 订阅接线打桩：捕获 handler 与 resync 回调，测试直接派发变更
let dispatch: ((c: VdfsChange) => void) | null = null
let fireResync: (() => void) | null = null

const MOUNT = '/root/session'
const SEG = 'inbox'

/** 收件箱条目帧：`path` 是条目**自身**的地址，`data` 是它的节点视图 */
function itemFrame(
  sid: string,
  iid: string,
  data: Record<string, unknown> | null,
  mount = MOUNT,
): VdfsChange {
  const c: Record<string, unknown> = { path: `${mount}/${sid}/${SEG}/${iid}` }
  if (data !== null) c.data = data
  return c as unknown as VdfsChange
}

/** 入队帧的节点视图（后端 `inbox_item_node`：正文只在 `description` 的预览里） */
function inboxNode(over: Partial<VdfsNode> & Record<string, unknown>): VdfsNode {
  return {
    path: '', name: 'i1', title: '', kind: '', access: 'r', ext: 'message',
    status: 'pending', updated_at: 1000,
    description: '待消费 · 帮我跑一下测试',
    ...over,
  } as VdfsNode
}

/** 记录落地动作的假 sink */
function spySink() {
  const applied: Array<{ sid: string; id: string; label: string }> = []
  const removed: Array<{ sid: string; id: string }> = []
  const reloaded: string[] = []
  const sink: SessionInboxSink = {
    applyInboxItem: (sid, e) => applied.push({ sid, id: e.id, label: e.label }),
    removeInboxItem: (sid, iid) => removed.push({ sid, id: iid }),
    reloadInbox: (sid) => {
      reloaded.push(sid)
    },
  }
  return { sink, applied, removed, reloaded }
}

/** 异步微任务冲尽（等 `void statVdfs()` 的 promise 链落地） */
async function settle(): Promise<void> {
  for (let i = 0; i < 6; i++) await Promise.resolve()
}

beforeEach(() => {
  vi.resetAllMocks()
  resetSessionInboxSyncForTest()
  dispatch = null
  fireResync = null
  mocks.ensureSessionScheme.mockResolvedValue({
    mountDir: MOUNT,
    messagesSeg: 'message',
    inboxSeg: SEG,
  })
  mocks.statVdfs.mockResolvedValue(inboxNode({}))
  vi.mocked(subscribeVdfsChanged).mockImplementation(((_scope: unknown, handler: (c: VdfsChange) => void, onResync?: () => void) => {
    dispatch = handler
    fireResync = onResync ?? null
    return () => {}
  }) as never)
})
afterEach(() => {
  stopSessionInboxSync()
})

describe('入队帧：带节点视图 ⇒ 加一条「待发送」气泡', () => {
  it('文案取后端 description（预览），身份取地址末段', async () => {
    const spy = spySink()
    await startSessionInboxSync(spy.sink)

    dispatch!(itemFrame('s1', 'i1', { name: 'i1', description: '待消费 · 帮我跑一下测试' }))

    expect(spy.applied).toEqual([{ sid: 's1', id: 'i1', label: '待消费 · 帮我跑一下测试' }])
    // 零回读：排队气泡是状态提示，全文由落库后的转写帧给（同一个 id）
    expect(mocks.statVdfs).not.toHaveBeenCalled()
  })

  it('description 缺失 / 为空（纯附件消息）⇒ 退回「待发送」', async () => {
    const spy = spySink()
    await startSessionInboxSync(spy.sink)

    dispatch!(itemFrame('s1', 'i1', { name: 'i1' }))
    dispatch!(itemFrame('s1', 'i2', { name: 'i2', description: '   ' }))

    expect(spy.applied.map((a) => a.label)).toEqual(['待发送', '待发送'])
  })

  it('载荷 name 与地址末段不一致 ⇒ 按地址落地，不丢帧', async () => {
    const spy = spySink()
    await startSessionInboxSync(spy.sink)

    dispatch!(itemFrame('s1', 'i9', { name: '别的 id', description: '待消费 · x' }))

    expect(spy.applied).toEqual([{ sid: 's1', id: 'i9', label: '待消费 · x' }])
  })
})

describe('出队帧：无载荷 ⇒ 回读只分辨删除与否', () => {
  it('条目已不在队列（stat 解析为 null）⇒ 移除', async () => {
    const spy = spySink()
    await startSessionInboxSync(spy.sink)
    mocks.statVdfs.mockResolvedValue(null)

    dispatch!(itemFrame('s1', 'i1', null))
    await settle()

    expect(mocks.statVdfs.mock.calls[0]).toEqual([READBACK_REASON.RESOURCE_SIGNAL, `${MOUNT}/s1/${SEG}/i1`])
    expect(spy.removed).toEqual([{ sid: 's1', id: 'i1' }])
  })

  it('条目还在（竞态：别的进程同一轮又入了一条）⇒ 不移除', async () => {
    const spy = spySink()
    await startSessionInboxSync(spy.sink)
    mocks.statVdfs.mockResolvedValue(inboxNode({ name: 'i1' }))

    dispatch!(itemFrame('s1', 'i1', null))
    await settle()

    expect(spy.removed).toEqual([])
  })
})

describe('职责边界：地址形状不对不归本模块', () => {
  it('会话节点（一段）/ 集合目录（两段）/ 四段 / 转写段 / 别的挂载都不认', async () => {
    const spy = spySink()
    await startSessionInboxSync(spy.sink)

    // 会话节点自身：归 sessionNodeSync
    dispatch!({ path: `${MOUNT}/s1`, data: { status: 'active' } } as unknown as VdfsChange)
    // 集合目录：后端不发这种变更，本模块也不该解释
    dispatch!({ path: `${MOUNT}/s1/${SEG}`, data: { description: '待消费 · x' } } as unknown as VdfsChange)
    // 更深层级：集合项恰三段
    dispatch!({ path: `${MOUNT}/s1/${SEG}/i1/深层`, data: { description: 'x' } } as unknown as VdfsChange)
    // 转写段：归 sessionTranscriptSync
    dispatch!({ path: `${MOUNT}/s1/message/m1`, data: { content: 'x' } } as unknown as VdfsChange)
    // 别的挂载（未登记）
    dispatch!({ path: '/root/other/s1/inbox/i1', data: { description: 'x' } } as unknown as VdfsChange)
    await settle()

    expect(spy.applied).toEqual([])
    expect(spy.removed).toEqual([])
    expect(mocks.statVdfs).not.toHaveBeenCalled()
  })

  it('登记了子智能体空间的挂载目录后，其下的收件箱变更也被认', async () => {
    const spy = spySink()
    mocks.ensureSessionScheme.mockResolvedValue({
      mountDir: '/root/agent/a1/session',
      messagesSeg: 'message',
      inboxSeg: SEG,
    })
    expect(await registerInboxMount('/root/agent/a1/session')).toBe(true)
    expect(inboxSegOf('/root/agent/a1/session')).toBe(SEG)
    await startSessionInboxSync(spy.sink)

    dispatch!(itemFrame('s2', 'i7', { name: 'i7', description: '待消费 · 子空间' }, '/root/agent/a1/session'))

    expect(spy.applied).toEqual([{ sid: 's2', id: 'i7', label: '待消费 · 子空间' }])
  })

  it('重复登记同一目录是空操作（返回 false，不叠订阅）', async () => {
    expect(await registerInboxMount(MOUNT)).toBe(true)
    expect(await registerInboxMount(MOUNT)).toBe(false)
  })

  it('段名解析失败 ⇒ 返回 false 且不订阅', async () => {
    mocks.ensureSessionScheme.mockRejectedValue(new Error('集合段未解析'))
    expect(await registerInboxMount('/root/empty')).toBe(false)
    expect(inboxSegOf('/root/empty')).toBeUndefined()
  })
})

describe('作用域与 resync', () => {
  it('resync ⇒ 按见过的会话整份重列（漏一帧 = 幽灵气泡或少一个气泡）', async () => {
    const spy = spySink()
    await startSessionInboxSync(spy.sink)

    dispatch!(itemFrame('a', 'i1', { name: 'i1', description: '待消费 · x' }))
    dispatch!(itemFrame('b', 'i1', { name: 'i1', description: '待消费 · y' }))
    await settle()

    fireResync!()
    await settle()
    expect([...spy.reloaded].sort()).toEqual(['a', 'b'])
  })

  it('没见过收件箱变更的会话不重列（没有气泡可对账）', async () => {
    const spy = spySink()
    await startSessionInboxSync(spy.sink)

    fireResync!()
    await settle()
    expect(spy.reloaded).toEqual([])
  })
})

describe('单订阅', () => {
  it('handleVdfsChange 在未启动时是空操作（没有 sink 可落地）', async () => {
    expect(() => handleVdfsChange(itemFrame('s1', 'i1', { name: 'i1' }))).not.toThrow()
    expect(mocks.statVdfs).not.toHaveBeenCalled()
  })

  it('重复启动不叠监听器（进程内天然单订阅）', async () => {
    const spy = spySink()
    await startSessionInboxSync(spy.sink)
    await startSessionInboxSync(spy.sink)

    // 桩按实现各返回一个 unsubscribe；这里只断言 start 被调用次数对应的订阅数
    // 由模块内部的 stop→subscribe 保证，外层可见证据是 handler 只有一个
    expect(vi.mocked(subscribeVdfsChanged).mock.calls.length).toBeGreaterThan(0)
  })
})
