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
  treeVdfs: vi.fn(),
  writeVdfs: vi.fn(),
  writeVdfsBinary: vi.fn(),
  deleteVdfs: vi.fn(),
  mkdirVdfs: vi.fn(),
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
  clearSession: vi.fn(),
  createSessionId: vi.fn(),
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

import { useSessionsStore } from '@/stores/sessions'
import {
  drain,
  messageFromNode,
  startTranscriptSync,
  stopTranscriptSync,
} from '../vdfsTranscriptSync'
import {
  VFDS_CHANGE_APPENDED,
  VFDS_CHANGE_CREATED,
  VFDS_CHANGE_DELETED,
  VFDS_CHANGE_UPDATED,
  VFDS_EXT_MESSAGE,
  parseTranscriptPath,
  vdfsMessageAddr,
  vdfsMessagesAddr,
  type VdfsChange,
  type VdfsNode,
} from '@/schemas/vdfs'

const SID = 'abc'

/** 一条消息节点（后端 `message_node` 的形状：正文在内容里，结构在 attributes） */
function node(partial: Partial<VdfsNode> & { name: string }): VdfsNode {
  return {
    path: vdfsMessageAddr(SID, partial.name),
    title: '助手',
    kind: 'file',
    status: 'streaming',
    access: 'r',
    ext: VFDS_EXT_MESSAGE,
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
  startTranscriptSync()
})

describe('parseTranscriptPath（地址的「解」与后端「拼」同源）', () => {
  it('转写列表 / 列表项两级可解', () => {
    expect(parseTranscriptPath(vdfsMessagesAddr(SID))).toEqual({ sessionId: SID })
    expect(parseTranscriptPath(vdfsMessageAddr(SID, 'm1'))).toEqual({
      sessionId: SID,
      messageId: 'm1',
    })
  })

  it('非转写路径一律 null（会话清单 / 会话叶子 / 子会话 / 工作目录）', () => {
    expect(parseTranscriptPath('.vdfs/session')).toBeNull()
    expect(parseTranscriptPath('.vdfs/session/abc')).toBeNull()
    expect(parseTranscriptPath('.vdfs/session/abc/子会话/sub')).toBeNull()
    expect(parseTranscriptPath('.vdfs/session/abc/工作目录/a.md')).toBeNull()
    expect(parseTranscriptPath('.vdfs/model/p1')).toBeNull()
    expect(parseTranscriptPath('.vdfs/session/abc/消息/m1/deeper')).toBeNull()
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

  it('节点状态词 active（= completed 或未标注）落到消息的 completed', () => {
    expect(messageFromNode(node({ name: 'm1', status: 'active' }), '').status).toBe('completed')
    expect(messageFromNode(node({ name: 'm2', status: 'streaming' }), '').status).toBe('streaming')
    expect(messageFromNode(node({ name: 'm3', status: 'waiting_user_action' }), '').status).toBe(
      'waiting_user_action',
    )
  })
})

describe('变更 → store', () => {
  it('created 带载荷：零回读写入整条消息', async () => {
    const store = useSessionsStore()
    emit({
      path: vdfsMessageAddr(SID, 'm1'),
      change: VFDS_CHANGE_CREATED,
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

    emit({ path: vdfsMessageAddr(SID, 'm1'), change: VFDS_CHANGE_CREATED })
    await drain()

    expect(store.getSessionMessages(SID).map((m) => m.content)).toEqual(['回读正文'])
  })

  it('appended 就地拼接，零回读（热路径）', async () => {
    const store = useSessionsStore()
    emit({
      path: vdfsMessageAddr(SID, 'm1'),
      change: VFDS_CHANGE_CREATED,
      node: node({ name: 'm1' }),
      content: '你好',
    })
    await drain()

    emit({ path: vdfsMessageAddr(SID, 'm1'), change: VFDS_CHANGE_APPENDED, delta: '，世界' })
    emit({ path: vdfsMessageAddr(SID, 'm1'), change: VFDS_CHANGE_APPENDED, delta: '！' })
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

    emit({ path: vdfsMessageAddr(SID, 'm1'), change: VFDS_CHANGE_CREATED })
    // 回读还没回来，增量就到了
    emit({ path: vdfsMessageAddr(SID, 'm1'), change: VFDS_CHANGE_APPENDED, delta: 'X' })

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
      path: vdfsMessageAddr(SID, 'm1'),
      change: VFDS_CHANGE_CREATED,
      node: node({ name: 'm1' }),
      content: '半截',
    })
    await drain()

    emit({
      path: vdfsMessageAddr(SID, 'm1'),
      change: VFDS_CHANGE_UPDATED,
      node: node({ name: 'm1', status: 'failed', error: '上游 429' }),
      content: '半截',
    })
    await drain()

    const [msg] = store.getSessionMessages(SID)
    expect(msg.status).toBe('failed')
    expect(msg.error).toBe('上游 429')
  })

  it('deleted 精确移除；落在 `消息` 目录上则清空整份转写', async () => {
    const store = useSessionsStore()
    for (const id of ['m1', 'm2']) {
      emit({
        path: vdfsMessageAddr(SID, id),
        change: VFDS_CHANGE_CREATED,
        node: node({ name: id, seq: id === 'm1' ? 1 : 2 }),
        content: id,
      })
    }
    await drain()
    expect(store.getSessionMessages(SID)).toHaveLength(2)

    emit({ path: vdfsMessageAddr(SID, 'm1'), change: VFDS_CHANGE_DELETED })
    await drain()
    expect(store.getSessionMessages(SID).map((m) => m.id)).toEqual(['m2'])

    emit({ path: vdfsMessagesAddr(SID), change: VFDS_CHANGE_DELETED })
    await drain()
    expect(store.getSessionMessages(SID)).toHaveLength(0)
  })

  it('非转写路径的变更一律不碰 store', async () => {
    const store = useSessionsStore()
    emit({ path: '.vdfs/session/abc/子会话/sub', change: VFDS_CHANGE_UPDATED, content: 'x' })
    emit({ path: '.vdfs/session/abc', change: VFDS_CHANGE_UPDATED, content: 'x' })
    emit({ path: '.vdfs/session', change: VFDS_CHANGE_DELETED })
    await drain()

    expect(store.getSessionMessages(SID)).toHaveLength(0)
    expect(mocks.readVdfs).not.toHaveBeenCalled()
  })

  it('会话级状态由权威消息派生（等待审批角标 / 活动文字）', async () => {
    const store = useSessionsStore()
    emit({
      path: vdfsMessageAddr(SID, 'p1'),
      change: VFDS_CHANGE_CREATED,
      node: node({ name: 'p1', status: 'waiting_user_action', type: 'user_prompt' }),
      content: '',
    })
    await drain()
    expect(store.getSessionStatus(SID).is_waiting_approval).toBe(true)
    expect(store.getSessionStatus(SID).activity).toBe('等待审批…')

    emit({
      path: vdfsMessageAddr(SID, 'p1'),
      change: VFDS_CHANGE_UPDATED,
      node: node({ name: 'p1', status: 'active', type: 'user_prompt' }),
      content: '',
    })
    await drain()
    expect(store.getSessionStatus(SID).is_waiting_approval).toBe(false)
  })
})
