/**
 * `sessionTranscriptSync` 的单元测试。
 *
 * 断言的是**最终信封形状**（S27 收口：`path` 恒为**被变更节点自身的地址**，
 * `data` = 业务载荷，没有操作枚举）：会话是容器，其下是若干并列的集合，集合项
 * 形状统一为 `<sid>/<集合段>/<项 id>`——**身份就是地址末段**，不在载荷里。
 *
 * 语义全在字段上：`delta` 追加 / `content` 整条替换 / `status = removed` 移除 /
 * 只有 `status` 的是状态帧。**只有「本端缺基线」（身份未知）才回读**。
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
  ensureVdfsSessionScheme: vi.fn(async () => ({ mountDir: '/root/session', messagesSeg: 'message' })),
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
import { MESSAGE_TYPE_TURN } from '@/schemas/chat_message'
// 回读理由是**词表**（独立模块，未被替身），断言按它取值——替身里不抄第二份
import { READBACK_REASON } from '@/services/readback'

// 订阅接线打桩：捕获 handler 与 resync 回调，测试直接派发变更
let dispatch: ((c: VdfsChange) => void) | null = null
let fireResync: (() => void) | null = null

const MOUNT = '/root/session'
const MSG = `${MOUNT}/s1/message`

/**
 * 一条消息帧：信封的 `path` 是那条消息**节点自身**的地址
 * （`<根>/session/<sid>/message/<mid>`），载荷 `data` 就是那条 ChatMessage。
 *
 * `mid` 独立成参，正是为了钉住「**身份在地址上**」——载荷里有没有 `id` 不影响分派。
 */
function frame(sid: string, mid: string, data: Record<string, unknown> | null): VdfsChange {
  const c: Record<string, unknown> = { path: `${MOUNT}/${sid}/message/${mid}` }
  if (data !== null) c.data = data
  return c as unknown as VdfsChange
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

    dispatch!(frame('s1', 'm1', { id: 'm1', delta: '第一段' }))
    dispatch!(frame('s1', 'm1', { id: 'm1', delta: '，第二段' }))

    // 零回读：一次 I/O 都没有
    expect(mocks.statVdfs).not.toHaveBeenCalled()
    expect(mocks.readVdfs).not.toHaveBeenCalled()

    await vi.advanceTimersByTimeAsync(48)
    expect(spy.calls, '增量逐条、按到达顺序落地').toEqual([
      'message:s1:m1:第一段',
      'message:s1:m1:，第二段',
    ])
  })

  it('全量帧（载荷带 content）⇒ 零回读：就地整条替换，队列里的增量一并丢弃', async () => {
    vi.useFakeTimers()
    const spy = spySink()
    spy.has['s1/m1'] = true
    await startSessionTranscriptSync(spy.sink)

    // 先来一条增量（进合帧队列），随后首帧/全量帧到达——全量是权威全文，
    // 队列里的增量必然被它包含，再应用就是重复追加
    dispatch!(frame('s1', 'm1', { id: 'm1', delta: '半句' }))
    dispatch!(frame('s1', 'm1', { id: 'm1', content: '整条正文', status: 'completed' }))

    expect(spy.calls, '全量帧立即落地（不等合帧窗口）').toEqual(['message:s1:m1:整条正文'])
    await vi.advanceTimersByTimeAsync(48)

    expect(mocks.statVdfs, '零回读：载荷自带正文').not.toHaveBeenCalled()
    expect(mocks.readVdfs).not.toHaveBeenCalled()
    expect(spy.calls, '队列里的增量已随全量帧丢弃').toEqual(['message:s1:m1:整条正文'])
  })

  it('状态帧（只有 status）+ 身份已知 ⇒ 零回读：逐字段合并只迁移状态', async () => {
    vi.useFakeTimers()
    const spy = spySink()
    spy.has['s1/m1'] = true
    await startSessionTranscriptSync(spy.sink)

    // `state_frame` 剥掉了正文：载荷只有身份 + 状态，本地正文就是权威前缀
    dispatch!(frame('s1', 'm1', { id: 'm1', status: 'completed', seq: 3 }))
    await settle()

    expect(spy.calls).toEqual(['message:s1:m1:completed'])
    expect(mocks.statVdfs).not.toHaveBeenCalled()
    expect(mocks.readVdfs).not.toHaveBeenCalled()
  })

  it('状态帧 + 身份未知（本端缺基线）⇒ 回读 stat + read 补齐身份与正文', async () => {
    vi.useFakeTimers()
    const spy = spySink()
    await startSessionTranscriptSync(spy.sink)
    mocks.statVdfs.mockResolvedValue(messageNode({ status: 'completed', seq: 3 }))
    mocks.readVdfs.mockResolvedValue({ path: `${MSG}/m1`, text: '正文', binary: false, size: 6 })

    dispatch!(frame('s1', 'm1', { id: 'm1', status: 'completed' }))
    await vi.advanceTimersByTimeAsync(0)

    expect(spy.calls).toEqual(['message:s1:m1:正文'])
    // 两次回读都打消息节点（不打会话节点），且**理由**随请求给出
    // ——「为什么读」由此成为可断言的事实，而不是从时间戳反推
    expect(mocks.statVdfs.mock.calls[0]).toEqual([READBACK_REASON.MISSING_BASELINE, `${MSG}/m1`])
    expect(mocks.readVdfs.mock.calls[0]).toEqual([READBACK_REASON.MISSING_BASELINE, `${MSG}/m1`])
  })

  it('Turn 组合节点（本身无正文）⇒ 零回读：回读取不到任何新信息', async () => {
    vi.useFakeTimers()
    const spy = spySink()
    await startSessionTranscriptSync(spy.sink)

    // Turn 根节点的首帧：后端 `emit_streaming_start` 写死「Turn 组合节点无正文」
    // ⇒ 帧里没有 `content`，形状与「状态帧」逐字相同。但它**不是**「本端缺基线」：
    // 身份（role / type / meta）已在载荷里，而正文从来不存在——回读只会拿到一份
    // 帧里已有的 JSON 视图。这一对 `stat` + `read` 曾**每轮会话**白跑一次。
    dispatch!(
      frame('s1', 'T1', {
        id: 'T1',
        role: 'assistant',
        type: MESSAGE_TYPE_TURN,
        status: 'streaming',
        meta: { turn: 0 },
      })
    )
    await vi.advanceTimersByTimeAsync(48)

    expect(spy.calls, '身份就地落地（载荷自给自足）').toEqual(['message:s1:T1:streaming'])
    expect(mocks.statVdfs, 'Turn 无正文可补 ⇒ 不回读').not.toHaveBeenCalled()
    expect(mocks.readVdfs).not.toHaveBeenCalled()
  })

  it('身份取自**地址末段**：载荷漏了 id 也照常落地', async () => {
    vi.useFakeTimers()
    const spy = spySink()
    spy.has['s1/m7'] = true
    await startSessionTranscriptSync(spy.sink)

    dispatch!(frame('s1', 'm7', { delta: '正文' }))
    await vi.advanceTimersByTimeAsync(48)

    expect(spy.calls).toEqual(['message:s1:m7:正文'])
  })

  it('status = removed ⇒ 就地移除（信封无 deleted），队列里的增量一并丢弃', async () => {
    vi.useFakeTimers()
    const spy = spySink()
    spy.has['s1/m9'] = true
    await startSessionTranscriptSync(spy.sink)

    // 先来一条增量（进合帧队列），随后删除
    dispatch!(frame('s1', 'm9', { id: 'm9', delta: 'x' }))
    dispatch!(frame('s1', 'm9', { id: 'm9', status: 'removed' }))

    expect(spy.calls, '删除立即落地（不等合帧窗口）').toEqual(['message:s1:m9:removed'])
    await vi.advanceTimersByTimeAsync(48)
    expect(spy.calls, '队列里的增量已随删除丢弃').toEqual(['message:s1:m9:removed'])
  })

  it('无载荷变更（`data` 缺失）不认：回读收敛不归本模块管', async () => {
    vi.useFakeTimers()
    const spy = spySink()
    await startSessionTranscriptSync(spy.sink)

    dispatch!(frame('s1', 'm1', null))
    await vi.advanceTimersByTimeAsync(0)

    expect(spy.calls).toEqual([])
    expect(mocks.statVdfs).not.toHaveBeenCalled()
  })
})

describe('读与流的协调（appendGuard 的代际记法）', () => {
  it('回读期间**没有**正文落地 ⇒ 接受 content，并丢弃队列里该 id 的待落地增量', async () => {
    vi.useFakeTimers()
    const spy = spySink()
    // id 未知 ⇒ 先落增量（进队列）再回读；队列里的增量发生在读取发起**之前**
    spy.has['s1/m1'] = false
    await startSessionTranscriptSync(spy.sink)
    mocks.statVdfs.mockResolvedValue(messageNode({}))
    mocks.readVdfs.mockResolvedValue({ path: `${MSG}/m1`, text: '基线＋已发增量', binary: false, size: 10 })

    dispatch!(frame('s1', 'm1', { id: 'm1', delta: '增量' }))
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

    dispatch!(frame('s1', 'm1', { id: 'm1' }))
    await settle()
    // 读取窗口内：身份已知的增量落地 ⇒ 代际 +1
    spy.has['s1/m1'] = true
    dispatch!(frame('s1', 'm1', { id: 'm1', delta: '窗口内的增量' }))
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
    dispatch!(frame('s1', 'm1', { id: 'm1' }))
    await vi.advanceTimersByTimeAsync(0)
    expect(spy.calls).toEqual(['message:s1:m1:全量'])

    // id 现在已知：增量走零回读，不再有读取窗口
    spy.has['s1/m1'] = true
    dispatch!(frame('s1', 'm1', { id: 'm1', delta: '后到的增量' }))
    await vi.advanceTimersByTimeAsync(48)
    expect(spy.calls).toEqual(['message:s1:m1:全量', 'message:s1:m1:后到的增量'])
  })
})

describe('职责边界：会话节点与别的集合归各自的消费端', () => {
  it('`<sid>`（恰一段）不归本模块：运行态 / 资源信号都交给 sessionNodeSync', async () => {
    vi.useFakeTimers()
    const spy = spySink()
    await startSessionTranscriptSync(spy.sink)

    dispatch!({ path: `${MOUNT}/s1`, data: { path: `${MOUNT}/s1`, status: 'active' } } as unknown as VdfsChange)
    dispatch!({ path: `${MOUNT}/s1` } as unknown as VdfsChange)
    await settle()

    expect(spy.calls).toEqual([])
    expect(mocks.statVdfs, '会话节点的那次 stat 归 sessionNodeSync，本模块不再重复').not.toHaveBeenCalled()
  })

  it('集合目录（两段）/ 更深层级（四段）/ 非消息集合段 / 别的挂载都不认', async () => {
    vi.useFakeTimers()
    const spy = spySink()
    await startSessionTranscriptSync(spy.sink)

    // 集合目录：列表收敛走各自的列表刷新，不是逐项实时面的职责
    dispatch!({ path: `${MOUNT}/s1/message` } as unknown as VdfsChange)
    // 更深层级：集合项的形状恰是 `<sid>/<集合段>/<项 id>` 三段
    dispatch!({ path: `${MSG}/m1/深层`, data: { id: 'm1', delta: 'x' } } as unknown as VdfsChange)
    // 非消息集合段（记忆 / 子会话 / 工作目录…）：各有各的消费端
    dispatch!({ path: `${MOUNT}/s1/记忆/note.md`, data: { id: 'note.md', content: 'x' } } as unknown as VdfsChange)
    // 别的挂载
    dispatch!({ path: '/root/other/x', data: { id: 'x' } } as unknown as VdfsChange)
    await vi.advanceTimersByTimeAsync(0)

    expect(spy.calls).toEqual([])
    expect(mocks.statVdfs).not.toHaveBeenCalled()
  })
})

describe('作用域与 resync', () => {
  it('resync ⇒ 按见过的会话整份重读，并丢弃待落地增量', async () => {
    vi.useFakeTimers()
    const spy = spySink()
    await startSessionTranscriptSync(spy.sink)

    dispatch!(frame('a', 'm1', { id: 'm1' }))
    dispatch!(frame('b', 'm1', { id: 'm1' }))
    mocks.statVdfs.mockResolvedValue(messageNode({}))
    mocks.readVdfs.mockResolvedValue({ path: '', text: '', binary: false, size: 0 })
    await vi.advanceTimersByTimeAsync(0)

    fireResync!()
    await vi.advanceTimersByTimeAsync(0)
    expect([...spy.reloaded].sort()).toEqual(['a', 'b'])
  })
})
