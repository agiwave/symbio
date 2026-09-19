/**
 * vdfsTranscriptSync — 转写实时消费端单测（node 环境）
 *
 * 锁定三件事：
 *
 * 1. **载荷优先，回读兜底**：后端在 `created` / `updated` 上附带节点视图与内容
 *    快照时**零回读**（VDFS 承载转写因此不比既有专用通道更贵）；未附带才回退。
 * 2. **逐路径串行**：`created` 的回读与紧随其后的 `appended` 不得竞争——回读
 *    挂起期间到达的增量必须等到回读落地之后再拼接，否则会被旧快照覆盖（静默丢字）。
 * 3. **非转写路径一律不碰**（子会话 / 工作目录 / 会话清单的变更与本模块无关）。
 * 4. **两种删除语义不可混淆**：`deleted` 只删一个节点（工具调用恢复删旧子节点，
 *    后面的节点必须留着）；`truncated` 删「该节点及其后全部」，区间由本地按 `seq` 算。
 */

import { describe, expect, it, vi, beforeEach } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'

const mocks = vi.hoisted(() => ({
  subscribe: vi.fn(),
  readVdfs: vi.fn(),
  statVdfs: vi.fn(),
}))

vi.mock('@/services/eventBus', () => ({
  subscribe: mocks.subscribe,
  publishVdfsChangedLocal: vi.fn(),
  subscribeVdfsChanged: vi.fn(() => () => {}),
}))
vi.mock('@/services/vdfs', () => ({
  readVdfs: mocks.readVdfs,
  statVdfs: mocks.statVdfs,
  listVdfs: vi.fn(),
  writeVdfs: vi.fn(),
  writeVdfsBinary: vi.fn(),
  deleteVdfs: vi.fn(),
  moveVdfs: vi.fn(),
  runVdfsAction: vi.fn(),
  watchVdfs: vi.fn(),
  unwatchVdfs: vi.fn(),
  arrayBufferToBase64: vi.fn(),
  base64ToBytes: vi.fn(),
  downloadBlob: vi.fn(),
}))
vi.mock('@/services/session', () => ({
  listSessions: vi.fn(),
  deleteSession: vi.fn(),
  updateSession: vi.fn(),
  clearMessages: vi.fn(),
  deleteMessage: vi.fn(),
  updateMessage: vi.fn(),
}))
vi.mock('@/services/plugin', () => ({
  callPlugin: vi.fn(),
  setLastWorkdir: vi.fn(),
  getLastWorkdir: vi.fn(),
}))
vi.mock('@/utils/logger', () => ({
  logger: { error: vi.fn(), warn: vi.fn(), debug: vi.fn(), info: vi.fn() },
}))
// 地址方案是运行期数据（列目录认出来）。被测模块在事件回调里**同步**读它，
// 因此注入 getter 而不是 async 解析——这正好也覆盖了「方案已就绪」的正常路径。
vi.mock('@/services/vdfsScheme', () => ({
  vdfsSessionScheme: vi.fn(() => SCHEME),
  ensureSessionMountDir: vi.fn(async () => SCHEME.mountDir),
  ensureVdfsSessionScheme: vi.fn(async () => SCHEME),
}))

import { useSessionsStore } from '@/stores/sessions'
import {
  drain,
  messageFromNode,
  startTranscriptSync,
  stopTranscriptSync,
} from '../vdfsTranscriptSync'
import {
  VDFS_CHANGE_APPENDED,
  VDFS_CHANGE_CREATED,
  VDFS_CHANGE_DELETED,
  VDFS_CHANGE_TRUNCATED,
  VDFS_CHANGE_UPDATED,
  VDFS_EXT_MESSAGE,
  sessionRouteOf,
  vdfsMessageAddr,
  vdfsMessagesAddr,
  vdfsSessionAddr,
  type VdfsChange,
  type VdfsNode,
} from '@/schemas/vdfs'

const SID = 'abc'

/**
 * 协议夹具：地址方案是运行期数据（列目录认出来），单测不去列目录，直接注入。
 */
const { SCHEME } = vi.hoisted(() => ({
  SCHEME: { mountDir: '@vfs/session', messagesSeg: '消息' },
}))

/** 一条消息节点（后端 `message_node` 的形状：正文在内容里，结构在 attributes） */
function node(partial: Partial<VdfsNode> & { name: string }): VdfsNode {
  return {
    path: vdfsMessageAddr(SCHEME, SID, partial.name),
    title: '助手',
    kind: 'file',
    status: 'streaming',
    access: 'r',
    ext: VDFS_EXT_MESSAGE,
    role: 'assistant',
    type: 'text',
    seq: 1,
    ...partial,
  }
}

function deferred<T>() {
  let resolve!: (v: T) => void
  const promise = new Promise<T>((r) => {
    resolve = r
  })
  return { promise, resolve }
}

/** 取出 startTranscriptSync 注册到总线上的回调，并按真实信封投递一条变更 */
function emit(change: VdfsChange) {
  const calls = mocks.subscribe.mock.calls
  const call = calls[calls.length - 1]
  expect(call, 'startTranscriptSync 必须订阅 vdfs 变更频道').toBeTruthy()
  const handler = call![1] as (e: unknown) => void
  handler({ type: 'bus_event', data: { kind: 'vdfs', session_id: null, data: change } })
}

beforeEach(() => {
  vi.resetAllMocks()
  setActivePinia(createPinia())
  mocks.subscribe.mockReturnValue(() => {})
  stopTranscriptSync()
  // 落地目标显式注入（本模块是 service，不认识 Pinia；这里注入真实 store 实例）
  startTranscriptSync(useSessionsStore())
})

describe('sessionRouteOf（地址 → 本域目标：按地址分派，不按事件类型）', () => {
  it('会话叶子 / 转写列表 / 单条消息三级可解', () => {
    expect(sessionRouteOf(SCHEME, vdfsSessionAddr(SCHEME.mountDir, SID))).toEqual({ target: 'session', sessionId: SID })
    expect(sessionRouteOf(SCHEME, vdfsMessagesAddr(SCHEME, SID))).toEqual({ target: 'messages', sessionId: SID })
    expect(sessionRouteOf(SCHEME, vdfsMessageAddr(SCHEME, SID, 'm1'))).toEqual({
      target: 'message',
      sessionId: SID,
      messageId: 'm1',
    })
  })

  it('非会话域路径一律 null（清单 / 子会话 / 工作目录 / 其他资源）', () => {
    expect(sessionRouteOf(SCHEME, '@vfs/session')).toBeNull()
    expect(sessionRouteOf(SCHEME, '@vfs/session/abc/子会话/sub')).toBeNull()
    expect(sessionRouteOf(SCHEME, '@vfs/session/abc/工作目录/a.md')).toBeNull()
    expect(sessionRouteOf(SCHEME, '@vfs/model/p1')).toBeNull()
    expect(sessionRouteOf(SCHEME, '@vfs/session/abc/消息/m1/deeper')).toBeNull()
  })
})

describe('messageFromNode（结构取 attributes，正文取内容）', () => {
  it('role / type / parent_id / seq / error / meta 全部就位', () => {
    const msg = messageFromNode(
      node({
        name: 'm1',
        status: 'failed',
        parent_id: 't1',
        seq: 9,
        error: '上游 429',
        meta: { x: 1 },
        updated_at: 1_700_000_000_000,
      }),
      '正文',
    )
    expect(msg).toMatchObject({
      id: 'm1',
      content: '正文',
      role: 'assistant',
      type: 'text',
      parent_id: 't1',
      seq: 9,
      error: '上游 429',
      meta: { x: 1 },
      status: 'failed',
      timestamp: 1_700_000_000_000,
    })
  })

  it('消息状态**原样透传**；active 只作旧数据别名保留', () => {
    expect(messageFromNode(node({ name: 'm1', status: 'completed' }), '').status).toBe('completed')
    expect(messageFromNode(node({ name: 'm2', status: 'streaming' }), '').status).toBe('streaming')
    expect(messageFromNode(node({ name: 'm3', status: 'pending' }), '').status).toBe('pending')
    expect(messageFromNode(node({ name: 'm4', status: 'waiting_user_action' }), '').status).toBe(
      'waiting_user_action',
    )
    expect(messageFromNode(node({ name: 'm5', status: 'failed' }), '').status).toBe('failed')
    // 旧数据别名：历史上 completed 与「未标注」都被写成 active，遇到即按已结束处理
    expect(messageFromNode(node({ name: 'm6', status: 'active' }), '').status).toBe('completed')
  })

  /**
   * `aborted` 必须在状态词表里。
   *
   * 漏掉它的代价不是"少一个标签"，而是**整条状态被静默丢弃**：节点以"无状态"
   * 落进 store，被 `registry/messageTypes` 的缺省兜底成 `completed`——
   * 于是"用户按了停止"被渲染成"正常结束"，挂在 `aborted` 终态上的**重试入口消失**。
   * 实测症状正是"中止后看不到重试入口，重新打开会话才有"
   * （重开走叶子 JSON 直读，状态原样保留）。
   */
  it('aborted 是终态词之一，不得被丢弃（丢了就会谎报成 completed）', () => {
    expect(messageFromNode(node({ name: 't1', status: 'aborted', type: 'turn' }), '').status).toBe(
      'aborted',
    )
  })
})

describe('变更 → store', () => {
  it('created 带载荷：零回读写入整条消息', async () => {
    const store = useSessionsStore()
    emit({
      path: vdfsMessageAddr(SCHEME, SID, 'm1'),
      change: VDFS_CHANGE_CREATED,
      node: node({ name: 'm1' }),
      content: '你好',
    })
    await drain()

    expect(store.getSessionMessages(SID).map((m) => [m.id, m.content])).toEqual([['m1', '你好']])
    expect(mocks.readVdfs, '带载荷的 created 不得回读').not.toHaveBeenCalled()
    expect(mocks.statVdfs).not.toHaveBeenCalled()
  })

  it('created 未带载荷：回退 stat + read（通用消费端的降级路径）', async () => {
    const store = useSessionsStore()
    mocks.statVdfs.mockResolvedValueOnce(node({ name: 'm1' }))
    mocks.readVdfs.mockResolvedValueOnce({ path: '', text: '回读正文', binary: false, size: 12 })

    emit({ path: vdfsMessageAddr(SCHEME, SID, 'm1'), change: VDFS_CHANGE_CREATED })
    await drain()

    expect(store.getSessionMessages(SID).map((m) => m.content)).toEqual(['回读正文'])
  })

  it('appended 就地拼接，零回读（热路径）', async () => {
    const store = useSessionsStore()
    emit({
      path: vdfsMessageAddr(SCHEME, SID, 'm1'),
      change: VDFS_CHANGE_CREATED,
      node: node({ name: 'm1' }),
      content: '你好',
    })
    await drain()

    emit({ path: vdfsMessageAddr(SCHEME, SID, 'm1'), change: VDFS_CHANGE_APPENDED, delta: '，世界' })
    emit({ path: vdfsMessageAddr(SCHEME, SID, 'm1'), change: VDFS_CHANGE_APPENDED, delta: '！' })
    await drain()

    expect(store.getSessionMessages(SID).map((m) => m.content)).toEqual(['你好，世界！'])
    expect(mocks.readVdfs).not.toHaveBeenCalled()
  })

  it('逐路径串行：回读挂起期间的增量不得被旧快照覆盖', async () => {
    const store = useSessionsStore()
    // created 未带载荷 → 触发回读，且回读**挂起**
    const inflight = deferred<{ path: string; text: string; binary: boolean; size: number }>()
    mocks.statVdfs.mockResolvedValueOnce(node({ name: 'm1' }))
    mocks.readVdfs.mockReturnValueOnce(inflight.promise)

    emit({ path: vdfsMessageAddr(SCHEME, SID, 'm1'), change: VDFS_CHANGE_CREATED })
    // 回读还没回来，增量就到了
    emit({ path: vdfsMessageAddr(SCHEME, SID, 'm1'), change: VDFS_CHANGE_APPENDED, delta: 'X' })

    // 旧快照（不含 X）此刻才返回
    inflight.resolve({ path: '', text: 'abc', binary: false, size: 3 })
    await drain()

    expect(
      store.getSessionMessages(SID).map((m) => m.content),
      '增量必须在回读落地之后拼接，而不是被旧快照抹掉',
    ).toEqual(['abcX'])
  })

  it('updated 整条替换（状态迁移 / 全量替换）', async () => {
    const store = useSessionsStore()
    emit({
      path: vdfsMessageAddr(SCHEME, SID, 'm1'),
      change: VDFS_CHANGE_CREATED,
      node: node({ name: 'm1' }),
      content: '半截',
    })
    await drain()

    emit({
      path: vdfsMessageAddr(SCHEME, SID, 'm1'),
      change: VDFS_CHANGE_UPDATED,
      node: node({ name: 'm1', status: 'failed', error: '上游 429' }),
      content: '半截',
    })
    await drain()

    const [msg] = store.getSessionMessages(SID)
    expect(msg.status).toBe('failed')
    expect(msg.error).toBe('上游 429')
  })

  it('deleted 精确移除**单个**节点（工具调用恢复：删旧子节点，与顺序无关）', async () => {
    const store = useSessionsStore()
    for (const id of ['m1', 'm2']) {
      emit({
        path: vdfsMessageAddr(SCHEME, SID, id),
        change: VDFS_CHANGE_CREATED,
        node: node({ name: id, seq: id === 'm1' ? 1 : 2 }),
        content: id,
      })
    }
    await drain()
    expect(store.getSessionMessages(SID)).toHaveLength(2)

    // 删中间/靠前的节点**不得**连带删掉它之后的节点——工具调用恢复删的是
    // 子树里的旧子节点，后面还有父节点的最终回答。这正是 deleted 与 truncated
    // 必须分开的原因（曾经两者共用 deleted，前端无从分辨）。
    emit({ path: vdfsMessageAddr(SCHEME, SID, 'm1'), change: VDFS_CHANGE_DELETED })
    await drain()
    expect(store.getSessionMessages(SID).map((m) => m.id)).toEqual(['m2'])

    emit({ path: vdfsMessagesAddr(SCHEME, SID), change: VDFS_CHANGE_DELETED })
    await drain()
    expect(store.getSessionMessages(SID)).toHaveLength(0)
  })

  it('truncated 由本地按 seq 取区间：目标及其后全部移除，之前的一条不动', async () => {
    const store = useSessionsStore()
    for (const [id, seq] of [
      ['m1', 1],
      ['m2', 2],
      ['m3', 3],
    ] as const) {
      emit({
        path: vdfsMessageAddr(SCHEME, SID, id),
        change: VDFS_CHANGE_CREATED,
        node: node({ name: id, seq }),
        content: id,
      })
    }
    await drain()
    expect(store.getSessionMessages(SID)).toHaveLength(3)

    // **一条**变更即可收敛整段区间：后端不再逐条通知被删的每一个节点
    emit({ path: vdfsMessageAddr(SCHEME, SID, 'm2'), change: VDFS_CHANGE_TRUNCATED })
    await drain()

    expect(
      store.getSessionMessages(SID).map((m) => m.id),
      '截断范围由本地按 seq 算，不等后端把 m2/m3 各通知一遍',
    ).toEqual(['m1'])
  })

  it('truncated 锚点不在本地时什么都不删（不拿不存在的锚点截断整个列表）', async () => {
    const store = useSessionsStore()
    for (const [id, seq] of [
      ['m1', 1],
      ['m2', 2],
    ] as const) {
      emit({
        path: vdfsMessageAddr(SCHEME, SID, id),
        change: VDFS_CHANGE_CREATED,
        node: node({ name: id, seq }),
        content: id,
      })
    }
    await drain()

    emit({ path: vdfsMessageAddr(SCHEME, SID, '不存在'), change: VDFS_CHANGE_TRUNCATED })
    await drain()

    expect(store.getSessionMessages(SID).map((m) => m.id)).toEqual(['m1', 'm2'])
  })

  it('非转写路径的变更一律不碰 store', async () => {
    const store = useSessionsStore()
    emit({ path: '@vfs/session/abc/子会话/sub', change: VDFS_CHANGE_UPDATED, content: 'x' })
    emit({ path: '@vfs/session/abc', change: VDFS_CHANGE_UPDATED, content: 'x' })
    emit({ path: '@vfs/session', change: VDFS_CHANGE_DELETED })
    await drain()

    expect(store.getSessionMessages(SID)).toHaveLength(0)
    expect(mocks.readVdfs).not.toHaveBeenCalled()
  })

  it('会话级状态由权威消息派生（等待审批角标 / 活动文字）', async () => {
    const store = useSessionsStore()
    emit({
      path: vdfsMessageAddr(SCHEME, SID, 'p1'),
      change: VDFS_CHANGE_CREATED,
      node: node({ name: 'p1', status: 'waiting_user_action', type: 'user_prompt' }),
      content: '',
    })
    await drain()
    expect(store.getSessionStatus(SID).is_waiting_approval).toBe(true)
    expect(store.getSessionStatus(SID).activity).toBe('等待审批…')

    emit({
      path: vdfsMessageAddr(SCHEME, SID, 'p1'),
      change: VDFS_CHANGE_UPDATED,
      node: node({ name: 'p1', status: 'active', type: 'user_prompt' }),
      content: '',
    })
    await drain()
    expect(store.getSessionStatus(SID).is_waiting_approval).toBe(false)
  })
})
